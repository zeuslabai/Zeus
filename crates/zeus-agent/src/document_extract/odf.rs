//! OpenDocument / ebook / RTF text extraction (pure-Rust, cross-platform).
//!
//! Covers five extensions routed here by `mod.rs`:
//! - odt/ods/odp → zip (`Cursor`) → `content.xml` → quick-xml event-strip
//! - epub → zip → `META-INF/container.xml` → OPF spine → concat XHTML in order
//! - rtf → pure byte-walk control-word stripper (no zip)
//!
//! Contract: `pub fn extract(ext: &str, bytes: &[u8]) -> Result<String>`, sync,
//! bytes-only beyond the routing `ext`. Mirrors the `Cursor`-zip primitive that
//! `ooxml.rs` uses; helpers are self-contained so the two modules stay
//! independent. Reference impl by zeus-freebsd, filled + gate-verified here.

use std::io::{Cursor, Read};
use zeus_core::{tool_err, Result};

type ZipMem<'a> = zip::ZipArchive<Cursor<&'a [u8]>>;

/// Extract text from an OpenDocument / epub / rtf file given its lowercased
/// extension and raw bytes.
pub fn extract(ext: &str, bytes: &[u8]) -> Result<String> {
    match ext {
        "odt" | "ods" | "odp" => extract_odf(bytes),
        "epub" => extract_epub(bytes),
        "rtf" => strip_rtf(bytes),
        other => Err(tool_err!(tool, "odf: unsupported extension '{}'", other)),
    }
}

fn open_zip(bytes: &[u8]) -> Result<ZipMem<'_>> {
    zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| tool_err!(tool, "not a valid zip container: {}", e))
}

fn open_part(archive: &mut ZipMem<'_>, name: &str) -> Result<Vec<u8>> {
    let mut file = archive
        .by_name(name)
        .map_err(|e| tool_err!(tool, "missing part '{}': {}", name, e))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .map_err(|e| tool_err!(tool, "failed reading part '{}': {}", name, e))?;
    Ok(buf)
}

// ---------------------------------------------------------------------------
// OpenDocument (odt/ods/odp): single `content.xml`, strip all text nodes.
// ---------------------------------------------------------------------------

fn extract_odf(bytes: &[u8]) -> Result<String> {
    let mut archive = open_zip(bytes)?;
    let xml = open_part(&mut archive, "content.xml")?;
    let text = strip_xml_text(&xml, &["text:p", "text:h", "text:span", "text:tab"])?;
    Ok(normalize(&text))
}

// ---------------------------------------------------------------------------
// EPUB: zip → META-INF/container.xml → OPF rootfile → spine order → XHTML.
// ---------------------------------------------------------------------------

fn extract_epub(bytes: &[u8]) -> Result<String> {
    let mut archive = open_zip(bytes)?;

    // 1. Locate the OPF package file via the container manifest.
    let container = open_part(&mut archive, "META-INF/container.xml")?;
    let opf_path = find_opf_path(&container)?;

    // 2. Parse the OPF: manifest (id → href) + spine (ordered idrefs).
    let opf = open_part(&mut archive, &opf_path)?;
    let (manifest, spine) = parse_opf(&opf)?;

    // OPF hrefs are relative to the OPF's own directory.
    let base = opf_path
        .rfind('/')
        .map(|i| &opf_path[..=i])
        .unwrap_or("");

    // 3. Concatenate the text of each spine item, in spine order.
    let mut out = String::new();
    for idref in &spine {
        let Some(href) = manifest.get(idref) else {
            continue;
        };
        let full = format!("{base}{href}");
        // Tolerate a missing/odd item rather than failing the whole book.
        let Ok(xhtml) = open_part(&mut archive, &full) else {
            continue;
        };
        let text = strip_xml_text(&xhtml, &["p", "h1", "h2", "h3", "h4", "h5", "h6", "li", "td", "th", "div", "span", "br"])?;
        let text = normalize(&text);
        if !text.is_empty() {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(&text);
        }
    }

    if out.is_empty() {
        return Err(tool_err!(tool, "epub: no readable text in spine"));
    }
    Ok(out)
}

