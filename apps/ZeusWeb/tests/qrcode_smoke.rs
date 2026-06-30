//! #65-F3 — qrcode crate native smoke-test
//!
//! Exercises the qrcode v0.14.1 API as used in onboarding_wizard.rs.
//! Tests run on the host target (native). WASM-target test pending #65-D2 harness.
//!
//! Real callsite: apps/ZeusWeb/src/pages/onboarding_wizard.rs:16-18
//!   QrCode::new(data.as_bytes()) → .to_colors() + .width()

use qrcode::types::Color;
use qrcode::QrCode;

/// Smoke-test 1: QrCode::new() succeeds for a simple ASCII payload
/// and returns a non-zero width matrix.
#[test]
fn qrcode_new_succeeds_and_has_nonzero_width() {
    let code = QrCode::new(b"https://zeuslab.ai").expect("QrCode::new failed");
    let width = code.width();
    assert!(width > 0, "expected non-zero QR matrix width, got {width}");
}

/// Smoke-test 2: to_colors() returns exactly width² cells,
/// each cell is Color::Dark or Color::Light — mirrors onboarding_wizard.rs logic.
#[test]
fn qrcode_to_colors_len_matches_width_squared() {
    let code = QrCode::new(b"https://zeuslab.ai/onboarding").expect("QrCode::new failed");
    let width = code.width();
    let colors = code.to_colors();
    assert_eq!(
        colors.len(),
        width * width,
        "to_colors() len {len} != width² {sq}",
        len = colors.len(),
        sq = width * width,
    );
    // Every cell must be a valid Color variant (Dark or Light) — no panics
    let dark_count = colors.iter().filter(|c| **c == Color::Dark).count();
    assert!(dark_count > 0, "expected at least one Dark cell in QR matrix");
}
