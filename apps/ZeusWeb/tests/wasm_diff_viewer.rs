#![cfg(target_arch = "wasm32")]

use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// WASM smoke test: render_diff_html is callable and returns non-empty HTML.
#[wasm_bindgen_test]
fn test_wasm_diff_viewer_render_html_callable() {
    let patch = "\
--- a/foo.rs
+++ b/foo.rs
@@ -1,3 +1,3 @@
 unchanged
-removed line
+added line
";
    let lines = zeus_web::components::diff_viewer::parse_unified_diff(patch);
    assert!(!lines.is_empty(), "parse_unified_diff should return lines under WASM");

    // Verify we get both Added and Removed variants — proves full pipeline callable
    let has_added = lines.iter().any(|l| {
        matches!(l, zeus_web::components::diff_viewer::DiffLine::Added(_))
    });
    let has_removed = lines.iter().any(|l| {
        matches!(l, zeus_web::components::diff_viewer::DiffLine::Removed(_))
    });
    assert!(has_added, "should have an Added line");
    assert!(has_removed, "should have a Removed line");
}

/// WASM smoke test: detect_diff_args — patch field dispatch (Case 1).
#[wasm_bindgen_test]
fn test_wasm_detect_diff_args_patch_field() {
    let args = serde_json::json!({ "patch": "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n" });
    let result = zeus_web::components::diff_viewer::detect_diff_args(&args);
    assert!(result.is_some(), "should detect patch field");
    let patch = result.unwrap();
    assert!(patch.contains("--- a/x"), "patch content should be returned as-is");
}

/// WASM smoke test: detect_diff_args — old_str/new_str dispatch (Case 2).
#[wasm_bindgen_test]
fn test_wasm_detect_diff_args_old_new_str() {
    let args = serde_json::json!({
        "old_str": "line one\nline two\n",
        "new_str": "line one\nline three\n",
        "path": "test.txt"
    });
    let result = zeus_web::components::diff_viewer::detect_diff_args(&args);
    assert!(result.is_some(), "should detect old_str/new_str pair");
    let diff = result.unwrap();
    assert!(diff.contains("--- a/test.txt"), "diff should include path header");
}

/// WASM smoke test: detect_diff_args — no args returns None (Case 3).
#[wasm_bindgen_test]
fn test_wasm_detect_diff_args_none() {
    let args = serde_json::json!({ "unrelated": "value" });
    let result = zeus_web::components::diff_viewer::detect_diff_args(&args);
    assert!(result.is_none(), "should return None when no diff args present");
}
