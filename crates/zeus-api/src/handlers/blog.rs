//! Blog CMS API handlers.
//!
//! Manages blog posts stored as markdown files with YAML frontmatter
//! in `~/.zeus/workspace/blog/`. Provides CRUD endpoints + image upload
//! for the ZeusWeb marketing site blog.

use axum::{
    Json,
    extract::{Multipart, Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::IntoResponse,
};
use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use tracing::{info, warn};
use uuid::Uuid;

use crate::SharedState;

// ============================================================================
// Constants
// ============================================================================

/// Maximum blog post content size (2 MB per Miguel directive).
const MAX_CONTENT_SIZE: usize = 2_097_152;

/// Maximum image upload size (10 MB).
const MAX_IMAGE_SIZE: usize = 10_485_760;

/// Allowed image extensions for blog media uploads.
const ALLOWED_IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg"];

// ============================================================================
// Types
// ============================================================================

/// Blog post metadata parsed from YAML frontmatter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlogPostMeta {
    pub title: String,
    #[serde(default = "default_author")]
    pub author: String,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub excerpt: Option<String>,
    #[serde(default)]
    pub cover_image: Option<String>,
    #[serde(default = "default_true")]
    pub published: bool,
}

fn default_author() -> String {
    "Zeus Team".into()
}
fn default_true() -> bool {
    true
}

/// Full blog post (metadata + content).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlogPost {
    pub slug: String,
    #[serde(flatten)]
    pub meta: BlogPostMeta,
    pub content_md: String,
    pub content_html: String,
}

/// Summary for listing pages (no full content).
#[derive(Debug, Clone, Serialize)]
pub struct BlogPostSummary {
    pub slug: String,
    pub title: String,
    pub author: String,
    pub date: Option<String>,
    pub tags: Vec<String>,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
}

/// Request body for creating/updating a blog post.
#[derive(Debug, Deserialize)]
pub struct BlogPostRequest {
    pub slug: Option<String>,
    pub title: String,
    #[serde(default = "default_author")]
    pub author: String,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub excerpt: Option<String>,
    #[serde(default)]
    pub cover_image: Option<String>,
    #[serde(default = "default_true")]
    pub published: bool,
    /// Markdown content body
    pub content: String,
}

// ============================================================================
// Helpers
// ============================================================================

/// Get the blog directory path from config workspace.
fn blog_dir(state: &crate::AppState) -> PathBuf {
    state.config.workspace.join("blog")
}

/// Get the blog media directory path.
fn media_dir(state: &crate::AppState) -> PathBuf {
    state.config.workspace.join("blog").join("img")
}

/// Validate a slug is safe (no path traversal, only alphanumeric + hyphens).
fn is_safe_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 200
        && !slug.contains("..")
        && !slug.contains('/')
        && !slug.contains('\\')
        && !slug.contains('\0')
        && !slug.starts_with('.')
        && !slug.starts_with('-')
        && slug.chars().all(|c| c.is_alphanumeric() || c == '-')
}

/// Validate a filename is safe for filesystem use.
fn is_safe_filename(name: &str) -> bool {
    !name.is_empty()
        && !name.contains("..")
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
        && !name.starts_with('.')
}

/// Convert a title to a URL-safe slug.
fn slugify(title: &str) -> String {
    let slug: String = title
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    // Truncate to 200 chars at a safe boundary
    if slug.len() > 200 {
        slug[..200].trim_end_matches('-').to_string()
    } else {
        slug
    }
}

/// Parse YAML frontmatter and markdown body from a file's content.
fn parse_frontmatter(content: &str) -> Option<(BlogPostMeta, String)> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }

    let after_first = &trimmed[3..];
    let end_idx = after_first.find("\n---")?;
    let yaml_str = &after_first[..end_idx];
    let body = after_first[end_idx + 4..].trim_start().to_string();

    let meta: BlogPostMeta = serde_yaml::from_str(yaml_str).ok()?;
    Some((meta, body))
}

/// Render markdown to HTML using pulldown-cmark.
fn render_markdown(md: &str) -> String {
    use pulldown_cmark::{Options, Parser, html};
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(md, options);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    html_output
}

