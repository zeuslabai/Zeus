//! Integration tests for `document_extract` OOXML extraction (#126 commit ②).
//!
//! Fixtures in `tests/fixtures/` are minimal hand-built OOXML packages
//! (generated via python3 zipfile, no Office toolchain) so they stay tiny and
//! deterministic.

use std::path::Path;
use zeus_agent::document_extract::extract_by_path;

fn read_fixture(name: &str) -> Vec<u8> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("read fixture {}: {}", p.display(), e))
}

fn extract(name: &str) -> String {
    let bytes = read_fixture(name);
    extract_by_path(Path::new(name), &bytes)
        .unwrap_or_else(|| panic!("{}: extension not dispatched", name))
        .unwrap_or_else(|e| panic!("{}: extraction failed: {}", name, e))
}

#[test]
fn docx_extracts_text_runs() {
    let text = extract("hello.docx");
    assert!(text.contains("Hello"), "docx missing 'Hello': {text:?}");
    assert!(text.contains("docx"), "docx missing 'docx': {text:?}");
}

#[test]
fn pptx_extracts_slides_in_order() {
    let text = extract("hello.pptx");
    assert!(text.contains("First slide"), "pptx missing slide 1: {text:?}");
    assert!(text.contains("Second slide"), "pptx missing slide 2: {text:?}");
    let first = text.find("First slide").unwrap();
    let second = text.find("Second slide").unwrap();
    assert!(first < second, "slides out of order: {text:?}");
}

#[test]
fn xlsx_extracts_cells() {
    let text = extract("hello.xlsx");
    assert!(text.contains("Name"), "xlsx missing 'Name': {text:?}");
    assert!(text.contains("Widget"), "xlsx missing 'Widget': {text:?}");
    assert!(text.contains("42"), "xlsx missing numeric '42': {text:?}");
}

#[test]
fn non_document_extension_returns_none() {
    // .txt is not a binary document format → dispatch returns None so the
    // caller falls back to a plain utf-8 read.
    assert!(
        extract_by_path(Path::new("notes.txt"), b"plain text").is_none(),
        "txt should not be dispatched to document_extract"
    );
}

#[test]
fn corrupt_ooxml_errors_not_panics() {
    // A .docx that is not a valid zip should surface an Err, not panic.
    let result = extract_by_path(Path::new("broken.docx"), b"not a zip file");
    match result {
        Some(Err(_)) => {}
        other => panic!("expected Some(Err(..)) for corrupt docx, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// ODF / epub / rtf extraction (#126 commit ③ — odf arm).
// Fixtures hand-built via python3 zipfile (odt/epub) and a literal RTF string,
// no external toolchain, tiny + deterministic.
// ---------------------------------------------------------------------------

#[test]
fn odt_extracts_paragraphs() {
    let text = extract("hello.odt");
    assert!(text.contains("Hello from odt"), "odt missing first para: {text:?}");
    assert!(text.contains("Second paragraph"), "odt missing second para: {text:?}");
}

#[test]
fn epub_extracts_spine_in_order() {
    let text = extract("hello.epub");
    assert!(text.contains("Chapter One"), "epub missing heading: {text:?}");
    assert!(text.contains("first chapter"), "epub missing chap1: {text:?}");
    assert!(text.contains("second chapter"), "epub missing chap2: {text:?}");
    let p1 = text.find("first chapter").unwrap();
    let p2 = text.find("second chapter").unwrap();
    assert!(p1 < p2, "epub spine order wrong (chap1 should precede chap2): {text:?}");
}

#[test]
fn rtf_strips_control_words_and_fonttbl() {
    let text = extract("hello.rtf");
    assert!(text.contains("Hello from rtf"), "rtf missing body: {text:?}");
    assert!(text.contains("Second line"), "rtf missing second line: {text:?}");
    // fonttbl contents must NOT leak into output
    assert!(!text.contains("Times New Roman"), "rtf leaked fonttbl: {text:?}");
    assert!(!text.contains("rtf1"), "rtf leaked control word: {text:?}");
}

#[test]
fn corrupt_odt_errors_not_panics() {
    // A .odt that is not a valid zip should surface an Err, not panic.
    let result = extract_by_path(Path::new("broken.odt"), b"not a zip file");
    match result {
        Some(Err(_)) => {}
        other => panic!("expected Some(Err(..)) for corrupt odt, got {other:?}"),
    }
}

#[test]
fn rtf_rejects_non_rtf() {
    // Bytes without the {\rtf header → Err, not silent garbage.
    let result = extract_by_path(Path::new("x.rtf"), b"plain not rtf");
    match result {
        Some(Err(_)) => {}
        other => panic!("expected Some(Err(..)) for non-rtf .rtf, got {other:?}"),
    }
}

// ---- PDF (#126 commit ②, zeus107) ----

#[test]
fn pdf_extracts_text() {
    // Minimal single-page PDF (generated via lopdf) carrying the text "Hello".
    let text = extract("hello.pdf");
    assert!(text.contains("Hello"), "pdf missing 'Hello': {text:?}");
}

#[test]
fn corrupt_pdf_errors_not_panics() {
    // A .pdf that is not a valid PDF should surface an Err, not panic.
    let result = extract_by_path(Path::new("broken.pdf"), b"not a pdf at all");
    match result {
        Some(Err(_)) => {}
        other => panic!("expected Some(Err(..)) for corrupt pdf, got {other:?}"),
    }
}
