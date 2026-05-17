//! Cross-Reference Linker — knowledge graph for document relationships.
//!
//! Builds and queries a graph of relationships between documents, actions,
//! and knowledge entries. Supports:
//! - **Bidirectional links**: A references B implies B is referenced by A
//! - **Link types**: Citation, dependency, related, supersedes, extends
//! - **Topic indexing**: Extract and index topics from documents for auto-linking
//! - **Orphan detection**: Find documents with no incoming or outgoing links
//! - **Path finding**: Discover chains of related documents
//! - **Backlink queries**: Find all documents that reference a given document

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for the cross-reference system.
#[derive(Debug, Clone)]
pub struct CrossRefConfig {
    /// Maximum number of links per document.
    pub max_links_per_doc: usize,
    /// Maximum depth for path finding between documents.
    pub max_path_depth: usize,
    /// Minimum topic overlap ratio (0.0–1.0) for auto-linking.
    pub auto_link_threshold: f64,
    /// Whether to automatically create bidirectional links.
    pub bidirectional: bool,
    /// Maximum number of documents to track.
    pub max_documents: usize,
}

impl Default for CrossRefConfig {
    fn default() -> Self {
        Self {
            max_links_per_doc: 50,
            max_path_depth: 5,
            auto_link_threshold: 0.3,
            bidirectional: true,
            max_documents: 10_000,
        }
    }
}

// ============================================================================
// Types
// ============================================================================

/// Type of relationship between two documents.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LinkType {
    /// Direct citation or reference.
    Citation,
    /// Target depends on source.
    Dependency,
    /// Related by topic or content.
    Related,
    /// Source supersedes/replaces target.
    Supersedes,
    /// Source extends or builds upon target.
    Extends,
    /// Custom relationship type.
    Custom(String),
}

impl std::fmt::Display for LinkType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkType::Citation => write!(f, "citation"),
            LinkType::Dependency => write!(f, "dependency"),
            LinkType::Related => write!(f, "related"),
            LinkType::Supersedes => write!(f, "supersedes"),
            LinkType::Extends => write!(f, "extends"),
            LinkType::Custom(s) => write!(f, "custom:{}", s),
        }
    }
}

/// A directed link between two documents.
#[derive(Debug, Clone)]
pub struct DocLink {
    /// Source document ID.
    pub source: String,
    /// Target document ID.
    pub target: String,
    /// Type of relationship.
    pub link_type: LinkType,
    /// Optional description of why they're linked.
    pub description: Option<String>,
    /// When the link was created (unix secs).
    pub created_at: u64,
}

/// Metadata about a tracked document.
#[derive(Debug, Clone)]
pub struct DocEntry {
    /// Document identifier (path, URL, or custom ID).
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// Extracted topics/keywords.
    pub topics: HashSet<String>,
    /// When the document was registered.
    pub registered_at: u64,
    /// When topics were last updated.
    pub topics_updated_at: u64,
}

/// Result of a backlink query.
#[derive(Debug, Clone)]
pub struct BacklinkResult {
    /// The queried document ID.
    pub document_id: String,
    /// Documents that link TO this document.
    pub incoming: Vec<DocLink>,
    /// Documents that this document links TO.
    pub outgoing: Vec<DocLink>,
}

/// A path between two documents through the link graph.
#[derive(Debug, Clone)]
pub struct DocPath {
    /// Ordered list of document IDs from source to target.
    pub nodes: Vec<String>,
    /// Link types along the path.
    pub link_types: Vec<LinkType>,
    /// Total path length.
    pub length: usize,
}

/// Statistics about the cross-reference system.
#[derive(Debug, Clone, Default)]
pub struct CrossRefStats {
    /// Total documents tracked.
    pub document_count: usize,
    /// Total links.
    pub link_count: usize,
    /// Orphan documents (no links).
    pub orphan_count: usize,
    /// Auto-links created.
    pub auto_links_created: usize,
}

// ============================================================================
// Cross-Reference Engine
// ============================================================================

