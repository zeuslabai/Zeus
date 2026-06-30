/// S103 #33: Team memory files — shared ~/.zeus/team/*.md coordination layer.
///
/// Multiple agents can read/write named memory files in a shared directory.
/// Files are plain markdown. Writes are atomic (write-to-temp, rename).
use std::path::PathBuf;

/// Returns the default team memory directory: ~/.zeus/team/
pub fn default_team_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".zeus").join("team"))
}

/// Resolve the directory to use: override if provided, otherwise default.
fn resolve_dir(override_dir: Option<&PathBuf>) -> std::io::Result<PathBuf> {
    override_dir
        .cloned()
        .map(Ok)
        .unwrap_or_else(|| {
            default_team_dir().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "no home dir")
            })
        })
}

/// Write (or overwrite) a team memory file by name.
/// `name` should be a simple identifier like "sprint-status" (no extension needed).
/// Creates the team directory if it doesn't exist.
///
/// If `override_dir` is Some, uses that directory instead of `default_team_dir()`.
/// This is intended for testing — production callers should pass None.
pub fn write_team_memory(name: &str, content: &str, override_dir: Option<&PathBuf>) -> std::io::Result<PathBuf> {
    let dir = resolve_dir(override_dir)?;
    std::fs::create_dir_all(&dir)?;
    let filename = sanitize_filename(name);
    let path = dir.join(&filename);
    // Atomic write: temp file + rename
    let tmp = dir.join(format!(".{}.tmp", filename));
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Read a team memory file by name. Returns None if it doesn't exist.
///
/// If `override_dir` is Some, uses that directory instead of `default_team_dir()`.
pub fn read_team_memory(name: &str, override_dir: Option<&PathBuf>) -> std::io::Result<Option<String>> {
    let dir = resolve_dir(override_dir)?;
    let filename = sanitize_filename(name);
    let path = dir.join(&filename);
    if !path.exists() {
        return Ok(None);
    }
    std::fs::read_to_string(&path).map(Some)
}

/// List all team memory file names (stems, no extension).
///
/// If `override_dir` is Some, uses that directory instead of `default_team_dir()`.
pub fn list_team_memory(override_dir: Option<&PathBuf>) -> std::io::Result<Vec<String>> {
    let dir = resolve_dir(override_dir)?;
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut names = vec![];
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map(|e| e == "md").unwrap_or(false) {
            if let Some(stem) = path.file_stem() {
                // Skip temp files (dot-prefixed)
                let stem_str = stem.to_string_lossy();
                if !stem_str.starts_with('.') {
                    names.push(stem_str.into_owned());
                }
            }
        }
    }
    names.sort();
    Ok(names)
}

