//! Graph Memory - Entity Relationship Graph for Multi-Hop Reasoning
//!
//! Implements an embedded knowledge graph in SQLite for Cognee-inspired
//! entity-relationship traversal. Enables multi-hop reasoning, shortest-path
//! queries, and graph-augmented search.
//!
//! ## Architecture
//!
//! - `relationships` table: directed edges between entities (source → target)
//! - `communities` table: clusters of related entities (label propagation)
//! - `community_members` table: entity ↔ community junction
//! - `promotions` table: episodic → semantic promotion audit log
//!
//! All graph operations run on the existing SQLite database — zero external infra.

use crate::{EntityRecord, MemoryStore};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use tracing::{debug, info};
use zeus_core::{Error, Result};

// ============================================================================
// Types
// ============================================================================

/// Relationship types between entities in the knowledge graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RelationType {
    /// Person → project
    WorksOn,
    /// Entity → message/document
    MentionedIn,
    /// Generic association
    RelatedTo,
    /// Entity → group/project
    PartOf,
    /// Artifact → person
    CreatedBy,
    /// Project → project
    DependsOn,
    /// Entity → place
    LocatedAt,
    /// Resource → person/org
    OwnedBy,
    /// Concept → older concept
    Supersedes,
    /// Entity ↔ entity (appear together frequently)
    CoOccurs,
    /// "X uses Y" / "X depends on Y" (usage relationship)
    Uses,
    /// "X owns Y" / "X manages Y" / "X maintains Y"
    Owns,
    /// "X talks to Y" / "X communicates with Y"
    CommunicatesWith,
}

impl RelationType {
    /// Parse a relationship type from a string label.
    pub fn from_label(s: &str) -> Self {
        match s.to_uppercase().replace('-', "_").as_str() {
            "WORKS_ON" => Self::WorksOn,
            "MENTIONED_IN" => Self::MentionedIn,
            "RELATED_TO" => Self::RelatedTo,
            "PART_OF" => Self::PartOf,
            "CREATED_BY" => Self::CreatedBy,
            "DEPENDS_ON" => Self::DependsOn,
            "LOCATED_AT" => Self::LocatedAt,
            "OWNED_BY" => Self::OwnedBy,
            "SUPERSEDES" => Self::Supersedes,
            "CO_OCCURS" | "COOCCURS" => Self::CoOccurs,
            "USES" => Self::Uses,
            "OWNS" => Self::Owns,
            "COMMUNICATES_WITH" => Self::CommunicatesWith,
            _ => Self::RelatedTo,
        }
    }

    /// Return the canonical string label.
    pub fn as_label(&self) -> &'static str {
        match self {
            Self::WorksOn => "WORKS_ON",
            Self::MentionedIn => "MENTIONED_IN",
            Self::RelatedTo => "RELATED_TO",
            Self::PartOf => "PART_OF",
            Self::CreatedBy => "CREATED_BY",
            Self::DependsOn => "DEPENDS_ON",
            Self::LocatedAt => "LOCATED_AT",
            Self::OwnedBy => "OWNED_BY",
            Self::Supersedes => "SUPERSEDES",
            Self::CoOccurs => "CO_OCCURS",
            Self::Uses => "USES",
            Self::Owns => "OWNS",
            Self::CommunicatesWith => "COMMUNICATES_WITH",
        }
    }

    /// All defined relationship types.
    pub fn all() -> &'static [RelationType] {
        &[
            Self::WorksOn,
            Self::MentionedIn,
            Self::RelatedTo,
            Self::PartOf,
            Self::CreatedBy,
            Self::DependsOn,
            Self::LocatedAt,
            Self::OwnedBy,
            Self::Supersedes,
            Self::CoOccurs,
            Self::Uses,
            Self::Owns,
            Self::CommunicatesWith,
        ]
    }
}

impl std::fmt::Display for RelationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_label())
    }
}

/// A directed relationship (edge) between two entities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    pub id: i64,
    pub source_entity_id: i64,
    pub target_entity_id: i64,
    pub relationship_type: RelationType,
    pub weight: f64,
    pub first_seen: String,
    pub last_seen: String,
    pub mention_count: i64,
    pub metadata: String,
}

/// Direction filter for relationship queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Relationships where entity is the source (outgoing).
    Outgoing,
    /// Relationships where entity is the target (incoming).
    Incoming,
    /// Both directions.
    Both,
}

/// A node in a graph traversal result, with depth info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub entity: EntityRecord,
    pub relationship_type: RelationType,
    /// Hops from the origin entity (0 = the origin itself).
    pub depth: u32,
}

/// Result of a graph traversal (BFS).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphTraversal {
    /// The origin entity.
    pub origin: EntityRecord,
    /// All connected nodes found within the requested depth.
    pub nodes: Vec<GraphNode>,
    /// All edges traversed.
    pub edges: Vec<Relationship>,
}

/// A community (cluster) of related entities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Community {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub entity_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

