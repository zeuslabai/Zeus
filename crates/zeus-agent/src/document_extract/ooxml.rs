//! OOXML text extraction: .docx / .pptx / .xlsx (.xlsm).
//!
//! OOXML files are a ZIP archive of XML parts. This module unzips the
//! relevant parts (in-memory, via `Cursor`) and extracts their text,
//! pure-Rust + cross-platform.
//!
//! - docx → `word/document.xml` (concatenate `<w:t>` runs)
//! - pptx → `ppt/slides/slide*.xml` (concatenate `<a:t>` runs, per slide)
//! - xlsx → delegated to `calamine` (handles sharedStrings indirection)

use std::io::{Cursor, Read};
use zeus_core::{tool_err, Result};

/// Extract text from an OOXML document given its extension and raw bytes.
pub fn extract(ext: &str, bytes: &[u8]) -> Result<String> {
    match ext {
        "docx" => extract_docx(bytes),
        "pptx" => extract_pptx(bytes),
        "xlsx" | "xlsm" => extract_xlsx(bytes),
        other => Err(tool_err!(tool, "ooxml: unsupported extension '{}'", other)),
    }
}

type ZipMem<'a> = zip::ZipArchive<Cursor<&'a [u8]>>;

fn open_zip(bytes: &[u8]) -> Result<ZipMem<'_>> {
    zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| tool_err!(tool, "not a valid OOXML (zip) container: {}", e))
}

fn open_part(archive: &mut ZipMem<'_>, name: &str) -> Result<Vec<u8>> {
    let mut file = archive
        .by_name(name)
        .map_err(|e| tool_err!(tool, "missing OOXML part '{}': {}", name, e))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .map_err(|e| tool_err!(tool, "failed reading OOXML part '{}': {}", name, e))?;
    Ok(buf)
}

fn extract_docx(bytes: &[u8]) -> Result<String> {
    let mut archive = open_zip(bytes)?;
    let xml = open_part(&mut archive, "word/document.xml")?;
    let text = extract_text_runs(&xml, "t")?;
    Ok(normalize(&text))
}

fn extract_pptx(bytes: &[u8]) -> Result<String> {
    let mut archive = open_zip(bytes)?;

    // Collect slide part names first (sorted for stable slide order), then
    // extract — can't hold an immutable borrow while mutably reading.
    let mut slide_names: Vec<String> = archive
        .file_names()
        .filter(|n| n.starts_with("ppt/slides/slide") && n.ends_with(".xml"))
        .map(|n| n.to_string())
        .collect();
    slide_names.sort_by(|a, b| natural_slide_cmp(a, b));

    let mut out = String::new();
    for (i, name) in slide_names.iter().enumerate() {
        let xml = open_part(&mut archive, name)?;
        let text = extract_text_runs(&xml, "t")?;
        let text = normalize(&text);
        if !text.is_empty() {
            if i > 0 {
                out.push_str("\n\n");
            }
            out.push_str(&format!("--- Slide {} ---\n", i + 1));
            out.push_str(&text);
        }
    }
    Ok(out)
}

fn extract_xlsx(bytes: &[u8]) -> Result<String> {
    use calamine::{open_workbook_auto_from_rs, Data, Reader};

    let cursor = Cursor::new(bytes.to_vec());
    let mut workbook = open_workbook_auto_from_rs(cursor)
        .map_err(|e| tool_err!(tool, "opening xlsx: {}", e))?;
    let mut out = String::new();
    let sheet_names = workbook.sheet_names().to_owned();

    for name in sheet_names {
        let range = match workbook.worksheet_range(&name) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if range.is_empty() {
            continue;
        }
        out.push_str(&format!("--- Sheet: {name} ---\n"));
        for row in range.rows() {
            let cells: Vec<String> = row
                .iter()
                .map(|c| match c {
                    Data::Empty => String::new(),
                    Data::String(s) => s.clone(),
                    Data::Float(f) => format_num(*f),
                    Data::Int(i) => i.to_string(),
                    Data::Bool(b) => b.to_string(),
                    Data::DateTime(d) => d.to_string(),
                    other => other.to_string(),
                })
                .collect();
            out.push_str(&cells.join("\t"));
            out.push('\n');
        }
        out.push('\n');
    }
    Ok(out.trim_end().to_string())
}

/// Extract text from a single XML part: concatenate the text content of every
/// element whose local name matches `tag` (e.g. "t" for `<w:t>` / `<a:t>`).
/// Inserts a space between runs so words don't fuse together.
fn extract_text_runs(xml: &[u8], tag: &str) -> Result<String> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);

    let mut out = String::new();
    let mut in_target = false;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                if local_name_is(e.name().as_ref(), tag) {
                    in_target = true;
                }
            }
            Ok(Event::End(e)) => {
                if local_name_is(e.name().as_ref(), tag) {
                    in_target = false;
                    out.push(' ');
                }
            }
            Ok(Event::Text(e)) if in_target => {
                let txt = e.unescape().unwrap_or_default();
                out.push_str(&txt);
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(tool_err!(tool, "XML parse error: {}", e)),
            _ => {}
        }
        buf.clear();
    }

    Ok(out)
}

/// Match an XML element's local name ignoring its namespace prefix.
/// e.g. `w:t` and `a:t` both match tag `t`.
fn local_name_is(name: &[u8], tag: &str) -> bool {
    let local = match name.iter().rposition(|&b| b == b':') {
        Some(i) => &name[i + 1..],
        None => name,
    };
    local == tag.as_bytes()
}

/// Render a float without a trailing `.0` when it's integral.
fn format_num(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() < 1e15 {
        format!("{}", f as i64)
    } else {
        format!("{f}")
    }
}

/// Compare slide part names numerically (slide2 before slide10).
fn natural_slide_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let num = |s: &str| -> u64 {
        s.trim_start_matches("ppt/slides/slide")
            .trim_end_matches(".xml")
            .parse()
            .unwrap_or(u64::MAX)
    };
    num(a).cmp(&num(b))
}

/// Collapse runs of whitespace introduced by run-boundary spacing, and trim.
fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = false;
    for ch in s.chars() {
        if ch == ' ' || ch == '\t' {
            if !last_space {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(ch);
            last_space = false;
        }
    }
    out.trim().to_string()
}
