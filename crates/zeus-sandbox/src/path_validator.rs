//! Path validation and traversal protection
//!
//! Validates file paths to prevent directory traversal attacks
//! by ensuring paths are canonicalized and bounded to a safe root.

use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PathValidationError {
    #[error("Path traversal detected: {0}")]
    TraversalDetected(String),
    #[error("Path is outside allowed root: {path} (root: {root})")]
    OutsideRoot { path: String, root: String },
    #[error("Failed to canonicalize path: {0}")]
    CanonicalizationFailed(String),
    #[error("Invalid path: {0}")]
    InvalidPath(String),
    #[error("Absolute paths outside workspace are not allowed: {0}")]
    AbsolutePathForbidden(String),
}

/// Validates a file path is safe and bounded to a root directory
///
/// Prevents:
/// - Directory traversal with ../ components
/// - Absolute paths outside the root
/// - Symlink attacks (via canonicalization)
///
/// # Arguments
/// * `path` - The path to validate (relative or absolute)
/// * `root` - The root directory that must contain the path
///
/// # Returns
/// The canonicalized absolute path if safe, error otherwise
///
/// # Example
/// ```
/// use std::path::PathBuf;
/// use zeus_sandbox::path_validator::validate_path_in_root;
///
/// let root = PathBuf::from("/workspace");
/// let safe = validate_path_in_root("file.txt", &root);
/// // Returns /workspace/file.txt
///
/// let unsafe_path = validate_path_in_root("../etc/passwd", &root);
/// // Returns error - path traversal detected
/// ```
pub fn validate_path_in_root(
    path: impl AsRef<Path>,
    root: impl AsRef<Path>,
) -> Result<PathBuf, PathValidationError> {
    let path = path.as_ref();
    let root = root.as_ref();

    // Check for obvious traversal attempts in the raw path
    let path_str = path
        .to_str()
        .ok_or_else(|| PathValidationError::InvalidPath("non-UTF8 path".to_string()))?;

    // Block paths with ../ components (before normalization)
    if path_str.contains("..") {
        return Err(PathValidationError::TraversalDetected(path_str.to_string()));
    }

    // Resolve the path against the root
    let resolved = if path.is_absolute() {
        // Absolute paths must be within root
        path.to_path_buf()
    } else {
        // Relative paths are joined to root
        root.join(path)
    };

    // Canonicalize to resolve symlinks and normalize
    let canonical = resolved.canonicalize().or_else(|_| {
        // If canonicalization fails (file doesn't exist), try to canonicalize parent
        // and join the filename
        if let Some(parent) = resolved.parent()
            && parent.exists()
        {
            let canonical_parent = parent.canonicalize().map_err(|e| {
                PathValidationError::CanonicalizationFailed(format!("parent: {}", e))
            })?;
            if let Some(filename) = resolved.file_name() {
                return Ok(canonical_parent.join(filename));
            }
        }
        Err(PathValidationError::CanonicalizationFailed(format!(
            "path: {}",
            resolved.display()
        )))
    })?;

    // Canonicalize root
    let canonical_root = root
        .canonicalize()
        .map_err(|e| PathValidationError::CanonicalizationFailed(format!("root: {}", e)))?;

    // Verify the canonical path is within the canonical root
    if !canonical.starts_with(&canonical_root) {
        return Err(PathValidationError::OutsideRoot {
            path: canonical.display().to_string(),
            root: canonical_root.display().to_string(),
        });
    }

    Ok(canonical)
}

