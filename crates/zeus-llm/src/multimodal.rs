//! Multimodal input support — image loading from URLs, files, and raw data.
//!
//! Provides helpers to create image attachments from various sources and
//! resolve URL references into base64 data for providers that require it.

use reqwest::Client;
use std::path::Path;
use tracing::{debug, warn};
use zeus_core::{Attachment, Error, Result};

// ============================================================================
// Image Source
// ============================================================================

/// Source for an image to attach to a message
#[derive(Debug, Clone)]
pub enum ImageSource {
    /// A publicly-accessible URL (HTTPS preferred)
    Url(String),
    /// A local file path
    File(String),
    /// Raw bytes with MIME type
    Data { data: Vec<u8>, mime_type: String },
}

// ============================================================================
// MIME inference
// ============================================================================

/// Infer MIME type from a file extension or URL path
pub fn infer_mime_type(path: &str) -> String {
    let lower = path.to_lowercase();
    if lower.ends_with(".png") {
        "image/png".into()
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg".into()
    } else if lower.ends_with(".gif") {
        "image/gif".into()
    } else if lower.ends_with(".webp") {
        "image/webp".into()
    } else if lower.ends_with(".svg") {
        "image/svg+xml".into()
    } else if lower.ends_with(".bmp") {
        "image/bmp".into()
    } else if lower.ends_with(".ico") {
        "image/x-icon".into()
    } else if lower.ends_with(".tiff") || lower.ends_with(".tif") {
        "image/tiff".into()
    } else if lower.ends_with(".avif") {
        "image/avif".into()
    } else {
        // Default to JPEG for unknown
        "image/jpeg".into()
    }
}

/// Sniff an image MIME type from the actual magic bytes.
///
/// Returns `Some(mime)` only when the bytes confidently match a known image
/// signature; `None` otherwise (caller should fall back to a declared/inferred
/// type). This is the authoritative source for `media_type` — extensions and
/// declared content-types lie, bytes don't (#140: PNG mislabeled as JPEG →
/// Anthropic 400).
pub fn sniff_image_mime(data: &[u8]) -> Option<String> {
    if data.len() < 4 {
        return None;
    }
    match &data[..4] {
        [0xFF, 0xD8, 0xFF, _] => Some("image/jpeg".into()),
        [0x89, 0x50, 0x4E, 0x47] => Some("image/png".into()),
        [0x47, 0x49, 0x46, 0x38] => Some("image/gif".into()),
        [0x52, 0x49, 0x46, 0x46] if data.len() >= 12 && &data[8..12] == b"WEBP" => {
            Some("image/webp".into())
        }
        _ => None,
    }
}

/// Resolve the true image MIME type: prefer byte-sniffing, fall back to the
/// declared/inferred type when the bytes don't match a known signature
/// (e.g. SVG/BMP/TIFF/AVIF which aren't byte-sniffed here).
pub fn resolve_image_mime(data: &[u8], declared: String) -> String {
    match sniff_image_mime(data) {
        Some(sniffed) => {
            if sniffed != declared {
                warn!(
                    declared = %declared,
                    sniffed = %sniffed,
                    "image media_type mismatch — using byte-sniffed type (#140)"
                );
            }
            sniffed
        }
        None => declared,
    }
}

// ============================================================================
// Image loading
// ============================================================================

/// Load an image from any source into an Attachment.
///
/// - `Url`: Downloads the image, infers MIME type from content-type or URL
/// - `File`: Reads from disk, infers MIME type from extension
/// - `Data`: Wraps raw bytes directly
pub async fn load_image(source: ImageSource) -> Result<Attachment> {
    match source {
        ImageSource::Url(url) => load_image_from_url(&url).await,
        ImageSource::File(path) => load_image_from_file(&path),
        ImageSource::Data { data, mime_type } => {
            // #140: trust the bytes, not the declared mime_type.
            let mime_type = resolve_image_mime(&data, mime_type);
            Ok(Attachment::from_data(mime_type, data))
        }
    }
}

