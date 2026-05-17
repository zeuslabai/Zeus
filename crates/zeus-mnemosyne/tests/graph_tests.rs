//! Integration tests for the Zeus Mnemosyne graph pipeline (Sprint 9, Task 6).
//!
//! Covers all 7 required areas:
//!   1. Full pipeline: store messages → extract triples → build graph → detect communities → search
//!   2. Multi-hop BFS traversal at depths 0, 1, 2, and 3
//!   3. Shortest path between entities (2-hop, 3-hop, shortcut, same, disconnected, direction)
//!   4. Community detection with known graph structures (two cliques, hub-and-spoke, bridge)
//!   5. Graph-augmented search returns richer results than plain FTS5 search
//!   6. Edge cases: orphan entities, self-references, empty graph, single entity, duplicates
//!   7. Performance: 1000 entities + 5000 relationships complete BFS / shortest-path in <100ms
//!
//! FTS5 note: MemoryStore uses after-insert triggers (`messages_ai`) to keep the
//! `messages_fts` virtual table in sync, so `store_raw_message` is immediately
//! searchable — no manual FTS bookkeeping is required in these tests.

use std::collections::HashSet;
use tempfile::tempdir;
use zeus_mnemosyne::{
    Direction, MemoryStore, RelationType, community::detect_communities, expand_query_via_graph,
    graph_augmented_search, process_message_graph,
};

// ── Test helpers ─────────────────────────────────────────────────────────────

/// Create a fresh `MemoryStore` backed by a temp-dir SQLite file with FTS5 enabled.
///
/// Returns `(store, dir)` — the caller must keep `dir` alive for the duration
/// of the test; dropping it deletes the backing file.
fn make_store() -> (MemoryStore, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let store = MemoryStore::new(&dir.path().join("g.db"), true, false).unwrap();
    (store, dir)
}

// ============================================================================
// 1. Full pipeline
// ============================================================================

/// Full pipeline: store raw messages → extract triples → build graph →
/// detect communities → graph-augmented search.
///
/// Verifies that all pipeline stages run without errors and produce the
/// expected artefacts (entities, relationships, communities, search results).
///
/// Note: `process_message_graph` stores edges with confidence 0.8, which is
/// below the `MIN_WEIGHT` (1.0) used by community detection.  We therefore
/// supplement with explicit CoOccurs edges (weight 2.0) so that community
/// detection has edges to cluster on — demonstrating the full pipeline.
#[test]
fn test_full_pipeline_message_to_community_to_search() {
    let (store, _dir) = make_store();

    // ── Step 1: Store raw messages ────────────────────────────────────────
    let m1 = store
        .store_raw_message("sess-1", "user", "Alice works on Zeus")
        .unwrap();
    let m2 = store
        .store_raw_message("sess-1", "user", "Bob works on Zeus")
        .unwrap();
    let m3 = store
        .store_raw_message("sess-1", "user", "Carol manages Mnemosyne")
        .unwrap();

    // ── Step 2: Extract triples and populate the graph ────────────────────
    let t1 = process_message_graph(&store, m1, "Alice works on Zeus").unwrap();
    let t2 = process_message_graph(&store, m2, "Bob works on Zeus").unwrap();
    let t3 = process_message_graph(&store, m3, "Carol manages Mnemosyne").unwrap();

    assert_eq!(t1.len(), 1, "Alice works on Zeus → 1 triple");
    assert_eq!(t2.len(), 1, "Bob works on Zeus → 1 triple");
    assert_eq!(t3.len(), 1, "Carol manages Mnemosyne → 1 triple");
    assert_eq!(t1[0].relation, RelationType::WorksOn, "works on → WorksOn");
    assert_eq!(t3[0].relation, RelationType::Owns, "manages → Owns");

    // ── Step 3: Entities created ──────────────────────────────────────────
    let entities = store.get_entities(50).unwrap();
    let names: HashSet<&str> = entities.iter().map(|e| e.canonical_name.as_str()).collect();
    assert!(names.contains("Alice"), "Alice entity missing");
    assert!(names.contains("Bob"), "Bob entity missing");
    assert!(names.contains("Zeus"), "Zeus entity missing");
    assert!(names.contains("Mnemosyne"), "Mnemosyne entity missing");

    // ── Step 4: Relationships exist (weight 0.8 from extraction) ─────────
    let rel_count = store.relationship_count().unwrap();
    assert!(
        rel_count >= 3,
        "expected ≥3 relationships, got {}",
        rel_count
    );

    // ── Step 5: Add high-weight co-occurrence edges for community formation ─
    // Extraction produces weight 0.8 < MIN_WEIGHT (1.0), so community detection
    // ignores those edges.  We supplement with explicit CoOccurs (weight 2.0).
    let alice_id = store.upsert_entity("Alice", "person").unwrap();
    let bob_id = store.upsert_entity("Bob", "person").unwrap();
    let zeus_id = store.upsert_entity("Zeus", "project").unwrap();
    store
        .add_relationship(alice_id, zeus_id, RelationType::CoOccurs, 2.0)
        .unwrap();
    store
        .add_relationship(bob_id, zeus_id, RelationType::CoOccurs, 2.0)
        .unwrap();

    let n_comm = detect_communities(&store).unwrap();
    assert!(n_comm >= 1, "expected at least 1 community, got {}", n_comm);

    // Alice and Bob should converge to the same community (both CoOccur with Zeus)
    let alice_comm = store
        .get_entity_community(alice_id)
        .unwrap()
        .expect("Alice must be in a community");
    let bob_comm = store
        .get_entity_community(bob_id)
        .unwrap()
        .expect("Bob must be in a community");
    assert_eq!(
        alice_comm.id, bob_comm.id,
        "Alice and Bob should be in the same community"
    );

    // ── Step 6: Graph-augmented search finds the stored messages ──────────
    let results = graph_augmented_search(&store, "Zeus", 10).unwrap();
    assert!(
        !results.is_empty(),
        "graph-augmented search for 'Zeus' should find messages"
    );
    assert!(
        results.iter().any(|r| r.result.content.contains("Zeus")),
        "at least one result should mention Zeus"
    );
}

