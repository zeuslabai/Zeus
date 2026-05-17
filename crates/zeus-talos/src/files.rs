//! File system tools

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use zeus_core::{Error, Result, ToolSchema};

/// Search for files
pub struct FileSearchTool;

#[async_trait]
impl TalosTool for FileSearchTool {
    fn name(&self) -> &'static str {
        "file_search"
    }
    fn description(&self) -> &'static str {
        "Search for files by name pattern"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("path", "string", "Directory to search in", true)
            .with_param("pattern", "string", "Filename pattern (supports *)", true)
            .with_param(
                "max_depth",
                "integer",
                "Max directory depth (default 3)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing path".to_string()))?;

        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing pattern".to_string()))?;

        let max_depth = args.get("max_depth").and_then(|v| v.as_u64()).unwrap_or(3) as usize;

        let mut results = Vec::new();
        search_files(Path::new(path), pattern, 0, max_depth, &mut results)?;

        Ok(serde_json::to_string_pretty(&results)?)
    }
}

fn search_files(
    dir: &Path,
    pattern: &str,
    depth: usize,
    max_depth: usize,
    results: &mut Vec<String>,
) -> Result<()> {
    if depth > max_depth || results.len() >= 100 {
        return Ok(());
    }

    if !dir.is_dir() {
        return Ok(());
    }

    let entries =
        fs::read_dir(dir).map_err(|e| Error::Tool(format!("Cannot read directory: {}", e)))?;

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Simple glob matching
        if matches_pattern(&name, pattern) {
            results.push(path.to_string_lossy().to_string());
        }

        if path.is_dir() {
            let _ = search_files(&path, pattern, depth + 1, max_depth, results);
        }
    }

    Ok(())
}

fn matches_pattern(name: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    if pattern.starts_with('*') && pattern.ends_with('*') {
        let middle = &pattern[1..pattern.len() - 1];
        return name.contains(middle);
    }

    if let Some(suffix) = pattern.strip_prefix('*') {
        return name.ends_with(suffix);
    }

    if let Some(prefix) = pattern.strip_suffix('*') {
        return name.starts_with(prefix);
    }

    name == pattern
}

/// Get file metadata
pub struct FileMetadataTool;

#[async_trait]
impl TalosTool for FileMetadataTool {
    fn name(&self) -> &'static str {
        "file_metadata"
    }
    fn description(&self) -> &'static str {
        "Get metadata for a file or directory"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "path",
            "string",
            "Path to file or directory",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing path".to_string()))?;

        let metadata =
            fs::metadata(path).map_err(|e| Error::Tool(format!("Cannot get metadata: {}", e)))?;

        let file_type = if metadata.is_dir() {
            "directory"
        } else if metadata.is_file() {
            "file"
        } else if metadata.is_symlink() {
            "symlink"
        } else {
            "unknown"
        };

        let info = json!({
            "path": path,
            "type": file_type,
            "size_bytes": metadata.len(),
            "size_human": format_size(metadata.len()),
            "readonly": metadata.permissions().readonly(),
            "modified": metadata.modified()
                .ok()
                .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339()),
            "created": metadata.created()
                .ok()
                .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339()),
        });

        Ok(serde_json::to_string_pretty(&info)?)
    }
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} bytes", bytes)
    }
}

// === FILES ADDITIONS ===

/// Copy a file or directory
pub struct FileCopyTool;

#[async_trait]
impl TalosTool for FileCopyTool {
    fn name(&self) -> &'static str {
        "file_copy"
    }
    fn description(&self) -> &'static str {
        "Copy a file or directory to a new location"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("source", "string", "Source file or directory path", true)
            .with_param("destination", "string", "Destination path", true)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let source = args
            .get("source")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing source".to_string()))?;

        let destination = args
            .get("destination")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing destination".to_string()))?;

        let src_path = Path::new(source);
        if !src_path.exists() {
            return Err(Error::Tool(format!("Source not found: {}", source)));
        }

        if src_path.is_dir() {
            let output = tokio::process::Command::new("cp")
                .arg("-r")
                .arg(source)
                .arg(destination)
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to copy directory: {}", e)))?;

            if output.status.success() {
                Ok(format!("Copied directory {} to {}", source, destination))
            } else {
                Err(Error::Tool(format!(
                    "Copy failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        } else {
            fs::copy(source, destination)
                .map_err(|e| Error::Tool(format!("Failed to copy file: {}", e)))?;
            Ok(format!("Copied {} to {}", source, destination))
        }
    }
}

/// Move/rename a file or directory
pub struct FileMoveTool;

#[async_trait]
impl TalosTool for FileMoveTool {
    fn name(&self) -> &'static str {
        "file_move"
    }
    fn description(&self) -> &'static str {
        "Move or rename a file or directory"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("source", "string", "Source file or directory path", true)
            .with_param("destination", "string", "Destination path", true)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let source = args
            .get("source")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing source".to_string()))?;

        let destination = args
            .get("destination")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing destination".to_string()))?;

        fs::rename(source, destination)
            .map_err(|e| Error::Tool(format!("Failed to move: {}", e)))?;

        Ok(format!("Moved {} to {}", source, destination))
    }
}