/// Delete a team memory file by name.
///
/// If `override_dir` is Some, uses that directory instead of `default_team_dir()`.
pub fn delete_team_memory(name: &str, override_dir: Option<&PathBuf>) -> std::io::Result<()> {
    let dir = resolve_dir(override_dir)?;
    let filename = sanitize_filename(name);
    let path = dir.join(&filename);
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

/// Convert a name to a filename: append .md if not already present.
/// Rejects path traversal attempts (names containing / or \ or ..).
fn sanitize_filename(name: &str) -> String {
    // Security: reject path traversal
    assert!(
        !name.contains('/') && !name.contains('\\') && !name.contains(".."),
        "team_memory name must not contain path separators or '..': {}",
        name
    );
    if name.ends_with(".md") {
        name.to_string()
    } else {
        format!("{}.md", name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper: create a temp dir and return its PathBuf.
    fn tmp_dir() -> PathBuf {
        TempDir::new().expect("TempDir::new should succeed").path().to_path_buf()
    }

    // ── write_team_memory tests ─────────────────────────────────────────

    #[test]
    fn test_write_creates_file() {
        let dir = tmp_dir();
        let path = write_team_memory("sprint-status", "# Sprint Status\nAll green!", Some(&dir)).unwrap();
        assert!(path.exists());
        assert_eq!(path.file_name().unwrap(), "sprint-status.md");
    }

    #[test]
    fn test_write_creates_directory_if_missing() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let dir = tmp.path().join("nested").join("team");
        assert!(!dir.exists());
        write_team_memory("test", "hello", Some(&dir)).unwrap();
        assert!(dir.exists());
        assert!(dir.join("test.md").exists());
    }

    #[test]
    fn test_write_overwrites_existing() {
        let dir = tmp_dir();
        write_team_memory("overwrite", "version 1", Some(&dir)).unwrap();
        write_team_memory("overwrite", "version 2", Some(&dir)).unwrap();
        let content = read_team_memory("overwrite", Some(&dir)).unwrap();
        assert_eq!(content, Some("version 2".to_string()));
    }

    #[test]
    fn test_write_appends_md_if_missing() {
        let dir = tmp_dir();
        let path = write_team_memory("notes", "data", Some(&dir)).unwrap();
        assert_eq!(path.file_name().unwrap(), "notes.md");
    }

    #[test]
    fn test_write_preserves_md_extension() {
        let dir = tmp_dir();
        let path = write_team_memory("notes.md", "data", Some(&dir)).unwrap();
        assert_eq!(path.file_name().unwrap(), "notes.md");
        // Should NOT be notes.md.md
        assert!(!path.to_string_lossy().contains("notes.md.md"));
    }

    #[test]
    fn test_write_atomic_no_temp_file_left() {
        let dir = tmp_dir();
        write_team_memory("atomic", "content", Some(&dir)).unwrap();
        // Temp file should not exist after write
        assert!(!dir.join(".atomic.md.tmp").exists());
        // Real file should exist
        assert!(dir.join("atomic.md").exists());
    }

    #[test]
    fn test_write_returns_correct_path() {
        let dir = tmp_dir();
        let path = write_team_memory("my-file", "x", Some(&dir)).unwrap();
        assert_eq!(path, dir.join("my-file.md"));
    }

    #[test]
    #[should_panic(expected = "must not contain path separators")]
    fn test_write_rejects_path_traversal_slash() {
        let dir = tmp_dir();
        let _ = write_team_memory("../etc/passwd", "hacked", Some(&dir));
    }

    #[test]
    #[should_panic(expected = "must not contain path separators")]
    fn test_write_rejects_path_traversal_backslash() {
        let dir = tmp_dir();
        let _ = write_team_memory("..\\etc\\passwd", "hacked", Some(&dir));
    }

    #[test]
    #[should_panic(expected = "must not contain")]
    fn test_write_rejects_double_dot() {
        let dir = tmp_dir();
        let _ = write_team_memory("foo..bar", "data", Some(&dir));
    }

    // ── read_team_memory tests ──────────────────────────────────────────

    #[test]
    fn test_read_existing_file() {
        let dir = tmp_dir();
        write_team_memory("read-test", "hello world", Some(&dir)).unwrap();
        let content = read_team_memory("read-test", Some(&dir)).unwrap();
        assert_eq!(content, Some("hello world".to_string()));
    }

    #[test]
    fn test_read_nonexistent_returns_none() {
        let dir = tmp_dir();
        let content = read_team_memory("nonexistent", Some(&dir)).unwrap();
        assert_eq!(content, None);
    }

    #[test]
    fn test_read_with_md_extension() {
        let dir = tmp_dir();
        write_team_memory("ext-test", "data", Some(&dir)).unwrap();
        // Reading with .md should find the same file
        let content = read_team_memory("ext-test.md", Some(&dir)).unwrap();
        assert_eq!(content, Some("data".to_string()));
    }

    #[test]
    fn test_read_unicode_content() {
        let dir = tmp_dir();
        let content = "# スプリント状況 🚀\nВсе отлично!\nÉcaillon";
        write_team_memory("unicode", content, Some(&dir)).unwrap();
        let read = read_team_memory("unicode", Some(&dir)).unwrap();
        assert_eq!(read, Some(content.to_string()));
    }

    #[test]
    fn test_read_large_content() {
        let dir = tmp_dir();
        let content = "x".repeat(1_000_000); // 1MB
        write_team_memory("large", &content, Some(&dir)).unwrap();
        let read = read_team_memory("large", Some(&dir)).unwrap();
        assert_eq!(read.map(|r| r.len()), Some(1_000_000));
    }

    // ── list_team_memory tests ──────────────────────────────────────────

    #[test]
    fn test_list_empty_directory() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let names = list_team_memory(Some(&dir)).unwrap();
        assert!(names.is_empty());
    }

    #[test]
    fn test_list_returns_sorted_stems() {
        let dir = tmp_dir();
        write_team_memory("gamma", "c", Some(&dir)).unwrap();
        write_team_memory("alpha", "a", Some(&dir)).unwrap();
        write_team_memory("beta", "b", Some(&dir)).unwrap();
        let names = list_team_memory(Some(&dir)).unwrap();
        assert_eq!(names, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn test_list_ignores_non_md_files() {
        let dir = tmp_dir();
        write_team_memory("keep", "data", Some(&dir)).unwrap();
        std::fs::write(dir.join("notes.txt"), "ignored").unwrap();
        std::fs::write(dir.join("data.json"), "{}").unwrap();
        let names = list_team_memory(Some(&dir)).unwrap();
        assert_eq!(names, vec!["keep"]);
    }

    #[test]
    fn test_list_nonexistent_directory_returns_empty() {
        let dir = PathBuf::from("/nonexistent/team/dir/that/does/not/exist");
        let names = list_team_memory(Some(&dir)).unwrap();
        assert!(names.is_empty());
    }

    #[test]
    fn test_list_ignores_temp_files() {
        let dir = tmp_dir();
        write_team_memory("real", "data", Some(&dir)).unwrap();
        // Simulate a leftover temp file
        std::fs::write(dir.join(".partial.md.tmp"), "partial").unwrap();
        let names = list_team_memory(Some(&dir)).unwrap();
        assert_eq!(names, vec!["real"]);
    }

    // ── delete_team_memory tests ────────────────────────────────────────

    #[test]
    fn test_delete_existing_file() {
        let dir = tmp_dir();
        write_team_memory("to-delete", "delete me", Some(&dir)).unwrap();
        assert!(dir.join("to-delete.md").exists());
        delete_team_memory("to-delete", Some(&dir)).unwrap();
        assert!(!dir.join("to-delete.md").exists());
    }

    #[test]
    fn test_delete_nonexistent_is_ok() {
        let dir = tmp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        // Should not error
        delete_team_memory("never-existed", Some(&dir)).unwrap();
    }

    #[test]
    fn test_delete_then_read_returns_none() {
        let dir = tmp_dir();
        write_team_memory("ephemeral", "temporary", Some(&dir)).unwrap();
        delete_team_memory("ephemeral", Some(&dir)).unwrap();
        let content = read_team_memory("ephemeral", Some(&dir)).unwrap();
        assert_eq!(content, None);
    }

    // ── full cycle test ─────────────────────────────────────────────────

    #[test]
    fn test_full_cycle_write_read_list_delete() {
        let dir = tmp_dir();

        // Write
        write_team_memory("cycle", "cycle content", Some(&dir)).unwrap();

        // Read
        let content = read_team_memory("cycle", Some(&dir)).unwrap();
        assert_eq!(content, Some("cycle content".to_string()));

        // List
        let names = list_team_memory(Some(&dir)).unwrap();
        assert!(names.contains(&"cycle".to_string()));

        // Delete
        delete_team_memory("cycle", Some(&dir)).unwrap();
        assert_eq!(read_team_memory("cycle", Some(&dir)).unwrap(), None);
        assert!(!list_team_memory(Some(&dir)).unwrap().contains(&"cycle".to_string()));
    }

    // ── default_team_dir test ───────────────────────────────────────────

    #[test]
    fn test_default_team_dir_path() {
        let dir = default_team_dir();
        assert!(dir.is_some(), "default_team_dir should return Some on systems with a home dir");
        let dir = dir.unwrap();
        let s = dir.to_string_lossy();
        assert!(s.contains(".zeus"), "path should contain .zeus: {}", s);
        assert!(s.contains("team"), "path should contain team: {}", s);
        assert!(dir.ends_with(".zeus/team"), "path should end with .zeus/team: {}", s);
    }

    // ── sanitize_filename tests ─────────────────────────────────────────

    #[test]
    fn test_sanitize_appends_md() {
        assert_eq!(sanitize_filename("sprint-status"), "sprint-status.md");
    }

    #[test]
    fn test_sanitize_preserves_md() {
        assert_eq!(sanitize_filename("sprint-status.md"), "sprint-status.md");
    }

    #[test]
    fn test_sanitize_simple_name() {
        assert_eq!(sanitize_filename("notes"), "notes.md");
    }

    #[test]
    #[should_panic(expected = "must not contain")]
    fn test_sanitize_rejects_slash() {
        sanitize_filename("foo/bar");
    }

    #[test]
    #[should_panic(expected = "must not contain")]
    fn test_sanitize_rejects_backslash() {
        sanitize_filename("foo\\bar");
    }

    #[test]
    #[should_panic(expected = "must not contain")]
    fn test_sanitize_rejects_dotdot() {
        sanitize_filename("..");
    }
}