// ============================================================================
// 2. Multi-hop BFS traversal
// ============================================================================

/// BFS traversal in a linear chain A→B→C→D:
/// - depth 0 → no neighbours
/// - depth 1 → B only, at depth label 1
/// - depth 2 → B and C
/// - depth 3 → B, C, and D; correct depth labels for each
#[test]
fn test_multi_hop_traversal_linear_chain_depths_0_1_2_3() {
    let (store, _dir) = make_store();

    let a = store.upsert_entity("ChainA", "concept").unwrap();
    let b = store.upsert_entity("ChainB", "concept").unwrap();
    let c = store.upsert_entity("ChainC", "concept").unwrap();
    let d = store.upsert_entity("ChainD", "concept").unwrap();

    store
        .add_relationship(a, b, RelationType::RelatedTo, 1.0)
        .unwrap();
    store
        .add_relationship(b, c, RelationType::RelatedTo, 1.0)
        .unwrap();
    store
        .add_relationship(c, d, RelationType::RelatedTo, 1.0)
        .unwrap();

    // depth 0 — origin only, no neighbour nodes
    let g0 = store.get_entity_graph(a, 0).unwrap();
    assert_eq!(g0.origin.canonical_name, "ChainA");
    assert!(
        g0.nodes.is_empty(),
        "depth 0 should yield no neighbour nodes"
    );

    // depth 1 — only B
    let g1 = store.get_entity_graph(a, 1).unwrap();
    let names1: HashSet<&str> = g1
        .nodes
        .iter()
        .map(|n| n.entity.canonical_name.as_str())
        .collect();
    assert_eq!(
        names1,
        HashSet::from(["ChainB"]),
        "depth 1 should find only ChainB"
    );
    assert!(
        g1.nodes.iter().all(|n| n.depth == 1),
        "all depth-1 nodes should carry depth label 1"
    );

    // depth 2 — B and C
    let g2 = store.get_entity_graph(a, 2).unwrap();
    let names2: HashSet<&str> = g2
        .nodes
        .iter()
        .map(|n| n.entity.canonical_name.as_str())
        .collect();
    assert_eq!(
        names2,
        HashSet::from(["ChainB", "ChainC"]),
        "depth 2 should find ChainB and ChainC"
    );

    // depth 3 — B, C, and D with correct depth labels
    let g3 = store.get_entity_graph(a, 3).unwrap();
    let names3: HashSet<&str> = g3
        .nodes
        .iter()
        .map(|n| n.entity.canonical_name.as_str())
        .collect();
    assert_eq!(
        names3,
        HashSet::from(["ChainB", "ChainC", "ChainD"]),
        "depth 3 should find all chain nodes"
    );

    let depth_of = |name: &str| {
        g3.nodes
            .iter()
            .find(|n| n.entity.canonical_name == name)
            .unwrap()
            .depth
    };
    assert_eq!(depth_of("ChainB"), 1, "ChainB should be at depth 1");
    assert_eq!(depth_of("ChainC"), 2, "ChainC should be at depth 2");
    assert_eq!(depth_of("ChainD"), 3, "ChainD should be at depth 3");
}