/// Rename a file (just the filename, not the full path)
pub struct FileRenameTool;

#[async_trait]
impl TalosTool for FileRenameTool {
    fn name(&self) -> &'static str {
        "file_rename"
    }
    fn description(&self) -> &'static str {
        "Rename a file (change just the filename)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("path", "string", "Path to the file to rename", true)
            .with_param(
                "new_name",
                "string",
                "New filename (just the name, not full path)",
                true,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing path".to_string()))?;

        let new_name = args
            .get("new_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing new_name".to_string()))?;

        let src = Path::new(path);
        if !src.exists() {
            return Err(Error::Tool(format!("File not found: {}", path)));
        }

        let parent = src
            .parent()
            .ok_or_else(|| Error::Tool("Cannot determine parent directory".to_string()))?;
        let new_path = parent.join(new_name);

        fs::rename(src, &new_path).map_err(|e| Error::Tool(format!("Failed to rename: {}", e)))?;

        Ok(format!("Renamed {} to {}", path, new_path.display()))
    }
}

/// Get detailed file stats
pub struct FileStatTool;

#[async_trait]
impl TalosTool for FileStatTool {
    fn name(&self) -> &'static str {
        "file_stat"
    }
    fn description(&self) -> &'static str {
        "Get detailed file statistics"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "path",
            "string",
            "Path to file or directory",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing path".to_string()))?;

        let metadata =
            fs::metadata(path).map_err(|e| Error::Tool(format!("Cannot get metadata: {}", e)))?;

        let symlink_metadata = fs::symlink_metadata(path)
            .map_err(|e| Error::Tool(format!("Cannot get symlink metadata: {}", e)))?;

        let info = json!({
            "path": path,
            "size": metadata.len(),
            "readonly": metadata.permissions().readonly(),
            "is_file": metadata.is_file(),
            "is_dir": metadata.is_dir(),
            "is_symlink": symlink_metadata.file_type().is_symlink(),
            "modified": metadata.modified()
                .ok()
                .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339()),
            "created": metadata.created()
                .ok()
                .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339()),
            "accessed": metadata.accessed()
                .ok()
                .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339()),
        });

        Ok(serde_json::to_string_pretty(&info)?)
    }
}

/// Find files using the `find` command
pub struct FindFilesTool;

