//! Pattern-based knowledge extraction from text.
//!
//! Extracts (subject, subject_type, relation, object, object_type) triples
//! using verb and preposition patterns — no LLM required.
//!
//! # Patterns
//!
//! **Verb patterns** (case-insensitive):
//! - "X works on Y" → WorksOn
//! - "X deployed to Y" / "X runs on Y" / "X installed on Y" → LocatedAt
//! - "X uses Y" / "X depends on Y" / "X requires Y" → Uses
//! - "X created Y" / "X built Y" / "X wrote Y" → CreatedBy (reversed: Y CreatedBy X)
//! - "X owns Y" / "X manages Y" / "X maintains Y" → Owns
//! - "X talks to Y" / "X communicates with Y" / "X messages Y" → CommunicatesWith
//!
//! **Preposition patterns**:
//! - "X of Y" → PartOf (X is part of Y)
//! - "X in Y" → LocatedAt
//! - "X on Y" (when Y looks like a host) → LocatedAt

use crate::MemoryStore;
use crate::graph::RelationType;
use zeus_core::Result;

/// Case-insensitive byte-position find that returns offsets valid for `haystack`.
///
/// Unlike `haystack.to_lowercase().find(needle)`, the returned position is
/// always a valid byte index into the *original* `haystack`.
/// `needle` **must** be ASCII lowercase.
fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    let n = needle.len();
    if n == 0 {
        return Some(0);
    }
    // Walk char boundaries so we never split a multi-byte char.
    haystack.char_indices().find_map(|(i, _)| {
        let remaining = &haystack[i..];
        if remaining.len() >= n
            && remaining.is_char_boundary(n)
            && remaining[..n].eq_ignore_ascii_case(needle)
        {
            Some(i)
        } else {
            None
        }
    })
}

/// A single extracted triple: (subject, subject_type, relation, object, object_type).
#[derive(Debug, Clone, PartialEq)]
pub struct Triple {
    pub entity_name: String,
    pub entity_type: String,
    pub relation: RelationType,
    pub target_name: String,
    pub target_type: String,
    /// Confidence score (0.0–1.0) based on pattern strength.
    pub confidence: f64,
}

/// Verb pattern definition: trigger phrase → relation type + whether to swap subject/object.
struct VerbPattern {
    trigger: &'static str,
    relation: RelationType,
    /// If true, the subject becomes the target (e.g., "X created Y" → Y CreatedBy X).
    swap: bool,
}