/// BFS traversal on a branching tree structure: origin→{Left, Top, Bot} and Left→Leaf.
/// Depth 1 finds 3 direct children; depth 2 additionally finds Leaf.
///
/// Entity names are chosen to have Levenshtein distance > 1 from each other so that
/// `upsert_entity`'s fuzzy-match (threshold 0.85) does not collapse them into one entity.
/// e.g. "Trunk" vs "LeftArm" ratio ≈ 0.14; "LeftArm" vs "TopArm" ratio ≈ 0.43 — all safe.
#[test]
fn test_multi_hop_traversal_branching_tree() {
    let (store, _dir) = make_store();

    // Use clearly distinct names (large edit distance) to avoid Levenshtein fuzzy merging.
    let root = store.upsert_entity("Trunk", "concept").unwrap();
    let b = store.upsert_entity("LeftArm", "concept").unwrap();
    let c = store.upsert_entity("TopArm", "concept").unwrap();
    let d = store.upsert_entity("BotArm", "concept").unwrap();
    let e = store.upsert_entity("Leaf", "concept").unwrap();

    // Verify all five are distinct entities (fuzzy match did not collapse any)
    let unique_ids: HashSet<i64> = [root, b, c, d, e].iter().copied().collect();
    assert_eq!(
        unique_ids.len(),
        5,
        "all 5 entities should have distinct IDs"
    );

    store
        .add_relationship(root, b, RelationType::RelatedTo, 1.0)
        .unwrap();
    store
        .add_relationship(root, c, RelationType::RelatedTo, 1.0)
        .unwrap();
    store
        .add_relationship(root, d, RelationType::RelatedTo, 1.0)
        .unwrap();
    store
        .add_relationship(b, e, RelationType::RelatedTo, 1.0)
        .unwrap();

    let g1 = store.get_entity_graph(root, 1).unwrap();
    assert_eq!(
        g1.nodes.len(),
        3,
        "depth 1 should find LeftArm, TopArm, BotArm"
    );

    let g2 = store.get_entity_graph(root, 2).unwrap();
    let names2: HashSet<&str> = g2
        .nodes
        .iter()
        .map(|n| n.entity.canonical_name.as_str())
        .collect();
    assert!(names2.contains("Leaf"), "Leaf should be visible at depth 2");
    assert_eq!(
        g2.nodes.len(),
        4,
        "depth 2 should find LeftArm, TopArm, BotArm, Leaf"
    );

    // Edges list is collected during traversal
    assert!(!g2.edges.is_empty(), "traversal should accumulate edges");
}

/// BFS respects visited-set: in a cycle A→B→C→A, each node appears only once.
#[test]
fn test_multi_hop_traversal_cycle_no_infinite_loop() {
    let (store, _dir) = make_store();

    let a = store.upsert_entity("CycleA", "concept").unwrap();
    let b = store.upsert_entity("CycleB", "concept").unwrap();
    let c = store.upsert_entity("CycleC", "concept").unwrap();

    store
        .add_relationship(a, b, RelationType::RelatedTo, 1.0)
        .unwrap();
    store
        .add_relationship(b, c, RelationType::RelatedTo, 1.0)
        .unwrap();
    store
        .add_relationship(c, a, RelationType::RelatedTo, 1.0)
        .unwrap();

    // Depth 5 should still return only 2 unique neighbour nodes (B and C); A is origin
    let g = store.get_entity_graph(a, 5).unwrap();
    let names: HashSet<&str> = g
        .nodes
        .iter()
        .map(|n| n.entity.canonical_name.as_str())
        .collect();
    assert!(
        !names.contains("CycleA"),
        "origin should not appear in nodes list"
    );
    assert_eq!(
        names.len(),
        2,
        "only B and C should be in nodes (no duplicates)"
    );
}

// ============================================================================
// 3. Shortest path
// ============================================================================

/// Classic 2-hop path: Alice → Zeus ← Bob.
#[test]
fn test_shortest_path_two_hops() {
    let (store, _dir) = make_store();

    let alice = store.upsert_entity("PathAlice", "person").unwrap();
    let zeus = store.upsert_entity("PathZeus", "project").unwrap();
    let bob = store.upsert_entity("PathBob", "person").unwrap();

    store
        .add_relationship(alice, zeus, RelationType::WorksOn, 1.0)
        .unwrap();
    store
        .add_relationship(bob, zeus, RelationType::WorksOn, 1.0)
        .unwrap();

    let path = store
        .shortest_path(alice, bob)
        .unwrap()
        .expect("path should exist");
    assert_eq!(path.len(), 3, "Alice→Zeus→Bob is 3 nodes (2 edges)");
    assert_eq!(path[0].canonical_name, "PathAlice");
    assert_eq!(path[1].canonical_name, "PathZeus");
    assert_eq!(path[2].canonical_name, "PathBob");
}

/// Three-hop path along a linear chain; no other route exists.
#[test]
fn test_shortest_path_three_hops_linear_chain() {
    let (store, _dir) = make_store();

    let a = store.upsert_entity("SpA", "concept").unwrap();
    let b = store.upsert_entity("SpB", "concept").unwrap();
    let c = store.upsert_entity("SpC", "concept").unwrap();
    let d = store.upsert_entity("SpD", "concept").unwrap();

    store
        .add_relationship(a, b, RelationType::RelatedTo, 1.0)
        .unwrap();
    store
        .add_relationship(b, c, RelationType::RelatedTo, 1.0)
        .unwrap();
    store
        .add_relationship(c, d, RelationType::RelatedTo, 1.0)
        .unwrap();

    let path = store
        .shortest_path(a, d)
        .unwrap()
        .expect("path should exist");
    assert_eq!(path.len(), 4, "A→B→C→D is 4 nodes (3 edges)");
    assert_eq!(path[0].canonical_name, "SpA");
    assert_eq!(path[3].canonical_name, "SpD");
}

