//! Upload API Handlers

use axum::{
    Json,
    extract::{Multipart, Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::IntoResponse,
};
use serde::Serialize;
use tracing::{debug, error};

use crate::SharedState;
use crate::uploads::{UploadedFile, detect_mime_type};

/// Response for file upload
#[derive(Debug, Serialize)]
pub struct UploadResponse {
    pub id: String,
    pub name: String,
    pub size: u64,
    #[serde(rename = "type")]
    pub mime_type: String,
    pub url: String,
    pub thumbnail_url: Option<String>,
    /// Extracted text content (PDF, DOCX, plain text, markdown).
    /// Populated server-side; `None` for binary/image uploads.
    pub extracted_text: Option<String>,
}

impl From<UploadedFile> for UploadResponse {
    fn from(file: UploadedFile) -> Self {
        let thumbnail_url = file
            .thumbnail_path
            .as_ref()
            .map(|_| format!("/v1/uploads/{}/thumbnail", file.id));

        Self {
            id: file.id.clone(),
            name: file.name,
            size: file.size,
            mime_type: file.mime_type,
            url: format!("/v1/uploads/{}", file.id),
            thumbnail_url,
            extracted_text: file.extracted_text,
        }
    }
}

/// POST /v1/uploads - Upload a file
pub async fn upload_file(
    State(state): State<SharedState>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<UploadResponse>), (StatusCode, String)> {
    debug!("Processing file upload");

    let mut state_write = state.write().await;

    // Process first file from multipart (single file upload)
    if let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid multipart: {}", e)))?
    {
        let name = field
            .file_name()
            .ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    "Missing filename in multipart field".to_string(),
                )
            })?
            .to_string();

        let content = field.bytes().await.map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                format!("Failed to read file: {}", e),
            )
        })?;

        // Detect MIME type
        let mime_type = detect_mime_type(&name, &content);

        // Save file
        let uploaded_file = state_write
            .upload_store
            .save_file(&name, &content, &mime_type)
            .map_err(|e| {
                error!("Failed to save file: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to save file: {}", e),
                )
            })?;

        debug!(
            "File uploaded: {} ({})",
            uploaded_file.name, uploaded_file.id
        );

        return Ok((StatusCode::CREATED, Json(uploaded_file.into())));
    }

    Err((StatusCode::BAD_REQUEST, "No file provided".to_string()))
}

/// GET /v1/uploads/:id - Get file metadata
pub async fn get_upload_metadata(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<UploadedFile>, (StatusCode, String)> {
    let state_read = state.read().await;

    let metadata = state_read
        .upload_store
        .get_metadata(&id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, "File not found".to_string()))?;

    Ok(Json(metadata.clone()))
}

/// GET /v1/uploads/:id/download - Download file content
pub async fn download_file(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let state_read = state.read().await;

    let metadata = state_read
        .upload_store
        .get_metadata(&id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, "File not found".to_string()))?
        .clone();

    let content = state_read.upload_store.get_file_content(&id).map_err(|e| {
        error!("Failed to read file: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to read file: {}", e),
        )
    })?;

    let mime_type = metadata.mime_type.clone();
    let filename = metadata.name.clone();

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&mime_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{}\"", filename))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
    );

    Ok((StatusCode::OK, headers, content))
}

/// GET /v1/uploads/:id/thumbnail - Get thumbnail for image
pub async fn get_thumbnail(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let state_read = state.read().await;

    let metadata = state_read
        .upload_store
        .get_metadata(&id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, "File not found".to_string()))?;

    if metadata.thumbnail_path.is_none() {
        return Err((
            StatusCode::NOT_FOUND,
            "No thumbnail available for this file".to_string(),
        ));
    }

    let content = state_read
        .upload_store
        .get_thumbnail_content(&id)
        .map_err(|e| {
            error!("Failed to read thumbnail: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read thumbnail: {}", e),
            )
        })?;

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));

    Ok((StatusCode::OK, headers, content))
}

/// DELETE /v1/uploads/:id - Delete file
pub async fn delete_upload(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut state_write = state.write().await;

    state_write.upload_store.delete_file(&id).map_err(|e| {
        error!("Failed to delete file: {}", e);
        if e.to_string().contains("not found") {
            (StatusCode::NOT_FOUND, "File not found".to_string())
        } else {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to delete file: {}", e),
            )
        }
    })?;

    debug!("File deleted: {}", id);
    Ok(StatusCode::NO_CONTENT)
}

/// GET /v1/uploads - List all uploads
pub async fn list_uploads(
    State(state): State<SharedState>,
) -> Result<Json<Vec<UploadResponse>>, (StatusCode, String)> {
    let state_read = state.read().await;

    let files: Vec<UploadResponse> = state_read
        .upload_store
        .list_files()
        .into_iter()
        .map(|f| f.into())
        .collect();

    Ok(Json(files))
}
