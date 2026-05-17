//! Graph-Augmented Search for Zeus Memory (Sprint 9)
//!
//! Enhances Mnemosyne's hybrid search with knowledge graph traversal.
//! Given a query, finds mentioned entities, walks their graph neighborhood
//! 1-2 hops, and uses related entity names to expand and enrich search results.
//!
//! # Components
//!
//! - `expand_query_via_graph`: Finds entities in query text, traverses graph,
//!   adds related entity names to the search string.
//! - `graph_augmented_search`: Runs expanded query through hybrid search,
//!   enriches results with graph context.
//! - `get_memory_graph_context`: Returns the full graph context for a message
//!   (entities, relationships, community membership).

use crate::graph::{Direction, Relationship};
use crate::{EntityRecord, MemoryStore, SearchResult};
use serde::{Deserialize, Serialize};
use zeus_core::Result;

// ============================================================================
// Types
// ============================================================================

/// Graph context for a memory/entity — relationships, community, and neighbors.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphContext {
    /// Entities directly mentioned in or related to the memory.
    pub entities: Vec<EntityRecord>,
    /// Relationships connecting those entities.
    pub relationships: Vec<Relationship>,
    /// Community name if the primary entity belongs to one.
    pub community: Option<String>,
}

/// A search result enriched with graph context.
#[derive(Debug, Clone)]
pub struct GraphSearchResult {
    /// The original search result.
    pub result: SearchResult,
    /// Graph context for this result (related entities, relationships).
    pub graph_context: GraphContext,
    /// Formatted context string for LLM prompt injection.
    pub context_text: String,
}

// ============================================================================
// Query Expansion
// ============================================================================

/// Expand a search query by finding mentioned entities and adding their
/// graph neighbors' names to the search terms.
///
/// For example, if the query is "Zeus security" and the graph has
/// Alice --WORKS_ON--> Zeus, the expanded query becomes
/// "Zeus security Alice" — increasing recall for related memories.
pub fn expand_query_via_graph(store: &MemoryStore, query: &str, max_hops: u32) -> Result<String> {
    let entities = store.get_entities(1000)?;
    if entities.is_empty() {
        return Ok(query.to_string());
    }

    let query_lower = query.to_lowercase();
    let mut matched_entity_ids: Vec<i64> = Vec::new();

    // Find entities mentioned in the query text
    for entity in &entities {
        if query_lower.contains(&entity.canonical_name.to_lowercase()) {
            matched_entity_ids.push(entity.id);
        }
        // Also check aliases
        for alias in &entity.aliases {
            if query_lower.contains(&alias.to_lowercase())
                && !matched_entity_ids.contains(&entity.id)
            {
                matched_entity_ids.push(entity.id);
            }
        }
    }

    if matched_entity_ids.is_empty() {
        return Ok(query.to_string());
    }

    // Traverse graph for each matched entity and collect neighbor names
    let mut expansion_names: Vec<String> = Vec::new();
    let mut seen_ids: std::collections::HashSet<i64> = matched_entity_ids.iter().copied().collect();

    for entity_id in &matched_entity_ids {
        let rels = store.get_relationships(*entity_id, Direction::Both)?;
        for rel in &rels {
            let neighbor_id = if rel.source_entity_id == *entity_id {
                rel.target_entity_id
            } else {
                rel.source_entity_id
            };

            if seen_ids.insert(neighbor_id)
                && let Ok(neighbor) = store.get_entity_by_id(neighbor_id)
            {
                expansion_names.push(neighbor.canonical_name.clone());
            }

            // Second hop if requested
            if max_hops > 1 {
                let hop2_rels = store.get_relationships(neighbor_id, Direction::Both)?;
                for rel2 in &hop2_rels {
                    let hop2_id = if rel2.source_entity_id == neighbor_id {
                        rel2.target_entity_id
                    } else {
                        rel2.source_entity_id
                    };

                    if seen_ids.insert(hop2_id)
                        && let Ok(hop2_entity) = store.get_entity_by_id(hop2_id)
                    {
                        expansion_names.push(hop2_entity.canonical_name.clone());
                    }
                }
            }
        }
    }

    if expansion_names.is_empty() {
        return Ok(query.to_string());
    }

    // Build FTS5-compatible query with OR for expanded terms
    // Original query terms are required, expansion terms use OR for broader recall
    let expansion_or = expansion_names.join(" OR ");
    let expanded = format!("{} OR {}", query, expansion_or);
    Ok(expanded)
}