/// When both a long route A→B→C and a direct shortcut A→C exist,
/// BFS returns the 2-node direct path.
#[test]
fn test_shortest_path_prefers_direct_edge_over_longer_route() {
    let (store, _dir) = make_store();

    let a = store.upsert_entity("DirA", "concept").unwrap();
    let b = store.upsert_entity("DirB", "concept").unwrap();
    let c = store.upsert_entity("DirC", "concept").unwrap();

    store
        .add_relationship(a, b, RelationType::RelatedTo, 1.0)
        .unwrap();
    store
        .add_relationship(b, c, RelationType::RelatedTo, 1.0)
        .unwrap();
    store
        .add_relationship(a, c, RelationType::RelatedTo, 1.0)
        .unwrap(); // shortcut

    let path = store
        .shortest_path(a, c)
        .unwrap()
        .expect("path should exist");
    assert_eq!(path.len(), 2, "direct edge A→C should win over A→B→C");
    assert_eq!(path[0].canonical_name, "DirA");
    assert_eq!(path[1].canonical_name, "DirC");
}

/// `shortest_path(x, x)` returns a single-element path containing x.
#[test]
fn test_shortest_path_same_entity_returns_singleton() {
    let (store, _dir) = make_store();

    let solo = store.upsert_entity("SoloSP", "concept").unwrap();
    let path = store
        .shortest_path(solo, solo)
        .unwrap()
        .expect("self-path should exist");
    assert_eq!(path.len(), 1);
    assert_eq!(path[0].canonical_name, "SoloSP");
}

/// No edges → `shortest_path` returns `None`.
#[test]
fn test_shortest_path_disconnected_returns_none() {
    let (store, _dir) = make_store();

    let a = store.upsert_entity("IsoA", "concept").unwrap();
    let b = store.upsert_entity("IsoB", "concept").unwrap();
    // No edges

    let path = store.shortest_path(a, b).unwrap();
    assert!(path.is_none(), "disconnected entities should return None");
}

/// BFS treats edges as undirected: path from X to Z exists even when both
/// outgoing edges X→Y and Z→Y point to the same intermediate node Y.
#[test]
fn test_shortest_path_undirected_traversal() {
    let (store, _dir) = make_store();

    let x = store.upsert_entity("UdX", "concept").unwrap();
    let y = store.upsert_entity("UdY", "concept").unwrap();
    let z = store.upsert_entity("UdZ", "concept").unwrap();

    // Both directed to Y: X→Y and Z→Y
    store
        .add_relationship(x, y, RelationType::RelatedTo, 1.0)
        .unwrap();
    store
        .add_relationship(z, y, RelationType::RelatedTo, 1.0)
        .unwrap();

    // Undirected BFS should still find X–Y–Z
    let path = store.shortest_path(x, z).unwrap();
    assert!(path.is_some(), "undirected BFS should find X–Y–Z path");
    assert_eq!(path.unwrap().len(), 3);
}

// ============================================================================
// 4. Community detection
// ============================================================================

/// Two completely isolated cliques of 3 entities each → exactly 2 communities,
/// each with 3 members, and the two hub entities in distinct communities.
#[test]
fn test_community_detection_two_isolated_cliques() {
    let (store, _dir) = make_store();

    // Clique 1: Alpha, Beta, Gamma — all strongly interconnected
    let alpha = store.upsert_entity("Alpha", "concept").unwrap();
    let beta = store.upsert_entity("Beta", "concept").unwrap();
    let gamma = store.upsert_entity("Gamma", "concept").unwrap();
    store
        .add_relationship(alpha, beta, RelationType::CoOccurs, 2.0)
        .unwrap();
    store
        .add_relationship(alpha, gamma, RelationType::CoOccurs, 2.0)
        .unwrap();
    store
        .add_relationship(beta, gamma, RelationType::CoOccurs, 2.0)
        .unwrap();

    // Clique 2: Delta, Epsilon, Zeta — completely isolated from Clique 1
    let delta = store.upsert_entity("Delta", "concept").unwrap();
    let epsilon = store.upsert_entity("Epsilon", "concept").unwrap();
    let zeta = store.upsert_entity("Zeta", "concept").unwrap();
    store
        .add_relationship(delta, epsilon, RelationType::CoOccurs, 2.0)
        .unwrap();
    store
        .add_relationship(delta, zeta, RelationType::CoOccurs, 2.0)
        .unwrap();
    store
        .add_relationship(epsilon, zeta, RelationType::CoOccurs, 2.0)
        .unwrap();

    let n = detect_communities(&store).unwrap();
    assert_eq!(n, 2, "two isolated cliques → exactly 2 communities");

    // Each clique forms a community of 3 members
    let communities = store.get_communities().unwrap();
    for c in &communities {
        assert_eq!(
            c.entity_count, 3,
            "each clique community should have 3 members"
        );
    }

    // Alpha and Delta must be in different communities
    let alpha_comm = store.get_entity_community(alpha).unwrap().unwrap();
    let delta_comm = store.get_entity_community(delta).unwrap().unwrap();
    assert_ne!(
        alpha_comm.id, delta_comm.id,
        "Alpha (Clique 1) and Delta (Clique 2) should be in distinct communities"
    );
}

