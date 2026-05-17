//! Apple Notes tools (macOS)

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
use zeus_core::{Result, ToolSchema};

#[cfg(target_os = "macos")]
use crate::run_applescript;

/// List notes
pub struct NotesListTool;

#[async_trait]
impl TalosTool for NotesListTool {
    fn name(&self) -> &'static str {
        "notes_list"
    }
    fn description(&self) -> &'static str {
        "List notes from Apple Notes"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("folder", "string", "Folder name (optional)", false)
            .with_param(
                "limit",
                "integer",
                "Max notes to return (default 20)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20);

            let script = format!(
                r#"
                set noteList to ""
                tell application "Notes"
                    set noteCount to 0
                    repeat with n in notes
                        if noteCount >= {} then exit repeat
                        set noteList to noteList & (name of n) & linefeed
                        set noteCount to noteCount + 1
                    end repeat
                end tell
                return noteList
            "#,
                limit
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Notes tools only available on macOS".to_string())
        }
    }
}

/// Create a note
pub struct NotesCreateTool;

#[async_trait]
impl TalosTool for NotesCreateTool {
    fn name(&self) -> &'static str {
        "notes_create"
    }
    fn description(&self) -> &'static str {
        "Create a new note in Apple Notes"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("title", "string", "Note title", true)
            .with_param("body", "string", "Note content", true)
            .with_param("folder", "string", "Folder name (optional)", false)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            use zeus_core::Error;

            let title = args
                .get("title")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing title".to_string()))?;

            let body = args
                .get("body")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing body".to_string()))?;

            let script = format!(
                r#"
                tell application "Notes"
                    make new note with properties {{name:"{}", body:"{}"}}
                end tell
                return "Note created"
            "#,
                crate::sanitize_applescript(title),
                crate::sanitize_applescript(body)
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Notes tools only available on macOS".to_string())
        }
    }
}

/// Read note content
pub struct NotesReadTool;

#[async_trait]
impl TalosTool for NotesReadTool {
    fn name(&self) -> &'static str {
        "notes_read"
    }
    fn description(&self) -> &'static str {
        "Read the content of a note by name"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "name",
            "string",
            "Note name/title to read",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            use zeus_core::Error;

            let name = args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing name".to_string()))?;

            let script = format!(
                r#"
                tell application "Notes"
                    set matchingNotes to (every note whose name is "{}")
                    if (count of matchingNotes) > 0 then
                        set noteBody to body of (item 1 of matchingNotes)
                        return noteBody
                    else
                        return "Note not found"
                    end if
                end tell
            "#,
                crate::sanitize_applescript(name)
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Notes tools only available on macOS".to_string())
        }
    }
}

/// Search notes
pub struct NotesSearchTool;

#[async_trait]
impl TalosTool for NotesSearchTool {
    fn name(&self) -> &'static str {
        "notes_search"
    }
    fn description(&self) -> &'static str {
        "Search notes by keyword"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "query",
            "string",
            "Search query",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            use zeus_core::Error;

            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing query".to_string()))?;

            let escaped = crate::sanitize_applescript(query);
            let script = format!(
                r#"
                set noteList to ""
                tell application "Notes"
                    repeat with n in notes
                        if (name of n) contains "{}" or (body of n) contains "{}" then
                            set noteList to noteList & (name of n) & linefeed
                        end if
                    end repeat
                end tell
                return noteList
            "#,
                escaped, escaped
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Notes tools only available on macOS".to_string())
        }
    }
}

// === NOTES ADDITIONS ===

/// Delete a note by name
pub struct NotesDeleteTool;

#[async_trait]
impl TalosTool for NotesDeleteTool {
    fn name(&self) -> &'static str {
        "notes_delete"
    }
    fn description(&self) -> &'static str {
        "Delete a note by name"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "name",
            "string",
            "Note name to delete",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            use zeus_core::Error;

            let name = args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing name".to_string()))?;

            let script = format!(
                r#"
                tell application "Notes"
                    set matchingNotes to (every note whose name is "{}")
                    if (count of matchingNotes) > 0 then
                        delete (item 1 of matchingNotes)
                        return "Note deleted"
                    else
                        return "Note not found"
                    end if
                end tell
            "#,
                crate::sanitize_applescript(name)
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Notes tools only available on macOS".to_string())
        }
    }
}