#[async_trait]
impl TalosTool for FindFilesTool {
    fn name(&self) -> &'static str {
        "find_files"
    }
    fn description(&self) -> &'static str {
        "Find files using the find command"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("path", "string", "Directory to search in", true)
            .with_param(
                "name",
                "string",
                "Filename pattern to match (optional)",
                false,
            )
            .with_param(
                "type",
                "string",
                "Type filter: 'f' for files, 'd' for directories (optional)",
                false,
            )
            .with_param(
                "max_depth",
                "integer",
                "Maximum directory depth (optional)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing path".to_string()))?;

        let mut cmd = tokio::process::Command::new("find");
        cmd.arg(path);

        if let Some(max_depth) = args.get("max_depth").and_then(|v| v.as_u64()) {
            cmd.arg("-maxdepth").arg(max_depth.to_string());
        }

        if let Some(file_type) = args.get("type").and_then(|v| v.as_str()) {
            let sanitized = crate::sanitize_shell_arg(file_type);
            // Only allow "f" or "d"
            match file_type {
                "f" | "d" => {
                    cmd.arg("-type").arg(file_type);
                }
                _ => return Err(Error::Tool("type must be 'f' or 'd'".to_string())),
            }
            let _ = sanitized;
        }

        if let Some(name) = args.get("name").and_then(|v| v.as_str()) {
            cmd.arg("-name").arg(name);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| Error::Tool(format!("find command failed: {}", e)))?;

        if output.status.success() {
            let result = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Ok(result)
        } else {
            Err(Error::Tool(format!(
                "find failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}

/// Create a directory
pub struct DirectoryCreateTool;

#[async_trait]
impl TalosTool for DirectoryCreateTool {
    fn name(&self) -> &'static str {
        "directory_create"
    }
    fn description(&self) -> &'static str {
        "Create a new directory"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("path", "string", "Path of the directory to create", true)
            .with_param(
                "recursive",
                "boolean",
                "Create parent directories if needed (default true)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing path".to_string()))?;

        let recursive = args
            .get("recursive")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        if recursive {
            fs::create_dir_all(path)
                .map_err(|e| Error::Tool(format!("Failed to create directory: {}", e)))?;
        } else {
            fs::create_dir(path)
                .map_err(|e| Error::Tool(format!("Failed to create directory: {}", e)))?;
        }

        Ok(format!("Created directory: {}", path))
    }
}

/// Move a file to the Trash
pub struct TrashFileTool;

#[async_trait]
impl TalosTool for TrashFileTool {
    fn name(&self) -> &'static str {
        "trash_file"
    }
    fn description(&self) -> &'static str {
        "Move a file to the Trash"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "path",
            "string",
            "Path to file or folder to trash",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing path".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let script = format!(
                r#"
                tell application "Finder"
                    move POSIX file "{}" to trash
                end tell
                return "Moved to Trash"
            "#,
                crate::sanitize_applescript(path)
            );

            crate::run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = path;
            Ok("Trash tool only available on macOS".to_string())
        }
    }
}

/// Create an alias (symlink) via Finder
pub struct CreateAliasTool;

#[async_trait]
impl TalosTool for CreateAliasTool {
    fn name(&self) -> &'static str {
        "create_alias"
    }
    fn description(&self) -> &'static str {
        "Create a Finder alias for a file or folder"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("source", "string", "Source file or folder path", true)
            .with_param(
                "destination",
                "string",
                "Destination folder for the alias",
                true,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let source = args
            .get("source")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing source".to_string()))?;

        let destination = args
            .get("destination")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing destination".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let script = format!(
                r#"
                tell application "Finder"
                    make alias file to POSIX file "{}" at POSIX file "{}"
                end tell
                return "Alias created"
            "#,
                crate::sanitize_applescript(source),
                crate::sanitize_applescript(destination)
            );

            crate::run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (source, destination);
            Ok("Alias tool only available on macOS".to_string())
        }
    }
}

/// Get the current Finder selection
pub struct FinderSelectionTool;

#[async_trait]
impl TalosTool for FinderSelectionTool {
    fn name(&self) -> &'static str {
        "finder_selection"
    }
    fn description(&self) -> &'static str {
        "Get the currently selected files in Finder"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let _ = &args;
            let script = r#"
                set fileList to ""
                tell application "Finder"
                    set theSelection to selection
                    if (count of theSelection) is 0 then
                        return "No files selected in Finder"
                    end if
                    repeat with f in theSelection
                        set fileList to fileList & (POSIX path of (f as alias)) & linefeed
                    end repeat
                end tell
                return fileList
            "#;

            crate::run_applescript(script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Finder selection tool only available on macOS".to_string())
        }
    }
}

/// Get Finder tags on a file
pub struct FileTagsTool;

#[async_trait]
impl TalosTool for FileTagsTool {
    fn name(&self) -> &'static str {
        "file_tags"
    }
    fn description(&self) -> &'static str {
        "Get Finder tags on a file"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "path",
            "string",
            "Path to file",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing path".to_string()))?;

            if !Path::new(path).exists() {
                return Err(Error::Tool(format!("File not found: {}", path)));
            }

            let output = std::process::Command::new("mdls")
                .arg("-name")
                .arg("kMDItemUserTags")
                .arg(path)
                .output()
                .map_err(|e| Error::Tool(format!("Failed to run mdls: {}", e)))?;

            if output.status.success() {
                let result = String::from_utf8_lossy(&output.stdout).trim().to_string();
                Ok(result)
            } else {
                Err(Error::Tool(format!(
                    "mdls error: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Err(Error::Tool(
                "File tags (mdls) only available on macOS".to_string(),
            ))
        }
    }
}

/// Set Finder tags on a file
pub struct SetFileTagsTool;

