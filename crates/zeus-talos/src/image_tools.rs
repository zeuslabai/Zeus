//! Image manipulation tools — wrappers around macOS `sips` CLI.
//!
//! Provides:
//! - `image_resize`   — resize an image to a target width/height
//! - `image_convert`  — convert between formats (jpeg, png, heic, tiff, gif, bmp)
//! - `image_compress` — re-encode JPEG with a target quality
//! - `image_exif`     — read EXIF / sips metadata
//!
//! `sips` ships with macOS by default; on Linux these tools will return a
//! "sips not available" error. Args are passed directly to
//! `tokio::process::Command` (no shell interpretation).

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
use std::path::{Path, PathBuf};
use zeus_core::{Error, Result, ToolSchema};

const MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// Allowed output formats for `image_convert`.
const ALLOWED_FORMATS: &[&str] = &["jpeg", "png", "heic", "tiff", "gif", "bmp"];

/// Validate that a path string is non-empty and contains no shell metacharacters
/// that would matter even though we use exec, just to fail early on obvious junk.
fn validate_path(s: &str, field: &str) -> Result<PathBuf> {
    if s.is_empty() {
        return Err(Error::Tool(format!("{} must not be empty", field)));
    }
    if s.len() > 4096 {
        return Err(Error::Tool(format!("{} too long", field)));
    }
    if s.chars().any(|c| c == '\0' || c == '\n') {
        return Err(Error::Tool(format!(
            "{} contains forbidden characters",
            field
        )));
    }
    Ok(PathBuf::from(s))
}

fn require_file_exists(p: &Path) -> Result<()> {
    if !p.exists() {
        return Err(Error::Tool(format!("file does not exist: {}", p.display())));
    }
    if !p.is_file() {
        return Err(Error::Tool(format!("not a regular file: {}", p.display())));
    }
    Ok(())
}

fn truncate_output(s: String) -> String {
    if s.len() <= MAX_OUTPUT_BYTES {
        return s;
    }
    let mut bytes = s.into_bytes();
    bytes.truncate(MAX_OUTPUT_BYTES);
    let mut s = String::from_utf8_lossy(&bytes).into_owned();
    s.push_str("\n... [truncated]");
    s
}

async fn run_sips(args: &[&str]) -> Result<String> {
    let output = tokio::process::Command::new("sips")
        .args(args)
        .output()
        .await
        .map_err(|e| Error::Tool(format!("Failed to run sips (macOS only): {}", e)))?;

    if output.status.success() {
        Ok(truncate_output(
            String::from_utf8_lossy(&output.stdout).to_string(),
        ))
    } else {
        Err(Error::Tool(format!(
            "sips failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

// ---------- image_resize ----------

/// Resize an image to a target width/height.
pub struct ImageResizeTool;

#[async_trait]
impl TalosTool for ImageResizeTool {
    fn name(&self) -> &'static str {
        "image_resize"
    }
    fn description(&self) -> &'static str {
        "Resize an image. Provide width and/or height (preserves aspect ratio if only one). Writes to output path."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("input", "string", "Input image path", true)
            .with_param("output", "string", "Output image path", true)
            .with_param("width", "integer", "Target width in pixels", false)
            .with_param("height", "integer", "Target height in pixels", false)
            .with_param(
                "max_dimension",
                "integer",
                "Resize so the longest side equals this (alternative to width/height)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let input = args
            .get("input")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("input required".to_string()))?;
        let output = args
            .get("output")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("output required".to_string()))?;
        let in_path = validate_path(input, "input")?;
        let out_path = validate_path(output, "output")?;
        require_file_exists(&in_path)?;

        let width = args.get("width").and_then(|v| v.as_u64());
        let height = args.get("height").and_then(|v| v.as_u64());
        let max_dim = args.get("max_dimension").and_then(|v| v.as_u64());

        if width.is_none() && height.is_none() && max_dim.is_none() {
            return Err(Error::Tool(
                "must provide at least one of: width, height, max_dimension".to_string(),
            ));
        }

        // Build args
        let in_str = in_path.to_string_lossy().into_owned();
        let out_str = out_path.to_string_lossy().into_owned();
        let mut argv: Vec<String> = Vec::new();

        if let Some(m) = max_dim {
            argv.push("-Z".to_string());
            argv.push(m.to_string());
        } else {
            if let Some(w) = width {
                argv.push("--resampleWidth".to_string());
                argv.push(w.to_string());
            }
            if let Some(h) = height {
                argv.push("--resampleHeight".to_string());
                argv.push(h.to_string());
            }
        }
        argv.push(in_str);
        argv.push("--out".to_string());
        argv.push(out_str);

        let argv_refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
        run_sips(&argv_refs).await?;
        Ok(format!("resized -> {}", out_path.display()))
    }
}

// ---------- image_convert ----------

/// Convert an image to a different format.
pub struct ImageConvertTool;

#[async_trait]
impl TalosTool for ImageConvertTool {
    fn name(&self) -> &'static str {
        "image_convert"
    }
    fn description(&self) -> &'static str {
        "Convert an image to another format (jpeg|png|heic|tiff|gif|bmp)."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("input", "string", "Input image path", true)
            .with_param("output", "string", "Output image path", true)
            .with_param(
                "format",
                "string",
                "Output format: jpeg|png|heic|tiff|gif|bmp",
                true,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let input = args
            .get("input")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("input required".to_string()))?;
        let output = args
            .get("output")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("output required".to_string()))?;
        let format = args
            .get("format")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("format required".to_string()))?
            .to_lowercase();

        if !ALLOWED_FORMATS.contains(&format.as_str()) {
            return Err(Error::Tool(format!(
                "format must be one of {} (got '{}')",
                ALLOWED_FORMATS.join("|"),
                format
            )));
        }

        let in_path = validate_path(input, "input")?;
        let out_path = validate_path(output, "output")?;
        require_file_exists(&in_path)?;

        run_sips(&[
            "-s",
            "format",
            &format,
            in_path.to_string_lossy().as_ref(),
            "--out",
            out_path.to_string_lossy().as_ref(),
        ])
        .await?;
        Ok(format!("converted to {} -> {}", format, out_path.display()))
    }
}

