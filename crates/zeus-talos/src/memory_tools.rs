//! Simple file-based memory tools for Zeus MCP.
//!
//! Provides `memory_recall`, `memory_store`, and `memory_search` tools that
//! read/write `~/.zeus/workspace/memory/MEMORY.md` — no database required.
//! These are the basic memory tools agents use for session persistence.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use zeus_core::{Result, ToolSchema};

/// Resolve the memory file path from env or default.
fn memory_file_path() -> String {
    let workspace = std::env::var("ZEUS_WORKSPACE").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        format!("{}/.zeus/workspace", home)
    });
    format!("{}/memory/MEMORY.md", workspace)
}

/// Ensure parent directories exist and the file exists.
fn ensure_memory_file(path: &str) {
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if !std::path::Path::new(path).exists() {
        let _ = std::fs::write(path, "# Zeus Memory\n\n");
    }
}

pub struct MemoryRecallTool;
pub struct MemoryStoreTool;
pub struct MemorySearchTool;

#[async_trait]
impl TalosTool for MemoryRecallTool {
    fn name(&self) -> &'static str {
        "memory_recall"
    }
    fn description(&self) -> &'static str {
        "Recall all stored memories from the workspace memory file"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "memory_recall".to_string(),
            description: "Read the full contents of the Zeus memory file (~/.zeus/workspace/memory/MEMORY.md). Call this at session start to load context.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        let path = memory_file_path();
        ensure_memory_file(&path);
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                if content.trim().is_empty() || content.trim() == "# Zeus Memory" {
                    Ok("No memories stored yet.".to_string())
                } else {
                    Ok(content)
                }
            }
            Err(e) => Ok(format!("Failed to read memory file: {}", e)),
        }
    }
}

#[async_trait]
impl TalosTool for MemoryStoreTool {
    fn name(&self) -> &'static str {
        "memory_store"
    }
    fn description(&self) -> &'static str {
        "Store a memory entry to the workspace memory file"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "memory_store".to_string(),
            description: "Append a memory entry to the Zeus memory file (~/.zeus/workspace/memory/MEMORY.md). Use for session summaries, learnings, and persistent facts. Supports 'append' (default) or 'replace' mode.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "Memory content to store (markdown)"
                    },
                    "mode": {
                        "type": "string",
                        "description": "Write mode: 'append' (add to end) or 'replace' (overwrite entire file). Default: append",
                        "enum": ["append", "replace"],
                        "default": "append"
                    }
                },
                "required": ["content"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                zeus_core::Error::Tool("Missing required 'content' parameter".to_string())
            })?;

        let mode = args
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("append");

        let path = memory_file_path();
        ensure_memory_file(&path);

        match mode {
            "replace" => {
                std::fs::write(&path, content).map_err(|e| {
                    zeus_core::Error::Tool(format!("Failed to write memory: {}", e))
                })?;
                Ok(format!("Memory file replaced ({} bytes)", content.len()))
            }
            _ => {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .append(true)
                    .open(&path)
                    .map_err(|e| zeus_core::Error::Tool(format!("Failed to open memory: {}", e)))?;
                writeln!(file, "\n{}", content)
                    .map_err(|e| zeus_core::Error::Tool(format!("Failed to append: {}", e)))?;
                Ok(format!("Memory appended ({} bytes)", content.len()))
            }
        }
    }
}

#[async_trait]
impl TalosTool for MemorySearchTool {
    fn name(&self) -> &'static str {
        "memory_search"
    }
    fn description(&self) -> &'static str {
        "Search memories by keyword"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "memory_search".to_string(),
            description: "Search the Zeus memory file for lines matching a query (case-insensitive). Returns matching sections with context.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query (case-insensitive substring match)"
                    }
                },
                "required": ["query"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let query = args.get("query").and_then(|v| v.as_str()).ok_or_else(|| {
            zeus_core::Error::Tool("Missing required 'query' parameter".to_string())
        })?;

        let path = memory_file_path();
        ensure_memory_file(&path);

        let content = std::fs::read_to_string(&path)
            .map_err(|e| zeus_core::Error::Tool(format!("Failed to read memory: {}", e)))?;

        let query_lower = query.to_lowercase();
        let mut matches: Vec<String> = Vec::new();
        let mut current_section = String::new();
        let mut section_matches = false;

        for line in content.lines() {
            if line.starts_with('#') {
                if section_matches && !current_section.is_empty() {
                    matches.push(current_section.clone());
                }
                current_section = line.to_string();
                section_matches = line.to_lowercase().contains(&query_lower);
            } else {
                current_section.push('\n');
                current_section.push_str(line);
                if line.to_lowercase().contains(&query_lower) {
                    section_matches = true;
                }
            }
        }
        if section_matches && !current_section.is_empty() {
            matches.push(current_section);
        }

        if matches.is_empty() {
            Ok(format!("No memories found matching '{}'", query))
        } else {
            Ok(format!(
                "Found {} matching sections:\n\n{}",
                matches.len(),
                matches.join("\n\n---\n\n")
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_recall_schema() {
        let tool = MemoryRecallTool;
        assert_eq!(tool.schema().name, "memory_recall");
    }

    #[test]
    fn test_memory_store_schema() {
        let tool = MemoryStoreTool;
        assert_eq!(tool.schema().name, "memory_store");
    }

    #[test]
    fn test_memory_search_schema() {
        let tool = MemorySearchTool;
        assert_eq!(tool.schema().name, "memory_search");
    }

    // Note: integration tests for recall/store/search require ZEUS_WORKSPACE env var
    // which cannot be set safely in parallel tests (set_var is unsafe in Rust 2024+).
    // The schema tests above validate tool registration; end-to-end testing is done
    // via MCP tool calls in the running Zeus instance.
}
