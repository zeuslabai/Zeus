//! Media processing pipeline for channel attachments

use std::path::{Path, PathBuf};
use tokio::fs;
use zeus_core::Result;

/// Media processing pipeline
pub struct MediaPipeline {
    /// Maximum image size in bytes (default: 5MB)
    pub max_image_size: usize,
    /// Maximum image dimension in pixels (default: 2048)
    pub max_image_dimension: u32,
    /// Directory for storing downloaded/processed media
    pub media_dir: PathBuf,
}

impl MediaPipeline {
    /// Create a new media pipeline with default settings
    pub fn new(media_dir: PathBuf) -> Self {
        Self {
            max_image_size: 5 * 1024 * 1024, // 5MB
            max_image_dimension: 2048,
            media_dir,
        }
    }

    /// Initialize the media directory
    pub async fn init(&self) -> Result<()> {
        fs::create_dir_all(&self.media_dir).await?;
        Ok(())
    }

    /// Process an image - validate and pass through if within limits.
    /// Uses simple heuristic: if data > max_image_size, reject it.
    /// (No heavy image crate dependency - just validate and pass through.)
    pub fn process_image(&self, data: &[u8], _mime_type: &str) -> Result<Vec<u8>> {
        // Validate it looks like an image based on magic bytes
        let detected_type = detect_mime_type(data);
        if !detected_type.starts_with("image/") {
            return Err(zeus_core::Error::Tool(format!(
                "Data does not appear to be an image (detected: {})",
                detected_type
            )));
        }

        // If within size limits, return as-is
        if data.len() <= self.max_image_size {
            return Ok(data.to_vec());
        }

        // For oversized images, reject rather than resizing
        // (avoiding heavy image processing deps)
        Err(zeus_core::Error::Tool(format!(
            "Image too large: {} bytes (max: {} bytes). Please use a smaller image.",
            data.len(),
            self.max_image_size
        )))
    }

    /// Download media from a URL
    pub async fn download_url(&self, url: &str) -> Result<(Vec<u8>, String)> {
        let response = reqwest::get(url)
            .await
            .map_err(|e| zeus_core::Error::Tool(format!("Failed to download: {}", e)))?;

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();

        let bytes = response
            .bytes()
            .await
            .map_err(|e| zeus_core::Error::Tool(format!("Failed to read response: {}", e)))?;

        Ok((bytes.to_vec(), content_type))
    }

    /// Save media to disk, returns the saved file path
    pub async fn save_media(&self, data: &[u8], filename: &str) -> Result<PathBuf> {
        self.init().await?;
        let path = self.media_dir.join(filename);
        fs::write(&path, data).await?;
        Ok(path)
    }

    /// Load media from disk
    pub async fn load_media(&self, filename: &str) -> Result<Vec<u8>> {
        let path = self.media_dir.join(filename);
        Ok(fs::read(&path).await?)
    }

    /// List media files in the media directory
    pub async fn list_media(&self) -> Result<Vec<String>> {
        self.init().await?;
        let mut files = Vec::new();
        let mut entries = fs::read_dir(&self.media_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if let Some(name) = entry.file_name().to_str() {
                files.push(name.to_string());
            }
        }
        Ok(files)
    }

    /// Get the media directory path
    pub fn media_dir(&self) -> &Path {
        &self.media_dir
    }
}

impl Default for MediaPipeline {
    fn default() -> Self {
        let media_dir = dirs::home_dir()
            .unwrap_or_default()
            .join(".zeus")
            .join("media");
        Self::new(media_dir)
    }
}

/// Detect MIME type from magic bytes
pub fn detect_mime_type(data: &[u8]) -> String {
    if data.len() < 4 {
        return "application/octet-stream".to_string();
    }

    // Check magic bytes
    match &data[..4] {
        [0xFF, 0xD8, 0xFF, _] => "image/jpeg".to_string(),
        [0x89, 0x50, 0x4E, 0x47] => "image/png".to_string(),
        [0x47, 0x49, 0x46, 0x38] => "image/gif".to_string(),
        [0x52, 0x49, 0x46, 0x46] if data.len() >= 12 && &data[8..12] == b"WEBP" => {
            "image/webp".to_string()
        }
        [0x25, 0x50, 0x44, 0x46] => "application/pdf".to_string(),
        [0x50, 0x4B, 0x03, 0x04] => "application/zip".to_string(),
        [0x1A, 0x45, 0xDF, 0xA3] => "video/webm".to_string(),
        _ => {
            // Check for audio formats
            if data.len() >= 12 {
                if &data[..4] == b"fLaC" {
                    return "audio/flac".to_string();
                }
                if &data[..3] == b"ID3" || (data[0] == 0xFF && (data[1] & 0xE0) == 0xE0) {
                    return "audio/mpeg".to_string();
                }
                if &data[..4] == b"OggS" {
                    return "audio/ogg".to_string();
                }
                if &data[..4] == b"RIFF" && &data[8..12] == b"WAVE" {
                    return "audio/wav".to_string();
                }
            }
            "application/octet-stream".to_string()
        }
    }
}