/// Serialize a blog post to frontmatter + markdown.
fn to_frontmatter_md(meta: &BlogPostMeta, content: &str) -> String {
    let yaml = serde_yaml::to_string(meta).unwrap_or_default();
    format!("---\n{}---\n\n{}\n", yaml, content)
}

/// Load a single blog post from its slug.
fn load_post(dir: &std::path::Path, slug: &str) -> Option<BlogPost> {
    if !is_safe_slug(slug) {
        return None;
    }
    let file_path = dir.join(format!("{}.md", slug));
    let content = std::fs::read_to_string(&file_path).ok()?;
    let (meta, body) = parse_frontmatter(&content)?;
    Some(BlogPost {
        slug: slug.to_string(),
        meta,
        content_html: render_markdown(&body),
        content_md: body,
    })
}

/// Load all blog posts from the blog directory, sorted by date descending.
fn load_all_posts(dir: &std::path::Path) -> Vec<BlogPost> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut posts: Vec<BlogPost> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|ext| ext == "md").unwrap_or(false))
        .filter_map(|e| {
            let slug = e.path().file_stem()?.to_str()?.to_string();
            load_post(dir, &slug)
        })
        .collect();

    // Sort by date descending (newest first)
    posts.sort_by(|a, b| {
        let da = a
            .meta
            .date
            .as_deref()
            .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
        let db = b
            .meta
            .date
            .as_deref()
            .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
        db.cmp(&da)
    });

    posts
}

// ============================================================================
// Handlers
// ============================================================================

/// GET /v1/blog/posts — List all published blog posts.
pub async fn list_posts(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;
    let dir = blog_dir(&state);

    let posts = load_all_posts(&dir);
    let summaries: Vec<BlogPostSummary> = posts
        .into_iter()
        .filter(|p| p.meta.published)
        .map(|p| BlogPostSummary {
            slug: p.slug,
            title: p.meta.title,
            author: p.meta.author,
            date: p.meta.date,
            tags: p.meta.tags,
            excerpt: p.meta.excerpt,
            cover_image: p.meta.cover_image,
        })
        .collect();

    let count = summaries.len();
    Json(json!({
        "posts": summaries,
        "count": count,
    }))
}

/// GET /v1/blog/posts/:slug — Get a single blog post by slug.
pub async fn get_post(
    State(state): State<SharedState>,
    Path(slug): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if !is_safe_slug(&slug) {
        warn!(slug = %slug, "Rejected invalid blog slug");
        return Err((StatusCode::BAD_REQUEST, "Invalid slug".into()));
    }

    let state = state.read().await;
    let dir = blog_dir(&state);

    let post = load_post(&dir, &slug)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Post '{}' not found", slug)))?;

    Ok(Json(json!({
        "slug": post.slug,
        "title": post.meta.title,
        "author": post.meta.author,
        "date": post.meta.date,
        "tags": post.meta.tags,
        "excerpt": post.meta.excerpt,
        "cover_image": post.meta.cover_image,
        "published": post.meta.published,
        "content_md": post.content_md,
        "content_html": post.content_html,
    })))
}

/// POST /v1/blog/posts — Create a new blog post.
pub async fn create_post(
    State(state): State<SharedState>,
    Json(req): Json<BlogPostRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    // Validate title
    if req.title.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Title cannot be empty".into()));
    }
    if req.title.len() > 200 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Title too long (max 200 chars)".into(),
        ));
    }
    // Validate content size (2 MB max)
    if req.content.len() > MAX_CONTENT_SIZE {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "Content too large ({} bytes, max {})",
                req.content.len(),
                MAX_CONTENT_SIZE
            ),
        ));
    }

    let state = state.read().await;
    let dir = blog_dir(&state);

    // Ensure blog directory exists
    std::fs::create_dir_all(&dir).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create blog dir: {}", e),
        )
    })?;

    let slug = req.slug.unwrap_or_else(|| slugify(&req.title));

    // Validate slug safety
    if !is_safe_slug(&slug) {
        warn!(slug = %slug, "Rejected invalid blog slug on create");
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid slug (alphanumeric and hyphens only)".into(),
        ));
    }

    // Check for duplicate
    let file_path = dir.join(format!("{}.md", slug));
    if file_path.exists() {
        return Err((
            StatusCode::CONFLICT,
            format!("Post '{}' already exists", slug),
        ));
    }

    let date = req
        .date
        .unwrap_or_else(|| Utc::now().format("%Y-%m-%d").to_string());
    let meta = BlogPostMeta {
        title: req.title.clone(),
        author: req.author,
        date: Some(date),
        tags: req.tags,
        excerpt: req.excerpt,
        cover_image: req.cover_image,
        published: req.published,
    };

    let file_content = to_frontmatter_md(&meta, &req.content);
    std::fs::write(&file_path, &file_content).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to write post: {}", e),
        )
    })?;

    info!(slug = %slug, title = %req.title, "Blog post created");

    Ok(Json(json!({
        "created": true,
        "slug": slug,
        "title": req.title,
    })))
}

