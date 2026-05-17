//! Deliverable Packaging
//!
//! Collects artifacts produced during orchestration workflows,
//! generates a summary README, and packages everything into a
//! downloadable ZIP archive at `~/.zeus/deliverables/{id}.zip`.

use chrono::Utc;
use std::io::Write;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Configuration for the packaging engine.
#[derive(Debug, Clone)]
pub struct PackagingConfig {
    /// Base directory for deliverables (default: ~/.zeus/deliverables)
    pub deliverables_dir: PathBuf,
}

impl Default for PackagingConfig {
    fn default() -> Self {
        let base = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".zeus")
            .join("deliverables");
        Self {
            deliverables_dir: base,
        }
    }
}

/// A file to include in the deliverable package.
#[derive(Debug, Clone)]
pub struct PackageEntry {
    /// Path within the ZIP archive (e.g., "src/main.rs")
    pub archive_path: String,
    /// Content of the file
    pub content: Vec<u8>,
}

/// Result of packaging an orchestration session.
#[derive(Debug, Clone)]
pub struct PackageResult {
    /// Absolute path to the generated ZIP file
    pub zip_path: PathBuf,
    /// Number of files included
    pub file_count: usize,
    /// Total size in bytes
    pub size_bytes: u64,
}

/// Package orchestration artifacts into a ZIP archive.
///
/// # Arguments
/// * `session_id` - Orchestration session ID (used for filename)
/// * `goal` - The original goal description
/// * `entries` - Files to include in the package
/// * `transcript` - Optional execution transcript (JSONL lines)
/// * `config` - Packaging configuration
///
/// # Returns
/// Path to the generated ZIP file
pub fn package_deliverable(
    session_id: &str,
    goal: &str,
    entries: &[PackageEntry],
    transcript: Option<&[String]>,
    config: &PackagingConfig,
) -> std::io::Result<PackageResult> {
    // Ensure deliverables directory exists
    std::fs::create_dir_all(&config.deliverables_dir)?;

    let zip_filename = format!("{session_id}.zip");
    let zip_path = config.deliverables_dir.join(&zip_filename);

    let file = std::fs::File::create(&zip_path)?;
    let mut zip = zip::ZipWriter::new(file);

    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // 1. Generate and add README.md
    let readme = generate_readme(session_id, goal, entries, transcript);
    zip.start_file("README.md", options)?;
    zip.write_all(readme.as_bytes())?;

    // 2. Add all artifact entries — validate paths before writing to prevent zip-slip
    for entry in entries {
        if let Err(e) = validate_zip_entry_path(&entry.archive_path) {
            warn!(
                archive_path = %entry.archive_path,
                reason = %e,
                "Skipping unsafe archive entry path"
            );
            continue;
        }
        zip.start_file(&entry.archive_path, options)?;
        zip.write_all(&entry.content)?;
    }

    // 3. Add execution transcript if provided
    if let Some(lines) = transcript
        && !lines.is_empty()
    {
        zip.start_file("transcript.jsonl", options)?;
        for line in lines {
            zip.write_all(line.as_bytes())?;
            zip.write_all(b"\n")?;
        }

        // Also generate a human-readable markdown version
        let md = transcript_to_markdown(lines);
        zip.start_file("transcript.md", options)?;
        zip.write_all(md.as_bytes())?;
    }

    let result = zip.finish()?;
    drop(result);

    let metadata = std::fs::metadata(&zip_path)?;
    let file_count = entries.len() + 1 + if transcript.is_some() { 2 } else { 0 };

    info!(
        session_id = session_id,
        file_count = file_count,
        size_bytes = metadata.len(),
        "Packaged deliverable"
    );

    Ok(PackageResult {
        zip_path,
        file_count,
        size_bytes: metadata.len(),
    })
}

/// Validate an archive entry path for ZIP path-traversal safety.
///
/// Rejects paths that:
/// - Are absolute (`/etc/passwd`, `C:\Windows\...`)
/// - Contain `..` components (directory traversal)
/// - Are empty
///
/// Returns `Err` with a descriptive message when the path is unsafe.
pub fn validate_zip_entry_path(archive_path: &str) -> Result<(), String> {
    if archive_path.is_empty() {
        return Err("archive entry path must not be empty".into());
    }

    let p = Path::new(archive_path);

    if p.is_absolute() {
        return Err(format!(
            "archive entry '{}' is an absolute path — potential zip-slip",
            archive_path
        ));
    }

    for component in p.components() {
        use std::path::Component;
        if matches!(component, Component::ParentDir) {
            return Err(format!(
                "archive entry '{}' contains '..' component — potential zip-slip",
                archive_path
            ));
        }
    }

    Ok(())
}

