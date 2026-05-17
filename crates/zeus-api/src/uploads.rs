//! File Upload Management
//!
//! Handles file uploads, storage, metadata, and retrieval.
//! Supports: PDF, Markdown, Text, HTML, DOCX, Excel, Images (PNG, JPG, GIF, WebP)

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use zeus_channels::media_extract;

const MIME_DOCX: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
const MIME_PPTX: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.presentation";
const MIME_XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
const MIME_XLS: &str = "application/vnd.ms-excel";

/// File upload metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadedFile {
    /// Unique file ID
    pub id: String,
    /// Original filename
    pub name: String,
    /// File size in bytes
    pub size: u64,
    /// MIME type
    pub mime_type: String,
    /// File extension
    pub extension: String,
    /// Upload timestamp
    pub uploaded_at: DateTime<Utc>,
    /// Storage path (relative to uploads dir)
    pub storage_path: String,
    /// Optional thumbnail path for images
    pub thumbnail_path: Option<String>,
    /// Extracted text (for PDF, DOCX)
    pub extracted_text: Option<String>,
    /// Additional metadata
    pub metadata: HashMap<String, String>,
}

/// Upload store managing files and metadata
pub struct UploadStore {
    /// Base directory for uploads
    uploads_dir: PathBuf,
    /// Metadata store (in-memory, could be persisted to JSON)
    metadata: HashMap<String, UploadedFile>,
    /// Metadata file path
    metadata_file: PathBuf,
}

impl UploadStore {
    /// Create a new upload store
    pub fn new(base_dir: &Path) -> Result<Self> {
        let uploads_dir = base_dir.join("uploads");
        let metadata_file = uploads_dir.join("metadata.json");

        // Create uploads directory if it doesn't exist
        fs::create_dir_all(&uploads_dir).context("Failed to create uploads directory")?;

        // Load existing metadata
        let metadata = if metadata_file.exists() {
            let data =
                fs::read_to_string(&metadata_file).context("Failed to read metadata file")?;
            serde_json::from_str(&data).unwrap_or_else(|_| HashMap::new())
        } else {
            HashMap::new()
        };

        Ok(Self {
            uploads_dir,
            metadata,
            metadata_file,
        })
    }

    /// Save a file with automatic ID generation and metadata
    pub fn save_file(
        &mut self,
        original_name: &str,
        content: &[u8],
        mime_type: &str,
    ) -> Result<UploadedFile> {
        let id = Uuid::new_v4().to_string();
        let extension = Path::new(original_name)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("bin")
            .to_string();

        let storage_filename = format!("{}.{}", id, extension);
        let storage_path = self.uploads_dir.join(&storage_filename);

        // Write file to disk
        fs::write(&storage_path, content).context("Failed to write uploaded file")?;

        let size = content.len() as u64;

        // Generate thumbnail for images
        let thumbnail_path = if mime_type.starts_with("image/") {
            self.generate_thumbnail(&id, content, &extension).ok()
        } else {
            None
        };

        // Extract text for supported document types
        let extracted_text = match mime_type {
            "application/pdf" => self.extract_pdf_text(&storage_path).ok(),
            MIME_DOCX | MIME_PPTX | MIME_XLSX | MIME_XLS => {
                media_extract::extract_text_from_doc(content, mime_type).ok()
            }
            "text/plain" | "text/markdown" | "text/html" => {
                String::from_utf8(content.to_vec()).ok()
            }
            _ => None,
        };

        let uploaded_file = UploadedFile {
            id: id.clone(),
            name: original_name.to_string(),
            size,
            mime_type: mime_type.to_string(),
            extension,
            uploaded_at: Utc::now(),
            storage_path: storage_filename,
            thumbnail_path,
            extracted_text,
            metadata: HashMap::new(),
        };

        // Store metadata
        self.metadata.insert(id.clone(), uploaded_file.clone());
        self.persist_metadata()?;

        Ok(uploaded_file)
    }

    /// Get file metadata by ID
    pub fn get_metadata(&self, id: &str) -> Option<&UploadedFile> {
        self.metadata.get(id)
    }