/// The cross-reference linker engine.
pub struct CrossRefLinker {
    config: CrossRefConfig,
    /// Registered documents.
    documents: HashMap<String, DocEntry>,
    /// All links (source → list of links).
    links: Vec<DocLink>,
    /// Statistics.
    stats: CrossRefStats,
}

impl CrossRefLinker {
    /// Create with default configuration.
    pub fn new() -> Self {
        Self {
            config: CrossRefConfig::default(),
            documents: HashMap::new(),
            links: Vec::new(),
            stats: CrossRefStats::default(),
        }
    }

    /// Create with custom configuration.
    pub fn with_config(config: CrossRefConfig) -> Self {
        Self {
            config,
            documents: HashMap::new(),
            links: Vec::new(),
            stats: CrossRefStats::default(),
        }
    }

    /// Update configuration.
    pub fn set_config(&mut self, config: CrossRefConfig) {
        self.config = config;
    }

    /// Get current statistics (recomputed).
    pub fn stats(&self) -> CrossRefStats {
        let orphan_count = self.find_orphans().len();
        CrossRefStats {
            document_count: self.documents.len(),
            link_count: self.links.len(),
            orphan_count,
            auto_links_created: self.stats.auto_links_created,
        }
    }

    /// Register a document with its topics.
    pub fn register_document(&mut self, id: &str, title: &str, topics: Vec<String>) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let topic_set: HashSet<String> = topics.into_iter().map(|t| t.to_lowercase()).collect();

        self.documents.insert(
            id.to_string(),
            DocEntry {
                id: id.to_string(),
                title: title.to_string(),
                topics: topic_set,
                registered_at: now,
                topics_updated_at: now,
            },
        );

