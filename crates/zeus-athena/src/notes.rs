//! Apple Notes integration (macOS only)

use zeus_core::{Error, Result};

#[allow(dead_code)]
/// Sanitize a string for safe interpolation into AppleScript double-quoted strings.
fn sanitize_applescript(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"), // must come first
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"), // prevent multi-line script injection
            '\r' => out.push_str("\\r"),
            '\0' => {} // strip NUL bytes
            c => out.push(c),
        }
    }
    out
}

/// Apple Notes adapter
#[cfg(target_os = "macos")]
pub struct AppleNotes;

#[cfg(target_os = "macos")]
impl AppleNotes {
    /// Create a new note
    pub async fn create_note(title: &str, content: &str, folder: Option<&str>) -> Result<String> {
        let folder_clause = folder
            .map(|f| format!(" in folder \"{}\"", sanitize_applescript(f)))
            .unwrap_or_default();

        let script = format!(
            r#"
            tell application "Notes"
                set newNote to make new note{} with properties {{name:"{}", body:"{}"}}
                return id of newNote
            end tell
            "#,
            folder_clause,
            sanitize_applescript(title),
            sanitize_applescript(content)
        );

        execute_applescript(&script).await
    }

    /// Append to an existing note
    pub async fn append_note(note_id: &str, content: &str) -> Result<()> {
        let script = format!(
            r#"
            tell application "Notes"
                set targetNote to note id "{}"
                set body of targetNote to (body of targetNote) & "
{}"
            end tell
            "#,
            sanitize_applescript(note_id),
            sanitize_applescript(content)
        );

        execute_applescript(&script).await?;
        Ok(())
    }

    /// Search notes
    pub async fn search_notes(query: &str) -> Result<Vec<NoteMatch>> {
        let escaped = sanitize_applescript(query);
        let script = format!(
            r#"
            tell application "Notes"
                set matchingNotes to notes whose name contains "{}" or body contains "{}"
                set results to {{}}
                repeat with n in matchingNotes
                    set end of results to {{id:id of n, name:name of n}}
                end repeat
                return results
            end tell
            "#,
            escaped, escaped
        );

        let raw = execute_applescript(&script).await?;
        Ok(parse_note_matches(&raw))
    }

    /// List folders
    pub async fn list_folders() -> Result<Vec<String>> {
        let script = r#"
            tell application "Notes"
                set folderNames to name of folders
                return folderNames
            end tell
        "#;

        let raw = execute_applescript(script).await?;
        Ok(parse_comma_list(&raw))
    }
}

/// A note search match
pub struct NoteMatch {
    /// Note ID
    pub id: String,
    /// Note title
    pub title: String,
}

/// Parse AppleScript output for note matches.
///
/// AppleScript returns records in the form:
/// `{{id:"x-coredata://...", name:"Title"}, {id:"x-coredata://...", name:"Title2"}}`
/// or for a single result: `{id:"x-coredata://...", name:"Title"}`
/// or an empty string / `{}` for no results.
#[cfg(target_os = "macos")]
fn parse_note_matches(raw: &str) -> Vec<NoteMatch> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "{}" {
        return Vec::new();
    }

    let mut results = Vec::new();

    // Find all id:..., name:... pairs
    // AppleScript output uses `id:` and `name:` fields within braces
    let mut remaining = trimmed;
    while let Some(id_pos) = remaining.find("id:") {
        remaining = &remaining[id_pos + 3..];
        // Extract the id value — could be quoted or unquoted
        let id_val = extract_applescript_value(remaining);
        if let Some(name_pos) = remaining.find("name:") {
            let after_name = &remaining[name_pos + 5..];
            let name_val = extract_applescript_value(after_name);
            if !id_val.is_empty() {
                results.push(NoteMatch {
                    id: id_val,
                    title: name_val,
                });
            }
            remaining = after_name;
        }
    }

    results
}