/// PUT /v1/blog/posts/:slug — Update an existing blog post.
pub async fn update_post(
    State(state): State<SharedState>,
    Path(slug): Path<String>,
    Json(req): Json<BlogPostRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if !is_safe_slug(&slug) {
        warn!(slug = %slug, "Rejected invalid blog slug on update");
        return Err((StatusCode::BAD_REQUEST, "Invalid slug".into()));
    }
    if req.title.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Title cannot be empty".into()));
    }
    // Validate content size (2 MB max)
    if req.content.len() > MAX_CONTENT_SIZE {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "Content too large ({} bytes, max {})",
                req.content.len(),
                MAX_CONTENT_SIZE
            ),
        ));
    }

    let state = state.read().await;
    let dir = blog_dir(&state);
    let file_path = dir.join(format!("{}.md", slug));

    if !file_path.exists() {
        return Err((StatusCode::NOT_FOUND, format!("Post '{}' not found", slug)));
    }

    // Preserve original date if not provided
    let original = load_post(&dir, &slug);
    let date = req
        .date
        .or_else(|| original.as_ref().and_then(|p| p.meta.date.clone()))
        .unwrap_or_else(|| Utc::now().format("%Y-%m-%d").to_string());

    let meta = BlogPostMeta {
        title: req.title.clone(),
        author: req.author,
        date: Some(date),
        tags: req.tags,
        excerpt: req.excerpt,
        cover_image: req
            .cover_image
            .or_else(|| original.and_then(|p| p.meta.cover_image)),
        published: req.published,
    };

    let file_content = to_frontmatter_md(&meta, &req.content);
    std::fs::write(&file_path, &file_content).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to write post: {}", e),
        )
    })?;

    info!(slug = %slug, title = %req.title, "Blog post updated");

    Ok(Json(json!({
        "updated": true,
        "slug": slug,
        "title": req.title,
    })))
}

/// DELETE /v1/blog/posts/:slug — Delete a blog post.
pub async fn delete_post(
    State(state): State<SharedState>,
    Path(slug): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if !is_safe_slug(&slug) {
        warn!(slug = %slug, "Rejected invalid blog slug on delete");
        return Err((StatusCode::BAD_REQUEST, "Invalid slug".into()));
    }

    let state = state.read().await;
    let dir = blog_dir(&state);
    let file_path = dir.join(format!("{}.md", slug));

    if !file_path.exists() {
        return Err((StatusCode::NOT_FOUND, format!("Post '{}' not found", slug)));
    }

    std::fs::remove_file(&file_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to delete post: {}", e),
        )
    })?;

    info!(slug = %slug, "Blog post deleted");

    Ok(Json(json!({
        "deleted": true,
        "slug": slug,
    })))
}

/// GET /v1/blog/tags — List all unique tags across published posts.
pub async fn list_tags(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;
    let dir = blog_dir(&state);
    let posts = load_all_posts(&dir);

    let mut tag_counts: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for post in posts.iter().filter(|p| p.meta.published) {
        for tag in &post.meta.tags {
            *tag_counts.entry(tag.clone()).or_insert(0) += 1;
        }
    }

    let tags: Vec<Value> = tag_counts
        .iter()
        .map(|(tag, count)| json!({ "name": tag, "count": count }))
        .collect();

    Json(json!({ "tags": tags, "count": tags.len() }))
}

