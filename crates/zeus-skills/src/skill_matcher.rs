//! Semantic skill matching — intent-based skill discovery.
//!
//! Embeds skill descriptions and matches them against user intent via
//! cosine similarity. Supplements keyword-based `read_when` matching
//! with semantic understanding.
//!
//! # Architecture
//!
//! `SkillMatcher` is embedding-provider agnostic — the caller injects an
//! `EmbeddingProvider` (typically backed by Mnemosyne) so zeus-skills
//! doesn't depend on zeus-mnemosyne directly.
//!
//! # Usage
//!
//! ```ignore
//! let matcher = SkillMatcher::new(provider, 0.4);
//! matcher.index_skills(&skills).await?;
//! let matches = matcher.match_intent("design a logo").await?;
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn};
use zeus_core::Result;

/// Trait for embedding text into vectors. Implemented by the caller
/// (typically wrapping Mnemosyne's `embed_text`).
#[async_trait::async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Embed text into a float vector. Returns None if embeddings are unavailable.
    async fn embed(&self, text: &str) -> Result<Option<Vec<f32>>>;
}

/// A skill match result with relevance score.
#[derive(Debug, Clone)]
pub struct SkillMatch {
    /// Skill name
    pub name: String,
    /// Skill description
    pub description: String,
    /// Cosine similarity score (0.0–1.0)
    pub score: f32,
}

/// Cached embedding for a skill description.
#[derive(Debug, Clone)]
struct SkillEmbedding {
    name: String,
    description: String,
    embedding: Vec<f32>,
}

/// Semantic skill matcher. Caches skill description embeddings and
/// matches user intent via cosine similarity.
pub struct SkillMatcher {
    /// Embedding provider (injected, typically Mnemosyne-backed)
    provider: Arc<dyn EmbeddingProvider>,
    /// Cached skill embeddings (name → embedding)
    cache: RwLock<Vec<SkillEmbedding>>,
    /// Minimum similarity threshold for a match (0.0–1.0)
    threshold: f32,
}

impl SkillMatcher {
    /// Create a new skill matcher.
    ///
    /// `threshold` is the minimum cosine similarity (0.0–1.0) for a skill
    /// to be considered a match. Recommended: 0.35–0.50.
    pub fn new(provider: Arc<dyn EmbeddingProvider>, threshold: f32) -> Self {
        Self {
            provider,
            cache: RwLock::new(Vec::new()),
            threshold: threshold.clamp(0.0, 1.0),
        }
    }

    /// Index a set of skills by embedding their descriptions.
    /// Call this on startup or when skills are added/removed.
    ///
    /// Skills with empty descriptions are skipped. Skills whose embeddings
    /// fail are logged and skipped (non-fatal).
    pub async fn index_skills(
        &self,
        skills: &HashMap<String, crate::Skill>,
    ) -> Result<usize> {
        let mut indexed = 0;
        let mut entries = Vec::new();

        for (name, skill) in skills {
            // Build a rich text representation for embedding:
            // name + description + tool names give the best semantic signal
            let embed_text = build_skill_text(skill);
            if embed_text.is_empty() {
                debug!(skill = %name, "Skipping skill with no description");
                continue;
            }

            match self.provider.embed(&embed_text).await {
                Ok(Some(embedding)) => {
                    entries.push(SkillEmbedding {
                        name: name.clone(),
                        description: skill.description.clone(),
                        embedding,
                    });
                    indexed += 1;
                }
                Ok(None) => {
                    warn!(skill = %name, "Embedding provider returned None — embeddings may be disabled");
                    break; // If provider returns None, all subsequent calls will too
                }
                Err(e) => {
                    warn!(skill = %name, error = %e, "Failed to embed skill description");
                }
            }
        }

        let mut cache = self.cache.write().await;
        *cache = entries;
        debug!(count = indexed, "Skill embeddings indexed");
        Ok(indexed)
    }

    /// Match user intent against indexed skills.
    /// Returns top-k matches above the similarity threshold, sorted by score descending.
    pub async fn match_intent(
        &self,
        intent: &str,
        top_k: usize,
    ) -> Result<Vec<SkillMatch>> {
        // Embed the user intent
        let intent_embedding = match self.provider.embed(intent).await? {
            Some(e) => e,
            None => return Ok(Vec::new()),
        };

        let cache = self.cache.read().await;
        if cache.is_empty() {
            return Ok(Vec::new());
        }

        // Score all skills by cosine similarity
        let mut matches: Vec<SkillMatch> = cache
            .iter()
            .filter_map(|entry| {
                let score = cosine_similarity(&intent_embedding, &entry.embedding);
                if score >= self.threshold {
                    Some(SkillMatch {
                        name: entry.name.clone(),
                        description: entry.description.clone(),
                        score,
                    })
                } else {
                    None
                }
            })
            .collect();

        // Sort by score descending
        matches.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        // Return top-k
        matches.truncate(top_k);

        debug!(
            intent = %intent,
            matches = matches.len(),
            top_score = matches.first().map(|m| m.score).unwrap_or(0.0),
            "Skill intent matching complete"
        );

        Ok(matches)
    }