const VERB_PATTERNS: &[VerbPattern] = &[
    VerbPattern {
        trigger: " works on ",
        relation: RelationType::WorksOn,
        swap: false,
    },
    VerbPattern {
        trigger: " working on ",
        relation: RelationType::WorksOn,
        swap: false,
    },
    VerbPattern {
        trigger: " deployed to ",
        relation: RelationType::LocatedAt,
        swap: false,
    },
    VerbPattern {
        trigger: " deployed on ",
        relation: RelationType::LocatedAt,
        swap: false,
    },
    VerbPattern {
        trigger: " runs on ",
        relation: RelationType::LocatedAt,
        swap: false,
    },
    VerbPattern {
        trigger: " running on ",
        relation: RelationType::LocatedAt,
        swap: false,
    },
    VerbPattern {
        trigger: " installed on ",
        relation: RelationType::LocatedAt,
        swap: false,
    },
    VerbPattern {
        trigger: " hosted on ",
        relation: RelationType::LocatedAt,
        swap: false,
    },
    VerbPattern {
        trigger: " lives on ",
        relation: RelationType::LocatedAt,
        swap: false,
    },
    VerbPattern {
        trigger: " located at ",
        relation: RelationType::LocatedAt,
        swap: false,
    },
    VerbPattern {
        trigger: " uses ",
        relation: RelationType::Uses,
        swap: false,
    },
    VerbPattern {
        trigger: " using ",
        relation: RelationType::Uses,
        swap: false,
    },
    VerbPattern {
        trigger: " depends on ",
        relation: RelationType::Uses,
        swap: false,
    },
    VerbPattern {
        trigger: " requires ",
        relation: RelationType::Uses,
        swap: false,
    },
    VerbPattern {
        trigger: " created ",
        relation: RelationType::CreatedBy,
        swap: true,
    },
    VerbPattern {
        trigger: " built ",
        relation: RelationType::CreatedBy,
        swap: true,
    },
    VerbPattern {
        trigger: " wrote ",
        relation: RelationType::CreatedBy,
        swap: true,
    },
    VerbPattern {
        trigger: " authored ",
        relation: RelationType::CreatedBy,
        swap: true,
    },
    VerbPattern {
        trigger: " owns ",
        relation: RelationType::Owns,
        swap: false,
    },
    VerbPattern {
        trigger: " manages ",
        relation: RelationType::Owns,
        swap: false,
    },
    VerbPattern {
        trigger: " maintains ",
        relation: RelationType::Owns,
        swap: false,
    },
    VerbPattern {
        trigger: " talks to ",
        relation: RelationType::CommunicatesWith,
        swap: false,
    },
    VerbPattern {
        trigger: " communicates with ",
        relation: RelationType::CommunicatesWith,
        swap: false,
    },
    VerbPattern {
        trigger: " messages ",
        relation: RelationType::CommunicatesWith,
        swap: false,
    },
    VerbPattern {
        trigger: " connects to ",
        relation: RelationType::CommunicatesWith,
        swap: false,
    },
];

/// Preposition pattern: " <prep> " → relation type.
struct PrepPattern {
    prep: &'static str,
    relation: RelationType,
}

const PREP_PATTERNS: &[PrepPattern] = &[
    PrepPattern {
        prep: " part of ",
        relation: RelationType::PartOf,
    },
    PrepPattern {
        prep: " member of ",
        relation: RelationType::PartOf,
    },
    PrepPattern {
        prep: " component of ",
        relation: RelationType::PartOf,
    },
    PrepPattern {
        prep: " subset of ",
        relation: RelationType::PartOf,
    },
    PrepPattern {
        prep: " belongs to ",
        relation: RelationType::PartOf,
    },
    PrepPattern {
        prep: " inside ",
        relation: RelationType::LocatedAt,
    },
];

/// Extract triples from a piece of text using pattern matching.
///
/// Scans each sentence for verb and preposition patterns, extracting
/// subject/object around the matched trigger phrase.
pub fn extract_triples(content: &str) -> Vec<Triple> {
    let mut triples = Vec::new();

    for sentence in split_sentences(content) {
        // Try verb patterns first (higher confidence)
        for pat in VERB_PATTERNS {
            if let Some(pos) = find_ci(sentence, pat.trigger) {
                let subject_raw = &sentence[..pos];
                let object_raw = &sentence[pos + pat.trigger.len()..];

                let subject = clean_entity(subject_raw);
                let object = clean_entity(object_raw);

                if subject.is_empty() || object.is_empty() {
                    continue;
                }

                let (entity, target) = if pat.swap {
                    (object.clone(), subject.clone())
                } else {
                    (subject.clone(), object.clone())
                };

                let entity_type = infer_entity_type(&entity);
                let target_type = infer_entity_type(&target);

                triples.push(Triple {
                    entity_name: entity,
                    entity_type,
                    relation: pat.relation,
                    target_name: target,
                    target_type,
                    confidence: 0.8,
                });
                break; // One match per sentence for verbs
            }
        }

        // Try preposition patterns (lower confidence, only if no verb matched)
        if triples.last().is_none_or(|t| {
            !sentence
                .to_lowercase()
                .contains(&t.entity_name.to_lowercase())
        }) {
            for pat in PREP_PATTERNS {
                if let Some(pos) = find_ci(sentence, pat.prep) {
                    let subject_raw = &sentence[..pos];
                    let object_raw = &sentence[pos + pat.prep.len()..];

                    let subject = clean_entity(subject_raw);
                    let object = clean_entity(object_raw);

                    if subject.is_empty() || object.is_empty() {
                        continue;
                    }

                    let entity_type = infer_entity_type(&subject);
                    let target_type = infer_entity_type(&object);

                    triples.push(Triple {
                        entity_name: subject,
                        entity_type,
                        relation: pat.relation,
                        target_name: object,
                        target_type,
                        confidence: 0.6,
                    });
                    break;
                }
            }
        }
    }

    triples
}