#[async_trait]
impl TalosTool for SetFileTagsTool {
    fn name(&self) -> &'static str {
        "set_file_tags"
    }
    fn description(&self) -> &'static str {
        "Set Finder tags on a file"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("path", "string", "Path to file", true)
            .with_param("tags", "string", "Comma-separated list of tags", true)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing path".to_string()))?;

            let tags = args
                .get("tags")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing tags".to_string()))?;

            if !Path::new(path).exists() {
                return Err(Error::Tool(format!("File not found: {}", path)));
            }

            // Build plist-format tag array for xattr
            let tag_list: Vec<&str> = tags.split(',').map(|t| t.trim()).collect();
            let mut plist_items = String::new();
            for tag in &tag_list {
                plist_items.push_str(&format!(
                    "<string>{}</string>",
                    tag.replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;")
                ));
            }
            let plist = format!(
                r#"<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><array>{}</array></plist>"#,
                plist_items
            );

            let sanitized_path = crate::sanitize_shell_arg(path);
            let output = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!(
                    "xattr -w com.apple.metadata:_kMDItemUserTags '{}' {}",
                    plist.replace('\'', "'\\''"),
                    sanitized_path
                ))
                .output()
                .map_err(|e| Error::Tool(format!("Failed to set tags: {}", e)))?;

            if output.status.success() {
                Ok(format!("Tags set: {}", tag_list.join(", ")))
            } else {
                Err(Error::Tool(format!(
                    "xattr error: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Err(Error::Tool(
                "File tags (xattr) only available on macOS".to_string(),
            ))
        }
    }
}

// ── Extended file tools ──────────────────────────────────────────────

/// Append content to a file
pub struct FileAppendTool;

#[async_trait]
impl TalosTool for FileAppendTool {
    fn name(&self) -> &'static str {
        "file_append"
    }
    fn description(&self) -> &'static str {
        "Append content to the end of a file"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("path", "string", "File path to append to", true)
            .with_param("content", "string", "Content to append", true)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("'path' is required".to_string()))?;
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("'content' is required".to_string()))?;
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .map_err(|e| Error::Tool(format!("Failed to open file: {}", e)))?;
        file.write_all(content.as_bytes())
            .map_err(|e| Error::Tool(format!("Failed to write: {}", e)))?;
        Ok(format!("Appended {} bytes to {}", content.len(), path))
    }
}

/// Create a new empty file
pub struct FileCreateTool;

#[async_trait]
impl TalosTool for FileCreateTool {
    fn name(&self) -> &'static str {
        "file_create"
    }
    fn description(&self) -> &'static str {
        "Create a new file (optionally with content)"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("path", "string", "File path to create", true)
            .with_param(
                "content",
                "string",
                "Initial content (default empty)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("'path' is required".to_string()))?;
        let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
        std::fs::write(path, content)
            .map_err(|e| Error::Tool(format!("Failed to create file: {}", e)))?;
        Ok(format!("Created {}", path))
    }
}

/// Delete a file (hard delete, not trash)
pub struct FileDeleteTool;

#[async_trait]
impl TalosTool for FileDeleteTool {
    fn name(&self) -> &'static str {
        "file_delete"
    }
    fn description(&self) -> &'static str {
        "Delete a file permanently"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "path",
            "string",
            "File path to delete",
            true,
        )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("'path' is required".to_string()))?;
        std::fs::remove_file(path).map_err(|e| Error::Tool(format!("Failed to delete: {}", e)))?;
        Ok(format!("Deleted {}", path))
    }
}

/// Search file contents using grep
pub struct GrepFilesTool;