/// Infer a MIME type from a filename's extension.
///
/// Returns `None` for unknown extensions so callers can fall back to other
/// detection methods (`detect_mime_type` magic-byte sniff, or default to
/// `application/octet-stream`). Used by channel adapters (e.g. Telegram
/// MTProto) when the upstream API doesn't provide a MIME for uploaded files.
///
/// Routes text-based files (HTML, MD, source code, configs) to `text/*` so
/// they reach the LLM via `process_attachments::text_contents` prompt-prefix
/// path. Routes Office binary documents (DOCX/PPTX/XLSX/XLS) to their
/// canonical OOXML/legacy MIMEs for downstream text-extraction handlers.
pub fn infer_mime_from_extension(filename: &str) -> Option<&'static str> {
    let ext = filename.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        // Markup
        "html" | "htm" => "text/html",
        "md" | "markdown" => "text/markdown",
        "xml" => "text/xml",
        // Tabular text
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        // Plain text / config / code (all UTF-8, LLM reads natively)
        "txt" | "log" | "json" | "yaml" | "yml" | "toml" | "ini" | "conf"
        | "env" | "rs" | "py" | "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs"
        | "go" | "java" | "kt" | "swift" | "c" | "cpp" | "cc" | "h" | "hpp"
        | "cs" | "rb" | "php" | "sh" | "bash" | "zsh" | "fish" | "ps1"
        | "lua" | "pl" | "r" | "sql" | "css" | "scss" | "sass" | "less"
        | "vue" | "svelte" | "elm" | "dart" | "ex" | "exs" | "erl" | "hrl"
        | "ml" | "fs" | "fsx" | "clj" | "edn" | "scala" | "nim" | "zig"
        | "dockerfile" | "makefile" => "text/plain",
        // Documents
        "pdf" => "application/pdf",
        // Office (OOXML)
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        // Office (legacy binary)
        "doc" => "application/msword",
        "ppt" => "application/vnd.ms-powerpoint",
        "xls" => "application/vnd.ms-excel",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_jpeg() {
        let data = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        assert_eq!(detect_mime_type(&data), "image/jpeg");
    }

    #[test]
    fn test_detect_png() {
        let data = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(detect_mime_type(&data), "image/png");
    }

    #[test]
    fn test_detect_gif() {
        let data = [0x47, 0x49, 0x46, 0x38, 0x39, 0x61];
        assert_eq!(detect_mime_type(&data), "image/gif");
    }

    #[test]
    fn test_detect_webp() {
        let data = [
            0x52, 0x49, 0x46, 0x46, 0x00, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50,
        ];
        assert_eq!(detect_mime_type(&data), "image/webp");
    }

    #[test]
    fn test_detect_pdf() {
        let data = [0x25, 0x50, 0x44, 0x46, 0x2D, 0x31, 0x2E, 0x34];
        assert_eq!(detect_mime_type(&data), "application/pdf");
    }

    #[test]
    fn test_detect_wav() {
        let data = [
            0x52, 0x49, 0x46, 0x46, 0x00, 0x00, 0x00, 0x00, 0x57, 0x41, 0x56, 0x45,
        ];
        assert_eq!(detect_mime_type(&data), "audio/wav");
    }

    #[test]
    fn test_detect_mp3_id3() {
        let data = [
            0x49, 0x44, 0x33, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(detect_mime_type(&data), "audio/mpeg");
    }

    #[test]
    fn test_detect_flac() {
        let data = [
            0x66, 0x4C, 0x61, 0x43, 0x00, 0x00, 0x00, 0x22, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(detect_mime_type(&data), "audio/flac");
    }

    #[test]
    fn test_detect_ogg() {
        let data = [
            0x4F, 0x67, 0x67, 0x53, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(detect_mime_type(&data), "audio/ogg");
    }

    #[test]
    fn test_detect_unknown() {
        let data = [0x00, 0x01, 0x02, 0x03];
        assert_eq!(detect_mime_type(&data), "application/octet-stream");
    }

    #[test]
    fn test_detect_short_data() {
        let data = [0xFF, 0xD8];
        assert_eq!(detect_mime_type(&data), "application/octet-stream");
    }

    #[test]
    fn test_pipeline_rejects_oversized_image() {
        let pipeline = MediaPipeline {
            max_image_size: 100, // 100 bytes
            max_image_dimension: 2048,
            media_dir: PathBuf::from("/tmp/test-media"),
        };

        // Valid JPEG header but data is too large
        let mut data = vec![0xFF, 0xD8, 0xFF, 0xE0];
        data.extend(vec![0x00; 200]);

        let result = pipeline.process_image(&data, "image/jpeg");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("too large"));
    }

    #[test]
    fn test_pipeline_accepts_valid_image() {
        let pipeline = MediaPipeline::new(PathBuf::from("/tmp/test-media"));

        // Valid JPEG header, small data
        let data = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];

        let result = pipeline.process_image(&data, "image/jpeg");
        assert!(result.is_ok());
        assert_eq!(result.expect("operation should succeed"), data);
    }

    #[test]
    fn test_pipeline_rejects_non_image() {
        let pipeline = MediaPipeline::new(PathBuf::from("/tmp/test-media"));

        // PDF magic bytes
        let data = vec![0x25, 0x50, 0x44, 0x46, 0x2D, 0x31, 0x2E, 0x34];

        let result = pipeline.process_image(&data, "image/jpeg");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("does not appear to be an image"));
    }

    #[tokio::test]
    async fn test_pipeline_save_and_load_roundtrip() {
        let tmp = tempfile::tempdir().expect("Failed to create temp directory");
        let pipeline = MediaPipeline::new(tmp.path().to_path_buf());

        let data = b"hello world media content";
        let path = pipeline
            .save_media(data, "test.bin")
            .await
            .expect("Failed to save test media");
        assert!(path.exists());

        let loaded = pipeline
            .load_media("test.bin")
            .await
            .expect("Failed to load test media");
        assert_eq!(loaded, data);
    }

    #[tokio::test]
    async fn test_pipeline_list_media() {
        let tmp = tempfile::tempdir().expect("should create temp dir");
        let pipeline = MediaPipeline::new(tmp.path().to_path_buf());

        pipeline
            .save_media(b"file1", "a.txt")
            .await
            .expect("async operation should succeed");
        pipeline
            .save_media(b"file2", "b.txt")
            .await
            .expect("async operation should succeed");

        let files = pipeline
            .list_media()
            .await
            .expect("async operation should succeed");
        assert_eq!(files.len(), 2);
        assert!(files.contains(&"a.txt".to_string()));
        assert!(files.contains(&"b.txt".to_string()));
    }

    #[test]
    fn test_pipeline_default() {
        let pipeline = MediaPipeline::default();
        assert_eq!(pipeline.max_image_size, 5 * 1024 * 1024);
        assert_eq!(pipeline.max_image_dimension, 2048);
        assert!(
            pipeline
                .media_dir()
                .to_str()
                .expect("to_str should succeed")
                .contains(".zeus")
        );
    }

    #[test]
    fn test_infer_mime_html() {
        assert_eq!(infer_mime_from_extension("page.html"), Some("text/html"));
        assert_eq!(infer_mime_from_extension("page.htm"), Some("text/html"));
    }

    #[test]
    fn test_infer_mime_markdown() {
        assert_eq!(infer_mime_from_extension("doc.md"), Some("text/markdown"));
        assert_eq!(
            infer_mime_from_extension("doc.markdown"),
            Some("text/markdown")
        );
    }

    #[test]
    fn test_infer_mime_office_ooxml() {
        assert_eq!(
            infer_mime_from_extension("report.docx"),
            Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document")
        );
        assert_eq!(
            infer_mime_from_extension("slides.pptx"),
            Some("application/vnd.openxmlformats-officedocument.presentationml.presentation")
        );
        assert_eq!(
            infer_mime_from_extension("data.xlsx"),
            Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet")
        );
    }

    #[test]
    fn test_infer_mime_office_legacy() {
        assert_eq!(infer_mime_from_extension("legacy.xls"), Some("application/vnd.ms-excel"));
        assert_eq!(infer_mime_from_extension("legacy.doc"), Some("application/msword"));
        assert_eq!(
            infer_mime_from_extension("legacy.ppt"),
            Some("application/vnd.ms-powerpoint")
        );
    }

    #[test]
    fn test_infer_mime_code_routes_to_text_plain() {
        assert_eq!(infer_mime_from_extension("script.py"), Some("text/plain"));
        assert_eq!(infer_mime_from_extension("comp.jsx"), Some("text/plain"));
        assert_eq!(infer_mime_from_extension("config.toml"), Some("text/plain"));
        assert_eq!(infer_mime_from_extension("data.json"), Some("text/plain"));
        assert_eq!(infer_mime_from_extension("module.rs"), Some("text/plain"));
    }

    #[test]
    fn test_infer_mime_pdf() {
        assert_eq!(infer_mime_from_extension("doc.pdf"), Some("application/pdf"));
    }

    #[test]
    fn test_infer_mime_case_insensitive() {
        assert_eq!(infer_mime_from_extension("PAGE.HTML"), Some("text/html"));
        assert_eq!(infer_mime_from_extension("Doc.MD"), Some("text/markdown"));
        assert_eq!(infer_mime_from_extension("Report.DOCX"),
            Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"));
    }

    #[test]
    fn test_infer_mime_unknown_extension_returns_none() {
        assert_eq!(infer_mime_from_extension("file.unknownext"), None);
        assert_eq!(infer_mime_from_extension("noextension"), None);
        assert_eq!(infer_mime_from_extension(""), None);
    }
}