// ============================================================================
// Graph-Augmented Search
// ============================================================================

/// Run a graph-augmented search: expand the query via graph traversal,
/// search with the expanded query, then enrich results with graph context.
pub fn graph_augmented_search(
    store: &MemoryStore,
    query: &str,
    limit: usize,
) -> Result<Vec<GraphSearchResult>> {
    // Step 1: Expand query via graph
    let expanded_query = expand_query_via_graph(store, query, 1)?;

    // Step 2: Run the search with expanded terms
    let results = store.search(&expanded_query, limit)?;

    // Step 3: Enrich each result with graph context
    let mut enriched = Vec::with_capacity(results.len());
    for result in results {
        let graph_context = build_result_graph_context(store, &result)?;
        let context_text = format_graph_context_for_llm(&graph_context);

        enriched.push(GraphSearchResult {
            result,
            graph_context,
            context_text,
        });
    }

    Ok(enriched)
}

/// Get the full graph context for a specific message ID.
///
/// Finds all entities mentioned in the message, their relationships,
/// and any community membership.
pub fn get_memory_graph_context(store: &MemoryStore, message_id: i64) -> Result<GraphContext> {
    // Get entities linked to this message
    let entity_mentions = store.get_message_entities(message_id)?;

    let mut entities: Vec<EntityRecord> = Vec::new();
    let mut relationships: Vec<Relationship> = Vec::new();
    let mut community: Option<String> = None;

    for (entity_id, _mention) in &entity_mentions {
        if let Ok(entity) = store.get_entity_by_id(*entity_id) {
            entities.push(entity);
        }

        // Get relationships for this entity
        let rels = store.get_relationships(*entity_id, Direction::Both)?;
        for rel in rels {
            // Avoid duplicate edges
            if !relationships.iter().any(|r| r.id == rel.id) {
                relationships.push(rel);
            }
        }

        // Check community membership (take the first one found)
        if community.is_none()
            && let Ok(Some(comm)) = store.get_entity_community(*entity_id)
        {
            community = Some(comm.name);
        }
    }

    Ok(GraphContext {
        entities,
        relationships,
        community,
    })
}

// ============================================================================
// Helpers
// ============================================================================

/// Build graph context for a search result by extracting entities from its content.
fn build_result_graph_context(store: &MemoryStore, result: &SearchResult) -> Result<GraphContext> {
    let entities = store.get_entities(1000)?;
    let content_lower = result.content.to_lowercase();

    let mut context_entities: Vec<EntityRecord> = Vec::new();
    let mut context_rels: Vec<Relationship> = Vec::new();
    let mut community: Option<String> = None;

    for entity in &entities {
        if content_lower.contains(&entity.canonical_name.to_lowercase()) {
            context_entities.push(entity.clone());

            let rels = store.get_relationships(entity.id, Direction::Both)?;
            for rel in rels {
                if !context_rels.iter().any(|r| r.id == rel.id) {
                    context_rels.push(rel);
                }
            }

            if community.is_none()
                && let Ok(Some(comm)) = store.get_entity_community(entity.id)
            {
                community = Some(comm.name);
            }
        }
    }

    Ok(GraphContext {
        entities: context_entities,
        relationships: context_rels,
        community,
    })
}