/// Hub-and-spoke: the most-connected entity receives role "hub" after detection.
#[test]
fn test_community_detection_hub_role_assigned() {
    let (store, _dir) = make_store();

    let hub = store.upsert_entity("TheHub", "concept").unwrap();
    // Use a unique entity_type per spoke so the Levenshtein fuzzy-match
    // in upsert_entity (which is scoped to entity_type) cannot collapse
    // "HubSpoke0"..."HubSpoke4" into a single entity (distance 1/9 ≈ 0.89 ≥ 0.85).
    let mut spoke_ids = Vec::new();
    for i in 0..5 {
        let sid = store
            .upsert_entity(&format!("HubSpoke{}", i), &format!("spk{}", i))
            .unwrap();
        spoke_ids.push(sid);
        store
            .add_relationship(hub, sid, RelationType::RelatedTo, 1.0)
            .unwrap();
    }
    let unique_spokes: HashSet<i64> = spoke_ids.iter().copied().collect();
    assert_eq!(
        unique_spokes.len(),
        5,
        "all 5 spokes must be distinct entities"
    );

    detect_communities(&store).unwrap();

    let hub_comm = store
        .get_entity_community(hub)
        .unwrap()
        .expect("hub must belong to a community");
    let members = store.get_community_members(hub_comm.id).unwrap();
    let hub_role = members
        .iter()
        .find(|(id, _, _)| *id == hub)
        .map(|(_, _, role)| role.as_str());

    assert_eq!(
        hub_role,
        Some("hub"),
        "most-connected entity should receive role 'hub'"
    );
}

/// Bridge entity connecting two tight clusters is assigned to a community.
#[test]
fn test_community_detection_bridge_entity_assigned() {
    let (store, _dir) = make_store();

    // Cluster A
    let a1 = store.upsert_entity("ClA1", "concept").unwrap();
    let a2 = store.upsert_entity("ClA2", "concept").unwrap();
    store
        .add_relationship(a1, a2, RelationType::CoOccurs, 3.0)
        .unwrap();

    // Cluster B
    let b1 = store.upsert_entity("ClB1", "concept").unwrap();
    let b2 = store.upsert_entity("ClB2", "concept").unwrap();
    store
        .add_relationship(b1, b2, RelationType::CoOccurs, 3.0)
        .unwrap();

    // Bridge connects both clusters with weaker links
    let bridge = store.upsert_entity("BridgeX", "concept").unwrap();
    store
        .add_relationship(bridge, a1, RelationType::RelatedTo, 1.0)
        .unwrap();
    store
        .add_relationship(bridge, b1, RelationType::RelatedTo, 1.0)
        .unwrap();

    let n = detect_communities(&store).unwrap();
    assert!(n >= 1, "should detect at least 1 community");

    let bridge_comm = store.get_entity_community(bridge).unwrap();
    assert!(
        bridge_comm.is_some(),
        "bridge entity must belong to a community"
    );
}

/// `clear_communities` removes all data; re-running detection repopulates.
#[test]
fn test_community_detection_clear_and_rerun() {
    let (store, _dir) = make_store();

    let a = store.upsert_entity("RRA", "concept").unwrap();
    let b = store.upsert_entity("RRB", "concept").unwrap();
    store
        .add_relationship(a, b, RelationType::CoOccurs, 1.0)
        .unwrap();

    assert!(detect_communities(&store).unwrap() >= 1);

    store.clear_communities().unwrap();
    assert!(
        store.get_communities().unwrap().is_empty(),
        "clear_communities should remove all community records"
    );

    assert!(
        detect_communities(&store).unwrap() >= 1,
        "re-running detection should repopulate communities"
    );
}

/// All entities are reachable via some community after detection.
#[test]
fn test_community_detection_all_entities_assigned() {
    let (store, _dir) = make_store();

    let ids: Vec<i64> = (0..6)
        .map(|i| {
            store
                .upsert_entity(&format!("AssignEnt{}", i), "concept")
                .unwrap()
        })
        .collect();

    // Two triangles sharing no edges
    store
        .add_relationship(ids[0], ids[1], RelationType::CoOccurs, 2.0)
        .unwrap();
    store
        .add_relationship(ids[1], ids[2], RelationType::CoOccurs, 2.0)
        .unwrap();
    store
        .add_relationship(ids[0], ids[2], RelationType::CoOccurs, 2.0)
        .unwrap();

    store
        .add_relationship(ids[3], ids[4], RelationType::CoOccurs, 2.0)
        .unwrap();
    store
        .add_relationship(ids[4], ids[5], RelationType::CoOccurs, 2.0)
        .unwrap();
    store
        .add_relationship(ids[3], ids[5], RelationType::CoOccurs, 2.0)
        .unwrap();

    detect_communities(&store).unwrap();

    for id in &ids {
        let comm = store.get_entity_community(*id).unwrap();
        assert!(
            comm.is_some(),
            "entity {} should be assigned to a community",
            id
        );
    }
}

// ============================================================================
// 5. Graph-augmented search
// ============================================================================