/// Community membership with role info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityMember {
    pub community_id: i64,
    pub entity_id: i64,
    pub role: String,
    pub added_at: String,
}

/// An episodic → semantic promotion record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Promotion {
    pub id: i64,
    pub source_message_id: i64,
    pub promoted_message_id: i64,
    pub reason: String,
    pub promoted_at: String,
}

/// Aggregated relationship type with count.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationshipTypeCount {
    pub relationship_type: String,
    pub count: usize,
}

// ============================================================================
// Schema — called from MemoryStore::new() in lib.rs
// ============================================================================

/// Create the graph-related tables and indexes in the existing database.
/// Safe to call multiple times (all CREATE IF NOT EXISTS).
pub fn init_graph_schema(conn: &rusqlite::Connection) -> Result<()> {
    // Entity-to-entity relationships (the graph edges)
    conn.execute(
        "CREATE TABLE IF NOT EXISTS relationships (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source_entity_id INTEGER NOT NULL,
            target_entity_id INTEGER NOT NULL,
            relationship_type TEXT NOT NULL,
            weight REAL NOT NULL DEFAULT 1.0,
            first_seen TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            last_seen TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            mention_count INTEGER NOT NULL DEFAULT 1,
            metadata TEXT DEFAULT '{}',
            FOREIGN KEY (source_entity_id) REFERENCES entities(id) ON DELETE CASCADE,
            FOREIGN KEY (target_entity_id) REFERENCES entities(id) ON DELETE CASCADE,
            UNIQUE(source_entity_id, target_entity_id, relationship_type)
        )",
        [],
    )
    .map_err(|e| Error::Database(format!("Failed to create relationships table: {}", e)))?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_rel_source ON relationships(source_entity_id)",
        [],
    )
    .map_err(|e| Error::Database(format!("Failed to create rel source index: {}", e)))?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_rel_target ON relationships(target_entity_id)",
        [],
    )
    .map_err(|e| Error::Database(format!("Failed to create rel target index: {}", e)))?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_rel_type ON relationships(relationship_type)",
        [],
    )
    .map_err(|e| Error::Database(format!("Failed to create rel type index: {}", e)))?;

    // Communities (clusters of related entities)
    conn.execute(
        "CREATE TABLE IF NOT EXISTS communities (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            description TEXT DEFAULT '',
            entity_count INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    )
    .map_err(|e| Error::Database(format!("Failed to create communities table: {}", e)))?;

    // Community membership
    conn.execute(
        "CREATE TABLE IF NOT EXISTS community_members (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            community_id INTEGER NOT NULL,
            entity_id INTEGER NOT NULL,
            role TEXT DEFAULT 'member',
            added_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE,
            FOREIGN KEY (entity_id) REFERENCES entities(id) ON DELETE CASCADE,
            UNIQUE(community_id, entity_id)
        )",
        [],
    )
    .map_err(|e| Error::Database(format!("Failed to create community_members table: {}", e)))?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_community_members ON community_members(community_id)",
        [],
    )
    .map_err(|e| Error::Database(format!("Failed to create community members index: {}", e)))?;

    // Episodic → semantic promotion log
    conn.execute(
        "CREATE TABLE IF NOT EXISTS promotions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source_message_id INTEGER NOT NULL,
            promoted_message_id INTEGER NOT NULL,
            reason TEXT NOT NULL,
            promoted_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (source_message_id) REFERENCES messages(id),
            FOREIGN KEY (promoted_message_id) REFERENCES messages(id)
        )",
        [],
    )
    .map_err(|e| Error::Database(format!("Failed to create promotions table: {}", e)))?;

    info!(
        "Mnemosyne: graph schema initialized (relationships, communities, community_members, promotions)"
    );
    Ok(())
}

// ============================================================================
// Graph CRUD operations — implemented on MemoryStore
// ============================================================================

impl MemoryStore {
    /// Add or update a relationship between two entities.
    /// If the (source, target, type) triple already exists, increments mention_count
    /// and updates last_seen and weight (takes the max).
    pub fn add_relationship(
        &self,
        source_id: i64,
        target_id: i64,
        rel_type: RelationType,
        weight: f64,
    ) -> Result<i64> {
        let type_label = rel_type.as_label();

        // Try upsert: increment mention_count if exists, else insert
        let updated = self
            .conn
            .execute(
                "UPDATE relationships
             SET mention_count = mention_count + 1,
                 last_seen = CURRENT_TIMESTAMP,
                 weight = MAX(weight, ?4)
             WHERE source_entity_id = ?1 AND target_entity_id = ?2 AND relationship_type = ?3",
                params![source_id, target_id, type_label, weight],
            )
            .map_err(|e| Error::Database(format!("Failed to update relationship: {}", e)))?;

        if updated > 0 {
            // Return existing ID
            let id: i64 = self
                .conn
                .query_row(
                    "SELECT id FROM relationships WHERE source_entity_id = ?1 AND target_entity_id = ?2 AND relationship_type = ?3",
                    params![source_id, target_id, type_label],
                    |row| row.get(0),
                )
                .map_err(|e| Error::Database(format!("Failed to get relationship id: {}", e)))?;
            debug!(
                "Updated relationship {}: {} --{}--> {} (weight={})",
                id, source_id, type_label, target_id, weight
            );
            return Ok(id);
        }

        // Insert new
        self.conn.execute(
            "INSERT INTO relationships (source_entity_id, target_entity_id, relationship_type, weight)
             VALUES (?1, ?2, ?3, ?4)",
            params![source_id, target_id, type_label, weight],
        )
        .map_err(|e| Error::Database(format!("Failed to insert relationship: {}", e)))?;

        let id = self.conn.last_insert_rowid();
        debug!(
            "Created relationship {}: {} --{}--> {} (weight={})",
            id, source_id, type_label, target_id, weight
        );
        Ok(id)
    }

