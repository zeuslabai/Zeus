//! OCR / Vision tools using the macOS Vision framework.
//!
//! Three tools:
//! - `ocr_screenshot` — capture the screen (or a display) and OCR it
//! - `ocr_image`      — OCR an image file on disk
//! - `ocr_region`     — capture a specific screen rectangle and OCR it
//!
//! Implementation strategy: we shell out to `swift -e '<script>'` running a
//! small Vision program (`VNImageRequestHandler` + `VNRecognizeTextRequest`).
//! This avoids pulling in objc / cocoa-foundation crates and keeps the
//! module to a single file with zero new deps.
//!
//! macOS only. On other platforms the tools return a "not supported" error.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use zeus_core::{Error, Result, ToolSchema};

// ---------------------------------------------------------------------------
// Shared Swift program — runs Vision OCR against a file path passed as argv[1]
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
const SWIFT_OCR_PROGRAM: &str = r#"
import Foundation
import Vision
import AppKit

guard CommandLine.arguments.count >= 2 else {
    FileHandle.standardError.write("usage: ocr <image-path> [lang1,lang2,...] [fast|accurate]\n".data(using: .utf8)!)
    exit(2)
}
let path = CommandLine.arguments[1]
let langs: [String] = CommandLine.arguments.count >= 3 && !CommandLine.arguments[2].isEmpty
    ? CommandLine.arguments[2].split(separator: ",").map { String($0) }
    : []
let level = CommandLine.arguments.count >= 4 ? CommandLine.arguments[3] : "accurate"

guard let img = NSImage(contentsOfFile: path),
      let cg = img.cgImage(forProposedRect: nil, context: nil, hints: nil) else {
    FileHandle.standardError.write("failed to load image: \(path)\n".data(using: .utf8)!)
    exit(3)
}

let req = VNRecognizeTextRequest()
req.recognitionLevel = (level == "fast") ? .fast : .accurate
req.usesLanguageCorrection = true
if !langs.isEmpty { req.recognitionLanguages = langs }

let handler = VNImageRequestHandler(cgImage: cg, options: [:])
do { try handler.perform([req]) } catch {
    FileHandle.standardError.write("vision error: \(error)\n".data(using: .utf8)!)
    exit(4)
}

var lines: [[String: Any]] = []
for obs in (req.results ?? []) {
    guard let top = obs.topCandidates(1).first else { continue }
    let bb = obs.boundingBox
    lines.append([
        "text": top.string,
        "confidence": top.confidence,
        "bbox": ["x": bb.origin.x, "y": bb.origin.y, "w": bb.size.width, "h": bb.size.height]
    ])
}
let out: [String: Any] = [
    "lines": lines,
    "text": lines.compactMap { $0["text"] as? String }.joined(separator: "\n")
]
let data = try JSONSerialization.data(withJSONObject: out, options: [])
FileHandle.standardOutput.write(data)
"#;

#[cfg(target_os = "macos")]
async fn run_vision_ocr(image_path: &str, langs: Option<&str>, level: Option<&str>) -> Result<Value> {
    use tokio::process::Command;

    // Write the Swift program to a temp file (faster + safer than -e for multi-line).
    let mut script_path = std::env::temp_dir();
    script_path.push(format!("zeus_ocr_{}.swift", std::process::id()));
    tokio::fs::write(&script_path, SWIFT_OCR_PROGRAM)
        .await
        .map_err(|e| Error::Tool(format!("failed to write swift script: {e}")))?;

    let output = Command::new("swift")
        .arg(&script_path)
        .arg(image_path)
        .arg(langs.unwrap_or(""))
        .arg(level.unwrap_or("accurate"))
        .output()
        .await
        .map_err(|e| Error::Tool(format!("failed to spawn swift: {e}")))?;

    // Best-effort cleanup
    let _ = tokio::fs::remove_file(&script_path).await;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::Tool(format!(
            "swift OCR failed (status {}): {}",
            output.status, stderr
        )));
    }

    serde_json::from_slice::<Value>(&output.stdout)
        .map_err(|e| Error::Tool(format!("failed to parse OCR JSON: {e}")))
}