/// Download an image from a URL and create an Attachment with base64-ready data.
///
/// The source_url is preserved so providers that support URL references
/// can use it directly without re-encoding.
pub async fn load_image_from_url(url: &str) -> Result<Attachment> {
    debug!(url, "Downloading image for multimodal input");

    let client = Client::new();
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| Error::Llm(format!("Failed to download image from {url}: {e}")))?;

    if !response.status().is_success() {
        return Err(Error::Llm(format!(
            "Image download failed: HTTP {}",
            response.status()
        )));
    }

    // Get MIME type from Content-Type header, fall back to URL inference
    let mime_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|ct| {
            // Strip parameters like "image/jpeg; charset=utf-8"
            ct.split(';').next().unwrap_or(ct).trim().to_string()
        })
        .unwrap_or_else(|| infer_mime_type(url));

    let data = response
        .bytes()
        .await
        .map_err(|e| Error::Llm(format!("Failed to read image bytes: {e}")))?
        .to_vec();

    if data.is_empty() {
        return Err(Error::Llm("Downloaded image is empty".to_string()));
    }

    // #140: servers mislabel content-type too — trust the bytes once we have them.
    let mime_type = resolve_image_mime(&data, mime_type);

    debug!(
        mime_type,
        bytes = data.len(),
        "Image downloaded for multimodal"
    );

    Ok(Attachment {
        mime_type,
        data,
        filename: None,
        source_url: Some(url.to_string()),
    })
}

/// Maximum image file size accepted by `load_image_from_file` (20 MiB).
pub const MAX_IMAGE_FILE_BYTES: u64 = 20 * 1024 * 1024;

/// Sensitive path prefixes that symlinks must not resolve into.
const BLOCKED_SYMLINK_PREFIXES: &[&str] = &[
    "/etc",
    "/private/etc",
    "/proc",
    "/sys",
    "/dev",
    "/boot",
    "/root",
    "/private/var/db",
];

/// Load an image from a local file path.
///
/// # Security
/// - Rejects files larger than [`MAX_IMAGE_FILE_BYTES`] before reading.
/// - Rejects symlinks that resolve into sensitive system directories.
pub fn load_image_from_file(path: &str) -> Result<Attachment> {
    let file_path = Path::new(path);

    if !file_path.exists() {
        return Err(Error::Llm(format!("Image file not found: {path}")));
    }

    // Symlink containment: resolve and check against sensitive prefixes.
    if file_path.is_symlink() {
        match file_path.canonicalize() {
            Ok(canonical) => {
                let canonical_str = canonical.to_string_lossy();
                for prefix in BLOCKED_SYMLINK_PREFIXES {
                    if canonical_str.starts_with(prefix) {
                        return Err(Error::Llm(format!(
                            "Image path '{path}' is a symlink resolving to a sensitive \
                             location: {canonical_str}"
                        )));
                    }
                }
            }
            Err(e) => {
                return Err(Error::Llm(format!(
                    "Failed to resolve symlink '{path}': {e}"
                )));
            }
        }
    }

    // Size check before reading to avoid large allocations.
    let meta = std::fs::metadata(file_path)
        .map_err(|e| Error::Llm(format!("Failed to stat '{path}': {e}")))?;
    if meta.len() > MAX_IMAGE_FILE_BYTES {
        return Err(Error::Llm(format!(
            "Image file '{path}' is too large: {} bytes (max {} bytes)",
            meta.len(),
            MAX_IMAGE_FILE_BYTES
        )));
    }

    let data =
        std::fs::read(file_path).map_err(|e| Error::Llm(format!("Failed to read {path}: {e}")))?;

    if data.is_empty() {
        return Err(Error::Llm(format!("Image file is empty: {path}")));
    }

    // #140: prefer byte-sniffed media_type over the extension guess.
    let mime_type = resolve_image_mime(&data, infer_mime_type(path));
    let filename = file_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string());

    debug!(
        mime_type,
        bytes = data.len(),
        ?filename,
        "Image loaded from file for multimodal"
    );

    Ok(Attachment {
        mime_type,
        data,
        filename,
        source_url: None,
    })
}