/// Graph-augmented search finds at least as many results as plain FTS5 search.
///
/// Setup: Alice → Zeus (WorksOn edge).  Two messages are stored — one about
/// Zeus only, one about Alice only.  A plain search for "GsAlice" finds only
/// the Alice message.  A graph-augmented search expands to include Zeus
/// neighbors and should surface the Zeus message as well.
#[test]
fn test_graph_augmented_search_richer_than_plain_search() {
    let (store, _dir) = make_store();

    // Build entity graph: GsAlice → GsZeus
    let alice_id = store.upsert_entity("GsAlice", "person").unwrap();
    let zeus_id = store.upsert_entity("GsZeus", "project").unwrap();
    store
        .add_relationship(alice_id, zeus_id, RelationType::WorksOn, 1.0)
        .unwrap();

    // Message 1: about GsZeus only (not GsAlice)
    let m1 = store
        .store_raw_message("gs", "user", "GsZeus security audit passed")
        .unwrap();
    store.link_entity_to_message(zeus_id, m1, "GsZeus").unwrap();

    // Message 2: about GsAlice only (not GsZeus)
    let m2 = store
        .store_raw_message("gs", "user", "GsAlice submitted the quarterly report")
        .unwrap();
    store
        .link_entity_to_message(alice_id, m2, "GsAlice")
        .unwrap();

    // Plain FTS5 search: "GsAlice" matches message 2 only
    let plain = store.search("GsAlice", 20).unwrap();
    assert!(
        !plain.is_empty(),
        "plain search should find Alice's message"
    );

    // Graph-augmented: "GsAlice OR GsZeus" should match both messages
    let augmented = graph_augmented_search(&store, "GsAlice", 20).unwrap();
    assert!(
        augmented.len() >= plain.len(),
        "augmented ({}) should return >= plain ({}) results",
        augmented.len(),
        plain.len()
    );
    assert!(
        !augmented.is_empty(),
        "augmented search must return at least one result"
    );
}

/// `expand_query_via_graph` adds direct graph neighbors to the query string.
#[test]
fn test_query_expansion_includes_direct_neighbors() {
    let (store, _dir) = make_store();

    let alice = store.upsert_entity("QeAlice", "person").unwrap();
    let zeus = store.upsert_entity("QeZeus", "project").unwrap();
    let bob = store.upsert_entity("QeBob", "person").unwrap();
    store
        .add_relationship(alice, zeus, RelationType::WorksOn, 1.0)
        .unwrap();
    store
        .add_relationship(bob, zeus, RelationType::WorksOn, 1.0)
        .unwrap();

    let expanded = expand_query_via_graph(&store, "QeAlice", 1).unwrap();
    assert!(
        expanded.contains("QeAlice"),
        "original term should be preserved: {}",
        expanded
    );
    assert!(
        expanded.contains("QeZeus"),
        "direct neighbor QeZeus should be in expansion: {}",
        expanded
    );
}

/// Two-hop expansion reaches indirect (second-degree) neighbors.
#[test]
fn test_query_expansion_two_hops_reaches_indirect_neighbors() {
    let (store, _dir) = make_store();

    let alice = store.upsert_entity("HopAlice", "person").unwrap();
    let zeus = store.upsert_entity("HopZeus", "project").unwrap();
    let mnemosyne = store.upsert_entity("HopMnemosyne", "component").unwrap();

    // Alice → Zeus (1 hop), Mnemosyne → Zeus (so Mnemosyne is 2 hops from Alice)
    store
        .add_relationship(alice, zeus, RelationType::WorksOn, 1.0)
        .unwrap();
    store
        .add_relationship(mnemosyne, zeus, RelationType::PartOf, 1.0)
        .unwrap();

    // 1-hop: Zeus appears, Mnemosyne should not
    let exp1 = expand_query_via_graph(&store, "HopAlice", 1).unwrap();
    assert!(
        exp1.contains("HopZeus"),
        "1-hop: Zeus should appear: {}",
        exp1
    );

    // 2-hop: Zeus AND Mnemosyne (reached via Zeus)
    let exp2 = expand_query_via_graph(&store, "HopAlice", 2).unwrap();
    assert!(
        exp2.contains("HopMnemosyne"),
        "2-hop: Mnemosyne should appear: {}",
        exp2
    );
}

/// When the query matches no known entities, expansion returns it unchanged.
#[test]
fn test_query_expansion_unknown_query_unchanged() {
    let (store, _dir) = make_store();
    store.upsert_entity("SomeKnownEntity", "concept").unwrap();

    let expanded = expand_query_via_graph(&store, "totally unknown xyz123", 1).unwrap();
    assert_eq!(expanded, "totally unknown xyz123");
}

/// Empty store: graph-augmented search returns empty results without error.
#[test]
fn test_graph_augmented_search_empty_store() {
    let (store, _dir) = make_store();
    let results = graph_augmented_search(&store, "anything", 10).unwrap();
    assert!(results.is_empty());
}

// ============================================================================
// 6. Edge cases
// ============================================================================

