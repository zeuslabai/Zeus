//! PDF text extraction (pure-Rust, cross-platform).
//!
//! Owned by zeus107 (#126 commit ②). The existing `zeus-talos` PDF path
//! shells out to python3/macOS AppleScript and returns "only available on
//! macOS" off-platform; this module replaces it with `lopdf` so extraction
//! works on FreeBSD/Linux seats too.
//!
//! Locked contract: `pub fn extract(bytes: &[u8]) -> Result<String>`, sync.

use zeus_core::{tool_err, Result};

/// Extract all text from a PDF byte buffer.
///
/// Returns concatenated page text in page order. A structurally-valid PDF
/// with no extractable text (e.g. a pure scan) yields `Ok("")` rather than an
/// error — the caller decides whether to fall back to OCR
/// (media_understanding). A corrupt / non-PDF buffer surfaces an `Err`.
pub fn extract(bytes: &[u8]) -> Result<String> {
    let doc = lopdf::Document::load_mem(bytes)
        .map_err(|e| tool_err!(tool, "failed to parse PDF (corrupt or not a PDF): {e}"))?;

    let mut out = String::new();
    // get_pages() returns a BTreeMap<page_number, object_id> — iterate in
    // ascending page order so extracted text matches reading order.
    for page_num in doc.get_pages().keys() {
        match doc.extract_text(&[*page_num]) {
            Ok(text) => {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    if !out.is_empty() {
                        out.push_str("\n\n");
                    }
                    out.push_str(trimmed);
                }
            }
            // A single bad page shouldn't kill the whole document.
            Err(_) => continue,
        }
    }

    Ok(out)
}
