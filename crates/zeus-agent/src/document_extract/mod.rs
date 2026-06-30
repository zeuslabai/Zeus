//! Transparent document text extraction for `read_file`.
//!
//! Many document formats are binary containers (OOXML = zip+XML, PDF, ODF =
//! zip+XML) that a naive `fs::read_to_string` cannot decode — it fails with
//! "stream did not contain valid UTF-8". This module detects such formats by
//! extension and extracts their text content, pure-Rust + cross-platform
//! (works on macOS and FreeBSD seats alike).
//!
//! Dispatch contract (per-format, by arity — routed by `extract_by_path`):
//! ```ignore
//! pub fn extract(bytes: &[u8]) -> Result<String>             // single-format: pdf
//! pub fn extract(ext: &str, bytes: &[u8]) -> Result<String>  // multi-format: ooxml, odf
//! ```
//! Single-format modules (pdf) take bytes only; multi-format modules
//! (ooxml: docx/pptx/xlsx; odf: odt/ods/odp/epub/rtf) take the lowercased
//! `ext` to self-route. Sync, `Cursor` for any zip reads.

use std::path::Path;
use zeus_core::Result;

pub mod ooxml;
pub mod pdf;
pub mod odf;

/// Returns `Some(extracted_text)` if `path`'s extension is a supported
/// document format, or `None` if it is not (caller falls back to a plain
/// UTF-8 text read).
///
/// `bytes` is the raw file content already read from disk; we never re-read.
pub fn extract_by_path(path: &Path, bytes: &[u8]) -> Option<Result<String>> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())?;

    match ext.as_str() {
        // OOXML
        "docx" | "pptx" | "xlsx" | "xlsm" => Some(ooxml::extract(ext.as_str(), bytes)),
        // PDF
        "pdf" => Some(pdf::extract(bytes)),
        // OpenDocument + ebook + rtf
        "odt" | "ods" | "odp" | "epub" | "rtf" => Some(odf::extract(ext.as_str(), bytes)),
        _ => None,
    }
}