        // Enforce max documents
        if self.documents.len() > self.config.max_documents {
            // Remove oldest document
            if let Some(oldest_id) = self
                .documents
                .values()
                .min_by_key(|d| d.registered_at)
                .map(|d| d.id.clone())
            {
                self.remove_document(&oldest_id);
            }
        }
    }

    /// Remove a document and all its links.
    pub fn remove_document(&mut self, id: &str) {
        self.documents.remove(id);
        self.links.retain(|l| l.source != id && l.target != id);
    }

    /// Get a document by ID.
    pub fn get_document(&self, id: &str) -> Option<&DocEntry> {
        self.documents.get(id)
    }

    /// List all document IDs.
    pub fn document_ids(&self) -> Vec<String> {
        self.documents.keys().cloned().collect()
    }

    /// Update topics for a document.
    pub fn update_topics(&mut self, id: &str, topics: Vec<String>) -> bool {
        if let Some(doc) = self.documents.get_mut(id) {
            doc.topics = topics.into_iter().map(|t| t.to_lowercase()).collect();
            doc.topics_updated_at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            true
        } else {
            false
        }
    }

    /// Add a link between two documents.
    pub fn add_link(
        &mut self,
        source: &str,
        target: &str,
        link_type: LinkType,
        description: Option<&str>,
    ) -> bool {
        // Both documents must exist
        if !self.documents.contains_key(source) || !self.documents.contains_key(target) {
            return false;
        }

        // Don't duplicate
        if self
            .links
            .iter()
            .any(|l| l.source == source && l.target == target && l.link_type == link_type)
        {
            return false;
        }

        // Check per-doc link limit
        let source_link_count = self.links.iter().filter(|l| l.source == source).count();
        if source_link_count >= self.config.max_links_per_doc {
            return false;
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        self.links.push(DocLink {
            source: source.to_string(),
            target: target.to_string(),
            link_type: link_type.clone(),
            description: description.map(String::from),
            created_at: now,
        });

        // Add reverse link if bidirectional and it's a symmetric type
        if self.config.bidirectional && matches!(link_type, LinkType::Related) {
            let reverse_exists = self
                .links
                .iter()
                .any(|l| l.source == target && l.target == source && l.link_type == link_type);
            if !reverse_exists {
                let target_link_count = self.links.iter().filter(|l| l.source == target).count();
                if target_link_count < self.config.max_links_per_doc {
                    self.links.push(DocLink {
                        source: target.to_string(),
                        target: source.to_string(),
                        link_type,
                        description: description.map(String::from),
                        created_at: now,
                    });
                }
            }
        }

        true
    }

    /// Remove a specific link.
    pub fn remove_link(&mut self, source: &str, target: &str, link_type: &LinkType) -> bool {
        let before = self.links.len();
        self.links
            .retain(|l| !(l.source == source && l.target == target && l.link_type == *link_type));
        self.links.len() < before
    }

    /// Get all links from a document.
    pub fn outgoing_links(&self, doc_id: &str) -> Vec<&DocLink> {
        self.links.iter().filter(|l| l.source == doc_id).collect()
    }

    /// Get all links to a document (backlinks).
    pub fn incoming_links(&self, doc_id: &str) -> Vec<&DocLink> {
        self.links.iter().filter(|l| l.target == doc_id).collect()
    }

    /// Get full backlink result for a document.
    pub fn backlinks(&self, doc_id: &str) -> BacklinkResult {
        BacklinkResult {
            document_id: doc_id.to_string(),
            incoming: self
                .links
                .iter()
                .filter(|l| l.target == doc_id)
                .cloned()
                .collect(),
            outgoing: self
                .links
                .iter()
                .filter(|l| l.source == doc_id)
                .cloned()
                .collect(),
        }
    }

    /// Find documents with no links (orphans).
    pub fn find_orphans(&self) -> Vec<String> {
        let linked: HashSet<String> = self
            .links
            .iter()
            .flat_map(|l| vec![l.source.clone(), l.target.clone()])
            .collect();

        self.documents
            .keys()
            .filter(|id| !linked.contains(*id))
            .cloned()
            .collect()
    }

    /// Find shortest path between two documents using BFS.
    pub fn find_path(&self, source: &str, target: &str) -> Option<DocPath> {
        if source == target {
            return Some(DocPath {
                nodes: vec![source.to_string()],
                link_types: vec![],
                length: 0,
            });
        }

        if !self.documents.contains_key(source) || !self.documents.contains_key(target) {
            return None;
        }

        // BFS
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<(String, Vec<String>, Vec<LinkType>)> = VecDeque::new();

        visited.insert(source.to_string());
        queue.push_back((source.to_string(), vec![source.to_string()], vec![]));

        while let Some((current, path, link_types)) = queue.pop_front() {
            if path.len() > self.config.max_path_depth + 1 {
                continue;
            }

            for link in self.links.iter().filter(|l| l.source == current) {
                if link.target == target {
                    let mut final_path = path.clone();
                    final_path.push(target.to_string());
                    let mut final_types = link_types.clone();
                    final_types.push(link.link_type.clone());
                    return Some(DocPath {
                        length: final_path.len() - 1,
                        nodes: final_path,
                        link_types: final_types,
                    });
                }

                if !visited.contains(&link.target) {
                    visited.insert(link.target.clone());
                    let mut new_path = path.clone();
                    new_path.push(link.target.clone());
                    let mut new_types = link_types.clone();
                    new_types.push(link.link_type.clone());
                    queue.push_back((link.target.clone(), new_path, new_types));
                }
            }
        }

        None
    }

    /// Auto-link documents based on topic overlap.
    pub fn auto_link_by_topics(&mut self) -> usize {
        let doc_ids: Vec<String> = self.documents.keys().cloned().collect();
        let mut new_links = 0;

        for i in 0..doc_ids.len() {
            for j in (i + 1)..doc_ids.len() {
                let id_a = &doc_ids[i];
                let id_b = &doc_ids[j];

                let topics_a = &self.documents[id_a].topics;
                let topics_b = &self.documents[id_b].topics;

                if topics_a.is_empty() || topics_b.is_empty() {
                    continue;
                }

                let intersection = topics_a.intersection(topics_b).count();
                let union = topics_a.union(topics_b).count();
                let jaccard = intersection as f64 / union as f64;

                if jaccard >= self.config.auto_link_threshold {
                    // Check if link already exists
                    let exists = self.links.iter().any(|l| {
                        (l.source == *id_a && l.target == *id_b)
                            || (l.source == *id_b && l.target == *id_a)
                    });

                    if !exists {
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();

                        self.links.push(DocLink {
                            source: id_a.clone(),
                            target: id_b.clone(),
                            link_type: LinkType::Related,
                            description: Some(format!(
                                "Auto-linked: {:.0}% topic overlap",
                                jaccard * 100.0
                            )),
                            created_at: now,
                        });

                        if self.config.bidirectional {
                            self.links.push(DocLink {
                                source: id_b.clone(),
                                target: id_a.clone(),
                                link_type: LinkType::Related,
                                description: Some(format!(
                                    "Auto-linked: {:.0}% topic overlap",
                                    jaccard * 100.0
                                )),
                                created_at: now,
                            });
                        }

                        new_links += 1;
                    }
                }
            }
        }

        self.stats.auto_links_created += new_links;
        new_links
    }

    /// Find documents by topic.
    pub fn find_by_topic(&self, topic: &str) -> Vec<&DocEntry> {
        let topic_lower = topic.to_lowercase();
        self.documents
            .values()
            .filter(|d| d.topics.contains(&topic_lower))
            .collect()
    }

    /// Get the most connected documents (highest link count).
    pub fn most_connected(&self, limit: usize) -> Vec<(String, usize)> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for link in &self.links {
            *counts.entry(link.source.clone()).or_insert(0) += 1;
            *counts.entry(link.target.clone()).or_insert(0) += 1;
        }
        let mut sorted: Vec<(String, usize)> = counts.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));
        sorted.truncate(limit);
        sorted
    }
}