// ---------- image_compress ----------

/// Re-encode an image with a target JPEG quality (0-100).
pub struct ImageCompressTool;

#[async_trait]
impl TalosTool for ImageCompressTool {
    fn name(&self) -> &'static str {
        "image_compress"
    }
    fn description(&self) -> &'static str {
        "Re-encode an image as JPEG with a quality 1-100 (lower = smaller file)."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("input", "string", "Input image path", true)
            .with_param("output", "string", "Output JPEG path", true)
            .with_param(
                "quality",
                "integer",
                "JPEG quality 1-100 (default 75)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let input = args
            .get("input")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("input required".to_string()))?;
        let output = args
            .get("output")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("output required".to_string()))?;
        let quality = args.get("quality").and_then(|v| v.as_u64()).unwrap_or(75);
        if !(1..=100).contains(&quality) {
            return Err(Error::Tool("quality must be in 1..=100".to_string()));
        }

        let in_path = validate_path(input, "input")?;
        let out_path = validate_path(output, "output")?;
        require_file_exists(&in_path)?;

        // sips: -s format jpeg -s formatOptions <quality>
        let q_str = quality.to_string();
        run_sips(&[
            "-s",
            "format",
            "jpeg",
            "-s",
            "formatOptions",
            &q_str,
            in_path.to_string_lossy().as_ref(),
            "--out",
            out_path.to_string_lossy().as_ref(),
        ])
        .await?;
        Ok(format!(
            "compressed (q={}) -> {}",
            quality,
            out_path.display()
        ))
    }
}

// ---------- image_exif ----------

/// Read image metadata (EXIF + sips properties).
pub struct ImageExifTool;

#[async_trait]
impl TalosTool for ImageExifTool {
    fn name(&self) -> &'static str {
        "image_exif"
    }
    fn description(&self) -> &'static str {
        "Read image properties / EXIF data via sips -g all."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "input",
            "string",
            "Input image path",
            true,
        )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let input = args
            .get("input")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("input required".to_string()))?;
        let in_path = validate_path(input, "input")?;
        require_file_exists(&in_path)?;
        run_sips(&["-g", "all", in_path.to_string_lossy().as_ref()]).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_paths() {
        assert!(validate_path("/tmp/foo.png", "x").is_ok());
        assert!(validate_path("relative.jpg", "x").is_ok());
        assert!(validate_path("", "x").is_err());
        assert!(validate_path("with\0null", "x").is_err());
        assert!(validate_path("with\nnewline", "x").is_err());
    }

    #[test]
    fn convert_rejects_unknown_format() {
        let t = ImageConvertTool;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let res = rt.block_on(t.execute(serde_json::json!({
            "input": "/tmp/x.png",
            "output": "/tmp/x.webp",
            "format": "webp",
        })));
        assert!(res.is_err());
    }

    #[test]
    fn resize_requires_a_dimension() {
        let t = ImageResizeTool;
        let rt = tokio::runtime::Runtime::new().unwrap();
        // Use a real existing file so we exercise the dimension check, not the existence check.
        let tmp = std::env::temp_dir().join("zeus_talos_resize_test.png");
        std::fs::write(&tmp, b"fake").ok();
        let res = rt.block_on(t.execute(serde_json::json!({
            "input": tmp.to_string_lossy(),
            "output": "/tmp/zeus_talos_resize_out.png",
        })));
        std::fs::remove_file(&tmp).ok();
        assert!(res.is_err());
        let msg = format!("{:?}", res.err().unwrap());
        assert!(msg.contains("width"), "expected hint about dimensions: {}", msg);
    }

    #[test]
    fn compress_rejects_bad_quality() {
        let t = ImageCompressTool;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let tmp = std::env::temp_dir().join("zeus_talos_compress_test.png");
        std::fs::write(&tmp, b"fake").ok();
        let res = rt.block_on(t.execute(serde_json::json!({
            "input": tmp.to_string_lossy(),
            "output": "/tmp/zeus_talos_compress_out.jpg",
            "quality": 200,
        })));
        std::fs::remove_file(&tmp).ok();
        assert!(res.is_err());
    }

    #[test]
    fn exif_requires_existing_file() {
        let t = ImageExifTool;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let res = rt.block_on(t.execute(serde_json::json!({
            "input": "/tmp/__zeus_does_not_exist__.png",
        })));
        assert!(res.is_err());
    }

    #[test]
    fn schemas_serialize() {
        for s in [
            ImageResizeTool.schema(),
            ImageConvertTool.schema(),
            ImageCompressTool.schema(),
            ImageExifTool.schema(),
        ] {
            assert!(!s.name.is_empty());
            assert!(!s.description.is_empty());
            assert!(s.parameters.get("properties").is_some());
            assert!(s.parameters.get("required").is_some());
        }
    }
}