    /// Check if any skills are indexed.
    pub async fn is_indexed(&self) -> bool {
        !self.cache.read().await.is_empty()
    }

    /// Number of indexed skills.
    pub async fn indexed_count(&self) -> usize {
        self.cache.read().await.len()
    }

    /// Re-index a single skill (add or update).
    pub async fn index_skill(&self, skill: &crate::Skill) -> Result<bool> {
        let embed_text = build_skill_text(skill);
        if embed_text.is_empty() {
            return Ok(false);
        }

        let embedding = match self.provider.embed(&embed_text).await? {
            Some(e) => e,
            None => return Ok(false),
        };

        let mut cache = self.cache.write().await;
        // Remove existing entry if present
        cache.retain(|e| e.name != skill.name);
        cache.push(SkillEmbedding {
            name: skill.name.clone(),
            description: skill.description.clone(),
            embedding,
        });
        Ok(true)
    }

    /// Remove a skill from the index.
    pub async fn remove_skill(&self, name: &str) {
        let mut cache = self.cache.write().await;
        cache.retain(|e| e.name != name);
    }
}

/// Build a rich text representation of a skill for embedding.
/// Combines name, description, and tool names for better semantic signal.
fn build_skill_text(skill: &crate::Skill) -> String {
    let mut parts = Vec::new();

    if !skill.name.is_empty() {
        parts.push(skill.name.clone());
    }
    if !skill.description.is_empty() {
        parts.push(skill.description.clone());
    }
    // Tool names add semantic signal (e.g. "canvas_draw" hints at visual skills)
    for tool in &skill.tools {
        if !tool.name.is_empty() {
            parts.push(tool.name.clone());
        }
        if !tool.description.is_empty() {
            parts.push(tool.description.clone());
        }
    }

    parts.join(". ")
}