/// Extract a value from AppleScript output, handling quoted and unquoted forms.
#[cfg(target_os = "macos")]
fn extract_applescript_value(s: &str) -> String {
    let s = s.trim();
    if let Some(stripped) = s.strip_prefix('"') {
        // Quoted value — find closing quote
        if let Some(end) = stripped.find('"') {
            return stripped[..end].to_string();
        }
    }
    // Unquoted — take until comma, brace, or end
    let end = s.find([',', '}', '\n']).unwrap_or(s.len());
    s[..end].trim().to_string()
}

/// Parse a comma-separated list from AppleScript output.
///
/// AppleScript returns folder names like: `"Notes", "Folder2", "Archive"`
/// or `Notes, Folder2, Archive` (unquoted).
#[cfg(target_os = "macos")]
fn parse_comma_list(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    trimmed
        .split(',')
        .map(|s| {
            let s = s.trim();
            // Strip surrounding quotes if present
            if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
                s[1..s.len() - 1].to_string()
            } else {
                s.to_string()
            }
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// Execute AppleScript (macOS only)
#[cfg(target_os = "macos")]
async fn execute_applescript(script: &str) -> Result<String> {
    use tokio::process::Command;

    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .await
        .map_err(|e| Error::Internal(format!("Failed to execute AppleScript: {}", e)))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(Error::Internal(format!(
            "AppleScript error: {}",
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

// ── Unit tests ──────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_applescript_passthrough() {
        assert_eq!(sanitize_applescript("hello world"), "hello world");
    }

    #[test]
    fn test_sanitize_applescript_escapes_backslash() {
        assert_eq!(sanitize_applescript("a\\b"), "a\\\\b");
    }

    #[test]
    fn test_sanitize_applescript_escapes_double_quote() {
        assert_eq!(sanitize_applescript(r#"say "hi""#), r#"say \"hi\""#);
    }

    #[test]
    fn test_sanitize_applescript_escapes_newline() {
        // Unescaped LF would break the single-line script string — must become \n
        assert_eq!(sanitize_applescript("line1\nline2"), "line1\\nline2");
    }

    #[test]
    fn test_sanitize_applescript_escapes_cr() {
        assert_eq!(sanitize_applescript("line1\rline2"), "line1\\rline2");
    }

    #[test]
    fn test_sanitize_applescript_strips_nul() {
        assert_eq!(sanitize_applescript("foo\0bar"), "foobar");
    }

    #[test]
    fn test_sanitize_applescript_multiline_injection() {
        // A classic multi-line injection attempt: the attacker closes the string
        // with `"`, then starts a new statement on a fresh line.
        let malicious = "legit\"\ndo shell script \"rm -rf ~";
        let sanitized = sanitize_applescript(malicious);
        // After sanitization the newline must not be literal
        assert!(
            !sanitized.contains('\n'),
            "raw newline must not survive sanitization, got: {:?}",
            sanitized
        );
        // The LF must be escaped as the two-character sequence \n
        assert!(
            sanitized.contains("\\n"),
            "newline must be represented as \\n, got: {:?}",
            sanitized
        );
    }

    #[test]
    fn test_sanitize_applescript_crlf_injection() {
        let malicious = "title\r\nmalicious line";
        let sanitized = sanitize_applescript(malicious);
        assert!(!sanitized.contains('\r'));
        assert!(!sanitized.contains('\n'));
    }
}

/// Stub for non-macOS platforms
#[cfg(not(target_os = "macos"))]
pub struct AppleNotes;

#[cfg(not(target_os = "macos"))]
impl AppleNotes {
    pub async fn create_note(
        _title: &str,
        _content: &str,
        _folder: Option<&str>,
    ) -> Result<String> {
        Err(Error::Internal(
            "Apple Notes only available on macOS".into(),
        ))
    }

    pub async fn append_note(_note_id: &str, _content: &str) -> Result<()> {
        Err(Error::Internal(
            "Apple Notes only available on macOS".into(),
        ))
    }

    pub async fn search_notes(_query: &str) -> Result<Vec<NoteMatch>> {
        Err(Error::Internal(
            "Apple Notes only available on macOS".into(),
        ))
    }

    pub async fn list_folders() -> Result<Vec<String>> {
        Err(Error::Internal(
            "Apple Notes only available on macOS".into(),
        ))
    }
}