/// Process a message: extract triples and persist entities + relationships into the store.
pub fn process_message_graph(
    store: &MemoryStore,
    message_id: i64,
    content: &str,
) -> Result<Vec<Triple>> {
    let triples = extract_triples(content);

    for triple in &triples {
        // Upsert both entities
        let source_id = store.upsert_entity(&triple.entity_name, &triple.entity_type)?;
        let target_id = store.upsert_entity(&triple.target_name, &triple.target_type)?;

        // Link entities to the source message
        store.link_entity_to_message(source_id, message_id, &triple.entity_name)?;
        store.link_entity_to_message(target_id, message_id, &triple.target_name)?;

        // Store the relationship edge (uses graph.rs add_relationship with upsert)
        store.add_relationship(source_id, target_id, triple.relation, triple.confidence)?;
    }

    Ok(triples)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Split text into sentences on `.`, `!`, `?`, `;`, and newlines.
/// Dots between digits (e.g., IP addresses, version numbers) are not sentence boundaries.
fn split_sentences(text: &str) -> Vec<&str> {
    let mut sentences = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();

    for (i, ch) in text.char_indices() {
        let is_boundary = match ch {
            '!' | '?' | ';' | '\n' => true,
            '.' => {
                // Not a sentence break if between digits (192.168.1.100, v1.2.3)
                let prev_digit = i > 0 && bytes[i - 1].is_ascii_digit();
                let next_digit = i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit();
                if prev_digit && next_digit {
                    false
                } else {
                    // Require a following space or end-of-text for sentence split
                    i + 1 >= bytes.len() || bytes[i + 1].is_ascii_whitespace()
                }
            }
            _ => false,
        };
        if is_boundary {
            let s = text[start..i].trim();
            if !s.is_empty() {
                sentences.push(s);
            }
            start = i + ch.len_utf8();
        }
    }
    // Trailing fragment
    let s = text[start..].trim();
    if !s.is_empty() {
        sentences.push(s);
    }
    sentences
}

/// Strip leading determiners, trailing punctuation, and excess whitespace.
fn clean_entity(raw: &str) -> String {
    let trimmed = raw
        .trim()
        .trim_end_matches(|c: char| c.is_ascii_punctuation() && c != '.' && c != '-' && c != '_');
    let trimmed = trimmed.trim();

    // Strip leading determiners / pronouns
    let lower = trimmed.to_lowercase();
    let stripped = if lower.starts_with("the ") {
        &trimmed[4..]
    } else if lower.starts_with("a ") {
        &trimmed[2..]
    } else if lower.starts_with("an ") {
        &trimmed[3..]
    } else if lower.starts_with("this ") || lower.starts_with("that ") {
        &trimmed[5..]
    } else {
        trimmed
    };

    // Take only the last meaningful noun phrase (after commas/conjunctions)
    let final_part = stripped
        .rsplit_once(", ")
        .map(|(_, r)| r)
        .unwrap_or(stripped);

    // Limit to ~5 words to avoid capturing entire clauses
    let words: Vec<&str> = final_part.split_whitespace().collect();
    if words.len() > 5 {
        words[words.len() - 5..].join(" ")
    } else {
        words.join(" ")
    }
}

/// Infer entity type from its name using simple heuristics.
fn infer_entity_type(name: &str) -> String {
    let lower = name.to_lowercase();

    // IP addresses / hostnames
    if lower.contains("192.168.") || lower.contains("10.0.") || lower.starts_with('.') {
        return "host".to_string();
    }

    // File paths
    if lower.contains('/')
        || lower.contains('\\')
        || lower.ends_with(".rs")
        || lower.ends_with(".py")
    {
        return "file".to_string();
    }

    // Known software / project patterns
    if lower.starts_with("zeus") || lower.starts_with("nova") || lower.starts_with("qubit") {
        return "project".to_string();
    }

    // Bot names
    if lower.contains("bot") || lower.starts_with('@') {
        return "agent".to_string();
    }

    // Capitalized single/two words → likely a proper noun (person/project)
    let words: Vec<&str> = name.split_whitespace().collect();
    if words.len() <= 2
        && words
            .iter()
            .all(|w| w.chars().next().is_some_and(|c| c.is_uppercase()))
    {
        return "person".to_string();
    }

    "unknown".to_string()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryStore;
    use crate::graph::Direction;
    use std::path::PathBuf;

    fn make_store() -> MemoryStore {
        MemoryStore::new(&PathBuf::from(":memory:"), true, false).unwrap()
    }

    // ---- extract_triples tests ----

    #[test]
    fn test_extract_works_on() {
        let triples = extract_triples("Miguel works on Zeus");
        assert_eq!(triples.len(), 1);
        assert_eq!(triples[0].entity_name, "Miguel");
        assert_eq!(triples[0].relation, RelationType::WorksOn);
        assert_eq!(triples[0].target_name, "Zeus");
    }

    #[test]
    fn test_extract_deployed_to() {
        let triples = extract_triples("Zeus deployed to 192.168.1.100");
        assert_eq!(triples.len(), 1);
        assert_eq!(triples[0].entity_name, "Zeus");
        assert_eq!(triples[0].relation, RelationType::LocatedAt);
        assert_eq!(triples[0].target_name, "192.168.1.100");
        assert_eq!(triples[0].target_type, "host");
    }

    #[test]
    fn test_extract_runs_on() {
        let triples = extract_triples("Ollama runs on the MacBook Pro");
        assert_eq!(triples.len(), 1);
        assert_eq!(triples[0].relation, RelationType::LocatedAt);
        assert_eq!(triples[0].entity_name, "Ollama");
        assert_eq!(triples[0].target_name, "MacBook Pro");
    }

    #[test]
    fn test_extract_created_by_swaps() {
        let triples = extract_triples("Miguel created Zeus");
        assert_eq!(triples.len(), 1);
        assert_eq!(triples[0].entity_name, "Zeus");
        assert_eq!(triples[0].relation, RelationType::CreatedBy);
        assert_eq!(triples[0].target_name, "Miguel");
    }

    #[test]
    fn test_extract_uses() {
        let triples = extract_triples("Zeus uses SQLite for storage");
        assert_eq!(triples.len(), 1);
        assert_eq!(triples[0].entity_name, "Zeus");
        assert_eq!(triples[0].relation, RelationType::Uses);
        assert_eq!(triples[0].target_name, "SQLite for storage");
    }

    #[test]
    fn test_extract_part_of() {
        let triples = extract_triples("Mnemosyne is part of Zeus");
        assert_eq!(triples.len(), 1);
        assert_eq!(triples[0].entity_name, "Mnemosyne is");
        assert_eq!(triples[0].relation, RelationType::PartOf);
        assert_eq!(triples[0].target_name, "Zeus");
    }

    #[test]
    fn test_extract_multiple_sentences() {
        let text =
            "Miguel works on Zeus. Zeus deployed to 192.168.1.100. The bot talks to Telegram";
        let triples = extract_triples(text);
        assert!(
            triples.len() >= 2,
            "expected at least 2 triples, got {}",
            triples.len()
        );
    }

    #[test]
    fn test_extract_empty_input() {
        let triples = extract_triples("");
        assert!(triples.is_empty());
    }

    #[test]
    fn test_extract_no_patterns() {
        let triples = extract_triples("Hello world, this is a test sentence");
        assert!(triples.is_empty());
    }

    #[test]
    fn test_extract_case_insensitive() {
        let triples = extract_triples("ZEUS DEPLOYED TO the server");
        assert_eq!(triples.len(), 1);
        assert_eq!(triples[0].relation, RelationType::LocatedAt);
    }

    #[test]
    fn test_extract_strips_determiners() {
        let triples = extract_triples("the agent works on the project");
        assert_eq!(triples.len(), 1);
        assert_eq!(triples[0].entity_name, "agent");
        assert_eq!(triples[0].target_name, "project");
    }

    // ---- infer_entity_type tests ----

    #[test]
    fn test_infer_type_host() {
        assert_eq!(infer_entity_type("192.168.1.100"), "host");
    }

    #[test]
    fn test_infer_type_project() {
        assert_eq!(infer_entity_type("zeus-mnemosyne"), "project");
    }

    #[test]
    fn test_infer_type_agent() {
        assert_eq!(infer_entity_type("@zeus_bot"), "agent");
    }

    #[test]
    fn test_infer_type_person() {
        assert_eq!(infer_entity_type("Miguel"), "person");
    }

    #[test]
    fn test_infer_type_file() {
        assert_eq!(infer_entity_type("src/main.rs"), "file");
    }

    // ---- process_message_graph tests ----

    #[test]
    fn test_process_message_graph_stores_entities() {
        let store = make_store();
        let msg_id = store
            .store_raw_message("test-session", "user", "Miguel works on Zeus")
            .unwrap();

        let triples = process_message_graph(&store, msg_id, "Miguel works on Zeus").unwrap();
        assert_eq!(triples.len(), 1);

        let entities = store.get_entities(10).unwrap();
        assert!(
            entities.len() >= 2,
            "expected at least 2 entities, got {}",
            entities.len()
        );

        let names: Vec<&str> = entities.iter().map(|e| e.canonical_name.as_str()).collect();
        assert!(names.contains(&"Miguel"), "missing Miguel in {:?}", names);
        assert!(names.contains(&"Zeus"), "missing Zeus in {:?}", names);
    }

    #[test]
    fn test_process_message_graph_stores_relationships() {
        let store = make_store();
        let msg_id = store
            .store_raw_message("test-session", "user", "Zeus deployed to 192.168.1.100")
            .unwrap();

        let triples =
            process_message_graph(&store, msg_id, "Zeus deployed to 192.168.1.100").unwrap();
        assert_eq!(triples.len(), 1);

        let zeus_id = store.upsert_entity("Zeus", "project").unwrap();
        let rels = store.get_relationships(zeus_id, Direction::Both).unwrap();
        assert!(!rels.is_empty(), "expected at least 1 relationship");
    }

    // ---- split_sentences tests ----

    #[test]
    fn test_split_sentences_basic() {
        let sentences = split_sentences("Hello world. How are you? Fine!");
        assert_eq!(sentences.len(), 3);
    }

    #[test]
    fn test_split_sentences_newlines() {
        let sentences = split_sentences("Line one\nLine two\nLine three");
        assert_eq!(sentences.len(), 3);
    }

    // ---- clean_entity tests ----

    #[test]
    fn test_clean_entity_strips_determiners() {
        assert_eq!(clean_entity("the server"), "server");
        assert_eq!(clean_entity("a project"), "project");
        assert_eq!(clean_entity("an agent"), "agent");
    }

    #[test]
    fn test_clean_entity_trims_punctuation() {
        assert_eq!(clean_entity("  Zeus, "), "Zeus");
    }

    #[test]
    fn test_clean_entity_limits_words() {
        let long = "this is a very long entity name with too many words here";
        let result = clean_entity(long);
        assert!(result.split_whitespace().count() <= 5);
    }
}
