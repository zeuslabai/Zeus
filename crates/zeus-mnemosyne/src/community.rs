//! Community Detection for Zeus Memory Graph (Sprint 9)
//!
//! Implements the Label Propagation Algorithm (LPA) to discover clusters of
//! related entities in the memory graph. Entities that co-occur frequently in
//! the same messages are grouped into communities.
//!
//! # Algorithm
//! 1. Load all entities and relationships from the graph.
//! 2. Each entity starts with its own unique label (community ID).
//! 3. Iteratively: each entity adopts the most frequent label among its
//!    neighbours, weighted by relationship weight.
//! 4. Repeat until labels stabilise or max_iterations (50) is reached.
//! 5. Entities sharing the same final label form a community.

use std::collections::HashMap;
use zeus_core::Result;

use crate::MemoryStore;

const MAX_ITERATIONS: usize = 50;
/// Minimum relationship weight for an edge to be considered.
const MIN_WEIGHT: f64 = 1.0;

// ============================================================================
// Public API
// ============================================================================

/// Run community detection on the memory graph.
///
/// Clears any previously detected communities, runs label propagation, persists
/// the results, then assigns hub/bridge/member roles.
///
/// Returns the number of communities detected.
pub fn detect_communities(store: &MemoryStore) -> Result<usize> {
    // Load graph data
    let entities = store.get_entities(10_000)?;
    if entities.is_empty() {
        return Ok(0);
    }

    let relationships = store.get_all_relationships()?;

    // Build adjacency: entity_id -> Vec<(neighbour_id, weight)>
    let mut adjacency: HashMap<i64, Vec<(i64, f64)>> = HashMap::new();
    for rel in &relationships {
        if rel.weight < MIN_WEIGHT {
            continue;
        }
        adjacency
            .entry(rel.source_entity_id)
            .or_default()
            .push((rel.target_entity_id, rel.weight));
        adjacency
            .entry(rel.target_entity_id)
            .or_default()
            .push((rel.source_entity_id, rel.weight));
    }

    // Initialise: each entity gets its own label
    let mut labels: HashMap<i64, i64> = entities.iter().map(|e| (e.id, e.id)).collect();

    // Label propagation
    for _ in 0..MAX_ITERATIONS {
        let mut changed = false;
        // Process entities in a stable order (by id) for reproducibility
        let mut ids: Vec<i64> = labels.keys().copied().collect();
        ids.sort_unstable();

        for &entity_id in &ids {
            let Some(neighbours) = adjacency.get(&entity_id) else {
                continue; // isolated node keeps its own label
            };

            // Tally weighted votes per label
            let mut label_weights: HashMap<i64, f64> = HashMap::new();
            for &(neighbour_id, weight) in neighbours {
                let neighbour_label = labels[&neighbour_id];
                *label_weights.entry(neighbour_label).or_default() += weight;
            }

            // Pick the label with the highest total weight (tie-break: smallest label)
            let best_label = label_weights
                .into_iter()
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap().then(b.0.cmp(&a.0)))
                .map(|(label, _)| label)
                .unwrap_or(entity_id);

            if labels[&entity_id] != best_label {
                labels.insert(entity_id, best_label);
                changed = true;
            }
        }

        if !changed {
            break; // converged
        }
    }

    // Group entities by final label
    let mut groups: HashMap<i64, Vec<i64>> = HashMap::new();
    for (&entity_id, &label) in &labels {
        groups.entry(label).or_default().push(entity_id);
    }

    // Build a name lookup for community labels
    let name_map: HashMap<i64, String> = entities
        .iter()
        .map(|e| (e.id, e.canonical_name.clone()))
        .collect();

    // Persist to store
    store.clear_communities()?;

    let mut community_count = 0;
    for (label_entity_id, members) in &groups {
        if members.is_empty() {
            continue;
        }
        let representative = name_map
            .get(label_entity_id)
            .map(|s| s.as_str())
            .unwrap_or("unknown");
        let community_label = format!("community:{}", representative);
        let description = format!(
            "{} entities clustered around {}",
            members.len(),
            representative
        );
        let community_id = store.create_community(&community_label, &description)?;

        for &entity_id in members {
            store.add_community_member(community_id, entity_id, "member")?;
        }

        // Assign roles based on degree centrality and cross-community edges
        assign_roles(store, community_id, &adjacency, &labels)?;

        community_count += 1;
    }

    Ok(community_count)
}

/// Assign hub / bridge / member roles to entities within a community.
///
/// - **Hub**: highest in-community degree (most connections within the group).
/// - **Bridge**: has at least one cross-community edge (connects to another cluster).
/// - **Member**: everyone else.
pub fn assign_roles(
    store: &MemoryStore,
    community_id: i64,
    adjacency: &HashMap<i64, Vec<(i64, f64)>>,
    labels: &HashMap<i64, i64>,
) -> Result<()> {
    let members = store.get_community_members(community_id)?;
    if members.is_empty() {
        return Ok(());
    }

    let member_ids: std::collections::HashSet<i64> = members.iter().map(|(id, _, _)| *id).collect();

    // Compute in-community degree for each member
    let mut in_degree: HashMap<i64, usize> = HashMap::new();
    let mut is_bridge: HashMap<i64, bool> = HashMap::new();

    for &entity_id in &member_ids {
        let mut internal = 0usize;
        let mut bridge = false;
        if let Some(neighbours) = adjacency.get(&entity_id) {
            for &(neighbour_id, _) in neighbours {
                if member_ids.contains(&neighbour_id) {
                    internal += 1;
                } else if labels.get(&neighbour_id) != labels.get(&entity_id) {
                    bridge = true;
                }
            }
        }
        in_degree.insert(entity_id, internal);
        is_bridge.insert(entity_id, bridge);
    }

    // Hub = entity with the highest in-community degree
    let hub_id = in_degree
        .iter()
        .max_by_key(|(_, deg)| *deg)
        .map(|(id, _)| *id);

    for &entity_id in &member_ids {
        let role = if Some(entity_id) == hub_id {
            "hub"
        } else if is_bridge.get(&entity_id).copied().unwrap_or(false) {
            "bridge"
        } else {
            "member"
        };
        store.add_community_member(community_id, entity_id, role)?;
    }

    Ok(())
}