/// Collect files from a directory into PackageEntries.
///
/// Reads all files under `root_dir` and creates entries with paths
/// relative to `root_dir`. Symlinks are checked to ensure their resolved
/// canonical path stays within `root_dir` — symlinks that escape the root
/// are skipped with a warning to prevent zip-slip via symlink escape.
pub fn collect_directory(root_dir: &Path) -> std::io::Result<Vec<PackageEntry>> {
    let mut entries = Vec::new();

    if !root_dir.exists() {
        return Ok(entries);
    }

    // Canonicalize root for symlink containment checks
    let canonical_root = root_dir.canonicalize()?;

    fn walk(
        dir: &Path,
        root: &Path,
        canonical_root: &Path,
        entries: &mut Vec<PackageEntry>,
    ) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            // Symlink containment check: resolve and verify it stays inside root_dir.
            // This prevents a malicious symlink like: artifacts/escape -> /etc/passwd
            // from being bundled into the ZIP and later extracted outside the target dir.
            if path.is_symlink() {
                match path.canonicalize() {
                    Ok(canonical) => {
                        if !canonical.starts_with(canonical_root) {
                            warn!(
                                path = %path.display(),
                                target = %canonical.display(),
                                "Skipping symlink that escapes root directory"
                            );
                            continue;
                        }
                    }
                    Err(e) => {
                        warn!(
                            path = %path.display(),
                            error = %e,
                            "Skipping symlink with unresolvable target"
                        );
                        continue;
                    }
                }
            }

            if path.is_dir() {
                walk(&path, root, canonical_root, entries)?;
            } else if path.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();

                // Validate archive path before adding
                if let Err(e) = validate_zip_entry_path(&relative) {
                    warn!(path = %path.display(), reason = %e, "Skipping unsafe archive path");
                    continue;
                }

                match std::fs::read(&path) {
                    Ok(content) => {
                        entries.push(PackageEntry {
                            archive_path: relative,
                            content,
                        });
                    }
                    Err(e) => {
                        warn!(path = %path.display(), error = %e, "Skipping unreadable file");
                    }
                }
            }
        }
        Ok(())
    }

    walk(root_dir, root_dir, &canonical_root, &mut entries)?;
    Ok(entries)
}

/// Generate a README.md summarizing the deliverable.
fn generate_readme(
    session_id: &str,
    goal: &str,
    entries: &[PackageEntry],
    transcript: Option<&[String]>,
) -> String {
    let now = Utc::now().format("%Y-%m-%d %H:%M UTC");
    let file_list: String = entries
        .iter()
        .map(|e| format!("- `{}`", e.archive_path))
        .collect::<Vec<_>>()
        .join("\n");

    let transcript_note = if transcript.is_some() {
        "\n## Execution Transcript\n\nSee `transcript.md` for a human-readable execution log, \
         or `transcript.jsonl` for the raw structured log.\n"
    } else {
        ""
    };

    format!(
        "# Zeus Deliverable\n\n\
         **Goal**: {goal}\n\
         **Session**: `{session_id}`\n\
         **Generated**: {now}\n\n\
         ## Files\n\n\
         {file_list}\n\
         {transcript_note}\n\
         ---\n\
         *Generated by Zeus Orchestration Engine*\n"
    )
}