/// Pull the first `<rootfile full-path="...">` from META-INF/container.xml.
fn find_opf_path(container: &[u8]) -> Result<String> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_reader(container);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(e)) | Ok(Event::Start(e)) if local_name_is(e.name().as_ref(), "rootfile") => {
                for attr in e.attributes().flatten() {
                    if attr.key.as_ref() == b"full-path" {
                        let v = attr.unescape_value().unwrap_or_default();
                        return Ok(v.into_owned());
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(tool_err!(tool, "epub container parse error: {}", e)),
            _ => {}
        }
        buf.clear();
    }
    Err(tool_err!(tool, "epub: no rootfile in container.xml"))
}

/// Parse the OPF: returns (manifest id→href, ordered spine idrefs).
fn parse_opf(opf: &[u8]) -> Result<(std::collections::HashMap<String, String>, Vec<String>)> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_reader(opf);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();

    let mut manifest = std::collections::HashMap::new();
    let mut spine = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(e)) | Ok(Event::Start(e)) => {
                let name = e.name();
                if local_name_is(name.as_ref(), "item") {
                    let mut id = None;
                    let mut href = None;
                    for attr in e.attributes().flatten() {
                        match attr.key.as_ref() {
                            b"id" => id = Some(attr.unescape_value().unwrap_or_default().into_owned()),
                            b"href" => href = Some(attr.unescape_value().unwrap_or_default().into_owned()),
                            _ => {}
                        }
                    }
                    if let (Some(id), Some(href)) = (id, href) {
                        manifest.insert(id, href);
                    }
                } else if local_name_is(name.as_ref(), "itemref") {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"idref" {
                            spine.push(attr.unescape_value().unwrap_or_default().into_owned());
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(tool_err!(tool, "epub OPF parse error: {}", e)),
            _ => {}
        }
        buf.clear();
    }
    Ok((manifest, spine))
}

// ---------------------------------------------------------------------------
// RTF: pure byte-walk control-word stripper (no zip, no XML).
// ---------------------------------------------------------------------------

fn strip_rtf(bytes: &[u8]) -> Result<String> {
    // Quick sanity check — RTF starts with "{\rtf".
    if !bytes.starts_with(b"{\\rtf") {
        return Err(tool_err!(tool, "rtf: not an RTF document (missing {{\\rtf header)"));
    }

    let mut out = String::new();
    let mut i = 0;
    let n = bytes.len();
    let mut skip_group_depth: Option<usize> = None;
    let mut group_depth: usize = 0;

    while i < n {
        let c = bytes[i];
        match c {
            b'{' => {
                group_depth += 1;
                i += 1;
            }
            b'}' => {
                if let Some(d) = skip_group_depth {
                    if group_depth == d {
                        skip_group_depth = None;
                    }
                }
                group_depth = group_depth.saturating_sub(1);
                i += 1;
            }
            b'\\' => {
                // Control word, control symbol, or escaped char.
                if i + 1 < n {
                    let next = bytes[i + 1];
                    if next == b'\'' && i + 3 < n {
                        // \'hh hex-escaped byte (Latin-1-ish); emit if not skipping.
                        let hi = hex_val(bytes[i + 2]);
                        let lo = hex_val(bytes[i + 3]);
                        if let (Some(hi), Some(lo)) = (hi, lo) {
                            if skip_group_depth.is_none() {
                                let byte = (hi << 4) | lo;
                                out.push(byte as char);
                            }
                            i += 4;
                            continue;
                        }
                        i += 2;
                    } else if next.is_ascii_alphabetic() {
                        // Control word: read letters, optional numeric arg, optional trailing space.
                        let start = i + 1;
                        let mut j = start;
                        while j < n && bytes[j].is_ascii_alphabetic() {
                            j += 1;
                        }
                        let word = &bytes[start..j];
                        // numeric parameter
                        let mut had_minus = false;
                        if j < n && bytes[j] == b'-' {
                            had_minus = true;
                            j += 1;
                        }
                        let _ = had_minus;
                        while j < n && bytes[j].is_ascii_digit() {
                            j += 1;
                        }
                        // a single trailing space is part of the control word
                        if j < n && bytes[j] == b' ' {
                            j += 1;
                        }
                        handle_control_word(word, group_depth, &mut skip_group_depth, &mut out);
                        i = j;
                    } else {
                        // Control symbol: \\ \{ \} etc — emit the literal char if escaping a real char.
                        if skip_group_depth.is_none() {
                            match next {
                                b'\\' | b'{' | b'}' => out.push(next as char),
                                b'~' => out.push(' '),     // non-breaking space
                                b'-' => {}                  // optional hyphen
                                b'_' => out.push('-'),      // non-breaking hyphen
                                _ => {}
                            }
                        }
                        i += 2;
                    }
                } else {
                    i += 1;
                }
            }
            b'\r' | b'\n' => {
                // Raw line breaks in RTF source are not content.
                i += 1;
            }
            _ => {
                if skip_group_depth.is_none() {
                    out.push(c as char);
                }
                i += 1;
            }
        }
    }

    Ok(normalize(&out))
}