/// Resolve a URL-reference attachment by downloading the image data.
///
/// If the attachment already has data, returns it unchanged.
/// If it's a URL reference with no data, downloads and populates data.
pub async fn resolve_attachment(attachment: &Attachment) -> Result<Attachment> {
    if attachment.has_data() {
        return Ok(attachment.clone());
    }

    if let Some(ref url) = attachment.source_url {
        load_image_from_url(url).await
    } else {
        Err(Error::Llm(
            "Attachment has no data and no source URL".to_string(),
        ))
    }
}

/// Validate that an attachment is suitable for vision APIs.
///
/// Checks MIME type is an image and size is within provider limits.
pub fn validate_image_attachment(attachment: &Attachment, max_bytes: usize) -> Result<()> {
    if !attachment.is_image() && !attachment.is_url_ref() {
        warn!(
            mime_type = %attachment.mime_type,
            "Attachment is not an image, vision API may reject it"
        );
    }

    if attachment.has_data() && attachment.data.len() > max_bytes {
        return Err(Error::Llm(format!(
            "Image too large: {} bytes (max: {} bytes)",
            attachment.data.len(),
            max_bytes
        )));
    }

    Ok(())
}

// ============================================================================
// Provider-specific formatting helpers
// ============================================================================

/// Format any attachment for Anthropic's Messages API, dispatching by MIME type.
///
/// Routes to the appropriate content block type:
/// - Images → `"type": "image"` (base64 or URL)
/// - PDFs → `"type": "document"` (base64 or URL)
/// - Other → skipped (caller should handle text extraction separately)
pub fn format_anthropic_attachment(attachment: &Attachment) -> Option<serde_json::Value> {
    if attachment.is_image() {
        Some(format_anthropic_image(attachment))
    } else if attachment.mime_type == "application/pdf" {
        Some(format_anthropic_document(attachment))
    } else {
        // Non-image, non-PDF attachments (audio, text, etc.) should be
        // handled upstream (transcription, text extraction) — not as content blocks.
        None
    }
}

/// Format an image attachment for Anthropic's Messages API.
///
/// Anthropic supports:
/// - `"type": "image"` with `"source": {"type": "base64", "media_type": ..., "data": ...}`
/// - `"type": "image"` with `"source": {"type": "url", "url": ...}` (for public URLs)
pub fn format_anthropic_image(attachment: &Attachment) -> serde_json::Value {
    // Prefer URL source if available and no inline data
    if attachment.is_url_ref()
        && let Some(ref url) = attachment.source_url
    {
        return serde_json::json!({
            "type": "image",
            "source": {
                "type": "url",
                "url": url
            }
        });
    }

    // Fall back to base64
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&attachment.data);
    serde_json::json!({
        "type": "image",
        "source": {
            "type": "base64",
            "media_type": attachment.mime_type,
            "data": b64
        }
    })
}

/// Format a PDF attachment for Anthropic's Messages API.
///
/// Anthropic supports:
/// - `"type": "document"` with `"source": {"type": "base64", "media_type": "application/pdf", "data": ...}`
/// - `"type": "document"` with `"source": {"type": "url", "url": ...}` (for public URLs)
pub fn format_anthropic_document(attachment: &Attachment) -> serde_json::Value {
    // Prefer URL source if available and no inline data
    if attachment.is_url_ref()
        && let Some(ref url) = attachment.source_url
    {
        return serde_json::json!({
            "type": "document",
            "source": {
                "type": "url",
                "url": url
            }
        });
    }

    // Fall back to base64
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&attachment.data);
    serde_json::json!({
        "type": "document",
        "source": {
            "type": "base64",
            "media_type": "application/pdf",
            "data": b64
        }
    })
}

/// Format any attachment for OpenAI's Chat API, dispatching by MIME type.
///
/// Routes to the appropriate content part:
/// - Images → `"type": "image_url"` (URL or data URI)
/// - Other → skipped (handled upstream via text extraction)
pub fn format_openai_attachment(attachment: &Attachment) -> Option<serde_json::Value> {
    if attachment.is_image() {
        Some(format_openai_image(attachment))
    } else {
        // OpenAI doesn't natively support document/audio content blocks.
        // PDFs and text files should be extracted upstream.
        None
    }
}