/// Format graph context as a string suitable for LLM system prompt injection.
///
/// Example output:
/// ```text
/// [Graph Context]
/// Entities: Alice (person), Zeus (project)
/// Relations: Alice --WORKS_ON--> Zeus
/// Community: community:Zeus
/// ```
fn format_graph_context_for_llm(ctx: &GraphContext) -> String {
    if ctx.entities.is_empty() {
        return String::new();
    }

    let mut lines = vec!["[Graph Context]".to_string()];

    // Entity list
    let entity_strs: Vec<String> = ctx
        .entities
        .iter()
        .map(|e| format!("{} ({})", e.canonical_name, e.entity_type))
        .collect();
    lines.push(format!("Entities: {}", entity_strs.join(", ")));

    // Relationship list
    if !ctx.relationships.is_empty() {
        let rel_strs: Vec<String> = ctx
            .relationships
            .iter()
            .take(10) // Cap at 10 to keep context concise
            .map(|r| {
                format!(
                    "#{} --{}--> #{}",
                    r.source_entity_id,
                    r.relationship_type.as_label(),
                    r.target_entity_id,
                )
            })
            .collect();
        lines.push(format!("Relations: {}", rel_strs.join(", ")));
    }

    // Community
    if let Some(comm) = &ctx.community {
        lines.push(format!("Community: {}", comm));
    }

    lines.join("\n")
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::RelationType;
    use tempfile::tempdir;

    fn make_store() -> MemoryStore {
        let dir = tempdir().unwrap();
        MemoryStore::new(&dir.path().join("test.db"), true, false).unwrap()
    }

    fn setup_graph(store: &MemoryStore) -> (i64, i64, i64, i64) {
        let alice = store.upsert_entity("Alice", "person").unwrap();
        let bob = store.upsert_entity("Bob", "person").unwrap();
        let zeus = store.upsert_entity("Zeus", "project").unwrap();
        let mnemosyne = store.upsert_entity("Mnemosyne", "component").unwrap();

        store
            .add_relationship(alice, zeus, RelationType::WorksOn, 1.0)
            .unwrap();
        store
            .add_relationship(bob, zeus, RelationType::WorksOn, 1.0)
            .unwrap();
        store
            .add_relationship(mnemosyne, zeus, RelationType::PartOf, 1.0)
            .unwrap();

        (alice, bob, zeus, mnemosyne)
    }

    #[test]
    fn test_expand_query_no_entities() {
        let store = make_store();
        let expanded = expand_query_via_graph(&store, "hello world", 1).unwrap();
        assert_eq!(expanded, "hello world");
    }

    #[test]
    fn test_expand_query_with_entity() {
        let store = make_store();
        setup_graph(&store);

        let expanded = expand_query_via_graph(&store, "Alice memory", 1).unwrap();
        // Should contain original query + related entity names
        assert!(expanded.contains("Alice memory"));
        assert!(
            expanded.contains("Zeus"),
            "expanded should include Zeus: {}",
            expanded
        );
    }

    #[test]
    fn test_expand_query_two_hops() {
        let store = make_store();
        setup_graph(&store);

        let expanded = expand_query_via_graph(&store, "Alice", 2).unwrap();
        // 2 hops: Alice -> Zeus -> Bob, Mnemosyne
        assert!(
            expanded.contains("Bob"),
            "2-hop expansion should include Bob: {}",
            expanded
        );
        assert!(
            expanded.contains("Mnemosyne"),
            "2-hop expansion should include Mnemosyne: {}",
            expanded
        );
    }

    #[test]
    fn test_expand_query_no_match() {
        let store = make_store();
        setup_graph(&store);

        let expanded = expand_query_via_graph(&store, "unknown topic", 1).unwrap();
        assert_eq!(expanded, "unknown topic");
    }

    #[test]
    fn test_graph_augmented_search_empty_store() {
        let store = make_store();
        let results = graph_augmented_search(&store, "anything", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_graph_augmented_search_with_data() {
        let store = make_store();
        setup_graph(&store);

        // Store a message that mentions Zeus
        let msg_id = store
            .store_raw_message("session-1", "user", "Working on Zeus security fixes")
            .unwrap();
        store
            .link_entity_to_message(
                store.upsert_entity("Zeus", "project").unwrap(),
                msg_id,
                "Zeus",
            )
            .unwrap();

        let results = graph_augmented_search(&store, "Zeus", 10).unwrap();
        // Should find the message about Zeus
        assert!(!results.is_empty(), "should find Zeus-related messages");
        assert!(results[0].result.content.contains("Zeus"));
    }

    #[test]
    fn test_format_graph_context_empty() {
        let ctx = GraphContext::default();
        let text = format_graph_context_for_llm(&ctx);
        assert!(text.is_empty());
    }

    #[test]
    fn test_format_graph_context_with_data() {
        let ctx = GraphContext {
            entities: vec![EntityRecord {
                id: 1,
                canonical_name: "Alice".to_string(),
                entity_type: "person".to_string(),
                aliases: vec![],
                first_seen: String::new(),
                last_seen: String::new(),
                mention_count: 1,
            }],
            relationships: vec![],
            community: Some("community:Zeus".to_string()),
        };

        let text = format_graph_context_for_llm(&ctx);
        assert!(text.contains("[Graph Context]"));
        assert!(text.contains("Alice (person)"));
        assert!(text.contains("community:Zeus"));
    }
}