/// React to known control words: emit whitespace for breaks, mark
/// non-content groups (fonttbl, colortbl, etc.) to be skipped.
fn handle_control_word(
    word: &[u8],
    group_depth: usize,
    skip_group_depth: &mut Option<usize>,
    out: &mut String,
) {
    match word {
        b"par" | b"line" | b"sect" | b"page" => {
            if skip_group_depth.is_none() {
                out.push('\n');
            }
        }
        b"tab" => {
            if skip_group_depth.is_none() {
                out.push('\t');
            }
        }
        // Destinations whose contents are not body text — skip the group.
        b"fonttbl" | b"colortbl" | b"stylesheet" | b"info" | b"pict" | b"object"
        | b"header" | b"footer" | b"footnote" | b"generator" | b"datafield"
        | b"themedata" | b"colorschememapping" | b"latentstyles" | b"rsidtbl" => {
            if skip_group_depth.is_none() {
                *skip_group_depth = Some(group_depth);
            }
        }
        _ => {}
    }
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Shared XML/text helpers (self-contained; parallel to ooxml's versions).
// ---------------------------------------------------------------------------

/// Strip text content from an XML document. `block_tags` are element local
/// names after whose close we insert a space, so word/paragraph boundaries
/// don't get glued together.
fn strip_xml_text(xml: &[u8], block_tags: &[&str]) -> Result<String> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);

    let mut out = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Text(e)) => {
                let txt = e.unescape().unwrap_or_default();
                out.push_str(&txt);
            }
            Ok(Event::End(e)) => {
                if block_tags
                    .iter()
                    .any(|t| local_name_is(e.name().as_ref(), t))
                {
                    out.push(' ');
                }
            }
            Ok(Event::Empty(e)) => {
                // self-closing block element (e.g. <br/>) → boundary space
                if block_tags
                    .iter()
                    .any(|t| local_name_is(e.name().as_ref(), t))
                {
                    out.push(' ');
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(tool_err!(tool, "XML parse error: {}", e)),
            _ => {}
        }
        buf.clear();
    }

    Ok(out)
}

/// True if `name`'s local part (after any `ns:` prefix) equals `tag`.
fn local_name_is(name: &[u8], tag: &str) -> bool {
    let local = match name.iter().rposition(|&b| b == b':') {
        Some(i) => &name[i + 1..],
        None => name,
    };
    local == tag.as_bytes()
}

/// Collapse runs of whitespace and trim — same shape as ooxml's normalize,
/// but preserves single newlines (RTF/epub paragraph breaks).
fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_was_space = false;
    let mut last_was_newline = false;
    for ch in s.chars() {
        match ch {
            ' ' | '\t' => {
                if !last_was_space && !last_was_newline {
                    out.push(' ');
                }
                last_was_space = true;
            }
            '\n' => {
                // collapse multiple blank lines to at most one blank line
                if !last_was_newline {
                    // trim trailing space before newline
                    if out.ends_with(' ') {
                        out.pop();
                    }
                    out.push('\n');
                }
                last_was_newline = true;
                last_was_space = false;
            }
            _ => {
                out.push(ch);
                last_was_space = false;
                last_was_newline = false;
            }
        }
    }
    out.trim().to_string()
}