/// Validates a path for file write operations
///
/// Similar to `validate_path_in_root` but also ensures parent directory exists
/// or can be created.
pub fn validate_write_path(
    path: impl AsRef<Path>,
    root: impl AsRef<Path>,
) -> Result<PathBuf, PathValidationError> {
    let path = path.as_ref();
    let root = root.as_ref();

    // First validate the path would be within root
    let path_str = path
        .to_str()
        .ok_or_else(|| PathValidationError::InvalidPath("non-UTF8 path".to_string()))?;

    if path_str.contains("..") {
        return Err(PathValidationError::TraversalDetected(path_str.to_string()));
    }

    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };

    // Canonicalize root
    let canonical_root = root
        .canonicalize()
        .map_err(|e| PathValidationError::CanonicalizationFailed(format!("root: {}", e)))?;

    // If file exists, use standard validation
    if resolved.exists() {
        return validate_path_in_root(path, root);
    }

    // For new files, validate the parent directory
    if let Some(parent) = resolved.parent() {
        // Create parent directories if needed
        std::fs::create_dir_all(parent).map_err(|e| {
            PathValidationError::CanonicalizationFailed(format!("failed to create parent: {}", e))
        })?;

        let canonical_parent = parent
            .canonicalize()
            .map_err(|e| PathValidationError::CanonicalizationFailed(format!("parent: {}", e)))?;

        if !canonical_parent.starts_with(&canonical_root) {
            return Err(PathValidationError::OutsideRoot {
                path: canonical_parent.display().to_string(),
                root: canonical_root.display().to_string(),
            });
        }

        if let Some(filename) = resolved.file_name() {
            return Ok(canonical_parent.join(filename));
        }
    }

    Err(PathValidationError::InvalidPath(
        "path has no parent directory".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_safe_relative_path() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let root = tmp.path();
        fs::write(root.join("test.txt"), "content").expect("should write file");

        let result = validate_path_in_root("test.txt", root);
        assert!(result.is_ok());
        assert_eq!(
            result.expect("operation should succeed"),
            root.join("test.txt")
                .canonicalize()
                .expect("should canonicalize path")
        );
    }

    #[test]
    fn test_traversal_blocked() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let root = tmp.path();

        // Traversal with ../
        assert!(validate_path_in_root("../etc/passwd", root).is_err());
        assert!(validate_path_in_root("subdir/../../etc/passwd", root).is_err());
        assert!(validate_path_in_root("./../../etc/passwd", root).is_err());
    }

    #[test]
    fn test_absolute_path_outside_root() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let root = tmp.path();

        assert!(validate_path_in_root("/etc/passwd", root).is_err());
        assert!(validate_path_in_root("/tmp/evil.txt", root).is_err());
    }

    #[test]
    fn test_subdirectory_allowed() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let root = tmp.path();
        fs::create_dir_all(root.join("subdir")).expect("should create directory");
        fs::write(root.join("subdir/file.txt"), "content").expect("should write file");

        let result = validate_path_in_root("subdir/file.txt", root);
        assert!(result.is_ok());
    }

    #[test]
    fn test_write_path_creates_parent() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let root = tmp.path();

        let result = validate_write_path("new/dir/file.txt", root);
        assert!(result.is_ok());
        assert!(root.join("new/dir").exists());
    }

    #[test]
    fn test_write_path_traversal_blocked() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let root = tmp.path();

        assert!(validate_write_path("../etc/passwd", root).is_err());
        assert!(validate_write_path("dir/../../etc/passwd", root).is_err());
    }

    #[test]
    fn test_symlink_resolution() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let root = tmp.path();

        // Create a file outside root
        let outside = TempDir::new().expect("TempDir::new should succeed");
        fs::write(outside.path().join("secret.txt"), "secret").expect("should write file");

        // Try to create symlink to outside file (on Unix)
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link = root.join("link");
            let _ = symlink(outside.path().join("secret.txt"), &link);

            // Validation should fail - symlink points outside root
            let result = validate_path_in_root("link", root);
            // This will fail because canonicalization resolves symlink
            if result.is_ok() {
                let canonical = result.expect("operation should succeed");
                assert!(
                    !canonical.starts_with(root.canonicalize().expect("should canonicalize path"))
                );
            }
        }
    }

    #[test]
    fn test_nonexistent_file_in_existing_dir() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let root = tmp.path();

        let result = validate_path_in_root("nonexistent.txt", root);
        assert!(result.is_ok());
        assert_eq!(
            result.expect("operation should succeed"),
            root.join("nonexistent.txt")
                .canonicalize()
                .unwrap_or_else(|_| {
                    root.canonicalize()
                        .expect("should canonicalize path")
                        .join("nonexistent.txt")
                })
        );
    }
}