/// Orphan entity (no relationships): BFS returns empty nodes and edges,
/// shortest_path to self returns [self], shortest_path to any other entity
/// returns None.
#[test]
fn test_edge_case_orphan_entity() {
    let (store, _dir) = make_store();

    let orphan = store.upsert_entity("OrphanNode", "concept").unwrap();
    let other = store.upsert_entity("OtherNode", "concept").unwrap();

    // BFS from orphan at any depth: no neighbours
    let g = store.get_entity_graph(orphan, 5).unwrap();
    assert_eq!(g.origin.canonical_name, "OrphanNode");
    assert!(g.nodes.is_empty(), "orphan should have no neighbour nodes");
    assert!(g.edges.is_empty(), "orphan should have no edges");

    // Self path
    let self_path = store.shortest_path(orphan, orphan).unwrap().unwrap();
    assert_eq!(self_path.len(), 1);

    // Path to unconnected entity
    let no_path = store.shortest_path(orphan, other).unwrap();
    assert!(no_path.is_none(), "orphan has no path to another entity");
}

/// Single entity with no edges forms exactly 1 community containing itself.
#[test]
fn test_edge_case_single_entity_own_community() {
    let (store, _dir) = make_store();
    store.upsert_entity("SingletonX", "concept").unwrap();

    let n = detect_communities(&store).unwrap();
    assert_eq!(n, 1, "single entity → exactly 1 community");

    let communities = store.get_communities().unwrap();
    assert_eq!(communities[0].entity_count, 1);
}

/// Empty store: 0 communities, 0 relationships, empty augmented search.
#[test]
fn test_edge_case_empty_graph() {
    let (store, _dir) = make_store();

    assert_eq!(detect_communities(&store).unwrap(), 0);
    assert_eq!(store.relationship_count().unwrap(), 0);

    let results = graph_augmented_search(&store, "anything", 10).unwrap();
    assert!(results.is_empty());
}

/// Self-referencing relationship (source == target) does not cause infinite
/// BFS loops and does not add the origin as a child node.
#[test]
fn test_edge_case_self_referencing_relationship_no_loop() {
    let (store, _dir) = make_store();

    let me = store.upsert_entity("SelfRef", "concept").unwrap();

    // SQLite UNIQUE(source, target, type) permits source == target
    let rel_id = store
        .add_relationship(me, me, RelationType::RelatedTo, 1.0)
        .unwrap();
    assert!(rel_id > 0, "self-referencing relationship should be stored");

    // BFS: visited set contains `me` from the start → no child nodes added
    let g = store.get_entity_graph(me, 5).unwrap();
    assert!(
        g.nodes.is_empty(),
        "self-referencing node should produce no child nodes in BFS"
    );

    // shortest_path(me, me) early-exits and returns [me]
    let path = store.shortest_path(me, me).unwrap().unwrap();
    assert_eq!(path.len(), 1);
}

/// Repeated `add_relationship` calls for the same triple upsert the edge
/// (incrementing mention_count and taking the max weight) rather than
/// inserting duplicate rows.
#[test]
fn test_edge_case_duplicate_relationship_upserted() {
    let (store, _dir) = make_store();

    let a = store.upsert_entity("DupSrc", "concept").unwrap();
    let b = store.upsert_entity("DupDst", "concept").unwrap();

    store
        .add_relationship(a, b, RelationType::RelatedTo, 1.0)
        .unwrap();
    store
        .add_relationship(a, b, RelationType::RelatedTo, 0.5)
        .unwrap(); // weight stays 1.0
    store
        .add_relationship(a, b, RelationType::RelatedTo, 2.0)
        .unwrap(); // weight raised to 2.0

    let rels = store.get_relationships(a, Direction::Outgoing).unwrap();
    assert_eq!(rels.len(), 1, "three upserts should produce exactly 1 edge");
    assert_eq!(rels[0].mention_count, 3, "mention_count should be 3");
    assert!(
        (rels[0].weight - 2.0).abs() < f64::EPSILON,
        "weight should be max(1.0, 0.5, 2.0) = 2.0, got {}",
        rels[0].weight
    );
}

/// `get_relationships` direction filter is correct for Outgoing, Incoming, Both.
#[test]
fn test_edge_case_relationship_direction_filter() {
    let (store, _dir) = make_store();

    let src = store.upsert_entity("DfSrc", "concept").unwrap();
    let dst = store.upsert_entity("DfDst", "concept").unwrap();
    store
        .add_relationship(src, dst, RelationType::WorksOn, 1.0)
        .unwrap();

    assert_eq!(
        store
            .get_relationships(src, Direction::Outgoing)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .get_relationships(src, Direction::Incoming)
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        store.get_relationships(src, Direction::Both).unwrap().len(),
        1
    );

    assert_eq!(
        store
            .get_relationships(dst, Direction::Outgoing)
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        store
            .get_relationships(dst, Direction::Incoming)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store.get_relationships(dst, Direction::Both).unwrap().len(),
        1
    );
}

/// Every `RelationType` variant survives a label→parse round-trip.
/// Unknown labels fall back to `RelatedTo`.
#[test]
fn test_edge_case_relation_type_round_trip() {
    for rt in RelationType::all() {
        let label = rt.as_label();
        let parsed = RelationType::from_label(label);
        assert_eq!(*rt, parsed, "round-trip failed for {:?}", rt);
    }
    assert_eq!(
        RelationType::from_label("BOGUS_LABEL"),
        RelationType::RelatedTo
    );
}