/// Update note content
pub struct NotesUpdateTool;

#[async_trait]
impl TalosTool for NotesUpdateTool {
    fn name(&self) -> &'static str {
        "notes_update"
    }
    fn description(&self) -> &'static str {
        "Update the content of a note"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("name", "string", "Note name to update", true)
            .with_param("body", "string", "New note content", true)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            use zeus_core::Error;

            let name = args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing name".to_string()))?;

            let body = args
                .get("body")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing body".to_string()))?;

            let script = format!(
                r#"
                tell application "Notes"
                    set matchingNotes to (every note whose name is "{}")
                    if (count of matchingNotes) > 0 then
                        set body of (item 1 of matchingNotes) to "{}"
                        return "Note updated"
                    else
                        return "Note not found"
                    end if
                end tell
            "#,
                crate::sanitize_applescript(name),
                crate::sanitize_applescript(body)
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Notes tools only available on macOS".to_string())
        }
    }
}

/// Move a note to a different folder
pub struct NotesMoveTool;

#[async_trait]
impl TalosTool for NotesMoveTool {
    fn name(&self) -> &'static str {
        "notes_move"
    }
    fn description(&self) -> &'static str {
        "Move a note to a different folder"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("name", "string", "Note name to move", true)
            .with_param("folder", "string", "Destination folder name", true)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            use zeus_core::Error;

            let name = args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing name".to_string()))?;

            let folder = args
                .get("folder")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing folder".to_string()))?;

            let script = format!(
                r#"
                tell application "Notes"
                    set matchingNotes to (every note whose name is "{}")
                    if (count of matchingNotes) > 0 then
                        set targetFolder to folder "{}"
                        move (item 1 of matchingNotes) to targetFolder
                        return "Note moved"
                    else
                        return "Note not found"
                    end if
                end tell
            "#,
                crate::sanitize_applescript(name),
                crate::sanitize_applescript(folder)
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Notes tools only available on macOS".to_string())
        }
    }
}

/// List all note folders
pub struct NotesFoldersTool;

#[async_trait]
impl TalosTool for NotesFoldersTool {
    fn name(&self) -> &'static str {
        "notes_folders"
    }
    fn description(&self) -> &'static str {
        "List all note folders"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let _ = &args;
            let script = r#"
                set folderList to ""
                tell application "Notes"
                    repeat with f in folders
                        set folderList to folderList & (name of f) & linefeed
                    end repeat
                end tell
                return folderList
            "#;

            run_applescript(script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Notes tools only available on macOS".to_string())
        }
    }
}

/// Append text to an existing note
pub struct NotesAppendTool;

#[async_trait]
impl TalosTool for NotesAppendTool {
    fn name(&self) -> &'static str {
        "notes_append"
    }
    fn description(&self) -> &'static str {
        "Append text to an existing note"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("name", "string", "Note name to append to", true)
            .with_param("text", "string", "Text to append", true)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            use zeus_core::Error;

            let name = args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing name".to_string()))?;

            let text = args
                .get("text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing text".to_string()))?;

            let script = format!(
                r#"
                tell application "Notes"
                    set matchingNotes to (every note whose name is "{}")
                    if (count of matchingNotes) > 0 then
                        set theNote to item 1 of matchingNotes
                        set currentBody to body of theNote
                        set body of theNote to currentBody & "<br>" & "{}"
                        return "Text appended"
                    else
                        return "Note not found"
                    end if
                end tell
            "#,
                crate::sanitize_applescript(name),
                crate::sanitize_applescript(text)
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Notes tools only available on macOS".to_string())
        }
    }
}