impl Default for CrossRefLinker {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_linker() -> CrossRefLinker {
        let mut l = CrossRefLinker::new();
        l.register_document(
            "doc-1",
            "Architecture Guide",
            vec!["rust".into(), "architecture".into(), "crates".into()],
        );
        l.register_document(
            "doc-2",
            "API Reference",
            vec!["api".into(), "rest".into(), "endpoints".into()],
        );
        l.register_document(
            "doc-3",
            "Security Guide",
            vec!["security".into(), "aegis".into(), "sandbox".into()],
        );
        l.register_document(
            "doc-4",
            "Rust Best Practices",
            vec!["rust".into(), "testing".into(), "performance".into()],
        );
        l
    }

    #[test]
    fn test_default_config() {
        let config = CrossRefConfig::default();
        assert_eq!(config.max_links_per_doc, 50);
        assert_eq!(config.max_path_depth, 5);
        assert!(config.bidirectional);
    }

    #[test]
    fn test_new_linker() {
        let l = CrossRefLinker::new();
        assert_eq!(l.stats().document_count, 0);
        assert_eq!(l.stats().link_count, 0);
    }

    #[test]
    fn test_register_document() {
        let mut l = CrossRefLinker::new();
        l.register_document("doc-1", "Test Doc", vec!["topic1".into()]);
        assert_eq!(l.stats().document_count, 1);
        let doc = l.get_document("doc-1").unwrap();
        assert_eq!(doc.title, "Test Doc");
        assert!(doc.topics.contains("topic1"));
    }

    #[test]
    fn test_register_replaces_existing() {
        let mut l = CrossRefLinker::new();
        l.register_document("doc-1", "Old Title", vec!["old".into()]);
        l.register_document("doc-1", "New Title", vec!["new".into()]);
        assert_eq!(l.stats().document_count, 1);
        assert_eq!(l.get_document("doc-1").unwrap().title, "New Title");
    }

    #[test]
    fn test_remove_document() {
        let mut l = setup_linker();
        l.add_link("doc-1", "doc-2", LinkType::Citation, None);
        l.remove_document("doc-1");
        assert!(l.get_document("doc-1").is_none());
        // Links involving doc-1 should also be removed
        assert!(l.outgoing_links("doc-1").is_empty());
        assert!(l.incoming_links("doc-1").is_empty());
    }

