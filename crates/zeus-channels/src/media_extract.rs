//! Document text extraction for binary office formats.
//!
//! This module extracts plain text from binary document containers that the
//! LLM cannot read directly (DOCX, PPTX, XLSX). The extracted text is then
//! injected into the user-message prompt prefix via
//! [`zeus_api::inbound::process_attachments`] — the same path used by text/\*
//! attachments.
//!
//! ## Filetype model (Cut B, P0 dispatch)
//!
//! - **HTML / MD / plain text** → pass through as `text/*` via Cut A's MIME
//!   detection. NOT handled here — the LLM reads raw HTML / Markdown natively
//!   (mirrors Claude Code's `Read` tool semantics).
//! - **DOCX / PPTX / XLSX** → extracted here. Returns structured plain text
//!   that mirrors the visible content (paragraphs / slide text / cell values).
//! - **PDF** → already handled by the vision multimodal pipeline. Deferred.
//!
//! ## Failure semantics
//!
//! All extractors return `Result<String, ExtractError>`. On error, the caller
//! is expected to **fail soft** — log a warning and skip the attachment —
//! matching the audio-transcript failure pattern in `process_attachments`.

use std::io::{Cursor, Read};

use thiserror::Error;

/// MIME types this module knows how to extract from.
pub const MIME_DOCX: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
pub const MIME_PPTX: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.presentation";
pub const MIME_XLSX: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
pub const MIME_XLS: &str = "application/vnd.ms-excel";

/// Returns `true` if `mime` is a binary document type we can extract text from.
///
/// Used by the inbound attachment pipeline to gate the extractable-doc branch.
pub fn is_extractable_doc(mime: &str) -> bool {
    matches!(mime, MIME_DOCX | MIME_PPTX | MIME_XLSX | MIME_XLS)
}

/// Errors that can occur during document text extraction.
#[derive(Debug, Error)]
pub enum ExtractError {
    #[error("unsupported MIME type for extraction: {0}")]
    UnsupportedMime(String),

    #[error("zip archive error: {0}")]
    Zip(String),

    #[error("XML parse error: {0}")]
    Xml(String),

    #[error("spreadsheet parse error: {0}")]
    Spreadsheet(String),

    #[error("io error: {0}")]
    Io(String),
}

/// Extract plain text from a binary document.
///
/// Dispatches to the appropriate extractor based on `mime`. Returns
/// human-readable plain text suitable for direct injection into an LLM prompt.
///
/// ## Output format
///
/// - **DOCX**: paragraphs separated by `\n`, in document order.
/// - **PPTX**: each slide's text grouped, slides separated by `\n\n--- Slide N ---\n`.
/// - **XLSX/XLS**: each sheet's data as tab-separated rows, sheets separated by
///   `\n\n--- Sheet: <name> ---\n`. Empty cells render as empty fields.
pub fn extract_text_from_doc(data: &[u8], mime: &str) -> Result<String, ExtractError> {
    match mime {
        MIME_DOCX => extract_docx(data),
        MIME_PPTX => extract_pptx(data),
        MIME_XLSX | MIME_XLS => extract_xlsx(data),
        other => Err(ExtractError::UnsupportedMime(other.to_string())),
    }
}

// ---------------------------------------------------------------------------
// DOCX
// ---------------------------------------------------------------------------

/// DOCX = ZIP container with `word/document.xml` holding the body.
/// We parse `<w:t>` text elements in document order.
fn extract_docx(data: &[u8]) -> Result<String, ExtractError> {
    let xml = read_zip_entry(data, "word/document.xml")?;
    parse_ooxml_text(&xml, b"w:t", Some(b"w:p"))
}

// ---------------------------------------------------------------------------
// PPTX
// ---------------------------------------------------------------------------