#[async_trait]
impl TalosTool for GrepFilesTool {
    fn name(&self) -> &'static str {
        "grep_files"
    }
    fn description(&self) -> &'static str {
        "Search file contents using pattern matching (grep)"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "pattern",
                "string",
                "Search pattern (regex supported)",
                true,
            )
            .with_param("path", "string", "File or directory to search in", true)
            .with_param(
                "recursive",
                "boolean",
                "Search recursively in directories (default true)",
                false,
            )
            .with_param(
                "case_insensitive",
                "boolean",
                "Case insensitive search (default false)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("'pattern' is required".to_string()))?;
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("'path' is required".to_string()))?;
        let recursive = args
            .get("recursive")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let case_insensitive = args
            .get("case_insensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut cmd = tokio::process::Command::new("grep");
        if recursive {
            cmd.arg("-r");
        }
        if case_insensitive {
            cmd.arg("-i");
        }
        cmd.arg("-n").arg(pattern).arg(path);
        let output = cmd
            .output()
            .await
            .map_err(|e| Error::Tool(format!("Failed: {}", e)))?;
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

/// Read the first N lines of a file
pub struct HeadFileTool;

#[async_trait]
impl TalosTool for HeadFileTool {
    fn name(&self) -> &'static str {
        "head_file"
    }
    fn description(&self) -> &'static str {
        "Read the first N lines of a file"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("path", "string", "File path to read", true)
            .with_param("lines", "integer", "Number of lines (default 10)", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("'path' is required".to_string()))?;
        let n = args.get("lines").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
        let content = std::fs::read_to_string(path)
            .map_err(|e| Error::Tool(format!("Failed to read: {}", e)))?;
        let lines: Vec<&str> = content.lines().take(n).collect();
        Ok(lines.join("\n"))
    }
}

/// Read the last N lines of a file
pub struct TailFileTool;

#[async_trait]
impl TalosTool for TailFileTool {
    fn name(&self) -> &'static str {
        "tail_file"
    }
    fn description(&self) -> &'static str {
        "Read the last N lines of a file"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("path", "string", "File path to read", true)
            .with_param("lines", "integer", "Number of lines (default 10)", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("'path' is required".to_string()))?;
        let n = args.get("lines").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
        let content = std::fs::read_to_string(path)
            .map_err(|e| Error::Tool(format!("Failed to read: {}", e)))?;
        let all_lines: Vec<&str> = content.lines().collect();
        let start = all_lines.len().saturating_sub(n);
        Ok(all_lines[start..].join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_copy_schema() {
        let tool = FileCopyTool;
        assert_eq!(tool.name(), "file_copy");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("source"));
        assert!(props.contains_key("destination"));
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("source")));
        assert!(required.iter().any(|v| v.as_str() == Some("destination")));
    }

    #[test]
    fn test_file_move_schema() {
        let tool = FileMoveTool;
        assert_eq!(tool.name(), "file_move");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("source"));
        assert!(props.contains_key("destination"));
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("source")));
        assert!(required.iter().any(|v| v.as_str() == Some("destination")));
    }

    #[test]
    fn test_file_rename_schema() {
        let tool = FileRenameTool;
        assert_eq!(tool.name(), "file_rename");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("path"));
        assert!(props.contains_key("new_name"));
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("path")));
        assert!(required.iter().any(|v| v.as_str() == Some("new_name")));
    }

    #[test]
    fn test_file_stat_schema() {
        let tool = FileStatTool;
        assert_eq!(tool.name(), "file_stat");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("path"));
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("path")));
    }

    #[test]
    fn test_find_files_schema() {
        let tool = FindFilesTool;
        assert_eq!(tool.name(), "find_files");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("path"));
        assert!(props.contains_key("name"));
        assert!(props.contains_key("type"));
        assert!(props.contains_key("max_depth"));
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("path")));
    }

    #[test]
    fn test_directory_create_schema() {
        let tool = DirectoryCreateTool;
        assert_eq!(tool.name(), "directory_create");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("path"));
        assert!(props.contains_key("recursive"));
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("path")));
    }

    #[tokio::test]
    async fn test_directory_create() {
        let dir = tempfile::tempdir().expect("should create temp dir");
        let new_dir = dir.path().join("test_subdir");

        let tool = DirectoryCreateTool;
        let result = tool
            .execute(json!({
                "path": new_dir.to_str().expect("to_str should succeed")
            }))
            .await
            .expect("async operation should succeed");

        assert!(result.contains("Created directory"));
        assert!(new_dir.exists());
        assert!(new_dir.is_dir());
    }

    // ── Extended file tool tests below ──

    #[tokio::test]
    async fn test_file_copy() {
        let dir = tempfile::tempdir().expect("should create temp dir");
        let src = dir.path().join("source.txt");
        let dst = dir.path().join("destination.txt");

        std::fs::write(&src, "hello world").expect("should write file");

        let tool = FileCopyTool;
        let result = tool
            .execute(json!({
                "source": src.to_str().expect("to_str should succeed"),
                "destination": dst.to_str().expect("to_str should succeed")
            }))
            .await
            .expect("async operation should succeed");

        assert!(result.contains("Copied"));
        assert!(dst.exists());
        assert_eq!(
            std::fs::read_to_string(&dst).expect("should read file"),
            "hello world"
        );
    }
}