    #[test]
    fn test_document_ids() {
        let l = setup_linker();
        let ids = l.document_ids();
        assert_eq!(ids.len(), 4);
    }

    #[test]
    fn test_update_topics() {
        let mut l = CrossRefLinker::new();
        l.register_document("doc-1", "Test", vec!["old".into()]);
        assert!(l.update_topics("doc-1", vec!["new".into(), "topics".into()]));
        let doc = l.get_document("doc-1").unwrap();
        assert!(doc.topics.contains("new"));
        assert!(!doc.topics.contains("old"));
    }

    #[test]
    fn test_update_topics_nonexistent() {
        let mut l = CrossRefLinker::new();
        assert!(!l.update_topics("nonexistent", vec!["topic".into()]));
    }

    #[test]
    fn test_add_link() {
        let mut l = setup_linker();
        assert!(l.add_link("doc-1", "doc-2", LinkType::Citation, Some("References API")));
        assert_eq!(l.outgoing_links("doc-1").len(), 1);
        assert_eq!(l.incoming_links("doc-2").len(), 1);
    }

    #[test]
    fn test_add_link_nonexistent_doc() {
        let mut l = setup_linker();
        assert!(!l.add_link("doc-1", "nonexistent", LinkType::Citation, None));
    }

    #[test]
    fn test_add_link_no_duplicate() {
        let mut l = setup_linker();
        assert!(l.add_link("doc-1", "doc-2", LinkType::Citation, None));
        assert!(!l.add_link("doc-1", "doc-2", LinkType::Citation, None));
        assert_eq!(l.outgoing_links("doc-1").len(), 1);
    }

    #[test]
    fn test_bidirectional_related_link() {
        let mut l = setup_linker();
        l.add_link("doc-1", "doc-2", LinkType::Related, None);
        // Should auto-create reverse link for Related type
        assert_eq!(l.outgoing_links("doc-1").len(), 1);
        assert_eq!(l.outgoing_links("doc-2").len(), 1);
    }

    #[test]
    fn test_no_bidirectional_for_citation() {
        let mut l = setup_linker();
        l.add_link("doc-1", "doc-2", LinkType::Citation, None);
        // Citation is not symmetric — no reverse link
        assert_eq!(l.outgoing_links("doc-1").len(), 1);
        assert_eq!(l.outgoing_links("doc-2").len(), 0);
    }

    #[test]
    fn test_remove_link() {
        let mut l = setup_linker();
        l.add_link("doc-1", "doc-2", LinkType::Citation, None);
        assert!(l.remove_link("doc-1", "doc-2", &LinkType::Citation));
        assert!(l.outgoing_links("doc-1").is_empty());
    }

    #[test]
    fn test_backlinks() {
        let mut l = setup_linker();
        l.add_link("doc-1", "doc-3", LinkType::Citation, None);
        l.add_link("doc-2", "doc-3", LinkType::Extends, None);
        l.add_link("doc-3", "doc-4", LinkType::Dependency, None);

        let bl = l.backlinks("doc-3");
        assert_eq!(bl.incoming.len(), 2);
        assert_eq!(bl.outgoing.len(), 1);
    }

    #[test]
    fn test_find_orphans() {
        let mut l = setup_linker();
        l.add_link("doc-1", "doc-2", LinkType::Citation, None);
        // doc-3 and doc-4 have no links
        let orphans = l.find_orphans();
        assert_eq!(orphans.len(), 2);
        assert!(orphans.contains(&"doc-3".to_string()));
        assert!(orphans.contains(&"doc-4".to_string()));
    }

    #[test]
    fn test_find_path_direct() {
        let mut l = setup_linker();
        l.add_link("doc-1", "doc-2", LinkType::Citation, None);

        let path = l.find_path("doc-1", "doc-2").unwrap();
        assert_eq!(path.length, 1);
        assert_eq!(path.nodes, vec!["doc-1", "doc-2"]);
    }