/// Format any attachment for Google Gemini API, dispatching by MIME type.
///
/// Routes to the appropriate content part:
/// - Images → `"inlineData"` (base64)
/// - Other → skipped (handled upstream via text extraction)
pub fn format_gemini_attachment(attachment: &Attachment) -> Option<serde_json::Value> {
    if attachment.is_image() {
        Some(format_gemini_image(attachment))
    } else {
        // Gemini supports inline data for images; other types handled upstream.
        None
    }
}

/// Format an attachment for OpenAI's Chat API (GPT-4 Vision).
///
/// OpenAI supports:
/// - `"type": "image_url"` with `"image_url": {"url": "https://...", "detail": "auto"}`
/// - `"type": "image_url"` with `"image_url": {"url": "data:image/...;base64,..."}`
pub fn format_openai_image(attachment: &Attachment) -> serde_json::Value {
    // Prefer base64 data when available — works with all providers.
    // URL-only is a fallback for when data wasn't downloaded.
    if attachment.has_data() {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&attachment.data);
        let data_url = format!("data:{};base64,{}", attachment.mime_type, b64);
        return serde_json::json!({
            "type": "image_url",
            "image_url": { "url": data_url, "detail": "auto" }
        });
    }

    // Fall back to direct URL if no data (URL-only ref)
    if let Some(ref url) = attachment.source_url {
        return serde_json::json!({
            "type": "image_url",
            "image_url": { "url": url, "detail": "auto" }
        });
    }

    // Empty attachment — return empty data URL
    serde_json::json!({
        "type": "image_url",
        "image_url": { "url": "data:image/png;base64,", "detail": "auto" }
    })
}

