#![cfg(target_arch = "wasm32")]

use wasm_bindgen_test::*;
use web_sys;

wasm_bindgen_test_configure!(run_in_browser);

/// Smoke test: verify DOM (window) is available in the wasm32 browser environment.
#[wasm_bindgen_test]
fn test_wasm_dom_available() {
    let win = web_sys::window().expect("window should exist in browser wasm env");
    let doc = win.document().expect("document should exist on window");
    // Just verify we can access the document element — proves DOM bindings work.
    let _body = doc.body().expect("body should exist on document");
}

/// Smoke test: verify parse_unified_diff is callable from wasm32 and returns correct results.
#[wasm_bindgen_test]
fn test_wasm_diff_viewer_parse() {
    let patch = "\
--- a/foo.rs
+++ b/foo.rs
@@ -1,3 +1,3 @@
 unchanged
-removed
+added
";

    let lines = zeus_web::components::diff_viewer::parse_unified_diff(patch);

    // Should have at least 4 lines: hunk header + context + removed + added
    assert!(lines.len() >= 4, "expected >= 4 diff lines, got {}", lines.len());

    // Verify classification: the removed line should be classified as Removed
    let has_removed = lines.iter().any(|l| {
        matches!(l, zeus_web::components::diff_viewer::DiffLine::Removed(_))
    });
    assert!(has_removed, "should have a Removed line");

    // Verify classification: the added line should be classified as Added
    let has_added = lines.iter().any(|l| {
        matches!(l, zeus_web::components::diff_viewer::DiffLine::Added(_))
    });
    assert!(has_added, "should have an Added line");
}