    /// Get file content by ID
    pub fn get_file_content(&self, id: &str) -> Result<Vec<u8>> {
        let metadata = self.metadata.get(id).context("File not found")?;

        let path = self.uploads_dir.join(&metadata.storage_path);
        fs::read(&path).context("Failed to read file")
    }

    /// Get thumbnail content by ID
    pub fn get_thumbnail_content(&self, id: &str) -> Result<Vec<u8>> {
        let metadata = self.metadata.get(id).context("File not found")?;

        let thumbnail_path = metadata
            .thumbnail_path
            .as_ref()
            .context("No thumbnail available")?;

        let path = self.uploads_dir.join(thumbnail_path);
        fs::read(&path).context("Failed to read thumbnail")
    }

    /// Delete file by ID
    pub fn delete_file(&mut self, id: &str) -> Result<()> {
        let metadata = self.metadata.remove(id).context("File not found")?;

        // Delete main file
        let path = self.uploads_dir.join(&metadata.storage_path);
        if path.exists() {
            fs::remove_file(&path).context("Failed to delete file")?;
        }

        // Delete thumbnail if exists
        if let Some(thumb_path) = &metadata.thumbnail_path {
            let thumb_full_path = self.uploads_dir.join(thumb_path);
            if thumb_full_path.exists() {
                let _ = fs::remove_file(&thumb_full_path);
            }
        }

        self.persist_metadata()?;
        Ok(())
    }

    /// List all uploaded files
    pub fn list_files(&self) -> Vec<UploadedFile> {
        let mut files: Vec<_> = self.metadata.values().cloned().collect();
        files.sort_by(|a, b| b.uploaded_at.cmp(&a.uploaded_at));
        files
    }

    /// Persist metadata to JSON file
    fn persist_metadata(&self) -> Result<()> {
        let json =
            serde_json::to_string_pretty(&self.metadata).context("Failed to serialize metadata")?;
        fs::write(&self.metadata_file, json).context("Failed to write metadata file")
    }

    /// Generate thumbnail for image (200x200 max)
    fn generate_thumbnail(&self, id: &str, content: &[u8], extension: &str) -> Result<String> {
        use image::ImageFormat;

        let format = match extension.to_lowercase().as_str() {
            "jpg" | "jpeg" => ImageFormat::Jpeg,
            "png" => ImageFormat::Png,
            "gif" => ImageFormat::Gif,
            "webp" => ImageFormat::WebP,
            _ => return Err(anyhow::anyhow!("Unsupported image format")),
        };

        let img = image::load_from_memory(content).context("Failed to load image")?;

        let thumbnail = img.thumbnail(200, 200);

        let thumb_filename = format!("{}_thumb.{}", id, extension);
        let thumb_path = self.uploads_dir.join(&thumb_filename);

        thumbnail
            .save_with_format(&thumb_path, format)
            .context("Failed to save thumbnail")?;

        Ok(thumb_filename)
    }

    /// Extract text from PDF using lopdf.
    fn extract_pdf_text(&self, path: &Path) -> Result<String> {
        let doc = lopdf::Document::load(path).context("Failed to load PDF document")?;
        let pages = doc.get_pages();
        if pages.is_empty() {
            return Ok(String::new());
        }
        let page_nums: Vec<u32> = pages.keys().copied().collect();
        let text = doc
            .extract_text(&page_nums)
            .context("Failed to extract text from PDF pages")?;
        Ok(text)
    }
}