    #[test]
    fn test_find_path_transitive() {
        let mut l = setup_linker();
        l.add_link("doc-1", "doc-2", LinkType::Citation, None);
        l.add_link("doc-2", "doc-3", LinkType::Extends, None);

        let path = l.find_path("doc-1", "doc-3").unwrap();
        assert_eq!(path.length, 2);
        assert_eq!(path.nodes, vec!["doc-1", "doc-2", "doc-3"]);
    }

    #[test]
    fn test_find_path_same_doc() {
        let l = setup_linker();
        let path = l.find_path("doc-1", "doc-1").unwrap();
        assert_eq!(path.length, 0);
        assert_eq!(path.nodes, vec!["doc-1"]);
    }

    #[test]
    fn test_find_path_no_connection() {
        let l = setup_linker();
        assert!(l.find_path("doc-1", "doc-3").is_none());
    }

    #[test]
    fn test_find_path_nonexistent() {
        let l = setup_linker();
        assert!(l.find_path("doc-1", "nonexistent").is_none());
    }

    #[test]
    fn test_auto_link_by_topics() {
        let mut l = CrossRefLinker::new();
        l.register_document(
            "doc-a",
            "Rust Guide",
            vec!["rust".into(), "programming".into(), "cargo".into()],
        );
        l.register_document(
            "doc-b",
            "Cargo Reference",
            vec!["rust".into(), "cargo".into(), "build".into()],
        );
        l.register_document(
            "doc-c",
            "Python Guide",
            vec!["python".into(), "pip".into(), "venv".into()],
        );

        let created = l.auto_link_by_topics();
        // doc-a and doc-b share "rust" and "cargo" (Jaccard = 2/4 = 0.5 >= 0.3 threshold)
        // doc-c has no overlap with either
        assert!(created >= 1);

        // Verify link exists between doc-a and doc-b
        let outgoing = l.outgoing_links("doc-a");
        assert!(outgoing.iter().any(|l| l.target == "doc-b"));
    }

    #[test]
    fn test_auto_link_no_overlap() {
        let mut l = CrossRefLinker::new();
        l.register_document("doc-a", "Rust", vec!["rust".into()]);
        l.register_document("doc-b", "Python", vec!["python".into()]);
        let created = l.auto_link_by_topics();
        assert_eq!(created, 0);
    }

    #[test]
    fn test_find_by_topic() {
        let l = setup_linker();
        let docs = l.find_by_topic("rust");
        assert_eq!(docs.len(), 2); // doc-1 and doc-4
    }

    #[test]
    fn test_find_by_topic_case_insensitive() {
        let l = setup_linker();
        let docs = l.find_by_topic("RUST");
        assert_eq!(docs.len(), 2);
    }

    #[test]
    fn test_most_connected() {
        let mut l = setup_linker();
        l.add_link("doc-1", "doc-2", LinkType::Citation, None);
        l.add_link("doc-1", "doc-3", LinkType::Citation, None);
        l.add_link("doc-1", "doc-4", LinkType::Extends, None);

        let top = l.most_connected(2);
        // doc-1 has 3 outgoing links = highest
        assert_eq!(top[0].0, "doc-1");
        assert!(top[0].1 >= 3);
    }

    #[test]
    fn test_link_type_display() {
        assert_eq!(LinkType::Citation.to_string(), "citation");
        assert_eq!(LinkType::Related.to_string(), "related");
        assert_eq!(LinkType::Custom("foo".into()).to_string(), "custom:foo");
    }

    #[test]
    fn test_stats() {
        let mut l = setup_linker();
        l.add_link("doc-1", "doc-2", LinkType::Citation, None);
        let stats = l.stats();
        assert_eq!(stats.document_count, 4);
        assert_eq!(stats.link_count, 1);
        assert_eq!(stats.orphan_count, 2); // doc-3, doc-4
    }
}