/// Format an attachment for Google Gemini API.
///
/// Gemini supports:
/// - `"inlineData": {"mimeType": ..., "data": ...}` (base64)
/// - `"fileData": {"mimeType": ..., "fileUri": ...}` (Google Cloud Storage URIs)
pub fn format_gemini_image(attachment: &Attachment) -> serde_json::Value {
    // Gemini only supports inline base64 or GCS URIs — use inline for general URLs
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&attachment.data);
    serde_json::json!({
        "inlineData": {
            "mimeType": attachment.mime_type,
            "data": b64
        }
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_infer_mime_type_common() {
        assert_eq!(infer_mime_type("photo.png"), "image/png");
        assert_eq!(infer_mime_type("photo.jpg"), "image/jpeg");
        assert_eq!(infer_mime_type("photo.jpeg"), "image/jpeg");
        assert_eq!(infer_mime_type("photo.gif"), "image/gif");
        assert_eq!(infer_mime_type("photo.webp"), "image/webp");
        assert_eq!(infer_mime_type("photo.svg"), "image/svg+xml");
        assert_eq!(infer_mime_type("photo.bmp"), "image/bmp");
        assert_eq!(infer_mime_type("photo.avif"), "image/avif");
    }

    #[test]
    fn test_infer_mime_type_case_insensitive() {
        assert_eq!(infer_mime_type("PHOTO.PNG"), "image/png");
        assert_eq!(infer_mime_type("image.JPG"), "image/jpeg");
    }

    #[test]
    fn test_infer_mime_type_unknown_defaults_jpeg() {
        assert_eq!(infer_mime_type("file.xyz"), "image/jpeg");
        assert_eq!(infer_mime_type("no_extension"), "image/jpeg");
    }

    // ---- #140: byte-sniffing regression guards ----

    // Minimal valid magic-byte headers per format.
    const PNG_MAGIC: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    const JPEG_MAGIC: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
    const GIF_MAGIC: &[u8] = b"GIF89a-data";
    const WEBP_MAGIC: &[u8] = b"RIFF\x00\x00\x00\x00WEBPVP8 ";

    #[test]
    fn test_sniff_image_mime_known_signatures() {
        assert_eq!(sniff_image_mime(PNG_MAGIC).as_deref(), Some("image/png"));
        assert_eq!(sniff_image_mime(JPEG_MAGIC).as_deref(), Some("image/jpeg"));
        assert_eq!(sniff_image_mime(GIF_MAGIC).as_deref(), Some("image/gif"));
        assert_eq!(sniff_image_mime(WEBP_MAGIC).as_deref(), Some("image/webp"));
    }

    #[test]
    fn test_sniff_image_mime_unknown_returns_none() {
        assert_eq!(sniff_image_mime(&[0x00, 0x01, 0x02, 0x03]), None);
        assert_eq!(sniff_image_mime(b"<svg></svg>"), None); // SVG isn't byte-sniffed
        assert_eq!(sniff_image_mime(&[0xFF]), None); // too short
    }

    #[test]
    fn test_resolve_image_mime_overrides_wrong_declared() {
        // The #140 bug exactly: PNG bytes declared as JPEG → must resolve to PNG.
        assert_eq!(resolve_image_mime(PNG_MAGIC, "image/jpeg".into()), "image/png");
        // JPEG bytes mislabeled as PNG → must resolve to JPEG.
        assert_eq!(
            resolve_image_mime(JPEG_MAGIC, "image/png".into()),
            "image/jpeg"
        );
    }

    #[test]
    fn test_resolve_image_mime_falls_back_when_unsniffable() {
        // SVG bytes can't be magic-sniffed → keep the declared/inferred type.
        assert_eq!(
            resolve_image_mime(b"<svg></svg>", "image/svg+xml".into()),
            "image/svg+xml"
        );
    }

    #[test]
    fn test_resolve_image_mime_agrees_when_correct() {
        assert_eq!(resolve_image_mime(PNG_MAGIC, "image/png".into()), "image/png");
    }

    #[tokio::test]
    async fn test_load_image_data_branch_sniffs_bytes() {
        // #140 end-to-end: Data source with a lying mime_type → corrected.
        let att = load_image(ImageSource::Data {
            data: PNG_MAGIC.to_vec(),
            mime_type: "image/jpeg".into(),
        })
        .await
        .expect("data image loads");
        assert_eq!(att.mime_type, "image/png", "must sniff PNG over declared JPEG");
    }

    #[test]
    fn test_infer_mime_type_url_path() {
        assert_eq!(
            infer_mime_type("https://example.com/images/cat.png"),
            "image/png"
        );
        assert_eq!(
            infer_mime_type("https://example.com/photo.webp?size=large"),
            // URL query params don't affect extension matching — falls through
            "image/jpeg"
        );
    }

    #[test]
    fn test_attachment_from_data() {
        let att = Attachment::from_data("image/png", vec![0x89, 0x50, 0x4e, 0x47]);
        assert_eq!(att.mime_type, "image/png");
        assert!(att.has_data());
        assert!(!att.is_url_ref());
        assert!(att.is_image());
        assert!(att.source_url.is_none());
    }

    #[test]
    fn test_attachment_from_url() {
        let att = Attachment::from_url("https://example.com/cat.png", "image/png");
        assert_eq!(att.mime_type, "image/png");
        assert!(!att.has_data());
        assert!(att.is_url_ref());
        assert!(att.is_image());
        assert_eq!(
            att.source_url.as_deref(),
            Some("https://example.com/cat.png")
        );
    }

    #[test]
    fn test_format_anthropic_url_ref() {
        let att = Attachment::from_url("https://example.com/cat.png", "image/png");
        let json = format_anthropic_image(&att);
        assert_eq!(json["type"], "image");
        assert_eq!(json["source"]["type"], "url");
        assert_eq!(json["source"]["url"], "https://example.com/cat.png");
    }

    #[test]
    fn test_format_anthropic_base64() {
        let att = Attachment::from_data("image/jpeg", vec![0xFF, 0xD8, 0xFF]);
        let json = format_anthropic_image(&att);
        assert_eq!(json["type"], "image");
        assert_eq!(json["source"]["type"], "base64");
        assert_eq!(json["source"]["media_type"], "image/jpeg");
        assert!(json["source"]["data"].as_str().is_some());
    }

    #[test]
    fn test_format_openai_url_ref() {
        let att = Attachment::from_url("https://example.com/dog.jpg", "image/jpeg");
        let json = format_openai_image(&att);
        assert_eq!(json["type"], "image_url");
        assert_eq!(json["image_url"]["url"], "https://example.com/dog.jpg");
        assert_eq!(json["image_url"]["detail"], "auto");
    }

    #[test]
    fn test_format_openai_base64() {
        let att = Attachment::from_data("image/png", vec![0x89, 0x50]);
        let json = format_openai_image(&att);
        assert_eq!(json["type"], "image_url");
        let url = json["image_url"]["url"].as_str().unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn test_format_gemini_base64() {
        let att = Attachment::from_data("image/webp", vec![0x52, 0x49]);
        let json = format_gemini_image(&att);
        assert_eq!(json["inlineData"]["mimeType"], "image/webp");
        assert!(json["inlineData"]["data"].as_str().is_some());
    }

    #[test]
    fn test_validate_image_ok() {
        let att = Attachment::from_data("image/png", vec![0; 100]);
        assert!(validate_image_attachment(&att, 1_000_000).is_ok());
    }

    #[test]
    fn test_validate_image_too_large() {
        let att = Attachment::from_data("image/png", vec![0; 50_000_000]);
        assert!(validate_image_attachment(&att, 20_000_000).is_err());
    }

    #[test]
    fn test_validate_url_ref_skips_size_check() {
        let att = Attachment::from_url("https://example.com/huge.png", "image/png");
        // URL refs have no data, so size check is skipped
        assert!(validate_image_attachment(&att, 100).is_ok());
    }

    #[test]
    fn test_load_image_from_file_not_found() {
        let result = load_image_from_file("/nonexistent/path/image.png");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_resolve_attachment_with_data() {
        let att = Attachment::from_data("image/png", vec![1, 2, 3]);
        let resolved = resolve_attachment(&att).await.unwrap();
        assert_eq!(resolved.data, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn test_resolve_attachment_no_data_no_url() {
        let att = Attachment {
            mime_type: "image/png".into(),
            data: vec![],
            filename: None,
            source_url: None,
        };
        assert!(resolve_attachment(&att).await.is_err());
    }

    #[test]
    fn test_attachment_is_image() {
        assert!(Attachment::from_data("image/png", vec![1]).is_image());
        assert!(Attachment::from_data("image/jpeg", vec![1]).is_image());
        assert!(!Attachment::from_data("audio/mp3", vec![1]).is_image());
        assert!(!Attachment::from_data("application/pdf", vec![1]).is_image());
    }

    #[test]
    fn test_load_image_size_limit_enforced() {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().expect("tempfile");
        // Write MAX_IMAGE_FILE_BYTES + 1 bytes
        let oversized = vec![0u8; (MAX_IMAGE_FILE_BYTES + 1) as usize];
        tmp.write_all(&oversized).expect("write");
        let path = tmp.path().to_string_lossy().to_string();
        let result = load_image_from_file(&path);
        assert!(result.is_err(), "oversized image should be rejected");
        assert!(
            result.unwrap_err().to_string().contains("too large"),
            "error should mention size"
        );
    }

    #[test]
    fn test_load_image_size_limit_accepted() {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().expect("tempfile");
        // Write a small valid PNG-ish file (just needs to be non-empty and under limit)
        tmp.write_all(&[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])
            .expect("write");
        let path = tmp.path().to_string_lossy().to_string();
        let result = load_image_from_file(&path);
        assert!(result.is_ok(), "small image should be accepted");
    }

    #[test]
    #[cfg(unix)]
    fn test_load_image_symlink_to_etc_rejected() {
        use std::os::unix::fs::symlink;
        let tmp_dir = tempfile::tempdir().expect("tmpdir");
        let link_path = tmp_dir.path().join("link_to_passwd");
        // Create a symlink pointing at /etc/hosts (blocked prefix)
        symlink("/etc/hosts", &link_path).expect("symlink");
        let path = link_path.to_string_lossy().to_string();
        let result = load_image_from_file(&path);
        assert!(result.is_err(), "symlink into /etc should be rejected");
        assert!(
            result.unwrap_err().to_string().contains("sensitive"),
            "error should mention sensitive location"
        );
    }
}