    /// Get relationships for an entity, filtered by direction.
    pub fn get_relationships(
        &self,
        entity_id: i64,
        direction: Direction,
    ) -> Result<Vec<Relationship>> {
        let query = match direction {
            Direction::Outgoing => {
                "SELECT id, source_entity_id, target_entity_id, relationship_type, weight, first_seen, last_seen, mention_count, metadata
                 FROM relationships WHERE source_entity_id = ?1 ORDER BY weight DESC"
            }
            Direction::Incoming => {
                "SELECT id, source_entity_id, target_entity_id, relationship_type, weight, first_seen, last_seen, mention_count, metadata
                 FROM relationships WHERE target_entity_id = ?1 ORDER BY weight DESC"
            }
            Direction::Both => {
                "SELECT id, source_entity_id, target_entity_id, relationship_type, weight, first_seen, last_seen, mention_count, metadata
                 FROM relationships WHERE source_entity_id = ?1 OR target_entity_id = ?1 ORDER BY weight DESC"
            }
        };

        let mut stmt = self
            .conn
            .prepare(query)
            .map_err(|e| Error::Database(format!("Failed to prepare relationship query: {}", e)))?;

        let results = stmt
            .query_map(params![entity_id], |row| {
                Ok(Relationship {
                    id: row.get(0)?,
                    source_entity_id: row.get(1)?,
                    target_entity_id: row.get(2)?,
                    relationship_type: RelationType::from_label(&row.get::<_, String>(3)?),
                    weight: row.get(4)?,
                    first_seen: row.get(5)?,
                    last_seen: row.get(6)?,
                    mention_count: row.get(7)?,
                    metadata: row.get(8)?,
                })
            })
            .map_err(|e| Error::Database(format!("Failed to query relationships: {}", e)))?;

        results
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(format!("Failed to collect relationships: {}", e)))
    }

    /// Delete a specific relationship by ID.
    pub fn delete_relationship(&self, relationship_id: i64) -> Result<()> {
        self.conn
            .execute(
                "DELETE FROM relationships WHERE id = ?1",
                params![relationship_id],
            )
            .map_err(|e| Error::Database(format!("Failed to delete relationship: {}", e)))?;
        Ok(())
    }

    /// BFS traversal: get the entity graph up to `max_depth` hops from an entity.
    pub fn get_entity_graph(&self, entity_id: i64, max_depth: u32) -> Result<GraphTraversal> {
        // Get the origin entity
        let origin = self.get_entity_by_id(entity_id)?;

        let mut visited: HashSet<i64> = HashSet::new();
        let mut queue: VecDeque<(i64, u32)> = VecDeque::new();
        let mut nodes: Vec<GraphNode> = Vec::new();
        let mut edges: Vec<Relationship> = Vec::new();

        visited.insert(entity_id);
        queue.push_back((entity_id, 0));

        while let Some((current_id, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }

            let rels = self.get_relationships(current_id, Direction::Both)?;
            for rel in rels {
                edges.push(rel.clone());

                // Determine neighbor
                let neighbor_id = if rel.source_entity_id == current_id {
                    rel.target_entity_id
                } else {
                    rel.source_entity_id
                };

                if visited.insert(neighbor_id)
                    && let Ok(entity) = self.get_entity_by_id(neighbor_id)
                {
                    nodes.push(GraphNode {
                        entity,
                        relationship_type: rel.relationship_type,
                        depth: depth + 1,
                    });
                    queue.push_back((neighbor_id, depth + 1));
                }
            }
        }

        Ok(GraphTraversal {
            origin,
            nodes,
            edges,
        })
    }

    /// Find the shortest path between two entities using BFS.
    /// Returns None if no path exists. Returns the sequence of entities on the path.
    pub fn shortest_path(&self, entity_a: i64, entity_b: i64) -> Result<Option<Vec<EntityRecord>>> {
        if entity_a == entity_b {
            let entity = self.get_entity_by_id(entity_a)?;
            return Ok(Some(vec![entity]));
        }

        let mut visited: HashSet<i64> = HashSet::new();
        let mut queue: VecDeque<i64> = VecDeque::new();
        // parent map: child → parent (for path reconstruction)
        let mut parent: HashMap<i64, i64> = HashMap::new();

        visited.insert(entity_a);
        queue.push_back(entity_a);

        let mut found = false;

        while let Some(current_id) = queue.pop_front() {
            let rels = self.get_relationships(current_id, Direction::Both)?;
            for rel in rels {
                let neighbor_id = if rel.source_entity_id == current_id {
                    rel.target_entity_id
                } else {
                    rel.source_entity_id
                };

                if visited.insert(neighbor_id) {
                    parent.insert(neighbor_id, current_id);

                    if neighbor_id == entity_b {
                        found = true;
                        break;
                    }

                    queue.push_back(neighbor_id);
                }
            }

            if found {
                break;
            }
        }

        if !found {
            return Ok(None);
        }

        // Reconstruct path from entity_b back to entity_a
        let mut path_ids = vec![entity_b];
        let mut current = entity_b;
        while current != entity_a {
            current = *parent.get(&current).unwrap();
            path_ids.push(current);
        }
        path_ids.reverse();

        // Resolve entity records
        let mut path = Vec::with_capacity(path_ids.len());
        for id in path_ids {
            path.push(self.get_entity_by_id(id)?);
        }

        Ok(Some(path))
    }

    /// Find entities connected to a given entity, optionally filtered by relationship types.
    /// Returns (entity, relationship_type, depth) tuples.
    pub fn find_connected_entities(
        &self,
        entity_id: i64,
        rel_types: Option<&[RelationType]>,
        max_depth: u32,
    ) -> Result<Vec<(EntityRecord, RelationType, u32)>> {
        let traversal = self.get_entity_graph(entity_id, max_depth)?;

        let results: Vec<(EntityRecord, RelationType, u32)> = traversal
            .nodes
            .into_iter()
            .filter(|node| {
                if let Some(types) = rel_types {
                    types.contains(&node.relationship_type)
                } else {
                    true
                }
            })
            .map(|node| (node.entity, node.relationship_type, node.depth))
            .collect();

        Ok(results)
    }

    /// Get a summary of all relationship types and their counts.
    pub fn get_relationship_types(&self) -> Result<Vec<RelationshipTypeCount>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT relationship_type, COUNT(*) as cnt
             FROM relationships
             GROUP BY relationship_type
             ORDER BY cnt DESC",
            )
            .map_err(|e| Error::Database(format!("Failed to query relationship types: {}", e)))?;

        let results = stmt
            .query_map([], |row| {
                Ok(RelationshipTypeCount {
                    relationship_type: row.get(0)?,
                    count: row.get::<_, i64>(1)? as usize,
                })
            })
            .map_err(|e| Error::Database(format!("Failed to read relationship types: {}", e)))?;

        results
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(format!("Failed to collect relationship types: {}", e)))
    }

    /// Get total relationship count in the graph.
    pub fn relationship_count(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM relationships", [], |row| row.get(0))
            .map_err(|e| Error::Database(format!("Failed to count relationships: {}", e)))?;
        Ok(count as usize)
    }

    /// Get ALL relationships in the graph (used by community detection).
    pub fn get_all_relationships(&self) -> Result<Vec<Relationship>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, source_entity_id, target_entity_id, relationship_type, weight, first_seen, last_seen, mention_count, metadata
             FROM relationships ORDER BY weight DESC"
        )
        .map_err(|e| Error::Database(format!("Failed to prepare all relationships query: {}", e)))?;

        let results = stmt
            .query_map([], |row| {
                Ok(Relationship {
                    id: row.get(0)?,
                    source_entity_id: row.get(1)?,
                    target_entity_id: row.get(2)?,
                    relationship_type: RelationType::from_label(&row.get::<_, String>(3)?),
                    weight: row.get(4)?,
                    first_seen: row.get(5)?,
                    last_seen: row.get(6)?,
                    mention_count: row.get(7)?,
                    metadata: row.get(8)?,
                })
            })
            .map_err(|e| Error::Database(format!("Failed to query all relationships: {}", e)))?;

        results
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(format!("Failed to collect all relationships: {}", e)))
    }

    /// Store a raw message by role and content (convenience for tests/extraction).
    pub fn store_raw_message(&self, session_id: &str, role: &str, content: &str) -> Result<i64> {
        let timestamp = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO messages (session_id, role, content, tool_calls, tool_results, timestamp, valid_from)
             VALUES (?1, ?2, ?3, '[]', '[]', ?4, ?4)",
            params![session_id, role, content, timestamp],
        ).map_err(|e| Error::Database(format!("Failed to insert raw message: {}", e)))?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Get a single entity by ID.
    pub fn get_entity_by_id(&self, entity_id: i64) -> Result<EntityRecord> {
        self.conn
            .query_row(
                "SELECT id, canonical_name, entity_type, aliases, first_seen, last_seen, mention_count
                 FROM entities WHERE id = ?1",
                params![entity_id],
                |row| {
                    let aliases_json: String = row.get(3)?;
                    Ok(EntityRecord {
                        id: row.get(0)?,
                        canonical_name: row.get(1)?,
                        entity_type: row.get(2)?,
                        aliases: serde_json::from_str(&aliases_json).unwrap_or_default(),
                        first_seen: row.get(4)?,
                        last_seen: row.get(5)?,
                        mention_count: row.get(6)?,
                    })
                },
            )
            .map_err(|e| Error::Database(format!("Entity {} not found: {}", entity_id, e)))
    }

    // ========================================================================
    // Community operations (stored here, populated by community.rs)
    // ========================================================================

    /// Create a new community. Returns the community ID.
    pub fn create_community(&self, name: &str, description: &str) -> Result<i64> {
        self.conn
            .execute(
                "INSERT INTO communities (name, description) VALUES (?1, ?2)",
                params![name, description],
            )
            .map_err(|e| Error::Database(format!("Failed to create community: {}", e)))?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Add an entity to a community with a role.
    pub fn add_community_member(
        &self,
        community_id: i64,
        entity_id: i64,
        role: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO community_members (community_id, entity_id, role) VALUES (?1, ?2, ?3)",
            params![community_id, entity_id, role],
        )
        .map_err(|e| Error::Database(format!("Failed to add community member: {}", e)))?;

        // Update entity count
        self.conn.execute(
            "UPDATE communities SET entity_count = (SELECT COUNT(*) FROM community_members WHERE community_id = ?1), updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
            params![community_id],
        )
        .map_err(|e| Error::Database(format!("Failed to update community count: {}", e)))?;

        Ok(())
    }

    /// Get the community an entity belongs to.
    pub fn get_entity_community(&self, entity_id: i64) -> Result<Option<Community>> {
        let result = self.conn.query_row(
            "SELECT c.id, c.name, c.description, c.entity_count, c.created_at, c.updated_at
             FROM communities c
             JOIN community_members cm ON cm.community_id = c.id
             WHERE cm.entity_id = ?1
             LIMIT 1",
            params![entity_id],
            |row| {
                Ok(Community {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    entity_count: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            },
        );

        match result {
            Ok(c) => Ok(Some(c)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(Error::Database(format!(
                "Failed to get entity community: {}",
                e
            ))),
        }
    }

    /// Get all entities in a community.
    pub fn get_community_entities(&self, community_id: i64) -> Result<Vec<(EntityRecord, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT e.id, e.canonical_name, e.entity_type, e.aliases, e.first_seen, e.last_seen, e.mention_count, cm.role
             FROM entities e
             JOIN community_members cm ON cm.entity_id = e.id
             WHERE cm.community_id = ?1
             ORDER BY e.mention_count DESC"
        )
        .map_err(|e| Error::Database(format!("Failed to query community entities: {}", e)))?;

        let results = stmt
            .query_map(params![community_id], |row| {
                let aliases_json: String = row.get(3)?;
                Ok((
                    EntityRecord {
                        id: row.get(0)?,
                        canonical_name: row.get(1)?,
                        entity_type: row.get(2)?,
                        aliases: serde_json::from_str(&aliases_json).unwrap_or_default(),
                        first_seen: row.get(4)?,
                        last_seen: row.get(5)?,
                        mention_count: row.get(6)?,
                    },
                    row.get::<_, String>(7)?,
                ))
            })
            .map_err(|e| Error::Database(format!("Failed to read community entities: {}", e)))?;

        results
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(format!("Failed to collect community entities: {}", e)))
    }

    /// Get community members as (entity_id, name, role) tuples.
    /// Lighter than get_community_entities when you don't need full EntityRecord.
    pub fn get_community_members(&self, community_id: i64) -> Result<Vec<(i64, String, String)>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT cm.entity_id, e.canonical_name, cm.role
             FROM community_members cm
             JOIN entities e ON e.id = cm.entity_id
             WHERE cm.community_id = ?1",
            )
            .map_err(|e| Error::Database(format!("Failed to query community members: {}", e)))?;

        let results = stmt
            .query_map(params![community_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| Error::Database(format!("Failed to read community members: {}", e)))?;

        results
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(format!("Failed to collect community members: {}", e)))
    }

    /// Get all communities.
    pub fn get_communities(&self) -> Result<Vec<Community>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, description, entity_count, created_at, updated_at
             FROM communities ORDER BY entity_count DESC",
            )
            .map_err(|e| Error::Database(format!("Failed to query communities: {}", e)))?;

        let results = stmt
            .query_map([], |row| {
                Ok(Community {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    entity_count: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })
            .map_err(|e| Error::Database(format!("Failed to read communities: {}", e)))?;

        results
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(format!("Failed to collect communities: {}", e)))
    }

    /// Clear all communities (used before re-running detection).
    pub fn clear_communities(&self) -> Result<()> {
        self.conn
            .execute("DELETE FROM community_members", [])
            .map_err(|e| Error::Database(format!("Failed to clear community members: {}", e)))?;
        self.conn
            .execute("DELETE FROM communities", [])
            .map_err(|e| Error::Database(format!("Failed to clear communities: {}", e)))?;
        Ok(())
    }

    // ========================================================================
    // Promotion log operations
    // ========================================================================

    /// Log an episodic → semantic promotion.
    pub fn log_promotion(
        &self,
        source_message_id: i64,
        promoted_message_id: i64,
        reason: &str,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO promotions (source_message_id, promoted_message_id, reason) VALUES (?1, ?2, ?3)",
            params![source_message_id, promoted_message_id, reason],
        )
        .map_err(|e| Error::Database(format!("Failed to log promotion: {}", e)))?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Get promotions for a source message.
    pub fn get_promotions(&self, source_message_id: i64) -> Result<Vec<Promotion>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, source_message_id, promoted_message_id, reason, promoted_at
             FROM promotions WHERE source_message_id = ?1 ORDER BY promoted_at DESC",
            )
            .map_err(|e| Error::Database(format!("Failed to query promotions: {}", e)))?;

        let results = stmt
            .query_map(params![source_message_id], |row| {
                Ok(Promotion {
                    id: row.get(0)?,
                    source_message_id: row.get(1)?,
                    promoted_message_id: row.get(2)?,
                    reason: row.get(3)?,
                    promoted_at: row.get(4)?,
                })
            })
            .map_err(|e| Error::Database(format!("Failed to read promotions: {}", e)))?;

        results
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(format!("Failed to collect promotions: {}", e)))
    }

    /// Count total promotions.
    pub fn promotion_count(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM promotions", [], |row| row.get(0))
            .map_err(|e| Error::Database(format!("Failed to count promotions: {}", e)))?;
        Ok(count as usize)
    }

    /// Format the graph neighborhood of an entity for LLM context injection.
    /// Returns a human-readable string like: "Alice (person) --WORKS_ON--> Zeus (project)"
    pub fn format_graph_context(&self, entity_id: i64, max_depth: u32) -> Result<String> {
        let rels = self.get_relationships(entity_id, Direction::Both)?;
        if rels.is_empty() {
            return Ok(String::new());
        }

        let mut lines = Vec::new();
        let origin = self.get_entity_by_id(entity_id)?;

        for rel in &rels {
            let (source, target) = if rel.source_entity_id == entity_id {
                let target = self.get_entity_by_id(rel.target_entity_id)?;
                (origin.clone(), target)
            } else {
                let source = self.get_entity_by_id(rel.source_entity_id)?;
                (source, origin.clone())
            };

            lines.push(format!(
                "{} ({}) --{}--> {} ({})",
                source.canonical_name,
                source.entity_type,
                rel.relationship_type.as_label(),
                target.canonical_name,
                target.entity_type,
            ));
        }

        // If depth > 1, also get second-hop relationships
        if max_depth > 1 {
            let traversal = self.get_entity_graph(entity_id, max_depth)?;
            for node in &traversal.nodes {
                if node.depth == 2 {
                    // Just list the entity, not full edges (keeps context concise)
                    lines.push(format!(
                        "  └─ {} ({}) [via {}]",
                        node.entity.canonical_name,
                        node.entity.type_label(),
                        node.relationship_type.as_label(),
                    ));
                }
            }
        }

        Ok(lines.join("\n"))
    }
}

