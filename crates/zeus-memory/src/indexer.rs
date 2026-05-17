//! Workspace Indexer for fast file lookup and search.
//!
//! Builds an in-memory index of workspace files for:
//!
//! - **FileIndex** — metadata index of all workspace files
//! - **FileEntry** — metadata for a single indexed file (size, modified, type)
//! - **SearchResult** — ranked search results across file contents
//! - **IndexStats** — index statistics (file count, total size, by type)

use std::collections::HashMap;

use chrono::{DateTime, Utc};

// ============================================================================
// File types
// ============================================================================

/// Type classification of a workspace file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileType {
    Markdown,
    Config,
    Data,
    Code,
    Unknown,
}

impl FileType {
    /// Classify a file by its extension.
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "md" | "markdown" => FileType::Markdown,
            "toml" | "yaml" | "yml" | "json" | "ini" | "cfg" => FileType::Config,
            "csv" | "tsv" | "jsonl" | "db" | "sqlite" => FileType::Data,
            "rs" | "py" | "js" | "ts" | "sh" | "lua" | "rb" => FileType::Code,
            _ => FileType::Unknown,
        }
    }

    /// Display name.
    pub fn as_str(&self) -> &'static str {
        match self {
            FileType::Markdown => "markdown",
            FileType::Config => "config",
            FileType::Data => "data",
            FileType::Code => "code",
            FileType::Unknown => "unknown",
        }
    }
}

// ============================================================================
// File entry
// ============================================================================

/// Metadata for a single indexed file.
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// Relative path from workspace root.
    pub path: String,
    /// File name (without directory).
    pub name: String,
    /// File extension.
    pub extension: String,
    /// Classified file type.
    pub file_type: FileType,
    /// Size in bytes.
    pub size_bytes: u64,
    /// Last modified timestamp.
    pub modified_at: DateTime<Utc>,
    /// When this entry was indexed.
    pub indexed_at: DateTime<Utc>,
    /// First line of the file (for previews).
    pub first_line: Option<String>,
    /// Number of lines in the file.
    pub line_count: usize,
    /// Tags extracted from file content (e.g., markdown headers).
    pub tags: Vec<String>,
}

impl FileEntry {
    /// Create a new file entry.
    pub fn new(path: &str, name: &str, size_bytes: u64) -> Self {
        let extension = name.rsplit('.').next().unwrap_or("").to_string();
        let file_type = FileType::from_extension(&extension);

        Self {
            path: path.to_string(),
            name: name.to_string(),
            extension,
            file_type,
            size_bytes,
            modified_at: Utc::now(),
            indexed_at: Utc::now(),
            first_line: None,
            line_count: 0,
            tags: Vec::new(),
        }
    }

    /// Set the first line preview.
    pub fn with_first_line(mut self, line: &str) -> Self {
        self.first_line = Some(line.to_string());
        self
    }

    /// Set line count.
    pub fn with_line_count(mut self, count: usize) -> Self {
        self.line_count = count;
        self
    }

    /// Set tags.
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Set modified time.
    pub fn with_modified(mut self, modified: DateTime<Utc>) -> Self {
        self.modified_at = modified;
        self
    }
}

// ============================================================================
// Search result
// ============================================================================

/// A search result from the file index.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// The matching file entry.
    pub entry: FileEntry,
    /// Relevance score (higher = more relevant).
    pub score: f64,
    /// Matching context (line containing the match).
    pub context: Option<String>,
    /// Line number of the match (1-based).
    pub line_number: Option<usize>,
}

// ============================================================================
// Index statistics
// ============================================================================

/// Statistics about the file index.
#[derive(Debug, Clone, Default)]
pub struct IndexStats {
    pub total_files: usize,
    pub total_size_bytes: u64,
    pub total_lines: usize,
    pub files_by_type: HashMap<FileType, usize>,
    pub last_indexed_at: Option<DateTime<Utc>>,
}

impl IndexStats {
    /// Average file size in bytes.
    pub fn avg_file_size(&self) -> f64 {
        if self.total_files == 0 {
            return 0.0;
        }
        self.total_size_bytes as f64 / self.total_files as f64
    }
}