/// POST /v1/blog/images — Upload an image for use in blog posts.
///
/// Accepts multipart/form-data with a single file field.
/// Returns the URL path to embed in markdown: `![alt](/v1/blog/images/filename.png)`
pub async fn upload_media(
    State(state): State<SharedState>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let state = state.read().await;
    let dir = media_dir(&state);

    // Ensure media directory exists
    std::fs::create_dir_all(&dir).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create media dir: {}", e),
        )
    })?;

    let field = multipart
        .next_field()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid multipart: {}", e)))?
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "No file provided".into()))?;

    let original_name = field
        .file_name()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing filename".into()))?
        .to_string();

    // Validate extension
    let extension = std::path::Path::new(&original_name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    if !ALLOWED_IMAGE_EXTENSIONS.contains(&extension.as_str()) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "Unsupported image type '{}'. Allowed: {}",
                extension,
                ALLOWED_IMAGE_EXTENSIONS.join(", ")
            ),
        ));
    }

    let content = field.bytes().await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Failed to read file: {}", e),
        )
    })?;

    // Validate size
    if content.len() > MAX_IMAGE_SIZE {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "Image too large ({} bytes, max {} MB)",
                content.len(),
                MAX_IMAGE_SIZE / 1_048_576
            ),
        ));
    }

    if content.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Empty file upload".into()));
    }

    // Generate unique filename preserving extension
    let id = Uuid::new_v4();
    let filename = format!("{}.{}", id, extension);
    let file_path = dir.join(&filename);

    std::fs::write(&file_path, &content).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to save image: {}", e),
        )
    })?;

    let url = format!("/v1/blog/images/{}", filename);
    let markdown = format!("![{}]({})", original_name, url);

    info!(filename = %filename, size = content.len(), "Blog media uploaded");

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "filename": filename,
            "original_name": original_name,
            "url": url,
            "markdown": markdown,
            "size": content.len(),
        })),
    ))
}

/// GET /v1/blog/images/:filename — Serve a blog media file.
pub async fn serve_media(
    State(state): State<SharedState>,
    Path(filename): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Validate filename safety
    if !is_safe_filename(&filename) {
        warn!(filename = %filename, "Rejected invalid blog image filename");
        return Err((StatusCode::BAD_REQUEST, "Invalid filename".into()));
    }

    let state = state.read().await;
    let dir = media_dir(&state);
    let file_path = dir.join(&filename);

    if !file_path.exists() {
        return Err((StatusCode::NOT_FOUND, "Media not found".into()));
    }

    // Ensure the resolved path is still within media dir (canonicalize to catch symlinks)
    let canonical = file_path
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "Media not found".into()))?;
    let canonical_dir = dir.canonicalize().unwrap_or_else(|_| dir.clone());
    if !canonical.starts_with(&canonical_dir) {
        warn!(filename = %filename, "Path traversal via symlink blocked");
        return Err((StatusCode::BAD_REQUEST, "Invalid filename".into()));
    }

    let content = std::fs::read(&file_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to read media: {}", e),
        )
    })?;

    // Determine content type from extension
    let extension = std::path::Path::new(&filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let content_type = match extension {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    };

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    // Cache blog media for 1 day
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=86400"),
    );

    Ok((StatusCode::OK, headers, content))
}

/// GET /v1/blog/images — List all uploaded blog media files.
pub async fn list_media(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;
    let dir = media_dir(&state);

    let files: Vec<Value> = match std::fs::read_dir(&dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_str()?.to_string();
                let meta = e.metadata().ok()?;
                Some(json!({
                    "filename": name,
                    "url": format!("/v1/blog/images/{}", name),
                    "size": meta.len(),
                }))
            })
            .collect(),
        Err(_) => Vec::new(),
    };

    let count = files.len();
    Json(json!({
        "media": files,
        "count": count,
    }))
}