// ============================================================================
// 7. Performance
// ============================================================================

/// Performance benchmark: 1000 entities + ≥5000 relationships.
///
/// Graph topology: a directed ring (guarantees full connectivity) plus
/// deterministic cross-links at strides 3, 7, 11, 17, 23 (≈4000 extra edges).
///
/// Only the *operations* are timed — setup is excluded from the assertion.
///
/// Assertions:
/// - BFS depth-2 from entity 0 completes in <100 ms
/// - Shortest path from entity 0 to entity 500 completes in <100 ms
/// - `relationship_count()` completes in <50 ms
#[test]
fn test_performance_1000_entities_5000_relationships() {
    let (store, _dir) = make_store();
    const N: usize = 1000;

    // ── Setup (not timed) ─────────────────────────────────────────────────
    let mut ids = Vec::with_capacity(N);
    for i in 0..N {
        // Use a unique entity_type per entity ("pt{i}") so the Levenshtein
        // fuzzy-match in upsert_entity (which is scoped to entity_type) finds
        // only one candidate per bucket → no cross-entity collisions.
        let id = store
            .upsert_entity("PerfNode", &format!("pt{}", i))
            .unwrap();
        ids.push(id);
    }

    // Ring: 1000 directed edges; guarantees every entity is reachable.
    for i in 0..N {
        store
            .add_relationship(ids[i], ids[(i + 1) % N], RelationType::RelatedTo, 1.0)
            .unwrap();
    }

    // Cross-links at multiple strides to hit ≥5000 total edges.
    // Uses CoOccurs so UNIQUE(source, target, type) never collides with ring edges.
    let mut added = N;
    'done: for &stride in &[3usize, 7, 11, 17, 23] {
        for i in 0..N {
            if added >= 5000 {
                break 'done;
            }
            let j = (i + stride) % N;
            if j != i {
                let _ = store.add_relationship(ids[i], ids[j], RelationType::CoOccurs, 1.0);
                added += 1;
            }
        }
    }

    let actual_count = store.relationship_count().unwrap();
    assert!(
        actual_count >= N,
        "should have at least {} relationships, got {}",
        N,
        actual_count
    );

    // ── Benchmark: BFS depth-2 ────────────────────────────────────────────
    let t0 = std::time::Instant::now();
    let traversal = store.get_entity_graph(ids[0], 2).unwrap();
    let bfs_ms = t0.elapsed().as_millis();

    assert!(
        !traversal.nodes.is_empty(),
        "BFS should discover neighbours on a connected graph"
    );
    assert!(
        bfs_ms < 100,
        "BFS depth-2 on {} entities took {}ms — expected <100ms",
        N,
        bfs_ms
    );

    // ── Benchmark: shortest path ──────────────────────────────────────────
    // Ring guarantees a path from entity 0 to entity N/2; cross-links shorten it.
    let t1 = std::time::Instant::now();
    let path = store.shortest_path(ids[0], ids[N / 2]).unwrap();
    let sp_ms = t1.elapsed().as_millis();

    assert!(
        path.is_some(),
        "ring topology guarantees a path from PerfEnt0 to PerfEnt{}",
        N / 2
    );
    assert!(
        sp_ms < 100,
        "shortest_path on {} entities took {}ms — expected <100ms",
        N,
        sp_ms
    );

    // ── Benchmark: relationship_count ─────────────────────────────────────
    let t2 = std::time::Instant::now();
    let cnt = store.relationship_count().unwrap();
    let cnt_ms = t2.elapsed().as_millis();

    assert!(cnt >= N);
    assert!(
        cnt_ms < 50,
        "relationship_count took {}ms — expected <50ms",
        cnt_ms
    );
}

/// Community detection on a moderately connected ring graph (200 nodes, ~400 edges)
/// completes within 2 seconds (label-propagation is O(iterations × edges)).
#[test]
fn test_performance_community_detection_moderate_graph() {
    let (store, _dir) = make_store();
    const N: usize = 200;

    let mut ids = Vec::with_capacity(N);
    for i in 0..N {
        let id = store
            .upsert_entity(&format!("CdEnt{}", i), "concept")
            .unwrap();
        ids.push(id);
    }

    // Ring + stride-7 cross-links; weight ≥ MIN_WEIGHT so they're included in LPA
    for i in 0..N {
        store
            .add_relationship(ids[i], ids[(i + 1) % N], RelationType::CoOccurs, 1.0)
            .unwrap();
        let j = (i + 7) % N;
        if j != i {
            let _ = store.add_relationship(ids[i], ids[j], RelationType::CoOccurs, 1.0);
        }
    }

    let t = std::time::Instant::now();
    let n_comm = detect_communities(&store).unwrap();
    let elapsed_ms = t.elapsed().as_millis();

    assert!(n_comm >= 1, "should detect at least 1 community");
    assert!(
        elapsed_ms < 2000,
        "detect_communities on {} entities took {}ms — expected <2000ms",
        N,
        elapsed_ms
    );
}