/// Cosine similarity between two vectors (0.0–1.0).
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0_f32;
    let mut norm_a = 0.0_f32;
    let mut norm_b = 0.0_f32;

    for i in 0..a.len() {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }

    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 {
        0.0
    } else {
        dot / denom
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// Mock embedding provider for tests — simple word-overlap heuristic
    /// that produces deterministic pseudo-embeddings.
    struct MockEmbedder;

    #[async_trait::async_trait]
    impl EmbeddingProvider for MockEmbedder {
        async fn embed(&self, text: &str) -> Result<Option<Vec<f32>>> {
            // Generate a deterministic 8-dim vector based on keyword presence.
            // Each dimension corresponds to a semantic domain.
            let lower = text.to_lowercase();
            Ok(Some(vec![
                if lower.contains("design") || lower.contains("logo") || lower.contains("visual") { 1.0 } else { 0.0 },
                if lower.contains("code") || lower.contains("rust") || lower.contains("program") { 1.0 } else { 0.0 },
                if lower.contains("deploy") || lower.contains("server") || lower.contains("infra") { 1.0 } else { 0.0 },
                if lower.contains("write") || lower.contains("blog") || lower.contains("content") { 1.0 } else { 0.0 },
                if lower.contains("trade") || lower.contains("swap") || lower.contains("defi") { 1.0 } else { 0.0 },
                if lower.contains("test") || lower.contains("review") || lower.contains("audit") { 1.0 } else { 0.0 },
                if lower.contains("chat") || lower.contains("message") || lower.contains("send") { 1.0 } else { 0.0 },
                if lower.contains("search") || lower.contains("find") || lower.contains("lookup") { 1.0 } else { 0.0 },
            ]))
        }
    }

    /// Mock provider that returns None (embeddings disabled)
    struct DisabledEmbedder;

    #[async_trait::async_trait]
    impl EmbeddingProvider for DisabledEmbedder {
        async fn embed(&self, _text: &str) -> Result<Option<Vec<f32>>> {
            Ok(None)
        }
    }

    fn make_skill(name: &str, description: &str, tool_names: &[&str]) -> crate::Skill {
        crate::Skill {
            name: name.to_string(),
            description: description.to_string(),
            version: "0.1.0".to_string(),
            author: None,
            system_prompt: String::new(),
            tools: tool_names
                .iter()
                .map(|t| crate::SkillTool {
                    name: t.to_string(),
                    description: String::new(),
                    input_schema: serde_json::json!({}),
                    implementation: crate::ToolImplementation::Native,
                })
                .collect(),
            permissions: vec![],
            path: PathBuf::new(),
            raw_content: String::new(),
            invocation: Default::default(),
            command_dispatch: None,
            metadata: None,
            frontmatter: HashMap::new(),
            read_when: vec![],
        }
    }

    #[tokio::test]
    async fn test_index_and_match() {
        let provider = Arc::new(MockEmbedder);
        let matcher = SkillMatcher::new(provider, 0.3);

        let mut skills = HashMap::new();
        skills.insert(
            "canvas".to_string(),
            make_skill("canvas", "Design logos and visual artwork", &["canvas_draw"]),
        );
        skills.insert(
            "code_review".to_string(),
            make_skill("code_review", "Review and audit Rust code", &["review_file"]),
        );
        skills.insert(
            "deployer".to_string(),
            make_skill("deployer", "Deploy to server infrastructure", &["deploy_app"]),
        );

        let count = matcher.index_skills(&skills).await.unwrap();
        assert_eq!(count, 3);
        assert!(matcher.is_indexed().await);

        // "design a logo" should match canvas skill
        let matches = matcher.match_intent("design a logo", 3).await.unwrap();
        assert!(!matches.is_empty());
        assert_eq!(matches[0].name, "canvas");
        assert!(matches[0].score > 0.5);
    }

    #[tokio::test]
    async fn test_threshold_filtering() {
        let provider = Arc::new(MockEmbedder);
        let matcher = SkillMatcher::new(provider, 0.9); // Very high threshold

        let mut skills = HashMap::new();
        skills.insert(
            "canvas".to_string(),
            make_skill("canvas", "Design logos and visual artwork", &[]),
        );

        matcher.index_skills(&skills).await.unwrap();

        // "deploy a server" has zero overlap with "design logos" → filtered out
        let matches = matcher.match_intent("deploy a server", 3).await.unwrap();
        assert!(matches.is_empty());
    }

    #[tokio::test]
    async fn test_top_k_limit() {
        let provider = Arc::new(MockEmbedder);
        let matcher = SkillMatcher::new(provider, 0.0); // Accept everything

        let mut skills = HashMap::new();
        for i in 0..10 {
            skills.insert(
                format!("skill_{}", i),
                make_skill(&format!("skill_{}", i), "design visual logo artwork", &[]),
            );
        }

        matcher.index_skills(&skills).await.unwrap();

        let matches = matcher.match_intent("design a logo", 3).await.unwrap();
        assert_eq!(matches.len(), 3); // Limited to top-k
    }

    #[tokio::test]
    async fn test_disabled_embeddings() {
        let provider = Arc::new(DisabledEmbedder);
        let matcher = SkillMatcher::new(provider, 0.3);

        let mut skills = HashMap::new();
        skills.insert(
            "canvas".to_string(),
            make_skill("canvas", "Design logos", &[]),
        );

        // Index returns 0 when embeddings are disabled
        let count = matcher.index_skills(&skills).await.unwrap();
        assert_eq!(count, 0);
        assert!(!matcher.is_indexed().await);

        // Match returns empty
        let matches = matcher.match_intent("anything", 3).await.unwrap();
        assert!(matches.is_empty());
    }

    #[tokio::test]
    async fn test_single_skill_index_and_remove() {
        let provider = Arc::new(MockEmbedder);
        let matcher = SkillMatcher::new(provider, 0.3);

        let skill = make_skill("canvas", "Design logos and visual artwork", &[]);
        let added = matcher.index_skill(&skill).await.unwrap();
        assert!(added);
        assert_eq!(matcher.indexed_count().await, 1);

        matcher.remove_skill("canvas").await;
        assert_eq!(matcher.indexed_count().await, 0);
    }

    #[tokio::test]
    async fn test_cosine_similarity_identical() {
        let a = vec![1.0, 0.0, 1.0, 0.0];
        let b = vec![1.0, 0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[tokio::test]
    async fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 0.0).abs() < 1e-6);
    }

    #[tokio::test]
    async fn test_cosine_similarity_empty() {
        let sim = cosine_similarity(&[], &[]);
        assert_eq!(sim, 0.0);
    }

    #[tokio::test]
    async fn test_build_skill_text() {
        let skill = make_skill("canvas", "Design visual art", &["draw", "paint"]);
        let text = build_skill_text(&skill);
        assert!(text.contains("canvas"));
        assert!(text.contains("Design visual art"));
        assert!(text.contains("draw"));
        assert!(text.contains("paint"));
    }

    #[tokio::test]
    async fn test_match_ranking_order() {
        let provider = Arc::new(MockEmbedder);
        let matcher = SkillMatcher::new(provider, 0.0);

        let mut skills = HashMap::new();
        skills.insert(
            "canvas".to_string(),
            make_skill("canvas", "Design logos and visual artwork", &[]),
        );
        skills.insert(
            "deployer".to_string(),
            make_skill("deployer", "Deploy to server infrastructure", &[]),
        );

        matcher.index_skills(&skills).await.unwrap();

        // "design a logo" should rank canvas higher than deployer
        let matches = matcher.match_intent("design a logo", 5).await.unwrap();
        if matches.len() >= 2 {
            assert!(matches[0].score >= matches[1].score, "Results should be sorted by score descending");
        }
    }
}