/// Detect MIME type from file content (magic bytes) or extension
pub fn detect_mime_type(filename: &str, content: &[u8]) -> String {
    // Check magic bytes first
    if content.len() >= 4 {
        match &content[0..4] {
            [0x25, 0x50, 0x44, 0x46] => return "application/pdf".to_string(),
            [0x50, 0x4B, 0x03, 0x04] => {
                // ZIP-based OOXML format, disambiguate by extension
                let lower = filename.to_lowercase();
                if lower.ends_with(".docx") {
                    return MIME_DOCX.to_string();
                } else if lower.ends_with(".xlsx") {
                    return MIME_XLSX.to_string();
                } else if lower.ends_with(".pptx") {
                    return MIME_PPTX.to_string();
                }
            }
            [0xD0, 0xCF, 0x11, 0xE0] => {
                // Legacy OLE Compound File Binary (XLS, DOC, PPT). Disambiguate by extension.
                let lower = filename.to_lowercase();
                if lower.ends_with(".xls") {
                    return MIME_XLS.to_string();
                }
            }
            [0x89, 0x50, 0x4E, 0x47] => return "image/png".to_string(),
            [0xFF, 0xD8, 0xFF, ..] => return "image/jpeg".to_string(),
            _ => {}
        }
    }

    if content.len() >= 6 && (&content[0..6] == b"GIF87a" || &content[0..6] == b"GIF89a") {
        return "image/gif".to_string();
    }

    if content.len() >= 12 && &content[0..4] == b"RIFF" && &content[8..12] == b"WEBP" {
        return "image/webp".to_string();
    }

    // Fallback to extension-based detection
    let extension = Path::new(filename)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    match extension.to_lowercase().as_str() {
        "pdf" => "application/pdf".to_string(),
        "docx" => MIME_DOCX.to_string(),
        "xlsx" => MIME_XLSX.to_string(),
        "pptx" => MIME_PPTX.to_string(),
        "xls" => MIME_XLS.to_string(),
        "png" => "image/png".to_string(),
        "jpg" | "jpeg" => "image/jpeg".to_string(),
        "gif" => "image/gif".to_string(),
        "webp" => "image/webp".to_string(),
        "txt" => "text/plain".to_string(),
        "md" => "text/markdown".to_string(),
        "html" | "htm" => "text/html".to_string(),
        _ => "application/octet-stream".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_upload_store_creation() {
        let temp_dir = std::env::temp_dir().join("zeus-test-uploads");
        let _ = fs::remove_dir_all(&temp_dir);

        let _store = UploadStore::new(&temp_dir).unwrap();
        assert!(temp_dir.join("uploads").exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_save_and_retrieve_file() {
        let temp_dir = std::env::temp_dir().join("zeus-test-uploads-2");
        let _ = fs::remove_dir_all(&temp_dir);

        let mut store = UploadStore::new(&temp_dir).unwrap();

        let content = b"Hello, Zeus!";
        let uploaded = store.save_file("test.txt", content, "text/plain").unwrap();

        assert_eq!(uploaded.name, "test.txt");
        assert_eq!(uploaded.size, content.len() as u64);
        assert_eq!(uploaded.mime_type, "text/plain");

        let retrieved = store.get_file_content(&uploaded.id).unwrap();
        assert_eq!(retrieved, content);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_detect_mime_type() {
        assert_eq!(
            detect_mime_type("test.pdf", &[0x25, 0x50, 0x44, 0x46]),
            "application/pdf"
        );
        assert_eq!(
            detect_mime_type("test.png", &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A]),
            "image/png"
        );
        assert_eq!(detect_mime_type("test.txt", b"hello"), "text/plain");
    }

    #[test]
    fn test_detect_mime_type_html() {
        assert_eq!(
            detect_mime_type("test.html", b"<html></html>"),
            "text/html"
        );
        assert_eq!(detect_mime_type("test.htm", b"<html></html>"), "text/html");
    }

    #[test]
    fn test_delete_file() {
        let temp_dir = std::env::temp_dir().join("zeus-test-uploads-3");
        let _ = fs::remove_dir_all(&temp_dir);

        let mut store = UploadStore::new(&temp_dir).unwrap();

        let content = b"Delete me";
        let uploaded = store
            .save_file("delete.txt", content, "text/plain")
            .unwrap();

        assert!(store.get_metadata(&uploaded.id).is_some());

        store.delete_file(&uploaded.id).unwrap();
        assert!(store.get_metadata(&uploaded.id).is_none());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_list_files() {
        let temp_dir = std::env::temp_dir().join("zeus-test-uploads-4");
        let _ = fs::remove_dir_all(&temp_dir);

        let mut _store = UploadStore::new(&temp_dir).unwrap();

        _store.save_file("file1.txt", b"one", "text/plain").unwrap();
        _store.save_file("file2.txt", b"two", "text/plain").unwrap();

        let files = _store.list_files();
        assert_eq!(files.len(), 2);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