/// DELETE /v1/blog/images/:filename — Delete a blog media file.
pub async fn delete_media(
    State(state): State<SharedState>,
    Path(filename): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    // Validate filename safety
    if !is_safe_filename(&filename) {
        warn!(filename = %filename, "Rejected invalid blog media filename on delete");
        return Err((StatusCode::BAD_REQUEST, "Invalid filename".into()));
    }

    let state = state.read().await;
    let dir = media_dir(&state);
    let file_path = dir.join(&filename);

    if !file_path.exists() {
        return Err((StatusCode::NOT_FOUND, "Media not found".into()));
    }

    std::fs::remove_file(&file_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to delete media: {}", e),
        )
    })?;

    info!(filename = %filename, "Blog media deleted");

    Ok(Json(json!({
        "deleted": true,
        "filename": filename,
    })))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("Zeus 2.0: The Future"), "zeus-2-0-the-future");
        assert_eq!(slugify("  Leading Spaces  "), "leading-spaces");
        assert_eq!(slugify("special!@#chars"), "special-chars");
    }

    #[test]
    fn test_slugify_ascii_only() {
        // slugify uses ascii_alphanumeric for URL safety
        assert_eq!(slugify("café au lait"), "caf-au-lait");
        assert_eq!(slugify("2026-02-26-first-post"), "2026-02-26-first-post");
    }

    #[test]
    fn test_slugify_truncation() {
        let long_title = "a".repeat(300);
        let slug = slugify(&long_title);
        assert!(slug.len() <= 200);
    }

    #[test]
    fn test_is_safe_slug() {
        assert!(is_safe_slug("hello-world"));
        assert!(is_safe_slug("2026-02-26-my-post"));
        assert!(is_safe_slug("post123"));

        // Dangerous slugs
        assert!(!is_safe_slug("../etc/passwd"));
        assert!(!is_safe_slug(".."));
        assert!(!is_safe_slug(".hidden"));
        assert!(!is_safe_slug("-leading-hyphen"));
        assert!(!is_safe_slug("has/slash"));
        assert!(!is_safe_slug("has\\backslash"));
        assert!(!is_safe_slug(""));
        assert!(!is_safe_slug("has space"));
        assert!(!is_safe_slug("has.dot"));
        assert!(!is_safe_slug("foo\0bar"));
    }

    #[test]
    fn test_is_safe_filename() {
        assert!(is_safe_filename("abc123.png"));
        assert!(is_safe_filename("photo-2026.jpg"));

        assert!(!is_safe_filename(""));
        assert!(!is_safe_filename("../etc/passwd"));
        assert!(!is_safe_filename(".hidden"));
        assert!(!is_safe_filename("foo/bar.png"));
        assert!(!is_safe_filename("foo\\bar.png"));
        assert!(!is_safe_filename("foo\0bar.png"));
    }

    #[test]
    fn test_parse_frontmatter_valid() {
        let content = "---\ntitle: Test Post\nauthor: Zeus\ntags:\n  - rust\n  - ai\n---\n\n# Hello\n\nBody here.";
        let (meta, body) = parse_frontmatter(content).unwrap();
        assert_eq!(meta.title, "Test Post");
        assert_eq!(meta.author, "Zeus");
        assert_eq!(meta.tags, vec!["rust", "ai"]);
        assert!(body.contains("# Hello"));
        assert!(body.contains("Body here."));
    }

    #[test]
    fn test_parse_frontmatter_no_frontmatter() {
        let content = "Just some markdown without frontmatter.";
        assert!(parse_frontmatter(content).is_none());
    }

    #[test]
    fn test_parse_frontmatter_minimal() {
        let content = "---\ntitle: Minimal\n---\n\nContent.";
        let (meta, body) = parse_frontmatter(content).unwrap();
        assert_eq!(meta.title, "Minimal");
        assert_eq!(meta.author, "Zeus Team"); // default
        assert!(meta.published); // default true
        assert_eq!(body, "Content.");
    }

    #[test]
    fn test_parse_frontmatter_with_cover_image() {
        let content =
            "---\ntitle: With Image\ncover_image: /v1/blog/images/abc.png\n---\n\nContent.";
        let (meta, _body) = parse_frontmatter(content).unwrap();
        assert_eq!(meta.cover_image.as_deref(), Some("/v1/blog/images/abc.png"));
    }

    #[test]
    fn test_to_frontmatter_roundtrip() {
        let meta = BlogPostMeta {
            title: "Roundtrip".into(),
            author: "Test".into(),
            date: Some("2026-02-26".into()),
            tags: vec!["test".into()],
            excerpt: Some("A test post".into()),
            cover_image: None,
            published: true,
        };
        let content = "Hello world";
        let serialized = to_frontmatter_md(&meta, content);
        let (parsed_meta, parsed_body) = parse_frontmatter(&serialized).unwrap();
        assert_eq!(parsed_meta.title, "Roundtrip");
        assert_eq!(parsed_meta.author, "Test");
        assert_eq!(parsed_body.trim(), "Hello world");
    }

    #[test]
    fn test_default_meta_values() {
        let yaml = "title: Just Title\n";
        let meta: BlogPostMeta = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(meta.author, "Zeus Team");
        assert!(meta.published);
        assert!(meta.tags.is_empty());
        assert!(meta.excerpt.is_none());
        assert!(meta.cover_image.is_none());
    }

    #[test]
    fn test_render_markdown_to_html() {
        let md = "# Hello\n\nThis is **bold** and *italic*.\n\n- item 1\n- item 2\n";
        let html = render_markdown(md);
        assert!(html.contains("<h1>Hello</h1>"));
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("<em>italic</em>"));
        assert!(html.contains("<li>item 1</li>"));
    }

    #[test]
    fn test_render_markdown_images() {
        let md = "![Zeus Logo](/v1/blog/images/logo.png)\n\nSome text after image.";
        let html = render_markdown(md);
        assert!(html.contains("<img"));
        assert!(html.contains("src=\"/v1/blog/images/logo.png\""));
        assert!(html.contains("alt=\"Zeus Logo\""));
    }

    #[test]
    fn test_render_markdown_code_blocks() {
        let md = "```rust\nfn main() {}\n```\n";
        let html = render_markdown(md);
        assert!(html.contains("<code"));
        assert!(html.contains("fn main()"));
    }

    #[test]
    fn test_render_markdown_tables() {
        let md = "| Col1 | Col2 |\n|------|------|\n| A | B |\n";
        let html = render_markdown(md);
        assert!(html.contains("<table>"));
        assert!(html.contains("<td>A</td>"));
    }

    #[test]
    fn test_content_size_limit() {
        // Verify the constant is 2 MB
        assert_eq!(MAX_CONTENT_SIZE, 2_097_152);
        assert_eq!(MAX_IMAGE_SIZE, 10_485_760);
    }

    #[test]
    fn test_allowed_image_extensions() {
        assert!(ALLOWED_IMAGE_EXTENSIONS.contains(&"png"));
        assert!(ALLOWED_IMAGE_EXTENSIONS.contains(&"jpg"));
        assert!(ALLOWED_IMAGE_EXTENSIONS.contains(&"jpeg"));
        assert!(ALLOWED_IMAGE_EXTENSIONS.contains(&"gif"));
        assert!(ALLOWED_IMAGE_EXTENSIONS.contains(&"webp"));
        assert!(ALLOWED_IMAGE_EXTENSIONS.contains(&"svg"));
        assert!(!ALLOWED_IMAGE_EXTENSIONS.contains(&"exe"));
        assert!(!ALLOWED_IMAGE_EXTENSIONS.contains(&"js"));
    }

    #[test]
    fn test_list_tags_aggregation() {
        // Verify BTreeMap-based tag counting logic
        let mut tag_counts: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();

        // Simulate 3 posts with overlapping tags
        let post_tags = vec![
            vec!["rust", "ai"],
            vec!["rust", "security"],
            vec!["ai", "llm"],
        ];
        for tags in &post_tags {
            for tag in tags {
                *tag_counts.entry(tag.to_string()).or_insert(0) += 1;
            }
        }

        assert_eq!(tag_counts.len(), 4);
        assert_eq!(tag_counts["rust"], 2);
        assert_eq!(tag_counts["ai"], 2);
        assert_eq!(tag_counts["security"], 1);
        assert_eq!(tag_counts["llm"], 1);

        // BTreeMap iterates in sorted order
        let keys: Vec<&String> = tag_counts.keys().collect();
        assert_eq!(keys, vec!["ai", "llm", "rust", "security"]);
    }
}