// ============================================================================
// File Index
// ============================================================================

/// In-memory index of workspace files.
pub struct FileIndex {
    entries: HashMap<String, FileEntry>,
    /// Inverted index: term → list of (path, score).
    term_index: HashMap<String, Vec<(String, f64)>>,
}

impl FileIndex {
    /// Create a new empty index.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            term_index: HashMap::new(),
        }
    }

    /// Add or update a file entry.
    pub fn add(&mut self, entry: FileEntry) {
        // Index terms from name, tags, and first_line
        let path = entry.path.clone();

        // Remove old terms if updating
        self.remove_from_term_index(&path);

        // Index the file name (high weight)
        for term in tokenize(&entry.name) {
            self.term_index
                .entry(term)
                .or_default()
                .push((path.clone(), 3.0));
        }

        // Index tags (medium weight)
        for tag in &entry.tags {
            for term in tokenize(tag) {
                self.term_index
                    .entry(term)
                    .or_default()
                    .push((path.clone(), 2.0));
            }
        }

        // Index first line (low weight)
        if let Some(ref line) = entry.first_line {
            for term in tokenize(line) {
                self.term_index
                    .entry(term)
                    .or_default()
                    .push((path.clone(), 1.0));
            }
        }

        self.entries.insert(path, entry);
    }

    /// Remove a file from the index.
    pub fn remove(&mut self, path: &str) -> Option<FileEntry> {
        self.remove_from_term_index(path);
        self.entries.remove(path)
    }

    /// Get a file entry by path.
    pub fn get(&self, path: &str) -> Option<&FileEntry> {
        self.entries.get(path)
    }

    /// Check if a file is indexed.
    pub fn contains(&self, path: &str) -> bool {
        self.entries.contains_key(path)
    }

    /// List all indexed files.
    pub fn list(&self) -> Vec<&FileEntry> {
        let mut entries: Vec<&FileEntry> = self.entries.values().collect();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        entries
    }

    /// List files by type.
    pub fn list_by_type(&self, file_type: FileType) -> Vec<&FileEntry> {
        self.entries
            .values()
            .filter(|e| e.file_type == file_type)
            .collect()
    }

    /// List files in a directory (non-recursive).
    pub fn list_dir(&self, dir: &str) -> Vec<&FileEntry> {
        let prefix = if dir.is_empty() || dir == "." {
            String::new()
        } else if dir.ends_with('/') {
            dir.to_string()
        } else {
            format!("{dir}/")
        };

        self.entries
            .values()
            .filter(|e| {
                if prefix.is_empty() {
                    !e.path.contains('/')
                } else {
                    e.path.starts_with(&prefix) && !e.path[prefix.len()..].contains('/')
                }
            })
            .collect()
    }

    /// Search the index by query text.
    pub fn search(&self, query: &str) -> Vec<SearchResult> {
        let terms = tokenize(query);
        if terms.is_empty() {
            return Vec::new();
        }

        // Aggregate scores per file
        let mut scores: HashMap<String, f64> = HashMap::new();
        for term in &terms {
            if let Some(postings) = self.term_index.get(term) {
                for (path, weight) in postings {
                    *scores.entry(path.clone()).or_insert(0.0) += weight;
                }
            }
            // Also check partial matches
            for (indexed_term, postings) in &self.term_index {
                if indexed_term.contains(term) && indexed_term != term {
                    for (path, weight) in postings {
                        *scores.entry(path.clone()).or_insert(0.0) += weight * 0.5;
                    }
                }
            }
        }

        let mut results: Vec<SearchResult> = scores
            .into_iter()
            .filter_map(|(path, score)| {
                let entry = self.entries.get(&path)?;
                Some(SearchResult {
                    entry: entry.clone(),
                    score,
                    context: entry.first_line.clone(),
                    line_number: None,
                })
            })
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results
    }

    /// Count indexed files.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if index is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get index statistics.
    pub fn stats(&self) -> IndexStats {
        let mut stats = IndexStats {
            total_files: self.entries.len(),
            ..Default::default()
        };

        let mut latest: Option<DateTime<Utc>> = None;

        for entry in self.entries.values() {
            stats.total_size_bytes += entry.size_bytes;
            stats.total_lines += entry.line_count;
            *stats.files_by_type.entry(entry.file_type).or_insert(0) += 1;

            if latest.map(|l| entry.indexed_at > l).unwrap_or(true) {
                latest = Some(entry.indexed_at);
            }
        }

        stats.last_indexed_at = latest;
        stats
    }

    /// Find files modified after a given timestamp.
    pub fn modified_since(&self, since: DateTime<Utc>) -> Vec<&FileEntry> {
        self.entries
            .values()
            .filter(|e| e.modified_at > since)
            .collect()
    }

    /// Find the largest files.
    pub fn largest(&self, n: usize) -> Vec<&FileEntry> {
        let mut entries: Vec<&FileEntry> = self.entries.values().collect();
        entries.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
        entries.truncate(n);
        entries
    }

    /// Clear the entire index.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.term_index.clear();
    }

    // -- Internal -----------------------------------------------------------

    fn remove_from_term_index(&mut self, path: &str) {
        for postings in self.term_index.values_mut() {
            postings.retain(|(p, _)| p != path);
        }
        // Clean up empty entries
        self.term_index.retain(|_, v| !v.is_empty());
    }
}