/// PPTX = ZIP container with `ppt/slides/slide<N>.xml` files.
/// We enumerate slides in numeric order and parse `<a:t>` text elements per slide.
fn extract_pptx(data: &[u8]) -> Result<String, ExtractError> {
    let cursor = Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| ExtractError::Zip(format!("open archive: {}", e)))?;

    // Collect slide entries (ppt/slides/slideN.xml), sorted by slide number.
    let mut slide_names: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| ExtractError::Zip(format!("read entry: {}", e)))?;
        let name = entry.name().to_string();
        if name.starts_with("ppt/slides/slide") && name.ends_with(".xml") {
            slide_names.push(name);
        }
    }
    // Sort by slide number, not lexicographic — slide2.xml before slide10.xml.
    slide_names.sort_by_key(|n| {
        n.strip_prefix("ppt/slides/slide")
            .and_then(|s| s.strip_suffix(".xml"))
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(u32::MAX)
    });

    let mut out = String::new();
    for (idx, name) in slide_names.iter().enumerate() {
        let mut xml = String::new();
        archive
            .by_name(name)
            .map_err(|e| ExtractError::Zip(format!("read {}: {}", name, e)))?
            .read_to_string(&mut xml)
            .map_err(|e| ExtractError::Io(format!("read {}: {}", name, e)))?;
        let slide_text = parse_ooxml_text(&xml, b"a:t", None)?;
        if !slide_text.trim().is_empty() {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(&format!("--- Slide {} ---\n", idx + 1));
            out.push_str(&slide_text);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// XLSX / XLS
// ---------------------------------------------------------------------------

/// XLSX / XLS extracted via the `calamine` crate. Each sheet rendered as
/// tab-separated rows, sheets separated by a header.
fn extract_xlsx(data: &[u8]) -> Result<String, ExtractError> {
    use calamine::{Data, Reader};

    let cursor = Cursor::new(data.to_vec());
    let mut workbook = calamine::open_workbook_auto_from_rs(cursor)
        .map_err(|e| ExtractError::Spreadsheet(format!("open workbook: {}", e)))?;

    let sheet_names: Vec<String> = workbook.sheet_names().to_vec();
    let mut out = String::new();

    for name in sheet_names {
        let range = workbook
            .worksheet_range(&name)
            .map_err(|e| ExtractError::Spreadsheet(format!("read sheet '{}': {}", name, e)))?;

        if range.is_empty() {
            continue;
        }

        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&format!("--- Sheet: {} ---\n", name));

        for row in range.rows() {
            let cells: Vec<String> = row
                .iter()
                .map(|cell| match cell {
                    Data::Empty => String::new(),
                    Data::String(s) => s.clone(),
                    Data::Float(f) => {
                        // Render integers without trailing ".0" for cleaner output.
                        if f.fract() == 0.0 && f.abs() < 1e15 {
                            format!("{}", *f as i64)
                        } else {
                            format!("{}", f)
                        }
                    }
                    Data::Int(i) => format!("{}", i),
                    Data::Bool(b) => format!("{}", b),
                    Data::DateTime(dt) => format!("{}", dt),
                    Data::DateTimeIso(s) => s.clone(),
                    Data::DurationIso(s) => s.clone(),
                    Data::Error(e) => format!("#ERR({:?})", e),
                })
                .collect();
            out.push_str(&cells.join("\t"));
            out.push('\n');
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Read a single entry from a ZIP archive as a UTF-8 string.
fn read_zip_entry(data: &[u8], name: &str) -> Result<String, ExtractError> {
    let cursor = Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| ExtractError::Zip(format!("open archive: {}", e)))?;
    let mut entry = archive
        .by_name(name)
        .map_err(|e| ExtractError::Zip(format!("read {}: {}", name, e)))?;
    let mut out = String::new();
    entry
        .read_to_string(&mut out)
        .map_err(|e| ExtractError::Io(format!("read {}: {}", name, e)))?;
    Ok(out)
}

/// Parse OOXML, extracting text content of every `<text_tag>` element.
///
/// If `para_tag` is `Some`, a newline is appended at the end of each
/// occurrence of that tag (so DOCX paragraphs render as separate lines).
fn parse_ooxml_text(
    xml: &str,
    text_tag: &[u8],
    para_tag: Option<&[u8]>,
) -> Result<String, ExtractError> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut out = String::new();
    let mut in_text = false;
    let mut buf = Vec::new();

    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| ExtractError::Xml(format!("{}", e)))?
        {
            Event::Start(ref e) if e.name().as_ref() == text_tag => {
                in_text = true;
            }
            Event::End(ref e) if e.name().as_ref() == text_tag => {
                in_text = false;
            }
            Event::End(ref e) if Some(e.name().as_ref()) == para_tag => {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            Event::Text(t) if in_text => {
                let s = t
                    .unescape()
                    .map_err(|e| ExtractError::Xml(format!("unescape: {}", e)))?;
                out.push_str(&s);
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_extractable_doc_recognises_office_mimes() {
        assert!(is_extractable_doc(MIME_DOCX));
        assert!(is_extractable_doc(MIME_PPTX));
        assert!(is_extractable_doc(MIME_XLSX));
        assert!(is_extractable_doc(MIME_XLS));
        assert!(!is_extractable_doc("text/html"));
        assert!(!is_extractable_doc("application/pdf"));
        assert!(!is_extractable_doc("application/octet-stream"));
    }

    #[test]
    fn extract_text_from_doc_rejects_unsupported_mime() {
        let err = extract_text_from_doc(b"junk", "image/png").unwrap_err();
        assert!(matches!(err, ExtractError::UnsupportedMime(_)));
    }

    #[test]
    fn extract_text_from_doc_rejects_corrupt_zip() {
        let err = extract_text_from_doc(b"not a zip", MIME_DOCX).unwrap_err();
        assert!(matches!(err, ExtractError::Zip(_)));
    }

    /// Build a minimal valid DOCX in-memory: zip with `word/document.xml`.
    fn build_minimal_docx(paragraphs: &[&str]) -> Vec<u8> {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        use zip::ZipWriter;

        let mut buf = Vec::new();
        {
            let cursor = Cursor::new(&mut buf);
            let mut zw = ZipWriter::new(cursor);
            zw.start_file("word/document.xml", SimpleFileOptions::default())
                .unwrap();
            let mut xml = String::from(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:body>"#,
            );
            for p in paragraphs {
                xml.push_str(&format!("<w:p><w:r><w:t>{}</w:t></w:r></w:p>", p));
            }
            xml.push_str("</w:body></w:document>");
            zw.write_all(xml.as_bytes()).unwrap();
            zw.finish().unwrap();
        }
        buf
    }

    #[test]
    fn extract_docx_reads_paragraph_text() {
        let docx = build_minimal_docx(&["Hello world", "Second paragraph"]);
        let text = extract_text_from_doc(&docx, MIME_DOCX).unwrap();
        assert!(text.contains("Hello world"), "got: {:?}", text);
        assert!(text.contains("Second paragraph"), "got: {:?}", text);
        // Paragraphs should be on separate lines.
        let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
        assert!(lines.len() >= 2, "expected ≥2 non-empty lines, got: {:?}", lines);
    }

    #[test]
    fn extract_docx_handles_empty_body() {
        let docx = build_minimal_docx(&[]);
        let text = extract_text_from_doc(&docx, MIME_DOCX).unwrap();
        assert_eq!(text.trim(), "");
    }

    /// Build a minimal PPTX with N slides, each with a single text run.
    fn build_minimal_pptx(slide_texts: &[&str]) -> Vec<u8> {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        use zip::ZipWriter;

        let mut buf = Vec::new();
        {
            let cursor = Cursor::new(&mut buf);
            let mut zw = ZipWriter::new(cursor);
            for (i, text) in slide_texts.iter().enumerate() {
                let name = format!("ppt/slides/slide{}.xml", i + 1);
                zw.start_file(&name, SimpleFileOptions::default()).unwrap();
                let xml = format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"
       xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
<p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>{}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld>
</p:sld>"#,
                    text
                );
                zw.write_all(xml.as_bytes()).unwrap();
            }
            zw.finish().unwrap();
        }
        buf
    }

    #[test]
    fn extract_pptx_reads_slide_text_in_order() {
        let pptx = build_minimal_pptx(&["Slide one content", "Slide two content"]);
        let text = extract_text_from_doc(&pptx, MIME_PPTX).unwrap();
        assert!(text.contains("Slide one content"), "got: {:?}", text);
        assert!(text.contains("Slide two content"), "got: {:?}", text);
        assert!(text.contains("--- Slide 1 ---"), "got: {:?}", text);
        assert!(text.contains("--- Slide 2 ---"), "got: {:?}", text);
        // Order check: slide 1 must appear before slide 2.
        let p1 = text.find("Slide one").unwrap();
        let p2 = text.find("Slide two").unwrap();
        assert!(p1 < p2);
    }

    #[test]
    fn extract_pptx_sorts_slides_numerically_not_lexicographically() {
        // Build 11 slides — naive lex sort would put slide10 before slide2.
        let texts: Vec<String> = (1..=11).map(|i| format!("S{}content", i)).collect();
        let texts_ref: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        let pptx = build_minimal_pptx(&texts_ref);
        let text = extract_text_from_doc(&pptx, MIME_PPTX).unwrap();
        let p2 = text.find("S2content").unwrap();
        let p10 = text.find("S10content").unwrap();
        assert!(p2 < p10, "slide 2 should come before slide 10");
    }

    #[test]
    fn extract_xlsx_reads_cell_values() {
        // calamine can't easily round-trip an in-memory xlsx without a real
        // file builder. Use a tiny fixture committed to the repo instead.
        let fixture =
            include_bytes!("../tests/fixtures/sample.xlsx");
        let text = extract_text_from_doc(fixture, MIME_XLSX).unwrap();
        assert!(text.contains("--- Sheet:"), "got: {:?}", text);
        assert!(text.contains("Hello"), "got: {:?}", text);
        assert!(text.contains("42"), "got: {:?}", text);
    }
}
