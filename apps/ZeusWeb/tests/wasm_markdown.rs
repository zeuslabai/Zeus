#![cfg(target_arch = "wasm32")]

use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// WASM smoke test: render_markdown produces non-empty HTML for basic input.
#[wasm_bindgen_test]
fn test_wasm_markdown_render_basic() {
    let html = zeus_web::components::markdown::render_markdown("# Hello\n\nWorld");
    assert!(!html.is_empty(), "render_markdown should return non-empty HTML");
    assert!(html.contains("Hello"), "output should contain heading text");
    assert!(html.contains("World"), "output should contain paragraph text");
}

/// WASM smoke test: XSS vectors are stripped by the sanitizer under WASM target.
#[wasm_bindgen_test]
fn test_wasm_markdown_xss_escape() {
    let md = "Safe text\n\n<script>alert('xss')</script>\n\n<iframe src='evil'></iframe>";
    let html = zeus_web::components::markdown::render_markdown(md);
    assert!(
        !html.to_lowercase().contains("<script"),
        "script tags must be stripped"
    );
    assert!(
        !html.to_lowercase().contains("<iframe"),
        "iframe tags must be stripped"
    );
    assert!(html.contains("Safe text"), "safe content must be preserved");
}

/// WASM smoke test: fenced code blocks are preserved (not stripped) under WASM target.
#[wasm_bindgen_test]
fn test_wasm_markdown_code_block() {
    let md = "Intro\n\n```rust\nfn main() {}\n```\n\nOutro";
    let html = zeus_web::components::markdown::render_markdown(md);
    assert!(!html.is_empty(), "output should not be empty");
    assert!(
        html.contains("fn main"),
        "code block content must be preserved"
    );
    assert!(html.contains("Intro"), "text before code block must be present");
    assert!(html.contains("Outro"), "text after code block must be present");
}