#[cfg(target_os = "macos")]
async fn capture_screen(region: Option<(i32, i32, i32, i32)>, display: Option<i32>) -> Result<PathBuf> {
    use tokio::process::Command;

    let mut path = std::env::temp_dir();
    path.push(format!("zeus_ocr_capture_{}.png", uuid_like()));

    let mut cmd = Command::new("screencapture");
    cmd.arg("-x"); // no sound
    cmd.arg("-t").arg("png");

    if let Some((x, y, w, h)) = region {
        cmd.arg("-R").arg(format!("{},{},{},{}", x, y, w, h));
    }
    if let Some(d) = display {
        cmd.arg("-D").arg(d.to_string());
    }
    cmd.arg(&path);

    let out = cmd
        .output()
        .await
        .map_err(|e| Error::Tool(format!("failed to spawn screencapture: {e}")))?;

    if !out.status.success() {
        return Err(Error::Tool(format!(
            "screencapture failed (status {}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    if !path.exists() {
        return Err(Error::Tool(
            "screencapture produced no file (permission denied? Grant Screen Recording in System Settings → Privacy)".into(),
        ));
    }
    Ok(path)
}

#[cfg(target_os = "macos")]
fn uuid_like() -> String {
    // Cheap unique-enough id without pulling uuid crate
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}_{}", nanos, std::process::id())
}

// ---------------------------------------------------------------------------
// Tool: ocr_image
// ---------------------------------------------------------------------------

pub struct OcrImageTool;

#[async_trait]
impl TalosTool for OcrImageTool {
    fn name(&self) -> &'static str {
        "ocr_image"
    }
    fn description(&self) -> &'static str {
        "Run OCR (text recognition) on an image file using the macOS Vision framework. Returns recognized text + per-line bounding boxes and confidence scores."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("path", "string", "Absolute path to image (png/jpg/heic/tiff)", true)
            .with_param(
                "languages",
                "string",
                "Comma-separated BCP-47 language codes (e.g. 'en-US,de-DE'). Default: auto.",
                false,
            )
            .with_param(
                "level",
                "string",
                "'accurate' (default) or 'fast'",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing path".to_string()))?;
        let langs = args.get("languages").and_then(|v| v.as_str());
        let level = args.get("level").and_then(|v| v.as_str());

        #[cfg(target_os = "macos")]
        {
            if !std::path::Path::new(path).exists() {
                return Err(Error::Tool(format!("image not found: {}", path)));
            }
            let result = run_vision_ocr(path, langs, level).await?;
            Ok(serde_json::to_string_pretty(&result)
                .map_err(|e| Error::Tool(format!("failed to serialize: {e}")))?)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (path, langs, level);
            Err(Error::Tool(
                "ocr_image is only supported on macOS (uses Vision framework)".into(),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Tool: ocr_screenshot
// ---------------------------------------------------------------------------

pub struct OcrScreenshotTool;

#[async_trait]
impl TalosTool for OcrScreenshotTool {
    fn name(&self) -> &'static str {
        "ocr_screenshot"
    }
    fn description(&self) -> &'static str {
        "Capture the screen and run OCR on it. Returns recognized text. Optionally captures a specific display (1, 2, ...)."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "display",
                "integer",
                "Display index (1 = main). Omit to capture all displays merged.",
                false,
            )
            .with_param(
                "languages",
                "string",
                "Comma-separated BCP-47 language codes",
                false,
            )
            .with_param("level", "string", "'accurate' (default) or 'fast'", false)
            .with_param(
                "keep_file",
                "boolean",
                "If true, retain the screenshot file and include its path in the response. Default false.",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let display = args.get("display").and_then(|v| v.as_i64()).map(|n| n as i32);
            let langs = args.get("languages").and_then(|v| v.as_str());
            let level = args.get("level").and_then(|v| v.as_str());
            let keep = args.get("keep_file").and_then(|v| v.as_bool()).unwrap_or(false);

            let path = capture_screen(None, display).await?;
            let path_str = path.to_string_lossy().to_string();

            let mut result = run_vision_ocr(&path_str, langs, level).await?;

            if keep {
                result["screenshot_path"] = json!(path_str);
            } else {
                let _ = tokio::fs::remove_file(&path).await;
            }

            Ok(serde_json::to_string_pretty(&result)
                .map_err(|e| Error::Tool(format!("failed to serialize: {e}")))?)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Err(Error::Tool(
                "ocr_screenshot is only supported on macOS".into(),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Tool: ocr_region
// ---------------------------------------------------------------------------

pub struct OcrRegionTool;

#[async_trait]
impl TalosTool for OcrRegionTool {
    fn name(&self) -> &'static str {
        "ocr_region"
    }
    fn description(&self) -> &'static str {
        "Capture a rectangular region of the screen and OCR it. Coordinates are in screen points (origin top-left)."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("x", "integer", "X coordinate (top-left origin)", true)
            .with_param("y", "integer", "Y coordinate", true)
            .with_param("width", "integer", "Region width in points", true)
            .with_param("height", "integer", "Region height in points", true)
            .with_param("languages", "string", "Comma-separated BCP-47 codes", false)
            .with_param("level", "string", "'accurate' or 'fast'", false)
            .with_param(
                "keep_file",
                "boolean",
                "Retain screenshot file and include path. Default false.",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let x = args.get("x").and_then(|v| v.as_i64()).ok_or_else(|| {
                Error::Tool("Missing x".into())
            })? as i32;
            let y = args.get("y").and_then(|v| v.as_i64()).ok_or_else(|| {
                Error::Tool("Missing y".into())
            })? as i32;
            let w = args.get("width").and_then(|v| v.as_i64()).ok_or_else(|| {
                Error::Tool("Missing width".into())
            })? as i32;
            let h = args.get("height").and_then(|v| v.as_i64()).ok_or_else(|| {
                Error::Tool("Missing height".into())
            })? as i32;
            if w <= 0 || h <= 0 {
                return Err(Error::Tool("width and height must be > 0".into()));
            }

            let langs = args.get("languages").and_then(|v| v.as_str());
            let level = args.get("level").and_then(|v| v.as_str());
            let keep = args.get("keep_file").and_then(|v| v.as_bool()).unwrap_or(false);

            let path = capture_screen(Some((x, y, w, h)), None).await?;
            let path_str = path.to_string_lossy().to_string();

            let mut result = run_vision_ocr(&path_str, langs, level).await?;
            if keep {
                result["screenshot_path"] = json!(path_str);
            } else {
                let _ = tokio::fs::remove_file(&path).await;
            }

            Ok(serde_json::to_string_pretty(&result)
                .map_err(|e| Error::Tool(format!("failed to serialize: {e}")))?)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Err(Error::Tool("ocr_region is only supported on macOS".into()))
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_have_required_params() {
        let img = OcrImageTool.schema();
        assert_eq!(img.name, "ocr_image");
        let shot = OcrScreenshotTool.schema();
        assert_eq!(shot.name, "ocr_screenshot");
        let region = OcrRegionTool.schema();
        assert_eq!(region.name, "ocr_region");
    }

    #[tokio::test]
    async fn ocr_image_rejects_missing_path() {
        let res = OcrImageTool.execute(json!({})).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn ocr_region_validates_dimensions() {
        let res = OcrRegionTool
            .execute(json!({"x": 0, "y": 0, "width": 0, "height": 100}))
            .await;
        #[cfg(target_os = "macos")]
        assert!(res.is_err());
        #[cfg(not(target_os = "macos"))]
        let _ = res; // non-macos always errors anyway
    }
}