// ============================================================================
// EntityRecord helper (extend with type_label)
// ============================================================================

impl EntityRecord {
    /// Short type label for display.
    pub fn type_label(&self) -> &str {
        &self.entity_type
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup_store() -> MemoryStore {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test_graph.db");
        let store = MemoryStore::new(&db_path, true, false).unwrap();
        // The graph schema is initialized by MemoryStore::new via init_graph_schema
        store
    }

    fn create_test_entities(store: &MemoryStore) -> (i64, i64, i64, i64) {
        let alice = store.upsert_entity("Alice", "person").unwrap();
        let bob = store.upsert_entity("Bob", "person").unwrap();
        let zeus = store.upsert_entity("Zeus", "project").unwrap();
        let mnemosyne = store.upsert_entity("Mnemosyne", "component").unwrap();
        (alice, bob, zeus, mnemosyne)
    }

    #[test]
    fn test_add_relationship() {
        let store = setup_store();
        let (alice, _bob, zeus, _) = create_test_entities(&store);

        let rel_id = store
            .add_relationship(alice, zeus, RelationType::WorksOn, 1.0)
            .unwrap();
        assert!(rel_id > 0);

        let rels = store.get_relationships(alice, Direction::Outgoing).unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].source_entity_id, alice);
        assert_eq!(rels[0].target_entity_id, zeus);
        assert_eq!(rels[0].relationship_type, RelationType::WorksOn);
        assert_eq!(rels[0].mention_count, 1);
    }

    #[test]
    fn test_relationship_upsert() {
        let store = setup_store();
        let (alice, _bob, zeus, _) = create_test_entities(&store);

        store
            .add_relationship(alice, zeus, RelationType::WorksOn, 0.8)
            .unwrap();
        store
            .add_relationship(alice, zeus, RelationType::WorksOn, 0.9)
            .unwrap();

        let rels = store.get_relationships(alice, Direction::Outgoing).unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].mention_count, 2);
        // Weight should be max(0.8, 0.9) = 0.9
        assert!((rels[0].weight - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn test_get_relationships_by_direction() {
        let store = setup_store();
        let (alice, bob, zeus, _) = create_test_entities(&store);

        store
            .add_relationship(alice, zeus, RelationType::WorksOn, 1.0)
            .unwrap();
        store
            .add_relationship(bob, zeus, RelationType::WorksOn, 1.0)
            .unwrap();
        store
            .add_relationship(zeus, alice, RelationType::OwnedBy, 0.5)
            .unwrap();

        // Outgoing from alice: alice → zeus
        let out = store.get_relationships(alice, Direction::Outgoing).unwrap();
        assert_eq!(out.len(), 1);

        // Incoming to alice: zeus → alice
        let inc = store.get_relationships(alice, Direction::Incoming).unwrap();
        assert_eq!(inc.len(), 1);

        // Both for alice: 2 relationships
        let both = store.get_relationships(alice, Direction::Both).unwrap();
        assert_eq!(both.len(), 2);

        // Incoming to zeus: alice + bob → zeus
        let zeus_in = store.get_relationships(zeus, Direction::Incoming).unwrap();
        assert_eq!(zeus_in.len(), 2);
    }

    #[test]
    fn test_delete_relationship() {
        let store = setup_store();
        let (alice, _bob, zeus, _) = create_test_entities(&store);

        let rel_id = store
            .add_relationship(alice, zeus, RelationType::WorksOn, 1.0)
            .unwrap();
        store.delete_relationship(rel_id).unwrap();

        let rels = store.get_relationships(alice, Direction::Outgoing).unwrap();
        assert!(rels.is_empty());
    }

    #[test]
    fn test_bfs_traversal() {
        let store = setup_store();
        let (alice, bob, zeus, mnemosyne) = create_test_entities(&store);

        // alice → zeus → mnemosyne, bob → zeus
        store
            .add_relationship(alice, zeus, RelationType::WorksOn, 1.0)
            .unwrap();
        store
            .add_relationship(bob, zeus, RelationType::WorksOn, 1.0)
            .unwrap();
        store
            .add_relationship(mnemosyne, zeus, RelationType::PartOf, 1.0)
            .unwrap();

        // Depth 1 from alice: should find Zeus
        let g1 = store.get_entity_graph(alice, 1).unwrap();
        assert_eq!(g1.nodes.len(), 1);
        assert_eq!(g1.nodes[0].entity.canonical_name, "Zeus");

        // Depth 2 from alice: should find Zeus, Bob, Mnemosyne
        let g2 = store.get_entity_graph(alice, 2).unwrap();
        assert_eq!(g2.nodes.len(), 3);
        let names: HashSet<String> = g2
            .nodes
            .iter()
            .map(|n| n.entity.canonical_name.clone())
            .collect();
        assert!(names.contains("Zeus"));
        assert!(names.contains("Bob"));
        assert!(names.contains("Mnemosyne"));
    }

    #[test]
    fn test_shortest_path() {
        let store = setup_store();
        let (alice, bob, zeus, mnemosyne) = create_test_entities(&store);

        // alice → zeus, bob → zeus, mnemosyne → zeus
        store
            .add_relationship(alice, zeus, RelationType::WorksOn, 1.0)
            .unwrap();
        store
            .add_relationship(bob, zeus, RelationType::WorksOn, 1.0)
            .unwrap();
        store
            .add_relationship(mnemosyne, zeus, RelationType::PartOf, 1.0)
            .unwrap();

        // alice → zeus → bob (2 hops)
        let path = store.shortest_path(alice, bob).unwrap().unwrap();
        assert_eq!(path.len(), 3);
        assert_eq!(path[0].canonical_name, "Alice");
        assert_eq!(path[1].canonical_name, "Zeus");
        assert_eq!(path[2].canonical_name, "Bob");

        // Same entity = length 1
        let self_path = store.shortest_path(alice, alice).unwrap().unwrap();
        assert_eq!(self_path.len(), 1);
    }

    #[test]
    fn test_shortest_path_no_connection() {
        let store = setup_store();
        let (alice, bob, _zeus, _) = create_test_entities(&store);
        // No edges: no path
        let path = store.shortest_path(alice, bob).unwrap();
        assert!(path.is_none());
    }

    #[test]
    fn test_find_connected_entities_filtered() {
        let store = setup_store();
        let (alice, bob, zeus, mnemosyne) = create_test_entities(&store);

        store
            .add_relationship(alice, zeus, RelationType::WorksOn, 1.0)
            .unwrap();
        store
            .add_relationship(alice, bob, RelationType::RelatedTo, 0.5)
            .unwrap();
        store
            .add_relationship(mnemosyne, zeus, RelationType::PartOf, 1.0)
            .unwrap();

        // Filter to only WorksOn
        let connected = store
            .find_connected_entities(alice, Some(&[RelationType::WorksOn]), 1)
            .unwrap();
        assert_eq!(connected.len(), 1);
        assert_eq!(connected[0].0.canonical_name, "Zeus");

        // No filter: both Zeus and Bob at depth 1
        let all = store.find_connected_entities(alice, None, 1).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_relationship_types_summary() {
        let store = setup_store();
        let (alice, bob, zeus, mnemosyne) = create_test_entities(&store);

        store
            .add_relationship(alice, zeus, RelationType::WorksOn, 1.0)
            .unwrap();
        store
            .add_relationship(bob, zeus, RelationType::WorksOn, 1.0)
            .unwrap();
        store
            .add_relationship(mnemosyne, zeus, RelationType::PartOf, 1.0)
            .unwrap();

        let types = store.get_relationship_types().unwrap();
        assert_eq!(types.len(), 2);
        assert_eq!(types[0].relationship_type, "WORKS_ON");
        assert_eq!(types[0].count, 2);
        assert_eq!(types[1].relationship_type, "PART_OF");
        assert_eq!(types[1].count, 1);
    }

    #[test]
    fn test_community_crud() {
        let store = setup_store();
        let (alice, bob, zeus, _) = create_test_entities(&store);

        let community_id = store
            .create_community("Zeus Team", "People working on Zeus")
            .unwrap();
        store
            .add_community_member(community_id, alice, "hub")
            .unwrap();
        store
            .add_community_member(community_id, bob, "member")
            .unwrap();
        store
            .add_community_member(community_id, zeus, "member")
            .unwrap();

        // Check community
        let communities = store.get_communities().unwrap();
        assert_eq!(communities.len(), 1);
        assert_eq!(communities[0].name, "Zeus Team");
        assert_eq!(communities[0].entity_count, 3);

        // Check membership
        let members = store.get_community_entities(community_id).unwrap();
        assert_eq!(members.len(), 3);

        // Check entity lookup
        let alice_community = store.get_entity_community(alice).unwrap().unwrap();
        assert_eq!(alice_community.name, "Zeus Team");
    }

    #[test]
    fn test_promotion_log() {
        let store = setup_store();

        // We need message IDs — store dummy messages
        let msg = zeus_core::Message {
            role: zeus_core::Role::User,
            content: "test episodic".to_string(),
            tool_calls: vec![],
            tool_results: vec![],
            timestamp: chrono::Utc::now(),
            attachments: vec![],
            message_id: None,
            parent_id: None,
            thread_id: None,
            direction: Default::default(),
            channel_source: None,
            compaction_hint: Default::default(),
        };
        let source_id = store.store_message("test-session", &msg).unwrap();

        let promoted_msg = zeus_core::Message {
            role: zeus_core::Role::Assistant,
            content: "promoted semantic fact".to_string(),
            tool_calls: vec![],
            tool_results: vec![],
            timestamp: chrono::Utc::now(),
            attachments: vec![],
            message_id: None,
            parent_id: None,
            thread_id: None,
            direction: Default::default(),
            channel_source: None,
            compaction_hint: Default::default(),
        };
        let promoted_id = store.store_message("test-session", &promoted_msg).unwrap();

        let promo_id = store
            .log_promotion(source_id, promoted_id, "high_importance")
            .unwrap();
        assert!(promo_id > 0);

        let promos = store.get_promotions(source_id).unwrap();
        assert_eq!(promos.len(), 1);
        assert_eq!(promos[0].reason, "high_importance");
        assert_eq!(promos[0].promoted_message_id, promoted_id);

        assert_eq!(store.promotion_count().unwrap(), 1);
    }

    #[test]
    fn test_format_graph_context() {
        let store = setup_store();
        let (alice, _bob, zeus, _) = create_test_entities(&store);

        store
            .add_relationship(alice, zeus, RelationType::WorksOn, 1.0)
            .unwrap();

        let ctx = store.format_graph_context(alice, 1).unwrap();
        assert!(ctx.contains("Alice"));
        assert!(ctx.contains("WORKS_ON"));
        assert!(ctx.contains("Zeus"));
    }

    #[test]
    fn test_relation_type_roundtrip() {
        for rt in RelationType::all() {
            let label = rt.as_label();
            let parsed = RelationType::from_label(label);
            assert_eq!(*rt, parsed, "Roundtrip failed for {:?}", rt);
        }

        // Unknown maps to RelatedTo
        assert_eq!(RelationType::from_label("UNKNOWN"), RelationType::RelatedTo);
    }
}