/// Convert JSONL transcript lines into readable markdown.
fn transcript_to_markdown(lines: &[String]) -> String {
    let mut md = String::from("# Execution Transcript\n\n");

    for (i, line) in lines.iter().enumerate() {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(line) {
            let step = parsed
                .get("step")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let status = parsed
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let output = parsed.get("output").and_then(|v| v.as_str()).unwrap_or("");

            md.push_str(&format!("### Step {} — {}\n\n", i + 1, step));
            md.push_str(&format!("**Status**: {status}\n\n"));
            if !output.is_empty() {
                md.push_str(&format!("```\n{output}\n```\n\n"));
            }
        } else {
            // Raw text line
            md.push_str(&format!("```\n{line}\n```\n\n"));
        }
    }

    md
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn test_package_deliverable() {
        let tmp = std::env::temp_dir().join("zeus-pkg-test");
        let _ = std::fs::remove_dir_all(&tmp);

        let config = PackagingConfig {
            deliverables_dir: tmp.clone(),
        };

        let entries = vec![
            PackageEntry {
                archive_path: "src/main.rs".to_string(),
                content: b"fn main() { println!(\"hello\"); }".to_vec(),
            },
            PackageEntry {
                archive_path: "Cargo.toml".to_string(),
                content: b"[package]\nname = \"test\"".to_vec(),
            },
        ];

        let transcript = vec![
            r#"{"step":"create_project","status":"completed","output":"Created project"}"#
                .to_string(),
        ];

        let result = package_deliverable(
            "orch-test-123",
            "Build a hello world app",
            &entries,
            Some(&transcript),
            &config,
        )
        .unwrap();

        assert!(result.zip_path.exists());
        assert_eq!(result.file_count, 5); // README + 2 entries + transcript.jsonl + transcript.md
        assert!(result.size_bytes > 0);

        // Verify ZIP contents
        let file = std::fs::File::open(&result.zip_path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();

        assert!(names.contains(&"README.md".to_string()));
        assert!(names.contains(&"src/main.rs".to_string()));
        assert!(names.contains(&"Cargo.toml".to_string()));
        assert!(names.contains(&"transcript.jsonl".to_string()));
        assert!(names.contains(&"transcript.md".to_string()));

        // Check README content
        let mut readme_content = String::new();
        archive
            .by_name("README.md")
            .unwrap()
            .read_to_string(&mut readme_content)
            .unwrap();
        assert!(readme_content.contains("Build a hello world app"));
        assert!(readme_content.contains("orch-test-123"));

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_collect_directory() {
        let tmp = std::env::temp_dir().join("zeus-collect-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("sub")).unwrap();

        std::fs::write(tmp.join("file1.txt"), "hello").unwrap();
        std::fs::write(tmp.join("sub/file2.txt"), "world").unwrap();

        let entries = collect_directory(&tmp).unwrap();
        assert_eq!(entries.len(), 2);

        let paths: Vec<&str> = entries.iter().map(|e| e.archive_path.as_str()).collect();
        assert!(paths.contains(&"file1.txt"));
        assert!(paths.contains(&"sub/file2.txt"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── ZIP path-traversal safety tests ─────────────────────────────────────

    #[test]
    fn test_validate_zip_entry_path_normal() {
        assert!(validate_zip_entry_path("src/main.rs").is_ok());
        assert!(validate_zip_entry_path("README.md").is_ok());
        assert!(validate_zip_entry_path("deep/nested/path/file.txt").is_ok());
    }

    #[test]
    fn test_validate_zip_entry_path_empty_rejected() {
        assert!(validate_zip_entry_path("").is_err());
    }

    #[test]
    fn test_validate_zip_entry_path_absolute_rejected() {
        let err = validate_zip_entry_path("/etc/passwd").unwrap_err();
        assert!(err.contains("absolute"));
    }

    #[test]
    fn test_validate_zip_entry_path_traversal_rejected() {
        let cases = ["../escape.txt", "a/../../etc/passwd", "foo/../../../etc"];
        for case in &cases {
            let result = validate_zip_entry_path(case);
            assert!(result.is_err(), "Should reject: {}", case);
            assert!(
                result.unwrap_err().contains(".."),
                "Error should mention '..'"
            );
        }
    }

    #[test]
    fn test_package_deliverable_skips_unsafe_paths() {
        let tmp = std::env::temp_dir().join("zeus-pkg-safe-test");
        let _ = std::fs::remove_dir_all(&tmp);
        let config = PackagingConfig {
            deliverables_dir: tmp.clone(),
        };

        let entries = vec![
            PackageEntry {
                archive_path: "safe.rs".to_string(),
                content: b"fn main() {}".to_vec(),
            },
            PackageEntry {
                archive_path: "../escape.txt".to_string(),
                content: b"should not appear".to_vec(),
            },
            PackageEntry {
                archive_path: "/etc/passwd".to_string(),
                content: b"should not appear".to_vec(),
            },
        ];

        let result = package_deliverable("pkg-safe-1", "test", &entries, None, &config).unwrap();
        assert!(result.zip_path.exists());

        // Verify only the safe entry made it in
        let file = std::fs::File::open(&result.zip_path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();

        assert!(names.contains(&"safe.rs".to_string()));
        assert!(!names.iter().any(|n| n.contains("escape")));
        assert!(!names.iter().any(|n| n.contains("passwd")));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(unix)]
    #[test]
    fn test_collect_directory_skips_escaping_symlinks() {
        use std::os::unix::fs::symlink;

        let tmp = std::env::temp_dir().join("zeus-symlink-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // Create a legitimate file
        std::fs::write(tmp.join("safe.txt"), "hello").unwrap();

        // Create a symlink that escapes the root directory
        let escape_target = std::env::temp_dir().join("zeus-symlink-escape-target.txt");
        std::fs::write(&escape_target, "secret").unwrap();
        symlink(&escape_target, tmp.join("escape_link.txt")).unwrap();

        let entries = collect_directory(&tmp).unwrap();

        // Only the safe file should be included
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].archive_path, "safe.txt");

        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_file(&escape_target);
    }

    #[test]
    fn test_generate_readme() {
        let entries = vec![PackageEntry {
            archive_path: "main.py".to_string(),
            content: b"print('hi')".to_vec(),
        }];
        let readme = generate_readme("orch-1", "Make a Python script", &entries, None);
        assert!(readme.contains("Make a Python script"));
        assert!(readme.contains("`main.py`"));
    }

    #[test]
    fn test_transcript_to_markdown() {
        let lines = vec![
            r#"{"step":"init","status":"completed","output":"Initialized project"}"#.to_string(),
            r#"{"step":"build","status":"failed","output":"Compile error"}"#.to_string(),
        ];
        let md = transcript_to_markdown(&lines);
        assert!(md.contains("Step 1 — init"));
        assert!(md.contains("Step 2 — build"));
        assert!(md.contains("Initialized project"));
    }
}