impl Default for FileIndex {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tokenizer
// ============================================================================

/// Simple tokenizer: splits on non-alphanumeric chars, lowercases, filters short tokens.
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| s.len() >= 2)
        .map(|s| s.to_string())
        .collect()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_entry(path: &str, name: &str, size: u64) -> FileEntry {
        FileEntry::new(path, name, size)
    }

    // -- FileType -----------------------------------------------------------

    #[test]
    fn test_file_type_from_extension() {
        assert_eq!(FileType::from_extension("md"), FileType::Markdown);
        assert_eq!(FileType::from_extension("toml"), FileType::Config);
        assert_eq!(FileType::from_extension("csv"), FileType::Data);
        assert_eq!(FileType::from_extension("rs"), FileType::Code);
        assert_eq!(FileType::from_extension("xyz"), FileType::Unknown);
    }

    #[test]
    fn test_file_type_case_insensitive() {
        assert_eq!(FileType::from_extension("MD"), FileType::Markdown);
        assert_eq!(FileType::from_extension("Json"), FileType::Config);
    }

    #[test]
    fn test_file_type_as_str() {
        assert_eq!(FileType::Markdown.as_str(), "markdown");
        assert_eq!(FileType::Config.as_str(), "config");
        assert_eq!(FileType::Data.as_str(), "data");
        assert_eq!(FileType::Code.as_str(), "code");
        assert_eq!(FileType::Unknown.as_str(), "unknown");
    }

    // -- FileEntry ----------------------------------------------------------

    #[test]
    fn test_file_entry_new() {
        let entry = FileEntry::new("memory/MEMORY.md", "MEMORY.md", 1024);
        assert_eq!(entry.path, "memory/MEMORY.md");
        assert_eq!(entry.name, "MEMORY.md");
        assert_eq!(entry.extension, "md");
        assert_eq!(entry.file_type, FileType::Markdown);
        assert_eq!(entry.size_bytes, 1024);
        assert_eq!(entry.line_count, 0);
        assert!(entry.tags.is_empty());
    }

    #[test]
    fn test_file_entry_builders() {
        let entry = FileEntry::new("test.rs", "test.rs", 512)
            .with_first_line("// Test file")
            .with_line_count(50)
            .with_tags(vec!["rust".into(), "test".into()]);
        assert_eq!(entry.first_line.as_deref(), Some("// Test file"));
        assert_eq!(entry.line_count, 50);
        assert_eq!(entry.tags.len(), 2);
    }

    #[test]
    fn test_file_entry_no_extension() {
        let entry = FileEntry::new("Makefile", "Makefile", 256);
        assert_eq!(entry.extension, "Makefile");
        assert_eq!(entry.file_type, FileType::Unknown);
    }

    // -- FileIndex basic ops ------------------------------------------------

    #[test]
    fn test_index_new_empty() {
        let idx = FileIndex::new();
        assert!(idx.is_empty());
        assert_eq!(idx.len(), 0);
    }

    #[test]
    fn test_index_add_and_get() {
        let mut idx = FileIndex::new();
        idx.add(test_entry("AGENTS.md", "AGENTS.md", 2048));
        assert_eq!(idx.len(), 1);
        assert!(idx.contains("AGENTS.md"));
        let entry = idx.get("AGENTS.md").unwrap();
        assert_eq!(entry.size_bytes, 2048);
    }

    #[test]
    fn test_index_remove() {
        let mut idx = FileIndex::new();
        idx.add(test_entry("test.md", "test.md", 100));
        let removed = idx.remove("test.md");
        assert!(removed.is_some());
        assert!(idx.is_empty());
    }

    #[test]
    fn test_index_remove_nonexistent() {
        let mut idx = FileIndex::new();
        assert!(idx.remove("ghost.md").is_none());
    }

    #[test]
    fn test_index_list_sorted() {
        let mut idx = FileIndex::new();
        idx.add(test_entry("c.md", "c.md", 100));
        idx.add(test_entry("a.md", "a.md", 100));
        idx.add(test_entry("b.md", "b.md", 100));
        let list = idx.list();
        assert_eq!(list[0].path, "a.md");
        assert_eq!(list[1].path, "b.md");
        assert_eq!(list[2].path, "c.md");
    }

    #[test]
    fn test_index_list_by_type() {
        let mut idx = FileIndex::new();
        idx.add(test_entry("readme.md", "readme.md", 100));
        idx.add(test_entry("config.toml", "config.toml", 200));
        idx.add(test_entry("notes.md", "notes.md", 150));
        assert_eq!(idx.list_by_type(FileType::Markdown).len(), 2);
        assert_eq!(idx.list_by_type(FileType::Config).len(), 1);
        assert_eq!(idx.list_by_type(FileType::Code).len(), 0);
    }

    #[test]
    fn test_index_list_dir() {
        let mut idx = FileIndex::new();
        idx.add(test_entry("AGENTS.md", "AGENTS.md", 100));
        idx.add(test_entry("SOUL.md", "SOUL.md", 100));
        idx.add(test_entry("memory/MEMORY.md", "MEMORY.md", 100));
        idx.add(test_entry("memory/facts.md", "facts.md", 100));

        let root_files = idx.list_dir(".");
        assert_eq!(root_files.len(), 2);

        let mem_files = idx.list_dir("memory");
        assert_eq!(mem_files.len(), 2);
    }

    #[test]
    fn test_index_update_entry() {
        let mut idx = FileIndex::new();
        idx.add(test_entry("test.md", "test.md", 100));
        idx.add(test_entry("test.md", "test.md", 200));
        assert_eq!(idx.len(), 1);
        assert_eq!(idx.get("test.md").unwrap().size_bytes, 200);
    }

    #[test]
    fn test_index_clear() {
        let mut idx = FileIndex::new();
        idx.add(test_entry("a.md", "a.md", 100));
        idx.add(test_entry("b.md", "b.md", 100));
        idx.clear();
        assert!(idx.is_empty());
    }

    // -- Search -------------------------------------------------------------

    #[test]
    fn test_search_by_name() {
        let mut idx = FileIndex::new();
        idx.add(test_entry("AGENTS.md", "AGENTS.md", 100));
        idx.add(test_entry("MEMORY.md", "MEMORY.md", 100));
        idx.add(test_entry("config.toml", "config.toml", 100));

        let results = idx.search("agents");
        assert!(!results.is_empty());
        assert_eq!(results[0].entry.name, "AGENTS.md");
    }

    #[test]
    fn test_search_by_tag() {
        let mut idx = FileIndex::new();
        idx.add(
            FileEntry::new("test.rs", "test.rs", 100)
                .with_tags(vec!["rust".into(), "testing".into()]),
        );
        idx.add(FileEntry::new("main.py", "main.py", 100).with_tags(vec!["python".into()]));

        let results = idx.search("rust");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.name, "test.rs");
    }

    #[test]
    fn test_search_by_first_line() {
        let mut idx = FileIndex::new();
        idx.add(
            FileEntry::new("readme.md", "readme.md", 100).with_first_line("# Zeus AI Assistant"),
        );
        idx.add(FileEntry::new("notes.md", "notes.md", 100).with_first_line("# Daily Notes"));

        let results = idx.search("zeus");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.name, "readme.md");
    }

    #[test]
    fn test_search_no_results() {
        let mut idx = FileIndex::new();
        idx.add(test_entry("test.md", "test.md", 100));
        let results = idx.search("nonexistent_xyz_123");
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_empty_query() {
        let mut idx = FileIndex::new();
        idx.add(test_entry("test.md", "test.md", 100));
        let results = idx.search("");
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_ranking() {
        let mut idx = FileIndex::new();
        // Name match (highest weight)
        idx.add(test_entry("memory.md", "memory.md", 100));
        // Tag match (medium weight)
        idx.add(FileEntry::new("notes.md", "notes.md", 100).with_tags(vec!["memory".into()]));
        // First line match (lowest weight)
        idx.add(
            FileEntry::new("other.md", "other.md", 100).with_first_line("This is about memory"),
        );

        let results = idx.search("memory");
        assert_eq!(results.len(), 3);
        // Name match should be first
        assert_eq!(results[0].entry.name, "memory.md");
    }

    // -- Statistics ---------------------------------------------------------

    #[test]
    fn test_stats_empty() {
        let idx = FileIndex::new();
        let stats = idx.stats();
        assert_eq!(stats.total_files, 0);
        assert_eq!(stats.total_size_bytes, 0);
        assert!((stats.avg_file_size() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_stats_with_files() {
        let mut idx = FileIndex::new();
        idx.add(test_entry("a.md", "a.md", 100).with_line_count(10));
        idx.add(test_entry("b.toml", "b.toml", 200).with_line_count(20));
        idx.add(test_entry("c.md", "c.md", 300).with_line_count(30));

        let stats = idx.stats();
        assert_eq!(stats.total_files, 3);
        assert_eq!(stats.total_size_bytes, 600);
        assert_eq!(stats.total_lines, 60);
        assert!((stats.avg_file_size() - 200.0).abs() < f64::EPSILON);
        assert_eq!(stats.files_by_type[&FileType::Markdown], 2);
        assert_eq!(stats.files_by_type[&FileType::Config], 1);
    }

    // -- modified_since -----------------------------------------------------

    #[test]
    fn test_modified_since() {
        let mut idx = FileIndex::new();
        let old_time = Utc::now() - chrono::Duration::seconds(3600);
        let recent_time = Utc::now() - chrono::Duration::seconds(60);

        idx.add(test_entry("old.md", "old.md", 100).with_modified(old_time));
        idx.add(test_entry("new.md", "new.md", 100).with_modified(recent_time));

        let cutoff = Utc::now() - chrono::Duration::seconds(600);
        let recent = idx.modified_since(cutoff);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].name, "new.md");
    }

    // -- largest ------------------------------------------------------------

    #[test]
    fn test_largest() {
        let mut idx = FileIndex::new();
        idx.add(test_entry("small.md", "small.md", 100));
        idx.add(test_entry("big.md", "big.md", 10000));
        idx.add(test_entry("medium.md", "medium.md", 1000));

        let largest = idx.largest(2);
        assert_eq!(largest.len(), 2);
        assert_eq!(largest[0].name, "big.md");
        assert_eq!(largest[1].name, "medium.md");
    }

    // -- Tokenizer ----------------------------------------------------------

    #[test]
    fn test_tokenize_basic() {
        let tokens = tokenize("Hello World");
        assert_eq!(tokens, vec!["hello", "world"]);
    }

    #[test]
    fn test_tokenize_filters_short() {
        let tokens = tokenize("a bb ccc");
        assert_eq!(tokens, vec!["bb", "ccc"]);
    }

    #[test]
    fn test_tokenize_special_chars() {
        let tokens = tokenize("AGENTS.md — Zeus AI");
        assert!(tokens.contains(&"agents".to_string()));
        assert!(tokens.contains(&"zeus".to_string()));
    }

    #[test]
    fn test_tokenize_underscores() {
        let tokens = tokenize("my_variable_name");
        assert_eq!(tokens, vec!["my_variable_name"]);
    }
}