/// Build a human-readable summary for a community.
///
/// Lists the hub entity, bridge entities, member count, and top members by name.
pub fn community_summary(store: &MemoryStore, community_id: i64) -> String {
    let members = match store.get_community_members(community_id) {
        Ok(m) => m,
        Err(e) => return format!("Error loading community {}: {}", community_id, e),
    };

    if members.is_empty() {
        return format!("Community {} is empty.", community_id);
    }

    let hub: Vec<&str> = members
        .iter()
        .filter(|(_, _, role)| role == "hub")
        .map(|(_, name, _)| name.as_str())
        .collect();

    let bridges: Vec<&str> = members
        .iter()
        .filter(|(_, _, role)| role == "bridge")
        .map(|(_, name, _)| name.as_str())
        .collect();

    let regular: Vec<&str> = members
        .iter()
        .filter(|(_, _, role)| role == "member")
        .map(|(_, name, _)| name.as_str())
        .take(5)
        .collect();

    let mut parts = vec![format!(
        "Community {} ({} entities)",
        community_id,
        members.len()
    )];

    if !hub.is_empty() {
        parts.push(format!("  Hub:     {}", hub.join(", ")));
    }
    if !bridges.is_empty() {
        parts.push(format!("  Bridges: {}", bridges.join(", ")));
    }
    if !regular.is_empty() {
        let suffix = if members.len() > 5 + bridges.len() + hub.len() {
            " …"
        } else {
            ""
        };
        parts.push(format!("  Members: {}{}", regular.join(", "), suffix));
    }

    parts.join("\n")
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
        MemoryStore::new(&dir.path().join("test.db"), false, false).unwrap()
    }

    fn make_store_with_graph() -> MemoryStore {
        let store = make_store();

        // Create entities
        let alice = store.upsert_entity("Alice", "person").unwrap();
        let bob = store.upsert_entity("Bob", "person").unwrap();
        let carol = store.upsert_entity("Carol", "person").unwrap();
        let dave = store.upsert_entity("Dave", "person").unwrap();
        let zeus_entity = store.upsert_entity("Zeus", "project").unwrap();

        // Alice <-> Bob: strong connection (weight 3, via multiple adds)
        store
            .add_relationship(alice, bob, RelationType::CoOccurs, 3.0)
            .unwrap();
        // Carol <-> Dave: strong connection (weight 2)
        store
            .add_relationship(carol, dave, RelationType::CoOccurs, 2.0)
            .unwrap();
        // Zeus bridges both clusters
        store
            .add_relationship(alice, zeus_entity, RelationType::WorksOn, 1.0)
            .unwrap();
        store
            .add_relationship(carol, zeus_entity, RelationType::WorksOn, 1.0)
            .unwrap();

        store
    }

    #[test]
    fn test_empty_store_returns_zero_communities() {
        let store = make_store();
        let count = detect_communities(&store).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_single_entity_forms_own_community() {
        let store = make_store();
        store.upsert_entity("Solo", "person").unwrap();
        let count = detect_communities(&store).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_two_connected_entities_form_one_community() {
        let store = make_store();
        let a = store.upsert_entity("A", "person").unwrap();
        let b = store.upsert_entity("B", "person").unwrap();
        store
            .add_relationship(a, b, RelationType::CoOccurs, 2.0)
            .unwrap();
        let count = detect_communities(&store).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_two_isolated_entities_form_two_communities() {
        let store = make_store();
        store.upsert_entity("Alpha", "person").unwrap();
        store.upsert_entity("Bravo", "person").unwrap();
        let count = detect_communities(&store).unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn test_community_count_with_graph() {
        let store = make_store_with_graph();
        let count = detect_communities(&store).unwrap();
        // Alice+Bob tightly connected; Carol+Dave tightly connected; Zeus bridges both.
        // LPA may merge into 1-2 communities depending on convergence. Either is valid.
        assert!(count >= 1 && count <= 3);
    }

    #[test]
    fn test_clear_communities_removes_all() {
        let store = make_store_with_graph();
        detect_communities(&store).unwrap();
        store.clear_communities().unwrap();
        let communities = store.get_communities().unwrap();
        assert!(communities.is_empty());
    }

    #[test]
    fn test_community_summary_non_empty() {
        let store = make_store_with_graph();
        detect_communities(&store).unwrap();
        let communities = store.get_communities().unwrap();
        assert!(!communities.is_empty());
        let summary = community_summary(&store, communities[0].id);
        assert!(summary.contains("Community"));
        assert!(summary.contains("entities"));
    }

    #[test]
    fn test_roles_assigned_after_detection() {
        let store = make_store_with_graph();
        detect_communities(&store).unwrap();
        let communities = store.get_communities().unwrap();
        let mut found_hub = false;
        for community in &communities {
            let members = store.get_community_members(community.id).unwrap();
            for (_, _, role) in &members {
                if role == "hub" {
                    found_hub = true;
                }
            }
        }
        // At least one community with >1 member should have a hub
        assert!(found_hub || communities.iter().all(|c| c.entity_count <= 1));
    }
}
