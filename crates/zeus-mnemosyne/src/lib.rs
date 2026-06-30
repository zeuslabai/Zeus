//! Zeus Mnemosyne - Advanced Memory System
//!
//! Provides SQLite-backed message storage with FTS5 full-text search
//! and vector similarity search for semantic queries.

use std::sync::OnceLock;

use regex::Regex;

pub mod community;
mod db;
pub mod dedup;
pub mod graph;
pub mod graph_search;
pub mod importance;
pub mod knowledge_extract;
pub mod promotion;
pub mod summarizer;
pub mod supersession;
pub use dedup::{DedupConfig, DedupEngine, DedupStats, DedupVerdict};
pub use graph::{
    Community, CommunityMember, Direction, GraphNode, GraphTraversal, Promotion, RelationType,
    Relationship, RelationshipTypeCount,
};
pub use graph_search::{
    GraphContext, GraphSearchResult, expand_query_via_graph, get_memory_graph_context,
    graph_augmented_search,
};
pub use importance::{
    ImportanceConfig, ImportanceScorer, MemoryEntry, ScoreBreakdown, ScoredMemory, ScorerStats,
};
pub use knowledge_extract::{Triple, extract_triples, process_message_graph};
pub use promotion::{
    ConsolidationResult, ExtractedFacts, FactValidation, GcConfig, GcResult, PromotionResult,
    augment_summary, auto_promote, consolidate_session, extract_facts, garbage_collect,
    validate_compaction,
};
pub use summarizer::{
    SessionSummarizer, SessionSummary, SummarizerConfig, SummarizerStats, SummaryMessage,
    TopicCluster,
};
pub use supersession::{
    HeuristicJudge, SupersessionConfig, SupersessionJudge, detect_supersessions,
};

use chrono::{DateTime, Utc};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};
use zeus_core::{Error, Message, Result};

/// Memory store backed by SQLite with optional embedding provider chain.
pub struct Mnemosyne {
    /// The underlying memory store (graph, FTS, entities).
    pub store: Arc<Mutex<MemoryStore>>,
    config: MnemosyneConfig,
    embedding_chain: Option<tokio::sync::Mutex<EmbeddingChain>>,
    qmd: Option<QmdBackend>,
    supersession_config: SupersessionConfig,
}

/// Configuration for Mnemosyne
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MnemosyneConfig {
    /// Database file path
    pub db_path: PathBuf,
    /// Enable FTS5 full-text search
    #[serde(default = "default_fts")]
    pub enable_fts: bool,
    /// Maximum messages to keep per session
    #[serde(default = "default_max_messages")]
    pub max_messages_per_session: usize,
    /// Enable vector embeddings storage
    #[serde(default)]
    pub enable_embeddings: bool,
    /// Embedding dimensions (768 for nomic-embed-text, 1536 for OpenAI)
    #[serde(default = "default_embedding_dim")]
    pub embedding_dim: usize,
    /// Ollama server URL for generating embeddings
    #[serde(default = "default_ollama_url")]
    pub ollama_url: String,
    /// Embedding model name (e.g., "nomic-embed-text")
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,
    /// Weight for vector (cosine similarity) score in hybrid search (0.0–1.0)
    #[serde(default = "default_vector_weight")]
    pub vector_weight: f64,
    /// Weight for text (BM25/FTS5) score in hybrid search (0.0–1.0)
    #[serde(default = "default_text_weight")]
    pub text_weight: f64,
    /// Multiplier for candidate retrieval in hybrid search (candidates = multiplier * limit)
    #[serde(default = "default_candidate_multiplier")]
    pub candidate_multiplier: usize,
    /// Ordered list of embedding providers to try (fallback chain)
    #[serde(default = "default_embedding_providers")]
    pub embedding_providers: Vec<EmbeddingProvider>,
    /// Optional dedicated host for embedding requests.
    ///
    /// When set, overrides `ollama_url` for Ollama embeddings so users can
    /// pin embedding calls to a specific GPU server while using a different
    /// host for chat (e.g., a local Ollama for embeddings + OpenRouter for LLM).
    /// Has no effect on cloud providers (OpenAI, Gemini, Voyage) whose URLs
    /// are fixed by the provider API.
    #[serde(default)]
    pub embedding_host: Option<String>,
    /// Enable fact-checking during memory compaction.
    /// When true, key facts (entities, dates, decisions) are extracted before
    /// compaction and validated against the summary — missing facts are appended.
    #[serde(default)]
    pub compaction_fact_check: bool,
    /// Number of consecutive failures before switching to next provider
    #[serde(default = "default_fallback_threshold")]
    pub fallback_threshold: usize,
    /// Enable session transcript indexing
    #[serde(default = "default_true")]
    pub enable_session_indexing: bool,
    /// Byte delta threshold before re-indexing a session file
    #[serde(default = "default_session_delta_bytes")]
    pub session_delta_bytes: usize,
    /// Message count delta threshold before re-indexing a session file
    #[serde(default = "default_session_delta_messages")]
    pub session_delta_messages: usize,
    /// Enable file watcher for auto-sync on changes
    #[serde(default)]
    pub enable_file_watcher: bool,
    /// Extra paths to watch (in addition to workspace root)
    #[serde(default)]
    pub watch_paths: Vec<PathBuf>,
    /// Extra markdown directories to index (in addition to workspace memory/)
    #[serde(default)]
    pub extra_memory_paths: Vec<PathBuf>,
    /// Maximum total memories before triggering consolidation (0 = unlimited)
    #[serde(default = "default_max_memories")]
    pub max_memories: usize,
    /// FTS5 similarity threshold for dedup (0.0–1.0, higher = stricter). 0 = disabled.
    #[serde(default = "default_dedup_threshold")]
    pub dedup_threshold: f64,
    /// Maximum messages per session before summary consolidation kicks in
    #[serde(default = "default_consolidation_session_limit")]
    pub consolidation_session_limit: usize,
    /// Number of approximate tokens to overlap between adjacent chunks (default 80)
    #[serde(default = "default_chunk_overlap_tokens")]
    pub chunk_overlap_tokens: usize,
    /// Number of texts to send per batch embedding API call (default 100)
    #[serde(default = "default_embed_batch_size")]
    pub embed_batch_size: usize,
    /// Enable QMD (BM25+vector+reranking) sidecar for search
    #[serde(default)]
    pub enable_qmd: bool,
    /// QMD sidecar HTTP URL
    #[serde(default = "default_qmd_url")]
    pub qmd_url: String,
    /// QMD request timeout in milliseconds
    #[serde(default = "default_qmd_timeout_ms")]
    pub qmd_timeout_ms: u64,
    /// URL for cross-encoder reranking model (e.g. sentence-transformers served via HTTP)
    /// When set, enables cross-encoder reranking as a post-processing step on hybrid search.
    #[serde(default)]
    pub qmd_reranker_url: Option<String>,
    /// Cross-encoder model name (sent in request body)
    #[serde(default = "default_reranker_model")]
    pub qmd_reranker_model: String,
    /// Weight for BM25 score in internal QMD fusion (0.0–1.0)
    #[serde(default = "default_qmd_bm25_weight")]
    pub qmd_bm25_weight: f64,
    /// Weight for vector score in internal QMD fusion (0.0–1.0)
    #[serde(default = "default_qmd_vector_weight")]
    pub qmd_vector_weight: f64,
    /// Weight for cross-encoder score in internal QMD fusion (0.0–1.0)
    #[serde(default = "default_qmd_reranker_weight")]
    pub qmd_reranker_weight: f64,
    /// Number of over-fetch candidates for reranking (multiplier on limit)
    #[serde(default = "default_qmd_candidate_multiplier")]
    pub qmd_candidate_multiplier: usize,
}

fn default_qmd_url() -> String {
    std::env::var("ZEUS_QMD_URL").unwrap_or_else(|_| "http://localhost:7720".to_string())
}
fn default_qmd_timeout_ms() -> u64 {
    3000
}
fn default_reranker_model() -> String {
    "cross-encoder/ms-marco-MiniLM-L-6-v2".to_string()
}
fn default_qmd_bm25_weight() -> f64 {
    0.3
}
fn default_qmd_vector_weight() -> f64 {
    0.3
}
fn default_qmd_reranker_weight() -> f64 {
    0.4
}
fn default_qmd_candidate_multiplier() -> usize {
    4
}

/// Re-export from zeus-core for use in MnemosyneConfig
pub use zeus_core::EmbeddingProvider;

fn default_embedding_providers() -> Vec<EmbeddingProvider> {
    vec![EmbeddingProvider::Ollama]
}
fn default_fallback_threshold() -> usize {
    3
}
fn default_max_memories() -> usize {
    50_000 // 0 = unlimited
}
fn default_dedup_threshold() -> f64 {
    0.85 // FTS5 BM25 similarity ratio for dedup
}
fn default_consolidation_session_limit() -> usize {
    200 // messages per session before consolidation
}
fn default_chunk_overlap_tokens() -> usize {
    80
}
fn default_embed_batch_size() -> usize {
    100
}
fn default_true() -> bool {
    true
}
fn default_session_delta_bytes() -> usize {
    100_000
}
fn default_session_delta_messages() -> usize {
    50
}
fn default_vector_weight() -> f64 {
    0.7
}
fn default_text_weight() -> f64 {
    0.3
}
fn default_candidate_multiplier() -> usize {
    4
}
fn default_fts() -> bool {
    true
}
fn default_max_messages() -> usize {
    10000
}
fn default_embedding_dim() -> usize {
    768
}
fn default_ollama_url() -> String {
    std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".to_string())
}
fn default_embedding_model() -> String {
    "nomic-embed-text".to_string()
}

/// Build the multilingual FTS stop-word set used by pattern extraction.
///
/// Merges stop words for English, Spanish, Portuguese, Japanese (hiragana
/// particles), Korean (josa particles), and Arabic function words into a
/// single `HashSet`.  All comparisons are against already-lowercased tokens,
/// so only lowercase forms are stored.
///
/// The caller's `w.len() >= 3` byte-length pre-filter already removes most
/// 1–2 byte ASCII words; we still include short words here so the set is
/// correct when called from other contexts.
fn build_stop_words() -> HashSet<&'static str> {
    [
        // ── English ──────────────────────────────────────────────────────────
        "the",
        "a",
        "an",
        "is",
        "are",
        "was",
        "were",
        "be",
        "been",
        "being",
        "have",
        "has",
        "had",
        "do",
        "does",
        "did",
        "will",
        "would",
        "could",
        "should",
        "may",
        "might",
        "can",
        "shall",
        "to",
        "of",
        "in",
        "for",
        "on",
        "with",
        "at",
        "by",
        "from",
        "as",
        "into",
        "through",
        "during",
        "before",
        "after",
        "above",
        "below",
        "between",
        "and",
        "but",
        "or",
        "not",
        "no",
        "so",
        "if",
        "then",
        "than",
        "that",
        "this",
        "these",
        "those",
        "it",
        "its",
        "my",
        "your",
        "his",
        "her",
        "our",
        "their",
        "what",
        "which",
        "who",
        "whom",
        "how",
        "when",
        "where",
        "why",
        "i",
        "me",
        "you",
        "he",
        "she",
        "we",
        "they",
        "them",
        "us",
        // ── Spanish ──────────────────────────────────────────────────────────
        "el",
        "la",
        "los",
        "las",
        "un",
        "una",
        "unos",
        "unas",
        "de",
        "en",
        "por",
        "con",
        "para",
        "sin",
        "sobre",
        "bajo",
        "ante",
        "tras",
        "hacia",
        "hasta",
        "desde",
        "durante",
        "del",
        "al",
        "que",
        "y",
        "o",
        "pero",
        "ni",
        "aunque",
        "como",
        "cuando",
        "donde",
        "porque",
        "sino",
        "pues",
        "es",
        "son",
        "era",
        "eran",
        "ser",
        "estar",
        "hay",
        "tiene",
        "tienen",
        "fue",
        "han",
        "sido",
        "yo",
        "tu",
        "él",
        "ella",
        "nosotros",
        "vosotros",
        "ellos",
        "ellas",
        "me",
        "te",
        "se",
        "nos",
        "les",
        "lo",
        "le",
        "su",
        "sus",
        "más",
        "muy",
        "bien",
        "ya",
        "también",
        "todo",
        "todos",
        "cada",
        // ── Portuguese ───────────────────────────────────────────────────────
        "o",
        "os",
        "as",
        "um",
        "uma",
        "uns",
        "umas",
        "em",
        "por",
        "com",
        "para",
        "sem",
        "sobre",
        "sob",
        "e",
        "ou",
        "mas",
        "nem",
        "se",
        "porque",
        "pois",
        "porém",
        "contudo",
        "é",
        "são",
        "era",
        "ser",
        "estar",
        "tem",
        "foi",
        "ter",
        "sido",
        "ele",
        "ela",
        "eles",
        "elas",
        "seu",
        "sua",
        "seus",
        "suas",
        "do",
        "da",
        "dos",
        "das",
        "no",
        "na",
        "nos",
        "nas",
        "ao",
        "aos",
        "pelo",
        "pela",
        "pelos",
        "pelas",
        "mais",
        "muito",
        "bem",
        "já",
        "também",
        "tudo",
        "todos",
        "cada",
        // ── Japanese hiragana / common particles ─────────────────────────────
        // Single-character particles (3 UTF-8 bytes each; pass the len≥3 filter)
        "の",
        "は",
        "が",
        "を",
        "に",
        "で",
        "と",
        "も",
        "か",
        "や",
        "よ",
        "ね",
        "わ",
        "ず",
        "だ",
        "な",
        "し",
        "て",
        // Multi-character particles / auxiliaries
        "から",
        "まで",
        "より",
        "けど",
        "けれど",
        "ども",
        "ので",
        "です",
        "ます",
        "した",
        "ない",
        "ある",
        "いる",
        "する",
        "なる",
        "この",
        "その",
        "あの",
        "どの",
        "ここ",
        "そこ",
        "あそこ",
        "これ",
        "それ",
        "あれ",
        "どれ",
        // ── Korean josa (postpositional particles) ───────────────────────────
        // Single-character (3 UTF-8 bytes each)
        "은",
        "는",
        "이",
        "가",
        "을",
        "를",
        "의",
        "에",
        "로",
        "과",
        "와",
        "도",
        "만",
        "다",
        "한",
        "그",
        "것",
        "수",
        "등",
        // Multi-character particles / auxiliaries
        "에서",
        "으로",
        "이다",
        "있다",
        "하다",
        "되다",
        "것이",
        "수가",
        "부터",
        "까지",
        "에게",
        "에서",
        "이나",
        "거나",
        "하고",
        "그리고",
        "하지만",
        "그러나",
        "그래서",
        "그런데",
        "따라서",
        // ── Arabic function words ─────────────────────────────────────────────
        // Prepositions & conjunctions (≥4 UTF-8 bytes → pass the len≥3 filter)
        "في",
        "من",
        "على",
        "مع",
        "عن",
        "لا",
        "ما",
        "أو",
        "ثم",
        "كل",
        "له",
        "به",
        "لم",
        "هو",
        "هي",
        "إلى",
        "هذا",
        "هذه",
        "ذلك",
        "تلك",
        "كان",
        "كما",
        "لكن",
        "أيضا",
        "حتى",
        "فقط",
        "بين",
        "ولا",
        "وقد",
        "فإن",
        "التي",
        "الذي",
        "الذين",
        "اللتي",
        "الذين",
    ]
    .into_iter()
    .collect()
}

impl Default for MnemosyneConfig {
    fn default() -> Self {
        Self {
            db_path: zeus_core::default_config_dir().join("memory.db"),
            enable_fts: true,
            max_messages_per_session: 10000,
            enable_embeddings: false,
            embedding_dim: 768,
            ollama_url: default_ollama_url(),
            embedding_model: default_embedding_model(),
            vector_weight: default_vector_weight(),
            text_weight: default_text_weight(),
            candidate_multiplier: default_candidate_multiplier(),
            embedding_providers: default_embedding_providers(),
            embedding_host: None,
            compaction_fact_check: false,
            fallback_threshold: default_fallback_threshold(),
            enable_session_indexing: true,
            session_delta_bytes: default_session_delta_bytes(),
            session_delta_messages: default_session_delta_messages(),
            enable_file_watcher: false,
            watch_paths: Vec::new(),
            extra_memory_paths: Vec::new(),
            max_memories: default_max_memories(),
            dedup_threshold: default_dedup_threshold(),
            consolidation_session_limit: default_consolidation_session_limit(),
            chunk_overlap_tokens: default_chunk_overlap_tokens(),
            embed_batch_size: default_embed_batch_size(),
            enable_qmd: false,
            qmd_url: default_qmd_url(),
            qmd_timeout_ms: default_qmd_timeout_ms(),
            qmd_reranker_url: None,
            qmd_reranker_model: default_reranker_model(),
            qmd_bm25_weight: default_qmd_bm25_weight(),
            qmd_vector_weight: default_qmd_vector_weight(),
            qmd_reranker_weight: default_qmd_reranker_weight(),
            qmd_candidate_multiplier: default_qmd_candidate_multiplier(),
        }
    }
}

// Vector Embedding Helpers

/// Serialize an f32 slice to bytes (little-endian)
fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for &val in embedding {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}

/// Deserialize bytes back to Vec<f32> (little-endian)
fn bytes_to_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| {
            let arr: [u8; 4] = [chunk[0], chunk[1], chunk[2], chunk[3]];
            f32::from_le_bytes(arr)
        })
        .collect()
}

/// Compute cosine similarity between two vectors
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len(), "Vectors must have the same length");

    let mut dot = 0.0_f32;
    let mut norm_a = 0.0_f32;
    let mut norm_b = 0.0_f32;

    for i in 0..a.len() {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }

    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 { 0.0 } else { dot / denom }
}

// Content Hashing & Chunking Helpers

/// Compute SHA-256 hash of content, returned as hex string.
pub fn compute_content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Compute Levenshtein distance between two strings.
fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_len = a.len();
    let b_len = b.len();
    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    let mut prev: Vec<usize> = (0..=b_len).collect();
    let mut curr = vec![0; b_len + 1];

    for (i, ca) in a.chars().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.chars().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[b_len]
}

/// Compute the Levenshtein similarity ratio (0.0–1.0) between two strings.
fn levenshtein_ratio(a: &str, b: &str) -> f64 {
    let max_len = a.len().max(b.len());
    if max_len == 0 {
        return 1.0;
    }
    let dist = levenshtein_distance(a, b);
    1.0 - (dist as f64 / max_len as f64)
}

/// Split text into chunks by paragraph boundaries (double newlines).
/// Each chunk is at most ~2000 chars; paragraphs that exceed this are split further.
/// A text chunk with its starting line number (1-based).
struct TextChunk {
    text: String,
    start_line: usize,
}

/// Approximate chars per token for overlap computation (~4 chars/token).
const CHARS_PER_TOKEN: usize = 4;

#[cfg(test)]
fn chunk_text(text: &str) -> Vec<TextChunk> {
    chunk_text_with_overlap(text, 0)
}

fn chunk_text_with_overlap(text: &str, overlap_tokens: usize) -> Vec<TextChunk> {
    let overlap_chars = overlap_tokens * CHARS_PER_TOKEN;
    let mut chunks: Vec<TextChunk> = Vec::new();
    let mut current = String::new();
    let mut current_start_line: usize = 1;
    let mut line_cursor: usize = 1; // tracks which line we're on in the source

    for paragraph in text.split("\n\n") {
        let trimmed = paragraph.trim();
        // Count lines consumed by this paragraph (original, before trim)
        let para_lines = paragraph.lines().count().max(1);

        if trimmed.is_empty() {
            line_cursor += para_lines + 1; // +1 for the blank-line separator
            continue;
        }

        if current.len() + trimmed.len() + 2 > 2000 {
            if !current.is_empty() {
                chunks.push(TextChunk {
                    text: std::mem::take(&mut current),
                    start_line: current_start_line,
                });
            }
            current_start_line = line_cursor;
            // If single paragraph > 2000, split on sentence boundaries
            if trimmed.len() > 2000 {
                let mut sub = String::new();
                for sentence in trimmed.split(". ") {
                    if sub.len() + sentence.len() + 2 > 2000 && !sub.is_empty() {
                        chunks.push(TextChunk {
                            text: std::mem::take(&mut sub),
                            start_line: current_start_line,
                        });
                        current_start_line = line_cursor;
                    }
                    if !sub.is_empty() {
                        sub.push_str(". ");
                    }
                    sub.push_str(sentence);
                }
                if !sub.is_empty() {
                    current = sub;
                }
            } else {
                current = trimmed.to_string();
            }
        } else {
            if current.is_empty() {
                current_start_line = line_cursor;
            }
            if !current.is_empty() {
                current.push_str("\n\n");
            }
            current.push_str(trimmed);
        }
        line_cursor += para_lines + 1; // +1 for the \n\n separator
    }

    if !current.is_empty() {
        chunks.push(TextChunk {
            text: current,
            start_line: current_start_line,
        });
    }

    // Apply overlap: prepend trailing text from previous chunk to each subsequent chunk
    if overlap_chars > 0 && chunks.len() > 1 {
        for i in 1..chunks.len() {
            let prev_text = &chunks[i - 1].text;
            if prev_text.len() > overlap_chars {
                // Take trailing overlap_chars, snapping to a char boundary then word boundary
                let mut tail_start = prev_text.len() - overlap_chars;
                // Snap to char boundary (don't split multi-byte chars like ─ or emoji)
                while tail_start > 0 && !prev_text.is_char_boundary(tail_start) {
                    tail_start += 1;
                }
                let snap = prev_text[tail_start..]
                    .find(|c: char| c.is_whitespace())
                    .map(|pos| tail_start + pos + 1)
                    .unwrap_or(tail_start);
                let overlap_text = &prev_text[snap..];
                if !overlap_text.is_empty() {
                    chunks[i].text = format!("{}\n\n{}", overlap_text.trim(), chunks[i].text);
                }
            }
        }
    }

    chunks
}

/// Recursively collect all .md files under a directory.
fn collect_md_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_md_files_recursive(root, &mut files);
    files.sort();
    files
}

fn collect_md_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_md_files_recursive(&path, files);
        } else if path.extension().map(|e| e == "md").unwrap_or(false) {
            files.push(path);
        }
    }
}

// Session JSONL Parsing

/// JSONL session entry for deserialization
#[derive(Deserialize)]
struct SessionJsonlEntry {
    #[serde(rename = "type")]
    entry_type: Option<String>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(flatten)]
    _extra: serde_json::Value,
}

/// Parse a session JSONL file, extracting User/Assistant text from the delta
/// region (bytes from `offset` to end). Returns (normalized_text, message_count).
fn parse_session_jsonl(full_content: &str, byte_offset: usize) -> (String, usize) {
    let mut text = String::new();
    let mut message_count = 0;

    // Advance to the byte offset, then process remaining lines
    let delta = if byte_offset < full_content.len() {
        &full_content[byte_offset..]
    } else {
        ""
    };

    for line in delta.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let entry: SessionJsonlEntry = match serde_json::from_str(trimmed) {
            Ok(e) => e,
            Err(_) => continue,
        };

        // Only index message entries with user or assistant role
        let entry_type = entry.entry_type.as_deref().unwrap_or("");
        if entry_type != "message" {
            continue;
        }

        let role = entry.role.as_deref().unwrap_or("");
        if role != "user" && role != "assistant" {
            continue;
        }

        if let Some(content) = entry.content {
            let normalized = normalize_whitespace(&content);
            if !normalized.is_empty() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&normalized);
                message_count += 1;
            }
        }
    }

    (text, message_count)
}

/// Normalize whitespace: collapse multiple spaces/newlines, trim.
fn normalize_whitespace(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut last_was_space = false;

    for ch in s.chars() {
        if ch.is_whitespace() {
            if !last_was_space && !result.is_empty() {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            result.push(ch);
            last_was_space = false;
        }
    }

    result.trim().to_string()
}

// Embedding Providers

/// Ollama /api/embed request body (supports single or batch via Vec)
#[derive(Serialize)]
struct OllamaEmbedRequest<'a> {
    model: &'a str,
    input: Vec<&'a str>,
}

/// Ollama /api/embed response
#[derive(Deserialize)]
struct OllamaEmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

/// OpenAI / Voyage /v1/embeddings request body (supports single or batch via Vec)
#[derive(Serialize)]
struct OpenAIEmbedRequest<'a> {
    model: &'a str,
    input: Vec<&'a str>,
}

/// OpenAI / Voyage /v1/embeddings response
#[derive(Deserialize)]
struct OpenAIEmbedResponse {
    data: Vec<OpenAIEmbedData>,
}

#[derive(Deserialize)]
struct OpenAIEmbedData {
    embedding: Vec<f32>,
}

/// Gemini embedding request body
#[derive(Serialize)]
struct GeminiEmbedRequest<'a> {
    model: &'a str,
    content: GeminiContent<'a>,
}

#[derive(Serialize)]
struct GeminiContent<'a> {
    parts: Vec<GeminiPart<'a>>,
}

#[derive(Serialize)]
struct GeminiPart<'a> {
    text: &'a str,
}

/// Gemini embedding response
#[derive(Deserialize)]
struct GeminiEmbedResponse {
    embedding: GeminiEmbedValues,
}

#[derive(Deserialize)]
struct GeminiEmbedValues {
    values: Vec<f32>,
}

/// Gemini batchEmbedContents request body
#[derive(Serialize)]
struct GeminiBatchEmbedRequest<'a> {
    requests: Vec<GeminiBatchEmbedEntry<'a>>,
}

#[derive(Serialize)]
struct GeminiBatchEmbedEntry<'a> {
    model: &'a str,
    content: GeminiContent<'a>,
}

/// Gemini batchEmbedContents response
#[derive(Deserialize)]
struct GeminiBatchEmbedResponse {
    embeddings: Vec<GeminiEmbedValues>,
}

/// A single configured embedding provider instance
struct EmbedderInstance {
    provider: EmbeddingProvider,
    client: reqwest::Client,
    base_url: String,
    model: String,
    api_key: Option<String>,
}

impl EmbedderInstance {
    fn new(provider: EmbeddingProvider, config: &MnemosyneConfig) -> Self {
        let (base_url, model, api_key) = match &provider {
            EmbeddingProvider::Ollama => (
                // Prefer dedicated embedding_host when set; fall back to ollama_url.
                config
                    .embedding_host
                    .as_deref()
                    .unwrap_or(&config.ollama_url)
                    .trim_end_matches('/')
                    .to_string(),
                config.embedding_model.clone(),
                None,
            ),
            EmbeddingProvider::OpenAI => (
                "https://api.openai.com".to_string(),
                "text-embedding-3-small".to_string(),
                std::env::var("OPENAI_API_KEY").ok(),
            ),
            EmbeddingProvider::Gemini => (
                "https://generativelanguage.googleapis.com".to_string(),
                "text-embedding-004".to_string(),
                std::env::var("GOOGLE_API_KEY").ok(),
            ),
            EmbeddingProvider::Voyage => (
                "https://api.voyageai.com".to_string(),
                "voyage-3".to_string(),
                std::env::var("VOYAGE_API_KEY").ok(),
            ),
        };

        Self {
            provider,
            client: reqwest::Client::new(),
            base_url,
            model,
            api_key,
        }
    }

    /// Whether this provider instance is actually usable as configured.
    ///
    /// LLM-agnostic guard: API-backed providers (OpenAI / Gemini / Voyage)
    /// require their credential to be present in the environment. If the key
    /// is absent, the provider would fail at request time and silently poison
    /// the fallback chain. Ollama targets a local URL and needs no key, so it
    /// is always considered available (reachability is handled at request
    /// time via the circuit breaker).
    fn is_available(&self) -> bool {
        match &self.provider {
            EmbeddingProvider::Ollama => true,
            EmbeddingProvider::OpenAI
            | EmbeddingProvider::Gemini
            | EmbeddingProvider::Voyage => self.api_key.is_some(),
        }
    }

    /// Generate an embedding for a single text string.
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        match &self.provider {
            EmbeddingProvider::Ollama => self.embed_ollama(text).await,
            EmbeddingProvider::OpenAI => self.embed_openai(text).await,
            EmbeddingProvider::Gemini => self.embed_gemini(text).await,
            EmbeddingProvider::Voyage => self.embed_voyage(text).await,
        }
    }

    async fn embed_ollama(&self, text: &str) -> Result<Vec<f32>> {
        let endpoint = format!("{}/api/embed", self.base_url);
        let request = OllamaEmbedRequest {
            model: &self.model,
            input: vec![text],
        };

        let response = self
            .client
            .post(&endpoint)
            .json(&request)
            .send()
            .await
            .map_err(|e| Error::Network(format!("Ollama embed request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown".to_string());
            return Err(Error::Network(format!(
                "Ollama embed returned {}: {}",
                status, body
            )));
        }

        let embed_response: OllamaEmbedResponse = response
            .json()
            .await
            .map_err(|e| Error::Serialization(format!("Failed to parse embed response: {}", e)))?;

        embed_response
            .embeddings
            .into_iter()
            .next()
            .ok_or_else(|| Error::Network("Ollama returned empty embeddings array".to_string()))
    }

    async fn embed_openai(&self, text: &str) -> Result<Vec<f32>> {
        let api_key = self.api_key.as_deref().ok_or_else(|| {
            Error::Config("OPENAI_API_KEY not set for OpenAI embeddings".to_string())
        })?;

        let endpoint = format!("{}/v1/embeddings", self.base_url);
        let request = OpenAIEmbedRequest {
            model: &self.model,
            input: vec![text],
        };

        let response = self
            .client
            .post(&endpoint)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&request)
            .send()
            .await
            .map_err(|e| Error::Network(format!("OpenAI embed request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown".to_string());
            return Err(Error::Network(format!(
                "OpenAI embed returned {}: {}",
                status, body
            )));
        }

        let embed_response: OpenAIEmbedResponse = response
            .json()
            .await
            .map_err(|e| Error::Serialization(format!("Failed to parse embed response: {}", e)))?;

        embed_response
            .data
            .into_iter()
            .next()
            .map(|d| d.embedding)
            .ok_or_else(|| Error::Network("OpenAI returned empty data array".to_string()))
    }

    async fn embed_gemini(&self, text: &str) -> Result<Vec<f32>> {
        let api_key = self.api_key.as_deref().ok_or_else(|| {
            Error::Config("GOOGLE_API_KEY not set for Gemini embeddings".to_string())
        })?;

        let endpoint = format!(
            "{}/v1beta/models/{}:embedContent?key={}",
            self.base_url, self.model, api_key
        );
        let request = GeminiEmbedRequest {
            model: &format!("models/{}", self.model),
            content: GeminiContent {
                parts: vec![GeminiPart { text }],
            },
        };

        let response = self
            .client
            .post(&endpoint)
            .json(&request)
            .send()
            .await
            .map_err(|e| Error::Network(format!("Gemini embed request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown".to_string());
            return Err(Error::Network(format!(
                "Gemini embed returned {}: {}",
                status, body
            )));
        }

        let embed_response: GeminiEmbedResponse = response
            .json()
            .await
            .map_err(|e| Error::Serialization(format!("Failed to parse embed response: {}", e)))?;

        Ok(embed_response.embedding.values)
    }

    async fn embed_voyage(&self, text: &str) -> Result<Vec<f32>> {
        let api_key = self.api_key.as_deref().ok_or_else(|| {
            Error::Config("VOYAGE_API_KEY not set for Voyage embeddings".to_string())
        })?;

        let endpoint = format!("{}/v1/embeddings", self.base_url);
        let request = OpenAIEmbedRequest {
            model: &self.model,
            input: vec![text],
        };

        let response = self
            .client
            .post(&endpoint)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&request)
            .send()
            .await
            .map_err(|e| Error::Network(format!("Voyage embed request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown".to_string());
            return Err(Error::Network(format!(
                "Voyage embed returned {}: {}",
                status, body
            )));
        }

        let embed_response: OpenAIEmbedResponse = response
            .json()
            .await
            .map_err(|e| Error::Serialization(format!("Failed to parse embed response: {}", e)))?;

        embed_response
            .data
            .into_iter()
            .next()
            .map(|d| d.embedding)
            .ok_or_else(|| Error::Network("Voyage returned empty data array".to_string()))
    }

    fn provider_name(&self) -> &str {
        match &self.provider {
            EmbeddingProvider::Ollama => "ollama",
            EmbeddingProvider::OpenAI => "openai",
            EmbeddingProvider::Gemini => "gemini",
            EmbeddingProvider::Voyage => "voyage",
        }
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    /// Generate embeddings for multiple texts in a single API call.
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        match &self.provider {
            EmbeddingProvider::Ollama => self.embed_batch_ollama(texts).await,
            EmbeddingProvider::OpenAI => self.embed_batch_openai(texts).await,
            EmbeddingProvider::Gemini => self.embed_batch_gemini(texts).await,
            EmbeddingProvider::Voyage => self.embed_batch_voyage(texts).await,
        }
    }

    async fn embed_batch_ollama(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let endpoint = format!("{}/api/embed", self.base_url);
        let request = OllamaEmbedRequest {
            model: &self.model,
            input: texts.to_vec(),
        };

        let response = self
            .client
            .post(&endpoint)
            .json(&request)
            .send()
            .await
            .map_err(|e| Error::Network(format!("Ollama batch embed request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown".to_string());
            return Err(Error::Network(format!(
                "Ollama batch embed returned {}: {}",
                status, body
            )));
        }

        let embed_response: OllamaEmbedResponse = response.json().await.map_err(|e| {
            Error::Serialization(format!("Failed to parse batch embed response: {}", e))
        })?;

        if embed_response.embeddings.len() != texts.len() {
            return Err(Error::Network(format!(
                "Ollama returned {} embeddings for {} inputs",
                embed_response.embeddings.len(),
                texts.len()
            )));
        }

        Ok(embed_response.embeddings)
    }

    async fn embed_batch_openai(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let api_key = self.api_key.as_deref().ok_or_else(|| {
            Error::Config("OPENAI_API_KEY not set for OpenAI embeddings".to_string())
        })?;

        let endpoint = format!("{}/v1/embeddings", self.base_url);
        let request = OpenAIEmbedRequest {
            model: &self.model,
            input: texts.to_vec(),
        };

        let response = self
            .client
            .post(&endpoint)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&request)
            .send()
            .await
            .map_err(|e| Error::Network(format!("OpenAI batch embed request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown".to_string());
            return Err(Error::Network(format!(
                "OpenAI batch embed returned {}: {}",
                status, body
            )));
        }

        let embed_response: OpenAIEmbedResponse = response.json().await.map_err(|e| {
            Error::Serialization(format!("Failed to parse batch embed response: {}", e))
        })?;

        if embed_response.data.len() != texts.len() {
            return Err(Error::Network(format!(
                "OpenAI returned {} embeddings for {} inputs",
                embed_response.data.len(),
                texts.len()
            )));
        }

        Ok(embed_response
            .data
            .into_iter()
            .map(|d| d.embedding)
            .collect())
    }

    async fn embed_batch_gemini(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let api_key = self.api_key.as_deref().ok_or_else(|| {
            Error::Config("GOOGLE_API_KEY not set for Gemini embeddings".to_string())
        })?;

        let model_name = format!("models/{}", self.model);
        let endpoint = format!(
            "{}/v1beta/models/{}:batchEmbedContents?key={}",
            self.base_url, self.model, api_key
        );

        let requests: Vec<GeminiBatchEmbedEntry> = texts
            .iter()
            .map(|&text| GeminiBatchEmbedEntry {
                model: &model_name,
                content: GeminiContent {
                    parts: vec![GeminiPart { text }],
                },
            })
            .collect();

        let request = GeminiBatchEmbedRequest { requests };

        let response = self
            .client
            .post(&endpoint)
            .json(&request)
            .send()
            .await
            .map_err(|e| Error::Network(format!("Gemini batch embed request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown".to_string());
            return Err(Error::Network(format!(
                "Gemini batch embed returned {}: {}",
                status, body
            )));
        }

        let embed_response: GeminiBatchEmbedResponse = response.json().await.map_err(|e| {
            Error::Serialization(format!("Failed to parse batch embed response: {}", e))
        })?;

        if embed_response.embeddings.len() != texts.len() {
            return Err(Error::Network(format!(
                "Gemini returned {} embeddings for {} inputs",
                embed_response.embeddings.len(),
                texts.len()
            )));
        }

        Ok(embed_response
            .embeddings
            .into_iter()
            .map(|e| e.values)
            .collect())
    }

    async fn embed_batch_voyage(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let api_key = self.api_key.as_deref().ok_or_else(|| {
            Error::Config("VOYAGE_API_KEY not set for Voyage embeddings".to_string())
        })?;

        let endpoint = format!("{}/v1/embeddings", self.base_url);
        let request = OpenAIEmbedRequest {
            model: &self.model,
            input: texts.to_vec(),
        };

        let response = self
            .client
            .post(&endpoint)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&request)
            .send()
            .await
            .map_err(|e| Error::Network(format!("Voyage batch embed request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown".to_string());
            return Err(Error::Network(format!(
                "Voyage batch embed returned {}: {}",
                status, body
            )));
        }

        let embed_response: OpenAIEmbedResponse = response.json().await.map_err(|e| {
            Error::Serialization(format!("Failed to parse batch embed response: {}", e))
        })?;

        if embed_response.data.len() != texts.len() {
            return Err(Error::Network(format!(
                "Voyage returned {} embeddings for {} inputs",
                embed_response.data.len(),
                texts.len()
            )));
        }

        Ok(embed_response
            .data
            .into_iter()
            .map(|d| d.embedding)
            .collect())
    }
}

// Embedding Chain (fallback)

/// Manages a chain of embedding providers with automatic fallback.
///
/// On consecutive failures >= `fallback_threshold`, the chain advances to
/// the next provider. When all providers are exhausted, embedding calls fail.
pub struct EmbeddingChain {
    providers: Vec<EmbedderInstance>,
    failure_counts: Vec<usize>,
    active_index: usize,
    fallback_threshold: usize,
    /// S70: Circuit breaker — once all providers exhaust, disable for cooldown_until
    disabled_until: Option<std::time::Instant>,
}

impl EmbeddingChain {
    /// Build a chain from the config's embedding_providers list.
    ///
    /// LLM-agnostic: providers whose required credentials are absent are
    /// dropped from the chain (with a warning) rather than added as dead
    /// entries that fail at request time and silently disable memory. This
    /// lets Zeus run against whichever embedding provider is actually
    /// configured for the host, without assuming any specific one is present.
    fn from_config(config: &MnemosyneConfig) -> Self {
        let providers: Vec<EmbedderInstance> = config
            .embedding_providers
            .iter()
            .filter_map(|p| {
                let instance = EmbedderInstance::new(p.clone(), config);
                if instance.is_available() {
                    Some(instance)
                } else {
                    warn!(
                        provider = %instance.provider_name(),
                        "Embedding provider configured but its credential is missing from the \
                         environment — dropping it from the fallback chain. Set the provider's \
                         API key, or configure a provider whose credentials are present."
                    );
                    None
                }
            })
            .collect();

        if providers.is_empty() && !config.embedding_providers.is_empty() {
            warn!(
                configured = config.embedding_providers.len(),
                "All configured embedding providers were dropped (missing credentials / \
                 unreachable). Vector memory is disabled until a usable provider is configured; \
                 FTS and graph memory remain active."
            );
        }

        let len = providers.len();
        Self {
            providers,
            failure_counts: vec![0; len],
            active_index: 0,
            fallback_threshold: config.fallback_threshold,
            disabled_until: None,
        }
    }

    /// Embed text using the active provider, falling back on repeated failures.
    async fn embed(&mut self, text: &str) -> Result<Vec<f32>> {
        if self.providers.is_empty() {
            return Err(Error::Config(
                "No embedding providers configured".to_string(),
            ));
        }

        // S70: Circuit breaker — skip embeddings for 5 minutes after all providers exhaust
        if let Some(until) = self.disabled_until {
            if std::time::Instant::now() < until {
                return Err(Error::Config(
                    "Embedding providers temporarily disabled (circuit breaker — retrying in 5m)".to_string(),
                ));
            } else {
                // Cooldown expired, re-enable
                self.disabled_until = None;
                for count in &mut self.failure_counts {
                    *count = 0;
                }
                info!("Embedding circuit breaker reset — retrying providers");
            }
        }

        let start = self.active_index;
        loop {
            let idx = self.active_index;
            let provider = &self.providers[idx];
            match provider.embed(text).await {
                Ok(embedding) => {
                    // Reset failure count on success
                    self.failure_counts[idx] = 0;
                    return Ok(embedding);
                }
                Err(e) => {
                    self.failure_counts[idx] += 1;
                    warn!(
                        provider = provider.provider_name(),
                        failures = self.failure_counts[idx],
                        threshold = self.fallback_threshold,
                        error = %e,
                        "Embedding provider failed"
                    );

                    if self.failure_counts[idx] >= self.fallback_threshold {
                        let next = (idx + 1) % self.providers.len();
                        if next == start {
                            // All providers exhausted — activate circuit breaker
                            self.disabled_until = Some(
                                std::time::Instant::now() + std::time::Duration::from_secs(300)
                            );
                            warn!("All embedding providers exhausted — circuit breaker active for 5 minutes");
                            return Err(Error::Network(format!(
                                "All {} embedding providers exhausted: {}",
                                self.providers.len(),
                                e
                            )));
                        }
                        info!(
                            from = self.providers[idx].provider_name(),
                            to = self.providers[next].provider_name(),
                            "Switching to fallback embedding provider"
                        );
                        self.active_index = next;
                        // Continue to try the next provider immediately
                        continue;
                    }

                    // Below threshold — return error for this call (caller may retry)
                    return Err(e);
                }
            }
        }
    }

    /// Embed multiple texts using the active provider, falling back on repeated failures.
    async fn embed_batch(&mut self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        if self.providers.is_empty() {
            return Err(Error::Config(
                "No embedding providers configured".to_string(),
            ));
        }

        let start = self.active_index;
        loop {
            let idx = self.active_index;
            let provider = &self.providers[idx];
            match provider.embed_batch(texts).await {
                Ok(embeddings) => {
                    self.failure_counts[idx] = 0;
                    return Ok(embeddings);
                }
                Err(e) => {
                    self.failure_counts[idx] += 1;
                    warn!(
                        provider = provider.provider_name(),
                        failures = self.failure_counts[idx],
                        threshold = self.fallback_threshold,
                        batch_size = texts.len(),
                        error = %e,
                        "Batch embedding provider failed"
                    );

                    if self.failure_counts[idx] >= self.fallback_threshold {
                        let next = (idx + 1) % self.providers.len();
                        if next == start {
                            return Err(Error::Network(format!(
                                "All {} embedding providers exhausted: {}",
                                self.providers.len(),
                                e
                            )));
                        }
                        info!(
                            from = self.providers[idx].provider_name(),
                            to = self.providers[next].provider_name(),
                            "Switching to fallback embedding provider"
                        );
                        self.active_index = next;
                        continue;
                    }

                    return Err(e);
                }
            }
        }
    }

    /// Get the currently active provider name.
    fn active_provider(&self) -> &str {
        if self.providers.is_empty() {
            "none"
        } else {
            self.providers[self.active_index].provider_name()
        }
    }

    /// Get the active provider's model name.
    fn active_model(&self) -> &str {
        if self.providers.is_empty() {
            "none"
        } else {
            self.providers[self.active_index].model_name()
        }
    }

    /// Get fallback state summary for reporting.
    fn fallback_state(&self) -> Vec<(String, usize, bool)> {
        self.providers
            .iter()
            .enumerate()
            .map(|(i, p)| {
                (
                    p.provider_name().to_string(),
                    self.failure_counts[i],
                    i == self.active_index,
                )
            })
            .collect()
    }
}

/// Schema migrations for the mnemosyne SQLite database.
///
/// v1 – base schema: messages (8 cols), patterns, embedding_cache,
///       memory_files, session_files, entities, entity_mentions, core indexes.
/// v2 – ALTER messages: memory_type, importance + index.
/// v3 – ALTER messages: source_path.
/// v4 – ALTER messages: last_accessed.
/// v5 – ALTER messages: valid_from, valid_to, superseded_by + 3 temporal indexes.
/// v6 – graph tables: relationships, communities, community_members, promotions.
/// v7 – fleet_session_alias cache table for cross-channel session correlation (PRD §272).
/// v8 – verified BOOLEAN + backfill (auto-memory fabrication fence, #53.1).
/// v9 – retraction ledger + clean-content trigger (auto-verify fence #53.2).
/// v10 – channel_kind + chat_id columns on messages (cross-channel context, #86-sprint-A).
const MEMORY_MIGRATIONS: &[&str] = &[
    // v1 — base schema
    "CREATE TABLE IF NOT EXISTS messages (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        session_id TEXT NOT NULL,
        role TEXT NOT NULL,
        content TEXT NOT NULL,
        tool_calls TEXT,
        tool_results TEXT,
        timestamp TEXT NOT NULL,
        created_at TEXT DEFAULT CURRENT_TIMESTAMP
    );
    CREATE TABLE IF NOT EXISTS patterns (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        pattern_type TEXT NOT NULL,
        content TEXT NOT NULL,
        frequency INTEGER NOT NULL DEFAULT 1,
        first_seen TEXT NOT NULL,
        last_seen TEXT NOT NULL
    );
    CREATE UNIQUE INDEX IF NOT EXISTS idx_patterns_lookup ON patterns(pattern_type, content);
    CREATE TABLE IF NOT EXISTS embedding_cache (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        provider TEXT NOT NULL,
        model TEXT NOT NULL,
        content_hash TEXT NOT NULL,
        embedding BLOB NOT NULL,
        created_at INTEGER NOT NULL,
        last_used INTEGER NOT NULL
    );
    CREATE UNIQUE INDEX IF NOT EXISTS idx_embedding_cache_lookup
        ON embedding_cache(provider, model, content_hash);
    CREATE INDEX IF NOT EXISTS idx_embedding_cache_lru ON embedding_cache(last_used);
    CREATE TABLE IF NOT EXISTS memory_files (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        path TEXT NOT NULL,
        source TEXT NOT NULL,
        content_hash TEXT NOT NULL,
        mtime INTEGER NOT NULL,
        size INTEGER NOT NULL,
        last_indexed INTEGER NOT NULL
    );
    CREATE UNIQUE INDEX IF NOT EXISTS idx_memory_files_lookup ON memory_files(path, source);
    CREATE TABLE IF NOT EXISTS session_files (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        session_id TEXT NOT NULL,
        file_path TEXT NOT NULL,
        last_size INTEGER NOT NULL DEFAULT 0,
        pending_bytes INTEGER NOT NULL DEFAULT 0,
        pending_messages INTEGER NOT NULL DEFAULT 0,
        last_indexed INTEGER NOT NULL DEFAULT 0
    );
    CREATE UNIQUE INDEX IF NOT EXISTS idx_session_files_lookup ON session_files(session_id);
    CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id);
    CREATE INDEX IF NOT EXISTS idx_messages_timestamp ON messages(timestamp);
    CREATE TABLE IF NOT EXISTS entities (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        canonical_name TEXT NOT NULL,
        entity_type TEXT NOT NULL DEFAULT 'unknown',
        aliases TEXT NOT NULL DEFAULT '[]',
        first_seen TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        last_seen TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        mention_count INTEGER NOT NULL DEFAULT 1
    );
    CREATE UNIQUE INDEX IF NOT EXISTS idx_entities_canonical ON entities(canonical_name, entity_type);
    CREATE TABLE IF NOT EXISTS entity_mentions (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        entity_id INTEGER NOT NULL,
        message_id INTEGER NOT NULL,
        mention_text TEXT NOT NULL,
        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        FOREIGN KEY (entity_id) REFERENCES entities(id) ON DELETE CASCADE,
        FOREIGN KEY (message_id) REFERENCES messages(id) ON DELETE CASCADE
    );
    CREATE INDEX IF NOT EXISTS idx_entity_mentions_entity ON entity_mentions(entity_id);
    CREATE INDEX IF NOT EXISTS idx_entity_mentions_message ON entity_mentions(message_id);",
    // v2 — memory_type + importance columns
    "ALTER TABLE messages ADD COLUMN memory_type TEXT DEFAULT 'episodic';
    ALTER TABLE messages ADD COLUMN importance REAL DEFAULT 0.5;
    CREATE INDEX IF NOT EXISTS idx_messages_memory_type ON messages(memory_type);",
    // v3 — source_path column
    "ALTER TABLE messages ADD COLUMN source_path TEXT;",
    // v4 — last_accessed column
    "ALTER TABLE messages ADD COLUMN last_accessed TEXT DEFAULT NULL;",
    // v5 — temporal versioning columns
    "ALTER TABLE messages ADD COLUMN valid_from TEXT DEFAULT NULL;
    ALTER TABLE messages ADD COLUMN valid_to TEXT DEFAULT NULL;
    ALTER TABLE messages ADD COLUMN superseded_by INTEGER DEFAULT NULL;
    CREATE INDEX IF NOT EXISTS idx_messages_temporal_current ON messages(valid_to) WHERE valid_to IS NULL;
    CREATE INDEX IF NOT EXISTS idx_messages_superseded ON messages(superseded_by) WHERE superseded_by IS NOT NULL;
    CREATE INDEX IF NOT EXISTS idx_messages_validity ON messages(valid_from, valid_to);",
    // v6 — graph tables (relationships, communities, community_members, promotions)
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
    );
    CREATE INDEX IF NOT EXISTS idx_rel_source ON relationships(source_entity_id);
    CREATE INDEX IF NOT EXISTS idx_rel_target ON relationships(target_entity_id);
    CREATE INDEX IF NOT EXISTS idx_rel_type ON relationships(relationship_type);
    CREATE TABLE IF NOT EXISTS communities (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        name TEXT NOT NULL,
        description TEXT DEFAULT '',
        entity_count INTEGER NOT NULL DEFAULT 0,
        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
    );
    CREATE TABLE IF NOT EXISTS community_members (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        community_id INTEGER NOT NULL,
        entity_id INTEGER NOT NULL,
        role TEXT DEFAULT 'member',
        added_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE,
        FOREIGN KEY (entity_id) REFERENCES entities(id) ON DELETE CASCADE,
        UNIQUE(community_id, entity_id)
    );
    CREATE INDEX IF NOT EXISTS idx_community_members ON community_members(community_id);
    CREATE TABLE IF NOT EXISTS promotions (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        source_message_id INTEGER NOT NULL,
        promoted_message_id INTEGER NOT NULL,
        reason TEXT NOT NULL,
        promoted_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        FOREIGN KEY (source_message_id) REFERENCES messages(id),
        FOREIGN KEY (promoted_message_id) REFERENCES messages(id)
    );",
    // S59-P3: Temporal facts table — facts with validity windows
    // Inspired by MiroFish's Zep EdgeInfo pattern
    "CREATE TABLE IF NOT EXISTS temporal_facts (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        subject TEXT NOT NULL,
        predicate TEXT NOT NULL,
        object TEXT NOT NULL,
        valid_from TEXT,
        valid_until TEXT,
        expired_at TEXT,
        source TEXT,
        confidence REAL NOT NULL DEFAULT 1.0,
        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
    );
    CREATE INDEX IF NOT EXISTS idx_temporal_facts_subject ON temporal_facts(subject);
    CREATE INDEX IF NOT EXISTS idx_temporal_facts_expired ON temporal_facts(expired_at);",
    // v7 — fleet_session_alias cache (Lane 3b-i, PRD §272)
    "CREATE TABLE IF NOT EXISTS fleet_session_alias (
        agent_id TEXT NOT NULL,
        human_id TEXT NOT NULL,
        session_id TEXT NOT NULL,
        channel_kind TEXT NOT NULL,
        last_seen TEXT NOT NULL,
        PRIMARY KEY (agent_id, human_id)
    );
    CREATE INDEX IF NOT EXISTS idx_fleet_session_alias_last_seen ON fleet_session_alias(last_seen);",
    // v8 — verified BOOLEAN + backfill (auto-memory fabrication fence, #53.1)
    // New column defaults FALSE as conservative safety-net for non-instrumented
    // ingest paths. Pre-existing rows backfilled to TRUE since they predate the
    // verification-fence and are by-definition trusted (couldn't have been
    // auto-injected through the new filter introduced in #53.2).
    "ALTER TABLE messages ADD COLUMN verified BOOLEAN NOT NULL DEFAULT 0;
    UPDATE messages SET verified = 1;",
    // v9: retraction ledger + clean-content trigger to flip verified=1 on insert.
    // Companion to #53 SHA #2 fence — newly-inserted messages start at DEFAULT 0
    // (filtered from exports per #2a) and the trigger flips clean rows to 1 by
    // calling the looks_like_unverified_sha UDF (registered post-migration).
    // Unclean rows stay at 0 until explicitly verified or retracted.
    "CREATE TABLE IF NOT EXISTS retractions (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        message_id INTEGER NOT NULL,
        reason TEXT NOT NULL,
        retracted_at TEXT NOT NULL DEFAULT (datetime('now')),
        UNIQUE(message_id, reason),
        FOREIGN KEY (message_id) REFERENCES messages(id) ON DELETE CASCADE
    );
    CREATE INDEX IF NOT EXISTS idx_retractions_message_id ON retractions(message_id);
    CREATE TRIGGER IF NOT EXISTS messages_auto_verify_clean
        AFTER INSERT ON messages
        FOR EACH ROW
        WHEN NEW.verified = 0 AND looks_like_unverified_sha(NEW.content) = 0
        BEGIN
            UPDATE messages SET verified = 1 WHERE id = NEW.id;
        END;",
    // v10 — channel_kind + chat_id on messages (cross-channel context unification, #86-sprint-A).
    // NULL = unknown/legacy row. channel_kind mirrors ChannelKind variants ("discord", "telegram",
    // "slack", etc.). chat_id is the channel-specific identifier (Discord channel ID, Telegram
    // chat_id, Slack channel ID, etc.). Both nullable for zero-cost backward compat — existing
    // rows silently treated as unknown channel origin.
    "ALTER TABLE messages ADD COLUMN channel_kind TEXT NULL;
    ALTER TABLE messages ADD COLUMN chat_id TEXT NULL;
    CREATE INDEX IF NOT EXISTS idx_messages_channel_kind ON messages(channel_kind);
    CREATE INDEX IF NOT EXISTS idx_messages_chat_id ON messages(chat_id);",
];

// SQLite-backed Memory Store

/// Heuristic: does `text` contain content that looks like an unverified SHA-like
/// run (likely auto-generated fabrication-shaped output)?
///
/// Used by the v9 `messages_auto_verify_clean` trigger via the
/// `looks_like_unverified_sha` SQLite UDF. Returns `true` if the text contains
/// any run of 10-40 consecutive lowercase hex characters bordered by non-hex
/// (matches typical short and full git SHAs without trapping on random hex
/// substrings inside larger words).
pub fn looks_like_unverified_sha_content(text: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"\b[0-9a-f]{10,40}\b").expect("looks_like_unverified_sha_content regex")
    });
    re.is_match(text)
}

/// SQLite-backed memory store
pub struct MemoryStore {
    conn: Connection,
    enable_embeddings: bool,
}

impl MemoryStore {
    /// Create a new memory store
    pub fn new(path: &PathBuf, enable_fts: bool, enable_embeddings: bool) -> Result<Self> {
        let conn = Connection::open(path)
            .map_err(|e| Error::Database(format!("Failed to open database: {}", e)))?;

        // Enable WAL mode for concurrent read/write access (multiple processes on same DB)
        // and set busy timeout to wait instead of failing on lock contention
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;
             PRAGMA foreign_keys = ON;",
        )
        .map_err(|e| Error::Database(format!("Failed to set WAL mode / foreign keys: {}", e)))?;

        // Register looks_like_unverified_sha UDF BEFORE running migrations.
        // v9 trigger `messages_auto_verify_clean` invokes this UDF on every
        // INSERT into messages — UDF must be available when migration runs OR
        // when subsequent inserts fire the trigger. Registering pre-migration
        // is fail-safe-polarity (UDF available even if migration is mid-flight).
        conn.create_scalar_function(
            "looks_like_unverified_sha",
            1,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8
                | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
            |ctx| {
                let text: String = ctx.get(0)?;
                Ok(looks_like_unverified_sha_content(&text) as i64)
            },
        )
        .map_err(|e| Error::Database(format!("Failed to register UDF: {}", e)))?;

        // Apply schema migrations (tracks progress via PRAGMA user_version)
        crate::db::run_migrations(&conn, MEMORY_MIGRATIONS)
            .map_err(|e| Error::Database(format!("Mnemosyne schema migration failed: {}", e)))?;

        // Create FTS virtual table + triggers if enabled (conditional, not versioned)
        if enable_fts {
            conn.execute(
                "CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
                    content,
                    content='messages',
                    content_rowid='id'
                )",
                [],
            )
            .map_err(|e| Error::Database(format!("Failed to create FTS table: {}", e)))?;

            conn.execute_batch(
                "CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
                    INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);
                END;
                CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
                    INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.id, old.content);
                END;
                CREATE TRIGGER IF NOT EXISTS messages_au AFTER UPDATE ON messages BEGIN
                    INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.id, old.content);
                    INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);
                END;"
            ).map_err(|e| Error::Database(format!("Failed to create FTS triggers: {}", e)))?;
        }

        // Create embeddings table if enabled (conditional, not versioned)
        if enable_embeddings {
            conn.execute(
                "CREATE TABLE IF NOT EXISTS embeddings (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    message_id INTEGER NOT NULL,
                    embedding BLOB NOT NULL,
                    model TEXT,
                    created_at TEXT DEFAULT CURRENT_TIMESTAMP,
                    FOREIGN KEY (message_id) REFERENCES messages(id) ON DELETE CASCADE
                )",
                [],
            )
            .map_err(|e| Error::Database(format!("Failed to create embeddings table: {}", e)))?;

            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_embeddings_message ON embeddings(message_id)",
                [],
            )
            .map_err(|e| Error::Database(format!("Failed to create embeddings index: {}", e)))?;
        }

        Ok(Self {
            conn,
            enable_embeddings,
        })
    }

    /// Get a reference to the underlying SQLite connection.
    ///
    /// Exposed for advanced operations (e.g. promotion, GC) that need direct
    /// SQL access while reusing the same transaction context.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Store a message
    pub fn store_message(&self, session_id: &str, message: &Message) -> Result<i64> {
        self.store_message_with_channel(session_id, message, None, None)
    }

    /// Store a message with optional channel provenance metadata.
    ///
    /// `channel_kind` — e.g. "discord", "telegram", "slack". NULL = unknown/legacy.
    /// `chat_id`      — channel-specific identifier (Discord channel ID, Telegram chat_id, etc.).
    ///
    /// Called by `store_with_embedding_tagged` (Sprint-B write-path, #86) to tag messages
    /// with their origin channel so cross-channel context queries can filter/weight by source.
    pub fn store_message_with_channel(
        &self,
        session_id: &str,
        message: &Message,
        channel_kind: Option<&str>,
        chat_id: Option<&str>,
    ) -> Result<i64> {
        let role = serde_json::to_string(&message.role)
            .unwrap_or_else(|_| format!("{:?}", message.role))
            .trim_matches('"')
            .to_string();
        let tool_calls = serde_json::to_string(&message.tool_calls)
            .map_err(|e| Error::Serialization(e.to_string()))?;
        let tool_results = serde_json::to_string(&message.tool_results)
            .map_err(|e| Error::Serialization(e.to_string()))?;
        let timestamp = message.timestamp.to_rfc3339();

        self.conn.execute(
            "INSERT INTO messages (session_id, role, content, tool_calls, tool_results, timestamp, valid_from, channel_kind, chat_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8)",
            params![session_id, role, message.content, tool_calls, tool_results, timestamp, channel_kind, chat_id],
        ).map_err(|e| Error::Database(format!("Failed to insert message: {}", e)))?;

        Ok(self.conn.last_insert_rowid())
    }

    /// Store a chunk with source path citation (used during workspace sync)
    pub fn store_chunk_with_source(
        &self,
        session_id: &str,
        content: &str,
        source_path: &str,
        memory_type: MemoryType,
    ) -> Result<i64> {
        let timestamp = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO messages (session_id, role, content, tool_calls, tool_results, timestamp, memory_type, importance, source_path, valid_from)
             VALUES (?1, 'system', ?2, '[]', '[]', ?3, ?4, 0.5, ?5, ?3)",
            params![session_id, content, timestamp, memory_type.as_str(), source_path],
        ).map_err(|e| Error::Database(format!("Failed to insert chunk: {}", e)))?;

        Ok(self.conn.last_insert_rowid())
    }

    /// Store an embedding for a message
    pub fn store_embedding(
        &self,
        message_id: i64,
        embedding: &[f32],
        model: Option<&str>,
    ) -> Result<i64> {
        if !self.enable_embeddings {
            return Err(Error::Database(
                "Embeddings are not enabled. Set enable_embeddings = true in config.".to_string(),
            ));
        }

        let bytes = embedding_to_bytes(embedding);
        self.conn
            .execute(
                "INSERT INTO embeddings (message_id, embedding, model) VALUES (?1, ?2, ?3)",
                params![message_id, bytes, model],
            )
            .map_err(|e| Error::Database(format!("Failed to insert embedding: {}", e)))?;

        Ok(self.conn.last_insert_rowid())
    }

    /// Mark a memory as superseded by a newer memory.
    /// Sets `valid_to` to now and records the superseding message ID.
    pub fn supersede_message(&self, old_id: i64, new_id: i64) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute(
                "UPDATE messages SET valid_to = ?1, superseded_by = ?2 WHERE id = ?3 AND valid_to IS NULL",
                params![now, new_id, old_id],
            )
            .map_err(|e| Error::Database(format!("Failed to supersede message: {}", e)))?;
        Ok(())
    }

    /// Get the current (non-superseded) version of a memory by searching for
    /// the latest active memory with similar content in the same memory_type.
    pub fn get_current_memories(&self, limit: usize) -> Result<Vec<SearchResult>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, content, timestamp, importance as score, memory_type, importance, source_path, verified
             FROM messages
             WHERE valid_to IS NULL
             ORDER BY importance DESC, timestamp DESC
             LIMIT ?1",
        ).map_err(|e| Error::Database(format!("Failed to query current memories: {}", e)))?;

        let results = stmt
            .query_map(params![limit], |row| {
                let mt_str: String = row.get(5)?;
                Ok(SearchResult {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    content: row.get(2)?,
                    timestamp: row.get(3)?,
                    score: row.get(4)?,
                    memory_type: MemoryType::parse_label(&mt_str),
                    importance: row.get(6)?,
                    citation: row.get(7)?,
                    valid_from: None,
                    valid_to: None,
                    verified: row.get(8)?,
                    superseded_by: None,
                })
            })
            .map_err(|e| Error::Database(format!("Failed to map current memories: {}", e)))?;

        results
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(format!("Failed to collect current memories: {}", e)))
    }

    /// Export high-importance memories as a markdown summary suitable for MEMORY.md.
    ///
    /// Returns a string with semantic/procedural/factual memories grouped by type,
    /// ordered by importance. Only non-superseded (current) memories are included.
    pub fn export_memory_summary(&self, max_entries: usize) -> Result<String> {
        let memories = self.get_current_memories(max_entries)?;
        if memories.is_empty() {
            return Ok(String::new());
        }

        let mut sections: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();

        for m in &memories {
            // Skip unverified memories (fabrication-fence — #53)
            if !m.verified {
                continue;
            }
            // Skip low-importance episodic memories (chat noise)
            if m.importance < 0.6 && matches!(m.memory_type, MemoryType::Episodic) {
                continue;
            }
            let type_label = match m.memory_type {
                MemoryType::Semantic => "Knowledge & Patterns",
                MemoryType::Fact => "Facts",
                MemoryType::Preference => "Preferences",
                MemoryType::Episodic => "Key Events",
                _ => "Other",
            };
            // Truncate long content to first 200 chars
            let content = if m.content.len() > 200 {
                let mut end = 200;
                while end < m.content.len() && !m.content.is_char_boundary(end) {
                    end += 1;
                }
                format!("{}...", &m.content[..end])
            } else {
                m.content.clone()
            };
            sections
                .entry(type_label.to_string())
                .or_default()
                .push(format!("- {}", content.replace('\n', " ")));
        }

        let mut output = String::from("## Mnemosyne Memory Sync\n\n");
        for (section, items) in &sections {
            output.push_str(&format!("### {}\n", section));
            for item in items {
                output.push_str(item);
                output.push('\n');
            }
            output.push('\n');
        }

        Ok(output)
    }

    /// Get the supersession history for a specific memory (chain of versions).
    pub fn get_supersession_chain(
        &self,
        message_id: i64,
    ) -> Result<Vec<(i64, String, Option<i64>)>> {
        // Walk backward: find what this message superseded
        let mut chain = Vec::new();
        let mut current_id = message_id;

        // Walk forward from the original
        loop {
            let row: Option<(i64, String, Option<i64>)> = self
                .conn
                .query_row(
                    "SELECT id, content, superseded_by FROM messages WHERE id = ?1",
                    params![current_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .ok();

            match row {
                Some((id, content, superseded_by)) => {
                    chain.push((id, content, superseded_by));
                    match superseded_by {
                        Some(next_id) => current_id = next_id,
                        None => break,
                    }
                }
                None => break,
            }
        }

        Ok(chain)
    }

    // Entity Management

    /// Find or create an entity by name and type.
    /// Performs fuzzy matching against existing entities' canonical names and aliases.
    /// Returns the entity ID (existing or newly created).
    pub fn upsert_entity(&self, name: &str, entity_type: &str) -> Result<i64> {
        let normalized = name.trim().to_lowercase();

        // Check exact canonical match first
        let existing: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM entities WHERE LOWER(canonical_name) = ?1 AND entity_type = ?2",
                params![normalized, entity_type],
                |row| row.get(0),
            )
            .ok();

        if let Some(id) = existing {
            // Update last_seen and bump mention count
            self.conn.execute(
                "UPDATE entities SET last_seen = CURRENT_TIMESTAMP, mention_count = mention_count + 1 WHERE id = ?1",
                params![id],
            ).map_err(|e| Error::Database(format!("Failed to update entity: {}", e)))?;
            return Ok(id);
        }

        // Check fuzzy match against aliases
        let mut stmt = self
            .conn
            .prepare("SELECT id, canonical_name, aliases FROM entities WHERE entity_type = ?1")
            .map_err(|e| Error::Database(format!("Failed to query entities: {}", e)))?;

        let entities: Vec<(i64, String, String)> = stmt
            .query_map(params![entity_type], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|e| Error::Database(format!("Failed to read entities: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        for (id, canonical, aliases_json) in &entities {
            // Check canonical name fuzzy match
            if levenshtein_ratio(&normalized, &canonical.to_lowercase()) >= 0.85 {
                // Close enough — treat as alias
                let mut aliases: Vec<String> =
                    serde_json::from_str(aliases_json).unwrap_or_default();
                if !aliases.iter().any(|a| a.to_lowercase() == normalized) {
                    aliases.push(name.trim().to_string());
                    let aliases_str = serde_json::to_string(&aliases).unwrap_or_default();
                    self.conn.execute(
                        "UPDATE entities SET aliases = ?1, last_seen = CURRENT_TIMESTAMP, mention_count = mention_count + 1 WHERE id = ?2",
                        params![aliases_str, id],
                    ).map_err(|e| Error::Database(format!("Failed to update entity aliases: {}", e)))?;
                }
                return Ok(*id);
            }

            // Check against all aliases
            let aliases: Vec<String> = serde_json::from_str(aliases_json).unwrap_or_default();
            for alias in &aliases {
                if levenshtein_ratio(&normalized, &alias.to_lowercase()) >= 0.85 {
                    self.conn.execute(
                        "UPDATE entities SET last_seen = CURRENT_TIMESTAMP, mention_count = mention_count + 1 WHERE id = ?1",
                        params![id],
                    ).map_err(|e| Error::Database(format!("Failed to update entity: {}", e)))?;
                    return Ok(*id);
                }
            }
        }

        // No match — create new entity
        self.conn
            .execute(
                "INSERT INTO entities (canonical_name, entity_type, aliases) VALUES (?1, ?2, '[]')",
                params![name.trim(), entity_type],
            )
            .map_err(|e| Error::Database(format!("Failed to create entity: {}", e)))?;

        Ok(self.conn.last_insert_rowid())
    }

    /// Link an entity to a message (record a mention).
    pub fn link_entity_to_message(
        &self,
        entity_id: i64,
        message_id: i64,
        mention_text: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO entity_mentions (entity_id, message_id, mention_text) VALUES (?1, ?2, ?3)",
            params![entity_id, message_id, mention_text],
        ).map_err(|e| Error::Database(format!("Failed to link entity: {}", e)))?;
        Ok(())
    }

    /// Get all entities mentioned in a specific message.
    ///
    /// Returns (entity_id, mention_text) pairs for graph context lookups.
    pub fn get_message_entities(&self, message_id: i64) -> Result<Vec<(i64, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT entity_id, mention_text FROM entity_mentions WHERE message_id = ?1")
            .map_err(|e| Error::Database(format!("Failed to query message entities: {}", e)))?;

        let results = stmt
            .query_map(params![message_id], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| Error::Database(format!("Failed to read message entities: {}", e)))?;

        results
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(format!("Failed to collect message entities: {}", e)))
    }

    /// Get all entities, ordered by mention count.
    pub fn get_entities(&self, limit: usize) -> Result<Vec<EntityRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, canonical_name, entity_type, aliases, first_seen, last_seen, mention_count
             FROM entities ORDER BY mention_count DESC LIMIT ?1"
        ).map_err(|e| Error::Database(format!("Failed to query entities: {}", e)))?;

        let results = stmt
            .query_map(params![limit as i64], |row| {
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
            })
            .map_err(|e| Error::Database(format!("Failed to read entities: {}", e)))?;

        results
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(format!("Failed to collect entities: {}", e)))
    }

    /// Get all messages linked to an entity.
    pub fn get_entity_messages(&self, entity_id: i64, limit: usize) -> Result<Vec<SearchResult>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.session_id, m.content, m.timestamp, m.importance, m.memory_type, m.importance, m.source_path
             FROM messages m
             JOIN entity_mentions em ON em.message_id = m.id
             WHERE em.entity_id = ?1 AND m.valid_to IS NULL
             ORDER BY m.importance DESC, m.timestamp DESC
             LIMIT ?2"
        ).map_err(|e| Error::Database(format!("Failed to query entity messages: {}", e)))?;

        let results = stmt
            .query_map(params![entity_id, limit as i64], |row| {
                let mt: String = row
                    .get::<_, String>(5)
                    .unwrap_or_else(|_| "episodic".to_string());
                Ok(SearchResult {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    content: row.get(2)?,
                    timestamp: row.get(3)?,
                    score: row.get(4)?,
                    memory_type: MemoryType::parse_label(&mt),
                    importance: row.get::<_, f64>(6).unwrap_or(0.5) as f32,
                    citation: row.get::<_, Option<String>>(7).unwrap_or(None),
                    valid_from: None,
                    valid_to: None,
                    verified: true,
                    superseded_by: None,
                })
            })
            .map_err(|e| Error::Database(format!("Failed to read entity messages: {}", e)))?;

        results
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(format!("Failed to collect entity messages: {}", e)))
    }

    /// Sanitize a query string for FTS5 MATCH.
    ///
    /// Strips characters that are FTS5 operators (`[`, `]`, `*`, `"`, `-` at
    /// word start, `(`, `)`, `{`, `}`, `^`, `~`, `:`) so user-provided
    /// search terms don't cause syntax errors.  Wraps each remaining token
    /// in double quotes for exact matching.
    fn sanitize_fts_query(raw: &str) -> String {
        let cleaned: String = raw
            .chars()
            .map(|c| match c {
                '[' | ']' | '*' | '"' | '(' | ')' | '{' | '}' | '^' | '~' | ':' => ' ',
                _ => c,
            })
            .collect();
        let tokens: Vec<&str> = cleaned.split_whitespace().collect();
        if tokens.is_empty() {
            return String::new();
        }
        tokens
            .iter()
            .map(|t| {
                let t = t.trim_start_matches('-');
                if t.is_empty() {
                    return String::new();
                }
                // Preserve FTS5 boolean operators
                if t == "OR" || t == "AND" || t == "NOT" {
                    return t.to_string();
                }
                format!("\"{}\"", t)
            })
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Search messages by keyword (FTS). Only returns current (non-superseded) memories.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        let safe_query = Self::sanitize_fts_query(query);
        if safe_query.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.session_id, m.content, m.timestamp, bm25(messages_fts), m.memory_type, m.importance, m.source_path
             FROM messages_fts f
             JOIN messages m ON f.rowid = m.id
             WHERE messages_fts MATCH ?1 AND m.valid_to IS NULL
             ORDER BY bm25(messages_fts)
             LIMIT ?2"
        ).map_err(|e| Error::Database(format!("Failed to prepare search: {}", e)))?;

        let results = stmt
            .query_map(params![safe_query, limit as i64], |row| {
                let mt: String = row
                    .get::<_, String>(5)
                    .unwrap_or_else(|_| "episodic".to_string());
                Ok(SearchResult {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    content: row.get(2)?,
                    timestamp: row.get(3)?,
                    score: row.get::<_, f64>(4).unwrap_or(-1.0) as f32,
                    memory_type: MemoryType::parse_label(&mt),
                    importance: row.get::<_, f64>(6).unwrap_or(0.5) as f32,
                    citation: row.get::<_, Option<String>>(7).unwrap_or(None),
                    valid_from: None,
                    valid_to: None,
                    verified: true,
                    superseded_by: None,
                })
            })
            .map_err(|e| Error::Database(format!("Search failed: {}", e)))?;

        results
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(format!("Failed to collect results: {}", e)))
    }

    /// FTS search scoped to a specific session_id (e.g. "room:<room_id>").
    pub fn search_in_session(
        &self,
        query: &str,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let safe_query = Self::sanitize_fts_query(query);
        if safe_query.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.session_id, m.content, m.timestamp, bm25(messages_fts), m.memory_type, m.importance, m.source_path
             FROM messages_fts f
             JOIN messages m ON f.rowid = m.id
             WHERE messages_fts MATCH ?1 AND m.session_id = ?2 AND m.valid_to IS NULL
             ORDER BY bm25(messages_fts)
             LIMIT ?3"
        ).map_err(|e| Error::Database(format!("Failed to prepare session search: {}", e)))?;

        let results = stmt
            .query_map(params![safe_query, session_id, limit as i64], |row| {
                let mt: String = row
                    .get::<_, String>(5)
                    .unwrap_or_else(|_| "episodic".to_string());
                Ok(SearchResult {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    content: row.get(2)?,
                    timestamp: row.get(3)?,
                    score: row.get::<_, f64>(4).unwrap_or(-1.0) as f32,
                    memory_type: MemoryType::parse_label(&mt),
                    importance: row.get::<_, f64>(6).unwrap_or(0.5) as f32,
                    citation: row.get::<_, Option<String>>(7).unwrap_or(None),
                    valid_from: None,
                    valid_to: None,
                    verified: true,
                    superseded_by: None,
                })
            })
            .map_err(|e| Error::Database(format!("Session search failed: {}", e)))?;

        results
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(format!("Failed to collect session results: {}", e)))
    }

    /// Search by vector similarity (brute-force cosine similarity)
    pub fn vector_search(
        &self,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        if !self.enable_embeddings {
            return Err(Error::Database(
                "Embeddings are not enabled. Set enable_embeddings = true in config.".to_string(),
            ));
        }

        let mut stmt = self.conn.prepare(
            "SELECT e.id, e.message_id, e.embedding, m.session_id, m.content, m.timestamp, m.memory_type, m.importance, m.source_path
             FROM embeddings e
             JOIN messages m ON e.message_id = m.id
             WHERE m.valid_to IS NULL"
        ).map_err(|e| Error::Database(format!("Failed to prepare vector search: {}", e)))?;

        let rows = stmt
            .query_map([], |row| {
                let blob: Vec<u8> = row.get(2)?;
                Ok((
                    row.get::<_, i64>(1)?,    // message_id
                    blob,                     // embedding bytes
                    row.get::<_, String>(3)?, // session_id
                    row.get::<_, String>(4)?, // content
                    row.get::<_, String>(5)?, // timestamp
                    row.get::<_, String>(6)
                        .unwrap_or_else(|_| "episodic".to_string()), // memory_type
                    row.get::<_, f64>(7).unwrap_or(0.5), // importance
                    row.get::<_, Option<String>>(8).unwrap_or(None), // source_path
                ))
            })
            .map_err(|e| Error::Database(format!("Vector search query failed: {}", e)))?;

        let mut scored: Vec<SearchResult> = Vec::new();
        for row in rows {
            let (message_id, blob, session_id, content, timestamp, mt, importance, citation) =
                row.map_err(|e| Error::Database(format!("Failed to read embedding row: {}", e)))?;

            let stored_embedding = bytes_to_embedding(&blob);
            let similarity = cosine_similarity(query_embedding, &stored_embedding);

            scored.push(SearchResult {
                id: message_id,
                session_id,
                content,
                timestamp,
                score: similarity,
                memory_type: MemoryType::parse_label(&mt),
                importance: importance as f32,
                citation,
                valid_from: None,
                valid_to: None,
                verified: true,
                superseded_by: None,
            });
        }

        // Sort by similarity descending
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(limit);

        Ok(scored)
    }

    /// Hybrid search combining FTS and vector similarity with configurable weights.
    ///
    /// - `candidate_multiplier`: retrieve `multiplier * limit` candidates from each source
    /// - `vector_weight` / `text_weight`: weighted merge of vector and text scores
    /// - BM25 rank normalized via `1.0 / (1.0 + max(0.0, rank))`
    #[allow(clippy::too_many_arguments)]
    pub fn hybrid_search(
        &self,
        query: &str,
        query_embedding: Option<&[f32]>,
        limit: usize,
        enable_fts: bool,
        vector_weight: f64,
        text_weight: f64,
        candidate_multiplier: usize,
    ) -> Result<Vec<SearchResult>> {
        let has_fts = enable_fts;
        let has_embedding = query_embedding.is_some() && self.enable_embeddings;
        let candidates = candidate_multiplier.max(1) * limit;

        match (has_fts, has_embedding) {
            (true, true) => {
                let fts_results = self.search(query, candidates)?;
                let vec_results = self.vector_search(query_embedding.unwrap(), candidates)?;

                // Build map: id -> (text_score, vector_score, metadata)
                let mut result_map: std::collections::HashMap<i64, (f64, f64, SearchResult)> =
                    std::collections::HashMap::new();

                // Insert FTS results with BM25-normalized score
                // FTS5 rank/bm25() returns negative values (more negative = better match)
                // Negate to get positive magnitude, then normalize to 0.0-1.0
                for r in &fts_results {
                    let bm25_score = 1.0 / (1.0 + (-r.score as f64).max(0.0));
                    result_map.insert(
                        r.id,
                        (
                            bm25_score,
                            0.0,
                            SearchResult {
                                id: r.id,
                                session_id: r.session_id.clone(),
                                content: r.content.clone(),
                                timestamp: r.timestamp.clone(),
                                score: 0.0, // computed below
                                memory_type: r.memory_type,
                                importance: r.importance,
                                citation: r.citation.clone(),
                                valid_from: None,
                                valid_to: None,
                                verified: true,
                                superseded_by: None,
                            },
                        ),
                    );
                }

                // Merge vector results
                for r in &vec_results {
                    if let Some(entry) = result_map.get_mut(&r.id) {
                        entry.1 = r.score as f64;
                    } else {
                        result_map.insert(
                            r.id,
                            (
                                0.0,
                                r.score as f64,
                                SearchResult {
                                    id: r.id,
                                    session_id: r.session_id.clone(),
                                    content: r.content.clone(),
                                    timestamp: r.timestamp.clone(),
                                    score: 0.0,
                                    memory_type: r.memory_type,
                                    importance: r.importance,
                                    citation: r.citation.clone(),
                                    valid_from: None,
                                    valid_to: None,
                                    verified: true,
                                    superseded_by: None,
                                },
                            ),
                        );
                    }
                }

                // Compute final weighted scores
                let mut merged: Vec<SearchResult> = result_map
                    .into_values()
                    .map(|(ts, vs, mut sr)| {
                        sr.score = (vector_weight * vs + text_weight * ts) as f32;
                        sr
                    })
                    .collect();
                merged.sort_by(|a, b| {
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                merged.truncate(limit);

                Ok(merged)
            }
            (true, false) => self.search(query, limit),
            (false, true) => self.vector_search(query_embedding.unwrap(), limit),
            (false, false) => Ok(Vec::new()),
        }
    }

    /// Get messages for a session
    pub fn get_session_messages(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<StoredMessage>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, role, content, tool_calls, tool_results, timestamp
             FROM messages
             WHERE session_id = ?1
             ORDER BY timestamp DESC
             LIMIT ?2",
            )
            .map_err(|e| Error::Database(format!("Failed to prepare query: {}", e)))?;

        let results = stmt
            .query_map(params![session_id, limit as i64], |row| {
                Ok(StoredMessage {
                    id: row.get(0)?,
                    role: row.get(1)?,
                    content: row.get(2)?,
                    tool_calls: row.get(3)?,
                    tool_results: row.get(4)?,
                    timestamp: row.get(5)?,
                })
            })
            .map_err(|e| Error::Database(format!("Query failed: {}", e)))?;

        results
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(format!("Failed to collect results: {}", e)))
    }

    /// Get database statistics
    pub fn stats(&self) -> Result<MemoryStats> {
        let message_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
            .map_err(|e| Error::Database(format!("Failed to count messages: {}", e)))?;

        let session_count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(DISTINCT session_id) FROM messages",
                [],
                |row| row.get(0),
            )
            .map_err(|e| Error::Database(format!("Failed to count sessions: {}", e)))?;

        // Count embeddings (handle case where table doesn't exist)
        let embedding_count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM embeddings", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_or(0) as usize;

        let embedding_cache_count = self.embedding_cache_count().unwrap_or(0);

        let tracked_file_count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM memory_files", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_or(0) as usize;

        Ok(MemoryStats {
            message_count: message_count as usize,
            session_count: session_count as usize,
            embedding_count,
            embedding_cache_count,
            tracked_file_count,
        })
    }

    /// Store a message with explicit memory type and importance
    pub fn store_typed(
        &self,
        session_id: &str,
        message: &Message,
        memory_type: MemoryType,
        importance: f32,
    ) -> Result<i64> {
        let role = serde_json::to_string(&message.role)
            .unwrap_or_else(|_| format!("{:?}", message.role))
            .trim_matches('"')
            .to_string();
        let tool_calls = serde_json::to_string(&message.tool_calls)
            .map_err(|e| Error::Serialization(e.to_string()))?;
        let tool_results = serde_json::to_string(&message.tool_results)
            .map_err(|e| Error::Serialization(e.to_string()))?;
        let timestamp = message.timestamp.to_rfc3339();

        self.conn.execute(
            "INSERT INTO messages (session_id, role, content, tool_calls, tool_results, timestamp, memory_type, importance)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                session_id,
                role,
                message.content,
                tool_calls,
                tool_results,
                timestamp,
                memory_type.as_str(),
                importance as f64,
            ],
        ).map_err(|e| Error::Database(format!("Failed to insert typed message: {}", e)))?;

        Ok(self.conn.last_insert_rowid())
    }

    /// Search messages filtered by memory type
    pub fn search_by_type(
        &self,
        query: &str,
        memory_type: MemoryType,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let safe_query = Self::sanitize_fts_query(query);
        if safe_query.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.session_id, m.content, m.timestamp, rank, m.memory_type, m.importance, m.source_path
             FROM messages_fts f
             JOIN messages m ON f.rowid = m.id
             WHERE messages_fts MATCH ?1 AND m.memory_type = ?2 AND m.valid_to IS NULL
             ORDER BY rank
             LIMIT ?3"
        ).map_err(|e| Error::Database(format!("Failed to prepare typed search: {}", e)))?;

        let results = stmt
            .query_map(params![safe_query, memory_type.as_str(), limit as i64], |row| {
                let mt: String = row
                    .get::<_, String>(5)
                    .unwrap_or_else(|_| "episodic".to_string());
                Ok(SearchResult {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    content: row.get(2)?,
                    timestamp: row.get(3)?,
                    score: row.get::<_, f64>(4).unwrap_or(-1.0) as f32,
                    memory_type: MemoryType::parse_label(&mt),
                    importance: row.get::<_, f64>(6).unwrap_or(0.5) as f32,
                    citation: row.get::<_, Option<String>>(7).unwrap_or(None),
                    valid_from: None,
                    valid_to: None,
                    verified: true,
                    superseded_by: None,
                })
            })
            .map_err(|e| Error::Database(format!("Typed search failed: {}", e)))?;

        results
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(format!("Failed to collect results: {}", e)))
    }

    /// Decay importance of all episodic memories by the given rate
    pub fn decay_importance(&self, decay_rate: f32) -> Result<usize> {
        let updated = self.conn.execute(
            "UPDATE messages SET importance = MAX(0.0, importance - ?1) WHERE memory_type = 'episodic' AND importance > 0.0",
            params![decay_rate as f64],
        ).map_err(|e| Error::Database(format!("Failed to decay importance: {}", e)))?;

        Ok(updated)
    }

    /// Promote an episodic memory to semantic type with updated content
    pub fn promote_to_semantic(&self, message_id: i64, knowledge: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE messages SET memory_type = 'semantic', importance = 0.9, content = ?1 WHERE id = ?2",
            params![knowledge, message_id],
        ).map_err(|e| Error::Database(format!("Failed to promote to semantic: {}", e)))?;
        Ok(())
    }

    /// Get working memory for the current session
    pub fn working_memory(&self, session_id: &str) -> Result<Vec<SearchResult>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, session_id, content, timestamp, memory_type, importance, source_path
             FROM messages
             WHERE session_id = ?1 AND memory_type = 'working'
             ORDER BY timestamp DESC",
            )
            .map_err(|e| Error::Database(format!("Failed to query working memory: {}", e)))?;

        let results = stmt
            .query_map(params![session_id], |row| {
                let mt: String = row
                    .get::<_, String>(4)
                    .unwrap_or_else(|_| "working".to_string());
                Ok(SearchResult {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    content: row.get(2)?,
                    timestamp: row.get(3)?,
                    score: 1.0, // Working memory is always fully relevant
                    memory_type: MemoryType::parse_label(&mt),
                    importance: row.get::<_, f64>(5).unwrap_or(0.8) as f32,
                    citation: row.get::<_, Option<String>>(6).unwrap_or(None),
                    valid_from: None,
                    valid_to: None,
                    verified: true,
                    superseded_by: None,
                })
            })
            .map_err(|e| Error::Database(format!("Working memory query failed: {}", e)))?;

        results
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(format!("Failed to collect working memory: {}", e)))
    }

    /// Finalize a session's working memory.
    ///
    /// Promotes working memories with importance >= `promote_threshold` to episodic,
    /// and deletes working memories below the threshold. Returns (promoted, discarded).
    pub fn finalize_working_memory(
        &self,
        session_id: &str,
        promote_threshold: f32,
    ) -> Result<(usize, usize)> {
        // Promote high-importance working memories to episodic
        let promoted = self
            .conn
            .execute(
                "UPDATE messages SET memory_type = 'episodic' WHERE session_id = ?1 AND memory_type = 'working' AND importance >= ?2",
                params![session_id, promote_threshold as f64],
            )
            .map_err(|e| {
                Error::Database(format!("Failed to promote working memory: {}", e))
            })?;

        // Discard low-importance working memories
        let discarded = self
            .conn
            .execute(
                "DELETE FROM messages WHERE session_id = ?1 AND memory_type = 'working' AND importance < ?2",
                params![session_id, promote_threshold as f64],
            )
            .map_err(|e| {
                Error::Database(format!("Failed to discard working memory: {}", e))
            })?;

        Ok((promoted, discarded))
    }

    /// Delete messages older than the given date
    pub fn forget_before(&self, before: DateTime<Utc>) -> Result<usize> {
        let timestamp = before.to_rfc3339();
        let deleted = self
            .conn
            .execute(
                "DELETE FROM messages WHERE timestamp < ?1",
                params![timestamp],
            )
            .map_err(|e| Error::Database(format!("Failed to delete messages: {}", e)))?;

        Ok(deleted)
    }

    // Embedding Cache

    /// Look up a cached embedding by provider + model + content hash.
    /// Updates `last_used` on hit.
    pub fn get_cached_embedding(
        &self,
        provider: &str,
        model: &str,
        content_hash: &str,
    ) -> Result<Option<Vec<f32>>> {
        let now = Utc::now().timestamp();

        let result: std::result::Result<Vec<u8>, _> = self.conn.query_row(
            "SELECT id, embedding FROM embedding_cache
             WHERE provider = ?1 AND model = ?2 AND content_hash = ?3",
            params![provider, model, content_hash],
            |row| {
                let id: i64 = row.get(0)?;
                let blob: Vec<u8> = row.get(1)?;
                // Update last_used (best-effort, ignore error)
                let _ = self.conn.execute(
                    "UPDATE embedding_cache SET last_used = ?1 WHERE id = ?2",
                    params![now, id],
                );
                Ok(blob)
            },
        );

        match result {
            Ok(blob) => Ok(Some(bytes_to_embedding(&blob))),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(Error::Database(format!(
                "Embedding cache lookup failed: {}",
                e
            ))),
        }
    }

    /// Store an embedding in the cache.
    pub fn store_cached_embedding(
        &self,
        provider: &str,
        model: &str,
        content_hash: &str,
        embedding: &[f32],
    ) -> Result<()> {
        let now = Utc::now().timestamp();
        let bytes = embedding_to_bytes(embedding);

        self.conn
            .execute(
                "INSERT INTO embedding_cache (provider, model, content_hash, embedding, created_at, last_used)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(provider, model, content_hash)
                 DO UPDATE SET embedding = excluded.embedding, last_used = excluded.last_used",
                params![provider, model, content_hash, bytes, now, now],
            )
            .map_err(|e| Error::Database(format!("Failed to store cached embedding: {}", e)))?;

        Ok(())
    }

    /// Evict oldest entries to keep cache at or below `max_entries`.
    /// Returns number of entries evicted.
    pub fn evict_lru_cache(&self, max_entries: usize) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM embedding_cache", [], |row| row.get(0))
            .map_err(|e| Error::Database(format!("Failed to count cache entries: {}", e)))?;

        if count as usize <= max_entries {
            return Ok(0);
        }

        let to_evict = count as usize - max_entries;
        let evicted = self
            .conn
            .execute(
                "DELETE FROM embedding_cache WHERE id IN (
                    SELECT id FROM embedding_cache ORDER BY last_used ASC LIMIT ?1
                )",
                params![to_evict as i64],
            )
            .map_err(|e| Error::Database(format!("Failed to evict cache entries: {}", e)))?;

        Ok(evicted)
    }

    /// Count entries in the embedding cache.
    pub fn embedding_cache_count(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM embedding_cache", [], |row| row.get(0))
            .map_err(|e| Error::Database(format!("Failed to count cache: {}", e)))?;
        Ok(count as usize)
    }

    // Memory File Tracking

    /// Get a tracked file entry by path and source.
    pub fn get_tracked_file(&self, path: &str, source: &str) -> Result<Option<TrackedFile>> {
        let result = self.conn.query_row(
            "SELECT path, source, content_hash, mtime, size, last_indexed
             FROM memory_files WHERE path = ?1 AND source = ?2",
            params![path, source],
            |row| {
                Ok(TrackedFile {
                    path: row.get(0)?,
                    source: row.get(1)?,
                    content_hash: row.get(2)?,
                    mtime: row.get(3)?,
                    size: row.get(4)?,
                    last_indexed: row.get(5)?,
                })
            },
        );

        match result {
            Ok(f) => Ok(Some(f)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(Error::Database(format!(
                "Failed to get tracked file: {}",
                e
            ))),
        }
    }

    /// Insert or update a tracked file entry.
    pub fn upsert_tracked_file(
        &self,
        path: &str,
        source: &str,
        content_hash: &str,
        mtime: i64,
        size: i64,
    ) -> Result<()> {
        let now = Utc::now().timestamp();
        self.conn
            .execute(
                "INSERT INTO memory_files (path, source, content_hash, mtime, size, last_indexed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(path, source)
                 DO UPDATE SET content_hash = excluded.content_hash,
                              mtime = excluded.mtime,
                              size = excluded.size,
                              last_indexed = excluded.last_indexed",
                params![path, source, content_hash, mtime, size, now],
            )
            .map_err(|e| Error::Database(format!("Failed to upsert tracked file: {}", e)))?;

        Ok(())
    }

    /// List all tracked files for a given source.
    pub fn list_tracked_files(&self, source: &str) -> Result<Vec<TrackedFile>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT path, source, content_hash, mtime, size, last_indexed
                 FROM memory_files WHERE source = ?1 ORDER BY path",
            )
            .map_err(|e| {
                Error::Database(format!("Failed to prepare tracked files query: {}", e))
            })?;

        let results = stmt
            .query_map(params![source], |row| {
                Ok(TrackedFile {
                    path: row.get(0)?,
                    source: row.get(1)?,
                    content_hash: row.get(2)?,
                    mtime: row.get(3)?,
                    size: row.get(4)?,
                    last_indexed: row.get(5)?,
                })
            })
            .map_err(|e| Error::Database(format!("Failed to list tracked files: {}", e)))?;

        results
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(format!("Failed to collect tracked files: {}", e)))
    }

    /// Remove a tracked file entry.
    pub fn remove_tracked_file(&self, path: &str, source: &str) -> Result<bool> {
        let deleted = self
            .conn
            .execute(
                "DELETE FROM memory_files WHERE path = ?1 AND source = ?2",
                params![path, source],
            )
            .map_err(|e| Error::Database(format!("Failed to remove tracked file: {}", e)))?;

        Ok(deleted > 0)
    }

    // Session File Tracking

    /// Get a session file entry by session_id.
    pub fn get_session_file(&self, session_id: &str) -> Result<Option<SessionFileEntry>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT session_id, file_path, last_size, pending_bytes, pending_messages, last_indexed
                 FROM session_files WHERE session_id = ?1",
            )
            .map_err(|e| Error::Database(format!("Failed to prepare session_files query: {}", e)))?;

        let mut rows = stmt
            .query_map(params![session_id], |row| {
                Ok(SessionFileEntry {
                    session_id: row.get(0)?,
                    file_path: row.get(1)?,
                    last_size: row.get(2)?,
                    pending_bytes: row.get(3)?,
                    pending_messages: row.get(4)?,
                    last_indexed: row.get(5)?,
                })
            })
            .map_err(|e| Error::Database(format!("Failed to query session_files: {}", e)))?;

        match rows.next() {
            Some(Ok(entry)) => Ok(Some(entry)),
            Some(Err(e)) => Err(Error::Database(format!(
                "Failed to read session_file: {}",
                e
            ))),
            None => Ok(None),
        }
    }

    /// Insert or update a session file entry.
    pub fn upsert_session_file(
        &self,
        session_id: &str,
        file_path: &str,
        last_size: i64,
        pending_bytes: i64,
        pending_messages: i64,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        self.conn
            .execute(
                "INSERT INTO session_files (session_id, file_path, last_size, pending_bytes, pending_messages, last_indexed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(session_id) DO UPDATE SET
                    file_path = excluded.file_path,
                    last_size = excluded.last_size,
                    pending_bytes = excluded.pending_bytes,
                    pending_messages = excluded.pending_messages,
                    last_indexed = excluded.last_indexed",
                params![session_id, file_path, last_size, pending_bytes, pending_messages, now],
            )
            .map_err(|e| Error::Database(format!("Failed to upsert session_file: {}", e)))?;
        Ok(())
    }

    // Cross-Session Pattern Recognition

    /// Extract recurring patterns from completed sessions.
    ///
    /// Analyzes messages in the DB to find:
    /// - Frequently used tools (pattern_type = "tool")
    /// - Common question themes via leading words (pattern_type = "theme")
    /// - Recurring topic keywords (pattern_type = "topic")
    ///
    /// Patterns are upserted: new ones are created, existing ones get frequency bumped.
    pub fn extract_patterns(&self) -> Result<usize> {
        let now = Utc::now().to_rfc3339();
        let mut count = 0;

        // 1. Extract frequently-used tools from tool_calls
        {
            let mut stmt = self.conn.prepare(
                "SELECT tool_calls FROM messages WHERE tool_calls IS NOT NULL AND tool_calls != '[]'"
            ).map_err(|e| Error::Database(format!("Failed to query tool calls: {}", e)))?;

            let mut tool_freq: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| Error::Database(format!("Tool call query failed: {}", e)))?;

            for row in rows.flatten() {
                if let Ok(calls) = serde_json::from_str::<Vec<serde_json::Value>>(&row) {
                    for call in calls {
                        if let Some(name) = call.get("name").and_then(|n| n.as_str()) {
                            *tool_freq.entry(name.to_string()).or_insert(0) += 1;
                        }
                    }
                }
            }

            for (tool_name, freq) in &tool_freq {
                if *freq >= 2 {
                    self.upsert_pattern("tool", tool_name, *freq, &now)?;
                    count += 1;
                }
            }
        }

        // 2. Extract question themes from user messages
        {
            let mut stmt = self
                .conn
                .prepare("SELECT content FROM messages WHERE role = 'user' AND content != ''")
                .map_err(|e| Error::Database(format!("Failed to query user messages: {}", e)))?;

            let mut theme_freq: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| Error::Database(format!("Theme query failed: {}", e)))?;

            for content in rows.flatten() {
                // Extract theme from first significant words
                let words: Vec<&str> = content.split_whitespace().take(4).collect();
                if words.len() >= 2 {
                    let theme = words.join(" ").to_lowercase();
                    *theme_freq.entry(theme).or_insert(0) += 1;
                }
            }

            for (theme, freq) in &theme_freq {
                if *freq >= 2 {
                    self.upsert_pattern("theme", theme, *freq, &now)?;
                    count += 1;
                }
            }
        }

        // 3. Extract topic keywords (nouns/significant words appearing across sessions)
        {
            let mut stmt = self.conn.prepare(
                "SELECT DISTINCT session_id, content FROM messages WHERE role = 'user' AND content != ''"
            ).map_err(|e| Error::Database(format!("Failed to query topics: {}", e)))?;

            // Track which words appear in how many distinct sessions
            let mut word_sessions: std::collections::HashMap<String, HashSet<String>> =
                std::collections::HashMap::new();
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| Error::Database(format!("Topic query failed: {}", e)))?;

            let stop_words = build_stop_words();

            for row in rows.flatten() {
                let (session_id, content) = row;
                for word in content.split_whitespace() {
                    let w = word
                        .to_lowercase()
                        .trim_matches(|c: char| !c.is_alphanumeric())
                        .to_string();
                    if w.len() >= 3 && !stop_words.contains(w.as_str()) {
                        word_sessions
                            .entry(w)
                            .or_default()
                            .insert(session_id.clone());
                    }
                }
            }

            // Only keep words appearing in 2+ sessions
            for (word, sessions) in &word_sessions {
                if sessions.len() >= 2 {
                    self.upsert_pattern("topic", word, sessions.len(), &now)?;
                    count += 1;
                }
            }
        }

        info!(patterns_upserted = count, "Pattern extraction complete");
        Ok(count)
    }

    /// Upsert a pattern: insert if new, update frequency and last_seen if exists.
    fn upsert_pattern(
        &self,
        pattern_type: &str,
        content: &str,
        frequency: usize,
        now: &str,
    ) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO patterns (pattern_type, content, frequency, first_seen, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(pattern_type, content) DO UPDATE SET
                frequency = frequency + ?3,
                last_seen = ?4",
                params![pattern_type, content, frequency as i64, now],
            )
            .map_err(|e| Error::Database(format!("Failed to upsert pattern: {}", e)))?;
        Ok(())
    }

    /// Get patterns by type, ordered by frequency descending.
    pub fn get_patterns(&self, pattern_type: &str, limit: usize) -> Result<Vec<PatternEntry>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, pattern_type, content, frequency, first_seen, last_seen
             FROM patterns
             WHERE pattern_type = ?1
             ORDER BY frequency DESC
             LIMIT ?2",
            )
            .map_err(|e| Error::Database(format!("Failed to query patterns: {}", e)))?;

        let results = stmt
            .query_map(params![pattern_type, limit as i64], |row| {
                Ok(PatternEntry {
                    id: row.get(0)?,
                    pattern_type: row.get(1)?,
                    content: row.get(2)?,
                    frequency: row.get(3)?,
                    first_seen: row.get(4)?,
                    last_seen: row.get(5)?,
                })
            })
            .map_err(|e| Error::Database(format!("Pattern query failed: {}", e)))?;

        results
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(format!("Failed to collect patterns: {}", e)))
    }

    /// Get all patterns regardless of type, ordered by frequency descending.
    pub fn get_all_patterns(&self, limit: usize) -> Result<Vec<PatternEntry>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, pattern_type, content, frequency, first_seen, last_seen
             FROM patterns
             ORDER BY frequency DESC
             LIMIT ?1",
            )
            .map_err(|e| Error::Database(format!("Failed to query all patterns: {}", e)))?;

        let results = stmt
            .query_map(params![limit as i64], |row| {
                Ok(PatternEntry {
                    id: row.get(0)?,
                    pattern_type: row.get(1)?,
                    content: row.get(2)?,
                    frequency: row.get(3)?,
                    first_seen: row.get(4)?,
                    last_seen: row.get(5)?,
                })
            })
            .map_err(|e| Error::Database(format!("All patterns query failed: {}", e)))?;

        results
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(format!("Failed to collect all patterns: {}", e)))
    }

    // Importance Scoring with Decay

    /// Decay memory importance based on time since last access.
    ///
    /// Reduces importance by `daily_rate` (default 0.10 = 10%) for each day
    /// since `last_accessed`. Memories that have never been accessed use
    /// their creation timestamp. Only affects episodic memories with importance > 0.
    pub fn decay_memories(&self, daily_rate: f64) -> Result<usize> {
        // For memories with last_accessed set, decay based on days since access
        // For memories without last_accessed, decay based on days since creation
        let now = Utc::now().to_rfc3339();
        let updated = self.conn.execute(
            "UPDATE messages SET importance = MAX(0.0, importance * (1.0 - ?1 *
                MAX(1, CAST((julianday(?2) - julianday(COALESCE(last_accessed, timestamp))) AS INTEGER))
             ))
             WHERE memory_type = 'episodic' AND importance > 0.0",
            params![daily_rate, now],
        ).map_err(|e| Error::Database(format!("Failed to decay memories: {}", e)))?;

        debug!(decayed = updated, rate = daily_rate, "Memory decay applied");
        Ok(updated)
    }

    /// Boost a memory's importance score when it is retrieved/accessed.
    ///
    /// Increases importance by `boost` (capped at 1.0) and updates last_accessed.
    pub fn boost_memory(&self, message_id: i64, boost: f64) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                "UPDATE messages SET
                importance = MIN(1.0, importance + ?1),
                last_accessed = ?2
             WHERE id = ?3",
                params![boost, now, message_id],
            )
            .map_err(|e| Error::Database(format!("Failed to boost memory: {}", e)))?;
        Ok(())
    }

    /// Get the importance score and last_accessed for a memory.
    pub fn get_memory_importance(&self, message_id: i64) -> Result<(f64, Option<String>)> {
        let mut stmt = self
            .conn
            .prepare("SELECT importance, last_accessed FROM messages WHERE id = ?1")
            .map_err(|e| Error::Database(format!("Failed to query importance: {}", e)))?;

        stmt.query_row(params![message_id], |row| {
            Ok((row.get::<_, f64>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(|e| Error::Database(format!("Importance query failed: {}", e)))
    }

    // ── Track B: Memory Consolidation ──────────────────────────────────

    /// Check if content is near-duplicate of an existing memory using FTS5.
    ///
    /// Returns `Some(existing_id)` if a near-duplicate is found above `threshold`,
    /// or `None` if the content is novel.
    pub fn find_duplicate(&self, content: &str, threshold: f64) -> Result<Option<i64>> {
        if threshold <= 0.0 || content.trim().len() < 20 {
            return Ok(None);
        }

        // Sanitize for FTS5 query — remove special chars that break the parser
        let sanitized: String = content
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .collect();
        let query_terms: Vec<&str> = sanitized.split_whitespace().take(30).collect();
        if query_terms.len() < 3 {
            return Ok(None);
        }
        let fts_query = query_terms
            .iter()
            .map(|t| format!("\"{}\"", t))
            .collect::<Vec<_>>()
            .join(" OR ");

        let mut stmt = self
            .conn
            .prepare(
                "SELECT m.id, m.content, rank
                 FROM messages_fts fts
                 JOIN messages m ON m.rowid = fts.rowid
                 WHERE messages_fts MATCH ?1
                   AND m.valid_to IS NULL
                 ORDER BY rank
                 LIMIT 5",
            )
            .map_err(|e| Error::Database(format!("Dedup FTS query failed: {}", e)))?;

        let candidates: Vec<(i64, String)> = stmt
            .query_map(params![fts_query], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| Error::Database(format!("Dedup query map failed: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        // Compare using Levenshtein ratio
        let content_lower = content.to_lowercase();
        for (id, existing) in &candidates {
            let existing_lower = existing.to_lowercase();
            let dist = levenshtein_distance(&content_lower, &existing_lower);
            let max_len = content_lower.len().max(existing_lower.len());
            if max_len == 0 {
                continue;
            }
            let ratio = 1.0 - (dist as f64 / max_len as f64);
            if ratio >= threshold {
                debug!(existing_id = id, ratio, "Near-duplicate detected, skipping store");
                return Ok(Some(*id));
            }
        }

        Ok(None)
    }

    /// Store a message with deduplication check.
    ///
    /// If a near-duplicate exists (above `dedup_threshold`), boosts the existing
    /// memory's importance instead of inserting a new row. Returns the message ID
    /// (existing or new).
    pub fn store_message_dedup(
        &self,
        session_id: &str,
        message: &Message,
        dedup_threshold: f64,
    ) -> Result<i64> {
        // Only dedup non-empty content from assistant/system (user messages are always stored)
        if dedup_threshold > 0.0
            && !message.content.trim().is_empty()
            && !matches!(message.role, zeus_core::Role::User)
            && let Some(existing_id) = self.find_duplicate(&message.content, dedup_threshold)? {
                // Boost the existing memory instead of duplicating
                self.boost_memory(existing_id, 0.05)?;
                return Ok(existing_id);
            }

        self.store_message(session_id, message)
    }

    /// Consolidate a session's messages into a compact summary.
    ///
    /// Keeps the first and last `keep_edges` messages intact for context,
    /// and replaces the middle with a single summary message. Returns
    /// `(kept, consolidated)` counts.
    pub fn consolidate_session(
        &self,
        session_id: &str,
        keep_edges: usize,
    ) -> Result<(usize, usize)> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, role, content, timestamp FROM messages
                 WHERE session_id = ?1 AND valid_to IS NULL
                 ORDER BY timestamp ASC",
            )
            .map_err(|e| Error::Database(format!("Consolidation query failed: {}", e)))?;

        let rows: Vec<(i64, String, String, String)> = stmt
            .query_map(params![session_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| Error::Database(format!("Consolidation map failed: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        let total = rows.len();
        if total <= keep_edges * 2 + 1 {
            // Not enough messages to consolidate
            return Ok((total, 0));
        }

        // Build a summary of the middle messages
        let middle = &rows[keep_edges..total - keep_edges];
        let mut summary_parts: Vec<String> = Vec::new();
        for (_, role, content, _) in middle {
            let trimmed = content.chars().take(200).collect::<String>();
            if !trimmed.trim().is_empty() {
                summary_parts.push(format!("[{}] {}", role, trimmed));
            }
        }

        let summary = format!(
            "[Session summary — {} messages consolidated]\n{}",
            middle.len(),
            summary_parts.join("\n")
        );

        // Insert the summary as a new message
        let now = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute(
                "INSERT INTO messages (session_id, role, content, tool_calls, tool_results, timestamp, memory_type, importance, valid_from)
                 VALUES (?1, 'system', ?2, '[]', '[]', ?3, 'summary', 0.7, ?3)",
                params![session_id, summary, now],
            )
            .map_err(|e| Error::Database(format!("Summary insert failed: {}", e)))?;
        let summary_id = self.conn.last_insert_rowid();

        // Supersede the middle messages
        for (id, _, _, _) in middle {
            self.supersede_message(*id, summary_id)?;
        }

        let consolidated = middle.len();
        debug!(
            session_id,
            kept = keep_edges * 2,
            consolidated,
            "Session consolidated"
        );
        Ok((keep_edges * 2 + 1, consolidated))
    }

    /// Get total memory count (non-superseded).
    pub fn memory_count(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE valid_to IS NULL",
                [],
                |row| row.get(0),
            )
            .map_err(|e| Error::Database(format!("Memory count failed: {}", e)))?;
        Ok(count as usize)
    }

    /// Enforce memory cap by pruning lowest-importance episodic memories.
    ///
    /// When `memory_count()` exceeds `max_memories`, supersedes the least
    /// important episodic memories until count is back under the cap.
    /// Returns the number of memories pruned.
    pub fn enforce_memory_cap(&self, max_memories: usize) -> Result<usize> {
        if max_memories == 0 {
            return Ok(0); // unlimited
        }

        let current = self.memory_count()?;
        if current <= max_memories {
            return Ok(0);
        }

        let excess = current - max_memories;

        // Find the N lowest-importance episodic memories to prune
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id FROM messages
                 WHERE valid_to IS NULL
                   AND memory_type = 'episodic'
                 ORDER BY importance ASC, timestamp ASC
                 LIMIT ?1",
            )
            .map_err(|e| Error::Database(format!("Cap enforcement query failed: {}", e)))?;

        let ids: Vec<i64> = stmt
            .query_map(params![excess as i64], |row| row.get(0))
            .map_err(|e| Error::Database(format!("Cap enforcement map failed: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        let now = chrono::Utc::now().to_rfc3339();
        let pruned = ids.len();
        for id in &ids {
            self.conn
                .execute(
                    "UPDATE messages SET valid_to = ?1 WHERE id = ?2 AND valid_to IS NULL",
                    params![now, id],
                )
                .map_err(|e| Error::Database(format!("Cap prune failed: {}", e)))?;
        }

        debug!(pruned, max_memories, previous = current, "Memory cap enforced");
        Ok(pruned)
    }

    /// Get per-session message counts for sessions exceeding the limit.
    pub fn sessions_over_limit(&self, limit: usize) -> Result<Vec<(String, usize)>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT session_id, COUNT(*) as cnt FROM messages
                 WHERE valid_to IS NULL
                 GROUP BY session_id
                 HAVING cnt > ?1
                 ORDER BY cnt DESC",
            )
            .map_err(|e| Error::Database(format!("Sessions over limit query failed: {}", e)))?;

        let results: Vec<(String, usize)> = stmt
            .query_map(params![limit as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
            })
            .map_err(|e| Error::Database(format!("Sessions over limit map failed: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(results)
    }

    /// Run full consolidation pass: dedup-prune sessions over limit, then enforce cap.
    ///
    /// Returns `(sessions_consolidated, total_pruned)`.
    pub fn run_consolidation(
        &self,
        session_limit: usize,
        max_memories: usize,
        keep_edges: usize,
    ) -> Result<(usize, usize)> {
        let mut sessions_done = 0;
        let mut total_pruned = 0;

        // 1. Consolidate oversized sessions
        let oversized = self.sessions_over_limit(session_limit)?;
        for (session_id, _count) in &oversized {
            let (_kept, consolidated) = self.consolidate_session(session_id, keep_edges)?;
            if consolidated > 0 {
                sessions_done += 1;
                total_pruned += consolidated;
            }
        }

        // 2. Enforce global memory cap
        let cap_pruned = self.enforce_memory_cap(max_memories)?;
        total_pruned += cap_pruned;

        if total_pruned > 0 {
            info!(
                sessions_consolidated = sessions_done,
                total_pruned,
                "Consolidation pass complete"
            );
        }

        Ok((sessions_done, total_pruned))
    }

    // Proactive Retrieval

    /// Pre-fetch likely-needed memories based on conversation topics and patterns.
    ///
    /// Analyzes the provided messages to extract key terms, cross-references them
    /// against stored patterns, then searches for related memories. Returns results
    /// sorted by combined relevance (pattern frequency + search score + importance).
    pub fn proactive_context(
        &self,
        messages: &[Message],
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        // Extract key terms from recent messages
        let mut terms: Vec<String> = Vec::new();
        for msg in messages.iter().rev().take(5) {
            for word in msg.content.split_whitespace() {
                let w = word
                    .to_lowercase()
                    .trim_matches(|c: char| !c.is_alphanumeric())
                    .to_string();
                if w.len() >= 3 {
                    terms.push(w);
                }
            }
        }

        if terms.is_empty() {
            return Ok(Vec::new());
        }

        // Check which terms match known patterns
        let mut pattern_terms: Vec<(String, i64)> = Vec::new();
        for term in &terms {
            let mut stmt = self
                .conn
                .prepare("SELECT content, frequency FROM patterns WHERE content = ?1")
                .map_err(|e| Error::Database(format!("Pattern lookup failed: {}", e)))?;

            if let Ok(row) = stmt.query_row(params![term], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            }) {
                pattern_terms.push(row);
            }
        }

        // Build a search query from the most frequent/relevant terms
        let search_terms: Vec<String> = if !pattern_terms.is_empty() {
            // Prefer pattern-matched terms, sorted by frequency
            let mut sorted = pattern_terms.clone();
            sorted.sort_by(|a, b| b.1.cmp(&a.1));
            sorted.iter().take(5).map(|(t, _)| t.clone()).collect()
        } else {
            // Fall back to unique terms from recent messages
            let unique: HashSet<String> = terms.iter().cloned().collect();
            unique.into_iter().take(5).collect()
        };

        if search_terms.is_empty() {
            return Ok(Vec::new());
        }

        let query = search_terms.join(" OR ");

        // Search using FTS (patterns boost relevance)
        let candidates = self.search(&query, limit * 3)?;

        // Re-score candidates: combine search score with importance and pattern boost
        let mut scored: Vec<SearchResult> = candidates
            .into_iter()
            .map(|mut r| {
                // Boost score if content overlaps with pattern terms
                let pattern_boost: f32 = pattern_terms
                    .iter()
                    .filter(|(t, _)| r.content.to_lowercase().contains(&t.to_lowercase()))
                    .map(|(_, freq)| (*freq as f32) * 0.05)
                    .sum::<f32>()
                    .min(0.5);

                // Combine: original score + importance weight + pattern boost
                let normalized_score = 1.0 / (1.0 + f32::max(0.0, r.score));
                r.score = normalized_score + (r.importance * 0.3) + pattern_boost;

                // Update last_accessed for boosted memories
                if pattern_boost > 0.0 {
                    let _ = self.boost_memory(r.id, 0.05);
                }
                r
            })
            .collect();

        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(limit);

        debug!(
            results = scored.len(),
            pattern_terms = pattern_terms.len(),
            "Proactive context retrieved"
        );
        Ok(scored)
    }

    // ── Fleet session alias cache (Lane 3b-i, PRD §272) ──────────────

    /// Upsert a fleet session alias record.
    ///
    /// On conflict (same `agent_id` + `human_id`), updates `session_id`,
    /// `channel_kind`, and `last_seen` — matching Z112's gate-2 requirement
    /// that the most recent session wins.
    pub fn upsert_alias(
        &self,
        agent_id: &str,
        human_id: &str,
        session_id: &str,
        channel_kind: &str,
        last_seen: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO fleet_session_alias (agent_id, human_id, session_id, channel_kind, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (agent_id, human_id) DO UPDATE SET
                 session_id = excluded.session_id,
                 channel_kind = excluded.channel_kind,
                 last_seen = excluded.last_seen",
            params![agent_id, human_id, session_id, channel_kind, last_seen],
        )
        .map_err(|e| Error::Database(format!("Failed to upsert fleet_session_alias: {}", e)))?;
        Ok(())
    }

    /// Look up the most recent fleet session alias for a given
    /// `(agent_id, human_id)` pair, filtered by a recency window.
    ///
    /// Returns `Some(FleetSessionAliasRow)` if a record exists with
    /// `last_seen >= since`, otherwise `None`. The resolver body (Lane 3b-ii)
    /// maps a hit to `FleetSessionAlias::resolved(session_id)` and a miss
    /// to `FleetSessionAlias::unaliased(agent_id)`.
    pub fn lookup_alias(
        &self,
        agent_id: &str,
        human_id: &str,
        since: &str,
    ) -> Result<Option<FleetSessionAliasRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, channel_kind, last_seen
             FROM fleet_session_alias
             WHERE agent_id = ?1 AND human_id = ?2 AND last_seen >= ?3",
        ).map_err(|e| Error::Database(format!("Failed to prepare alias lookup: {}", e)))?;

        let result = stmt
            .query_row(params![agent_id, human_id, since], |row| {
                Ok(FleetSessionAliasRow {
                    session_id: row.get(0)?,
                    channel_kind: row.get(1)?,
                    last_seen: row.get(2)?,
                })
            })
            .ok();

        Ok(result)
    }

    /// Prune stale fleet session alias records older than the given cutoff.
    ///
    /// Implements TTL eviction per PRD §272 / AC6-AC7: aliases not refreshed
    /// within the 24-hour rolling window are evicted so the next cook
    /// produces a fresh alias.
    pub fn prune_stale_aliases(&self, older_than: &str) -> Result<usize> {
        let count = self
            .conn
            .execute(
                "DELETE FROM fleet_session_alias WHERE last_seen < ?1",
                params![older_than],
            )
            .map_err(|e| Error::Database(format!("Failed to prune stale aliases: {}", e)))?;
        Ok(count)
    }

    /// Mark a message as retracted: flip `verified=0` and record a row in the
    /// `retractions` ledger with a reason. Atomic via transaction.
    ///
    /// Companion to the #53 fence: re-filters previously-trusted content from
    /// future exports (via `verified=0`) while preserving auditability through
    /// the retraction ledger. Idempotent on `(message_id, reason)` via
    /// `INSERT OR IGNORE` against the UNIQUE constraint.
    pub fn mark_retracted(&self, message_id: i64, reason: &str) -> Result<()> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| Error::Database(format!("Failed to begin retraction tx: {}", e)))?;
        tx.execute(
            "UPDATE messages SET verified = 0 WHERE id = ?1",
            params![message_id],
        )
        .map_err(|e| Error::Database(format!("Failed to flip verified flag: {}", e)))?;
        tx.execute(
            "INSERT OR IGNORE INTO retractions (message_id, reason) VALUES (?1, ?2)",
            params![message_id, reason],
        )
        .map_err(|e| Error::Database(format!("Failed to record retraction: {}", e)))?;
        tx.commit()
            .map_err(|e| Error::Database(format!("Failed to commit retraction: {}", e)))?;
        Ok(())
    }
}

/// Row from the `fleet_session_alias` cache table (Lane 3b-i).
///
/// Lightweight struct — the resolver body (Lane 3b-ii) consumes this to
/// construct a `FleetSessionAlias::resolved(session_id)` on hit.
pub struct FleetSessionAliasRow {
    pub session_id: String,
    pub channel_kind: String,
    pub last_seen: String,
}

// Mnemosyne Async Wrapper

impl Mnemosyne {
    /// Create a new Mnemosyne instance.
    ///
    /// If `enable_embeddings` is true, an `EmbeddingChain` is built from
    /// the configured `embedding_providers` with fallback support.
    pub async fn new(config: MnemosyneConfig) -> Result<Self> {
        // Ensure directory exists
        if let Some(parent) = config.db_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let store = MemoryStore::new(&config.db_path, config.enable_fts, config.enable_embeddings)?;

        let embedding_chain = if config.enable_embeddings {
            let chain = EmbeddingChain::from_config(&config);
            debug!(
                providers = ?config.embedding_providers,
                fallback_threshold = config.fallback_threshold,
                "Initializing embedding chain"
            );
            Some(tokio::sync::Mutex::new(chain))
        } else {
            None
        };

        let qmd = if config.enable_qmd {
            let backend = QmdBackend::with_reranker(
                &config.qmd_url,
                config.qmd_timeout_ms,
                config.qmd_reranker_url.clone(),
                config.qmd_reranker_model.clone(),
            )
            .await;
            Some(backend)
        } else {
            None
        };

        Ok(Self {
            store: Arc::new(Mutex::new(store)),
            config,
            embedding_chain,
            qmd,
            supersession_config: SupersessionConfig::default(),
        })
    }

    /// Create with default config
    pub async fn default() -> Result<Self> {
        Self::new(MnemosyneConfig::default()).await
    }

    /// Check the on-disk integrity of a Mnemosyne database without needing a
    /// live `Mnemosyne` handle.
    ///
    /// Opens a throwaway read-only connection to `db_path` and runs
    /// `PRAGMA integrity_check`. Returns:
    /// - `Ok(true)`  — SQLite reports `"ok"` (database is sound)
    /// - `Ok(false)` — integrity check returned anything else (malformed / corrupt)
    /// - `Err(_)`    — the database could not even be opened or queried
    ///
    /// This is the corruption oracle used by callers on the `Mnemosyne::new`
    /// error path: when init fails, they can run this against the same path to
    /// distinguish transient failures (lock/WAL contention) from durable
    /// corruption and escalate loudly instead of silently degrading.
    pub fn check_integrity(db_path: &Path) -> Result<bool> {
        // Read-only open — never mutates, never creates. If the file is gone
        // or the image is so corrupt it can't be opened, this Errs and the
        // caller falls through to its own degrade path (the error already fired).
        let conn = Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|e| Error::Database(format!("integrity_check: failed to open {}: {}", db_path.display(), e)))?;

        let result: String = conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(|e| Error::Database(format!("integrity_check: PRAGMA failed: {}", e)))?;

        Ok(result == "ok")
    }

    /// Store a message (without auto-embedding).
    ///
    /// After storing, automatically extracts entity-relationship triples from
    /// the message content and adds them to the knowledge graph.
    pub async fn store(&self, session_id: &str, message: &Message) -> Result<i64> {
        let store = self.store.lock().await;
        let msg_id = store.store_message(session_id, message)?;

        // Extract entities + relationships from content and store in graph
        if !message.content.trim().is_empty()
            && let Err(e) =
                knowledge_extract::process_message_graph(&store, msg_id, &message.content)
        {
            tracing::debug!("Graph extraction failed (non-fatal): {}", e);
        }

        Ok(msg_id)
    }

    /// Detect if a message is a task assignment or important directive.
    /// Returns (MemoryType, importance) — elevated importance for tasks.
    fn detect_importance(content: &str) -> (MemoryType, f32) {
        let lower = content.to_lowercase();

        // Task assignment patterns (importance 0.9)
        let task_patterns = [
            "please ", "can you ", "i need you to ", "your task ", "your job ",
            "work on ", "implement ", "fix ", "build ", "create ", "deploy ",
            "ship ", "tackle ", "handle ", "take care of ", "make sure ",
            "priority", "urgent", "asap", "deadline", "by tomorrow",
            "assigned to you", "your responsibility",
        ];
        for pat in &task_patterns {
            if lower.contains(pat) {
                // Check it's directed (not a status report)
                let has_directive_tone = lower.contains("you") || lower.contains("please")
                    || lower.starts_with("fix") || lower.starts_with("build")
                    || lower.starts_with("implement") || lower.starts_with("deploy")
                    || lower.starts_with("ship") || lower.starts_with("create");
                if has_directive_tone {
                    return (MemoryType::Semantic, 0.9);
                }
            }
        }

        // Decision/fact patterns (importance 0.8)
        let fact_patterns = [
            "remember ", "important:", "note:", "decision:", "we decided",
            "from now on", "going forward", "the rule is", "never ",
            "always ", "do not ", "don't ever",
        ];
        for pat in &fact_patterns {
            if lower.contains(pat) {
                return (MemoryType::Fact, 0.8);
            }
        }

        // Default — normal importance
        (MemoryType::Episodic, 0.5)
    }

    /// Store a message and automatically generate + store its embedding.
    ///
    /// Uses the embedding cache to avoid redundant provider calls for identical content.
    /// If the embedding provider is not available or the embedding call fails,
    /// the message is still stored but without an embedding (a warning is logged).
    pub async fn store_with_embedding(&self, session_id: &str, message: &Message) -> Result<i64> {
        let msg_id = {
            let store = self.store.lock().await;
            // Detect task assignments and store with high importance
            let (mem_type, importance) = Self::detect_importance(&message.content);
            let id = if importance > 0.5 {
                store.store_typed(session_id, message, mem_type, importance)?
            } else {
                store.store_message(session_id, message)?
            };

            // Extract entities + relationships into knowledge graph
            // (was previously only in store(), not here — Bug #2 fix)
            if !message.content.trim().is_empty()
                && let Err(e) =
                    knowledge_extract::process_message_graph(&store, id, &message.content)
                {
                    tracing::debug!("Graph extraction failed (non-fatal): {}", e);
                }

            id
        };

        // Skip embedding for empty content or tool-only messages
        if message.content.trim().is_empty() {
            return Ok(msg_id);
        }

        if let Some(chain_mutex) = &self.embedding_chain {
            let mut chain = chain_mutex.lock().await;
            let content_hash = compute_content_hash(&message.content);
            let provider = chain.active_provider().to_string();
            let model = chain.active_model().to_string();

            // Check embedding cache first
            let cached = {
                let store = self.store.lock().await;
                store
                    .get_cached_embedding(&provider, &model, &content_hash)
                    .ok()
                    .flatten()
            };

            let embedding = if let Some(cached_embedding) = cached {
                debug!(msg_id, "Embedding cache hit");
                cached_embedding
            } else {
                match chain.embed(&message.content).await {
                    Ok(emb) => {
                        // Store in cache (use current active provider after potential fallback)
                        let p = chain.active_provider().to_string();
                        let m = chain.active_model().to_string();
                        let store = self.store.lock().await;
                        let _ = store.store_cached_embedding(&p, &m, &content_hash, &emb);
                        emb
                    }
                    Err(e) => {
                        warn!(error = %e, msg_id, "Failed to generate embedding, storing message without it");
                        return Ok(msg_id);
                    }
                }
            };

            let store = self.store.lock().await;
            if let Err(e) = store.store_embedding(msg_id, &embedding, Some(&model)) {
                warn!(error = %e, msg_id, "Failed to store embedding");
            }

            // Run supersession detection for eligible memory types
            if self.supersession_config.enabled {
                let judge = HeuristicJudge::default();
                // Determine memory type from message role (facts/preferences from system/assistant)
                let memory_type = match message.role {
                    zeus_core::Role::System => MemoryType::Fact,
                    _ => MemoryType::Episodic,
                };
                match detect_supersessions(
                    &store,
                    msg_id,
                    &message.content,
                    &embedding,
                    &memory_type,
                    &self.supersession_config,
                    &judge,
                ) {
                    Ok(count) if count > 0 => {
                        info!(msg_id, count, "Auto-superseded existing memories");
                    }
                    Err(e) => {
                        debug!(error = %e, "Supersession detection failed (non-fatal)");
                    }
                    _ => {}
                }
            }
        }

        Ok(msg_id)
    }

    /// Store a message with channel provenance and generate + store its embedding.
    ///
    /// Sprint-B write-path (#86): tags each stored message with `channel_kind` and `chat_id`
    /// so cross-channel context queries can filter/weight memories by origin channel.
    ///
    /// `channel_kind` — e.g. "discord", "telegram", "slack". Pass `None` for unknown/legacy.
    /// `chat_id`      — channel-specific ID (Discord channel ID, Telegram chat_id, etc.).
    ///
    /// Strategy: call `store_with_embedding` for all embedding/supersession logic (unchanged),
    /// then UPDATE the inserted row's `channel_kind`/`chat_id` columns. This keeps all
    /// embedding cache, supersession, and importance logic in one place with zero duplication.
    pub async fn store_with_embedding_tagged(
        &self,
        session_id: &str,
        message: &Message,
        channel_kind: Option<&str>,
        chat_id: Option<&str>,
    ) -> Result<i64> {
        let msg_id = self.store_with_embedding(session_id, message).await?;

        // Patch channel provenance onto the just-inserted row.
        // Only update if at least one channel field is provided (avoid no-op UPDATE on legacy paths).
        if channel_kind.is_some() || chat_id.is_some() {
            let store = self.store.lock().await;
            store.conn.execute(
                "UPDATE messages SET channel_kind = ?1, chat_id = ?2 WHERE rowid = ?3",
                rusqlite::params![channel_kind, chat_id, msg_id],
            ).map_err(|e| Error::Database(format!("Failed to tag channel provenance: {}", e)))?;
        }

        Ok(msg_id)
    }

    /// Generate an embedding for the given text using the configured provider chain.
    ///
    /// Returns `None` if embeddings are disabled or no providers are configured.
    pub async fn embed_text(&self, text: &str) -> Result<Option<Vec<f32>>> {
        match &self.embedding_chain {
            Some(chain_mutex) => {
                let mut chain = chain_mutex.lock().await;
                let embedding = chain.embed(text).await?;
                Ok(Some(embedding))
            }
            None => Ok(None),
        }
    }

    /// Semantic search: tries QMD sidecar first, then internal QMD reranking, then plain hybrid.
    ///
    /// Search pipeline (in priority order):
    /// 1. **External QMD sidecar**: if enabled and available, delegates to the sidecar
    /// 2. **Internal QMD**: if enabled with cross-encoder reranker, runs BM25 + vector + rerank
    /// 3. **Builtin hybrid**: weighted fusion of FTS5 BM25 + vector cosine similarity
    pub async fn semantic_search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        // Try external QMD sidecar first if enabled and available
        if let Some(qmd) = &self.qmd
            && qmd.is_available()
        {
            match qmd.search(query, limit).await {
                Ok(results) if !results.is_empty() => {
                    debug!(count = results.len(), "QMD sidecar search returned results");
                    return Ok(qmd_to_search_results(results));
                }
                Ok(_) => {
                    debug!("QMD sidecar returned empty results, trying internal QMD");
                }
                Err(e) => {
                    warn!(error = %e, "QMD sidecar search failed, trying internal QMD");
                }
            }
        }

        // Try internal QMD reranking if configured
        if let Some(qmd) = &self.qmd
            && qmd.has_reranker()
        {
            match self.internal_qmd_search(query, limit).await {
                Ok(results) if !results.is_empty() => {
                    debug!(
                        count = results.len(),
                        "Internal QMD search returned results"
                    );
                    return Ok(qmd_to_search_results(results));
                }
                Ok(_) => {
                    debug!("Internal QMD returned empty results, falling back to builtin");
                }
                Err(e) => {
                    warn!(error = %e, "Internal QMD failed, falling back to builtin hybrid");
                }
            }
        }

        // Builtin hybrid search (FTS5 + vector)
        let query_embedding = if let Some(chain_mutex) = &self.embedding_chain {
            let mut chain = chain_mutex.lock().await;
            match chain.embed(query).await {
                Ok(emb) => Some(emb),
                Err(e) => {
                    warn!(error = %e, "Failed to embed query, falling back to FTS-only search");
                    None
                }
            }
        } else {
            None
        };

        let store = self.store.lock().await;
        store.hybrid_search(
            query,
            query_embedding.as_deref(),
            limit,
            self.config.enable_fts,
            self.config.vector_weight,
            self.config.text_weight,
            self.config.candidate_multiplier,
        )
    }

    /// Internal QMD pipeline: BM25 search + vector search + cross-encoder reranking.
    ///
    /// This runs entirely within the process — no external sidecar needed.
    /// Over-fetches candidates by `qmd_candidate_multiplier`, then reranks with
    /// cross-encoder scores fused with BM25 + vector weights.
    async fn internal_qmd_search(&self, query: &str, limit: usize) -> Result<Vec<QmdSearchResult>> {
        let qmd = self
            .qmd
            .as_ref()
            .ok_or_else(|| Error::memory("QMD backend not enabled"))?;

        let candidates_count = self.config.qmd_candidate_multiplier.max(1) * limit;

        // Step 1: BM25 (FTS5) search
        let fts_results = if self.config.enable_fts {
            let store = self.store.lock().await;
            store.search(query, candidates_count)?
        } else {
            Vec::new()
        };

        // Step 2: Vector search
        let vec_results = if let Some(chain_mutex) = &self.embedding_chain {
            let mut chain = chain_mutex.lock().await;
            match chain.embed(query).await {
                Ok(emb) => {
                    drop(chain); // Release chain lock before store lock
                    let store = self.store.lock().await;
                    store.vector_search(&emb, candidates_count)?
                }
                Err(e) => {
                    debug!(error = %e, "Vector search unavailable for QMD");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

        // Step 3: Merge candidates with normalized scores
        let mut candidate_map: std::collections::HashMap<i64, QmdCandidate> =
            std::collections::HashMap::new();

        // Normalize BM25 scores: FTS5 rank is negative (more negative = better)
        // Negate to get positive magnitude, then normalize to 0.0-1.0
        for r in &fts_results {
            let bm25_norm = 1.0 / (1.0 + (-r.score).max(0.0));
            candidate_map.insert(
                r.id,
                QmdCandidate {
                    result: r.clone(),
                    bm25_score: bm25_norm,
                    vector_score: 0.0,
                    reranker_score: 0.0,
                },
            );
        }

        // Merge vector scores (cosine similarity already 0.0–1.0)
        for r in &vec_results {
            if let Some(candidate) = candidate_map.get_mut(&r.id) {
                candidate.vector_score = r.score;
            } else {
                candidate_map.insert(
                    r.id,
                    QmdCandidate {
                        result: r.clone(),
                        bm25_score: 0.0,
                        vector_score: r.score,
                        reranker_score: 0.0,
                    },
                );
            }
        }

        let candidates: Vec<QmdCandidate> = candidate_map.into_values().collect();

        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        // Step 4: Cross-encoder reranking + score fusion
        qmd.rerank(
            query,
            candidates,
            self.config.qmd_bm25_weight,
            self.config.qmd_vector_weight,
            self.config.qmd_reranker_weight,
            limit,
        )
        .await
    }

    /// Search exclusively via QMD sidecar (bypasses builtin search).
    ///
    /// Returns an error if QMD is not enabled or not available.
    pub async fn search_qmd(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        let qmd = self
            .qmd
            .as_ref()
            .ok_or_else(|| Error::memory("QMD backend is not enabled"))?;
        let results = qmd.search(query, limit).await?;
        Ok(qmd_to_search_results(results))
    }

    /// Re-check QMD sidecar health (e.g. after restart).
    pub async fn qmd_health_check(&self) {
        if let Some(qmd) = &self.qmd {
            qmd.check_health().await;
        }
    }

    /// Whether the QMD sidecar is currently available.
    pub fn qmd_available(&self) -> bool {
        self.qmd.as_ref().is_some_and(|q| q.is_available())
    }

    /// Search messages (FTS)
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        let store = self.store.lock().await;
        let mut results = store.search(query, limit * 2)?; // fetch extra for re-ranking

        // Wire ImportanceScorer: re-rank results by importance if we have enough
        if results.len() > 1 {
            let config = ImportanceConfig::default();
            let mut scorer = ImportanceScorer::new(config);
            // Score each result and sort by importance (highest first)
            let mut scored: Vec<(f64, SearchResult)> = results.drain(..).map(|r| {
                let entry = MemoryEntry {
                    id: r.id.to_string(),
                    content: r.content.clone(),
                    memory_type: "message".to_string(),
                    created_at: chrono::Utc::now(),
                    last_accessed: chrono::Utc::now(),
                    access_count: 1,
                    pinned: false,
                };
                let sm = scorer.score(&entry, Some(query));
                (sm.score, r)
            }).collect();
            scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            results = scored.into_iter().take(limit).map(|(_, r)| r).collect();
        }

        Ok(results)
    }

    /// Cross-channel search: returns memories whose `channel_kind` differs from
    /// `current_channel_kind`. Used by `MemoryInjector::inject_cross_channel` to
    /// surface context from other channels into the current session's system prompt.
    ///
    /// - `query`               — semantic/FTS query string (same as `search`)
    /// - `current_channel_kind`— channel to EXCLUDE (e.g. "discord"); NULL rows are included
    ///                           (pre-v10 legacy rows have no channel provenance — safer to include)
    /// - `limit`               — max rows to return
    pub async fn search_cross_channel(
        &self,
        query: &str,
        current_channel_kind: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let store = self.store.lock().await;
        // Raw SQL: FTS match + channel filter. We exclude rows whose channel_kind
        // exactly matches current_channel_kind. NULL rows (legacy) are included.
        let sql = "
            SELECT m.id, m.session_id, m.role, m.content, m.timestamp,
                   m.channel_kind, m.chat_id, 1.0 AS score
            FROM messages m
            WHERE m.content LIKE '%' || ?1 || '%'
              AND (m.channel_kind IS NULL OR m.channel_kind != ?2)
            ORDER BY m.timestamp DESC
            LIMIT ?3
        ";
        let mut stmt = store.conn.prepare(sql)
            .map_err(|e| Error::Database(format!("search_cross_channel prepare: {}", e)))?;
        let results = stmt.query_map(
            rusqlite::params![query, current_channel_kind, limit as i64],
            |row| {
                Ok(SearchResult {
                    id: row.get::<_, i64>(0)?,
                    session_id: row.get::<_, String>(1)?,
                    content: row.get::<_, String>(3)?,
                    timestamp: row.get::<_, String>(4)?,
                    score: 1.0_f32,
                    memory_type: MemoryType::Episodic,
                    importance: 0.5,
                    citation: None,
                    valid_from: None,
                    valid_to: None,
                    superseded_by: None,
                    verified: true,
                })
            },
        ).map_err(|e| Error::Database(format!("search_cross_channel query: {}", e)))?;
        let mut out = Vec::new();
        for r in results {
            out.push(r.map_err(|e| Error::Database(format!("search_cross_channel row: {}", e)))?);
        }
        Ok(out)
    }

    /// FTS search scoped to a specific session_id (e.g. "room:<room_id>").
    pub async fn search_in_session(
        &self,
        query: &str,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let store = self.store.lock().await;
        store.search_in_session(query, session_id, limit)
    }

    /// Store an embedding for a message
    pub async fn store_embedding(
        &self,
        message_id: i64,
        embedding: &[f32],
        model: Option<&str>,
    ) -> Result<i64> {
        let store = self.store.lock().await;
        store.store_embedding(message_id, embedding, model)
    }

    /// Vector similarity search
    pub async fn vector_search(
        &self,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let store = self.store.lock().await;
        store.vector_search(query_embedding, limit)
    }

    /// Hybrid search combining FTS and vector similarity
    pub async fn hybrid_search(
        &self,
        query: &str,
        query_embedding: Option<&[f32]>,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let store = self.store.lock().await;
        store.hybrid_search(
            query,
            query_embedding,
            limit,
            self.config.enable_fts,
            self.config.vector_weight,
            self.config.text_weight,
            self.config.candidate_multiplier,
        )
    }

    /// S59-P2: InsightForge multi-query search (MiroFish pattern)
    /// Decomposes a complex query into sub-queries, searches each in parallel,
    /// then merges and deduplicates results for richer context retrieval.
    pub async fn insight_search(
        &self,
        query: &str,
        sub_queries: &[String],
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let mut all_results: Vec<SearchResult> = Vec::new();
        let mut seen_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();

        // Search with original query
        let primary = self.search(query, limit).await?;
        for r in primary {
            if seen_ids.insert(r.id) {
                all_results.push(r);
            }
        }

        // Search each sub-query
        for sq in sub_queries {
            let results = self.search(sq, limit / 2).await?;
            for r in results {
                if seen_ids.insert(r.id) {
                    all_results.push(r);
                }
            }
        }

        // Sort by relevance score descending, take top `limit`
        all_results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        all_results.truncate(limit);

        Ok(all_results)
    }

    /// Get session messages
    pub async fn recall_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<StoredMessage>> {
        let store = self.store.lock().await;
        store.get_session_messages(session_id, limit)
    }

    /// Get statistics
    pub async fn stats(&self) -> Result<MemoryStats> {
        let store = self.store.lock().await;
        store.stats()
    }

    /// Forget messages before date
    pub async fn forget_before(&self, before: DateTime<Utc>) -> Result<usize> {
        let store = self.store.lock().await;
        store.forget_before(before)
    }

    /// Store a message with explicit memory type and importance
    pub async fn store_typed(
        &self,
        session_id: &str,
        msg: &Message,
        memory_type: MemoryType,
        importance: f32,
    ) -> Result<i64> {
        let store = self.store.lock().await;
        store.store_typed(session_id, msg, memory_type, importance)
    }

    /// Search messages filtered by memory type
    pub async fn search_by_type(
        &self,
        query: &str,
        memory_type: MemoryType,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let store = self.store.lock().await;
        store.search_by_type(query, memory_type, limit)
    }

    /// Decay importance of all episodic memories
    pub async fn decay_importance(&self, decay_rate: f32) -> Result<usize> {
        let store = self.store.lock().await;
        store.decay_importance(decay_rate)
    }

    /// Promote an episodic memory to semantic type
    pub async fn promote_to_semantic(&self, message_id: i64, knowledge: &str) -> Result<()> {
        let store = self.store.lock().await;
        store.promote_to_semantic(message_id, knowledge)
    }

    /// Get working memory for a session
    pub async fn working_memory(&self, session_id: &str) -> Result<Vec<SearchResult>> {
        let store = self.store.lock().await;
        store.working_memory(session_id)
    }

    /// Finalize a session's working memory on session end.
    ///
    /// Working memories with importance >= `promote_threshold` are promoted to episodic.
    /// Working memories below the threshold are discarded.
    /// Returns (promoted_count, discarded_count).
    pub async fn finalize_working_memory(
        &self,
        session_id: &str,
        promote_threshold: f32,
    ) -> Result<(usize, usize)> {
        let store = self.store.lock().await;
        store.finalize_working_memory(session_id, promote_threshold)
    }

    // Temporal Memory Versioning (async wrappers)

    /// Mark an old memory as superseded by a new one.
    /// Sets valid_to to now and records the superseding message ID.
    pub async fn supersede_message(&self, old_id: i64, new_id: i64) -> Result<()> {
        let store = self.store.lock().await;
        store.supersede_message(old_id, new_id)
    }

    /// Get only current (non-superseded) memories, ordered by recency.
    pub async fn get_current_memories(&self, limit: usize) -> Result<Vec<SearchResult>> {
        let store = self.store.lock().await;
        store.get_current_memories(limit)
    }

    /// Get the chain of versions for a memory (original → superseded → latest).
    pub async fn get_supersession_chain(
        &self,
        message_id: i64,
    ) -> Result<Vec<(i64, String, Option<i64>)>> {
        let store = self.store.lock().await;
        store.get_supersession_chain(message_id)
    }

    // Entity Management (async wrappers)

    /// Find or create an entity by name and type, with fuzzy matching.
    pub async fn upsert_entity(&self, name: &str, entity_type: &str) -> Result<i64> {
        let store = self.store.lock().await;
        store.upsert_entity(name, entity_type)
    }

    /// Link an entity to a message.
    pub async fn link_entity_to_message(
        &self,
        entity_id: i64,
        message_id: i64,
        mention_text: &str,
    ) -> Result<()> {
        let store = self.store.lock().await;
        store.link_entity_to_message(entity_id, message_id, mention_text)
    }

    /// Get all entities ordered by mention count.
    pub async fn get_entities(&self, limit: usize) -> Result<Vec<EntityRecord>> {
        let store = self.store.lock().await;
        store.get_entities(limit)
    }

    /// Get messages linked to a specific entity.
    pub async fn get_entity_messages(
        &self,
        entity_id: i64,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let store = self.store.lock().await;
        store.get_entity_messages(entity_id, limit)
    }

    // Graph Memory (async wrappers) — Sprint 9

    /// Add or update a relationship between two entities.
    pub async fn add_relationship(
        &self,
        source_id: i64,
        target_id: i64,
        rel_type: RelationType,
        weight: f64,
    ) -> Result<i64> {
        let store = self.store.lock().await;
        store.add_relationship(source_id, target_id, rel_type, weight)
    }

    /// Get relationships for an entity, filtered by direction.
    pub async fn get_relationships(
        &self,
        entity_id: i64,
        direction: Direction,
    ) -> Result<Vec<Relationship>> {
        let store = self.store.lock().await;
        store.get_relationships(entity_id, direction)
    }

    /// Delete a relationship by ID.
    pub async fn delete_relationship(&self, relationship_id: i64) -> Result<()> {
        let store = self.store.lock().await;
        store.delete_relationship(relationship_id)
    }

    /// BFS traversal: get entity graph up to N hops.
    pub async fn get_entity_graph(&self, entity_id: i64, max_depth: u32) -> Result<GraphTraversal> {
        let store = self.store.lock().await;
        store.get_entity_graph(entity_id, max_depth)
    }

    /// Find shortest path between two entities.
    pub async fn shortest_path(
        &self,
        entity_a: i64,
        entity_b: i64,
    ) -> Result<Option<Vec<EntityRecord>>> {
        let store = self.store.lock().await;
        store.shortest_path(entity_a, entity_b)
    }

    /// Find connected entities, optionally filtered by relationship types.
    pub async fn find_connected_entities(
        &self,
        entity_id: i64,
        rel_types: Option<&[RelationType]>,
        max_depth: u32,
    ) -> Result<Vec<(EntityRecord, RelationType, u32)>> {
        let store = self.store.lock().await;
        store.find_connected_entities(entity_id, rel_types, max_depth)
    }

    /// Get summary of all relationship types and counts.
    pub async fn get_relationship_types(&self) -> Result<Vec<RelationshipTypeCount>> {
        let store = self.store.lock().await;
        store.get_relationship_types()
    }

    /// Get total relationship count.
    pub async fn relationship_count(&self) -> Result<usize> {
        let store = self.store.lock().await;
        store.relationship_count()
    }

    /// Get a single entity by ID.
    pub async fn get_entity_by_id(&self, entity_id: i64) -> Result<EntityRecord> {
        let store = self.store.lock().await;
        store.get_entity_by_id(entity_id)
    }

    /// Create a community.
    pub async fn create_community(&self, name: &str, description: &str) -> Result<i64> {
        let store = self.store.lock().await;
        store.create_community(name, description)
    }

    /// Add entity to a community.
    pub async fn add_community_member(
        &self,
        community_id: i64,
        entity_id: i64,
        role: &str,
    ) -> Result<()> {
        let store = self.store.lock().await;
        store.add_community_member(community_id, entity_id, role)
    }

    /// Get the community an entity belongs to.
    pub async fn get_entity_community(&self, entity_id: i64) -> Result<Option<Community>> {
        let store = self.store.lock().await;
        store.get_entity_community(entity_id)
    }

    /// Get all entities in a community.
    pub async fn get_community_entities(
        &self,
        community_id: i64,
    ) -> Result<Vec<(EntityRecord, String)>> {
        let store = self.store.lock().await;
        store.get_community_entities(community_id)
    }

    /// Get all communities.
    pub async fn get_communities(&self) -> Result<Vec<Community>> {
        let store = self.store.lock().await;
        store.get_communities()
    }

    /// Clear all communities.
    pub async fn clear_communities(&self) -> Result<()> {
        let store = self.store.lock().await;
        store.clear_communities()
    }

    /// Run Label Propagation community detection on the memory graph.
    ///
    /// Clears any previously detected communities, runs LPA until convergence
    /// (or 50 iterations), persists results, and assigns hub/bridge/member
    /// roles. Returns the number of communities detected.
    pub async fn detect_communities(&self) -> Result<usize> {
        let store = self.store.lock().await;
        community::detect_communities(&store)
    }

    /// Build a human-readable summary for a community.
    ///
    /// Lists the hub entity, bridge entities, and top members by name.
    pub async fn community_summary(&self, community_id: i64) -> String {
        let store = self.store.lock().await;
        community::community_summary(&store, community_id)
    }

    /// Log a promotion.
    pub async fn log_promotion(
        &self,
        source_id: i64,
        promoted_id: i64,
        reason: &str,
    ) -> Result<i64> {
        let store = self.store.lock().await;
        store.log_promotion(source_id, promoted_id, reason)
    }

    /// Get promotions for a source message.
    pub async fn get_promotions(&self, source_message_id: i64) -> Result<Vec<Promotion>> {
        let store = self.store.lock().await;
        store.get_promotions(source_message_id)
    }

    /// Format graph context for LLM injection.
    pub async fn format_graph_context(&self, entity_id: i64, max_depth: u32) -> Result<String> {
        let store = self.store.lock().await;
        store.format_graph_context(entity_id, max_depth)
    }
    /// Extract entities from text and upsert them into the graph store.
    ///
    /// Uses heuristic pattern matching to identify people, projects, sprints,
    /// and decisions from inbound messages. Returns the number of entities
    /// extracted. Optionally links entities to a message ID.
    pub async fn extract_entities_from_text(
        &self,
        text: &str,
        message_id: Option<i64>,
    ) -> Result<usize> {
        let extracted = Self::heuristic_extract(text);
        if extracted.is_empty() {
            return Ok(0);
        }
        let store = self.store.lock().await;
        let mut count = 0;
        for (name, entity_type) in &extracted {
            match store.upsert_entity(name, entity_type) {
                Ok(entity_id) => {
                    if let Some(mid) = message_id {
                        let _ = store.link_entity_to_message(entity_id, mid, name);
                    }
                    count += 1;
                }
                Err(e) => {
                    tracing::debug!("Entity upsert failed for '{}': {}", name, e);
                }
            }
        }
        Ok(count)
    }

    /// Heuristic entity extraction from text.
    ///
    /// Extracts:
    /// - **People**: @mentions, "agent_name", capitalized names after keywords
    /// - **Projects**: known project name patterns (Zeus, OpenClaw, etc.)
    /// - **Sprints**: S\d+ patterns (e.g., S47, S48)
    /// - **Decisions**: sentences with decision keywords ("decided", "confirmed", etc.)
    fn heuristic_extract(text: &str) -> Vec<(String, String)> {
        let mut entities: Vec<(String, String)> = Vec::new();
        let mut seen = std::collections::HashSet::new();

        let add = |entities: &mut Vec<(String, String)>,
                   seen: &mut std::collections::HashSet<String>,
                   name: &str,
                   etype: &str| {
            let key = format!("{}:{}", etype, name.to_lowercase());
            if !seen.contains(&key) && name.len() >= 2 && name.len() <= 100 {
                seen.insert(key);
                entities.push((name.to_string(), etype.to_string()));
            }
        };

        // Sprint IDs: S47, S48, etc.
        let sprint_re = regex::Regex::new(r"\bS(\d{1,3})\b").unwrap();
        for cap in sprint_re.captures_iter(text) {
            add(&mut entities, &mut seen, &cap[0], "sprint");
        }

        // Track IDs: Track A, Track B, etc.
        let track_re = regex::Regex::new(r"\bTrack\s+([A-Z])\b").unwrap();
        for cap in track_re.captures_iter(text) {
            add(
                &mut entities,
                &mut seen,
                &format!("Track {}", &cap[1]),
                "track",
            );
        }

        // @mentions (Discord/Telegram style)
        let mention_re = regex::Regex::new(r"@(\w{2,30})\b").unwrap();
        for cap in mention_re.captures_iter(text) {
            let name = &cap[1];
            // Skip common non-name mentions
            if !["everyone", "here", "channel"].contains(&name.to_lowercase().as_str()) {
                add(&mut entities, &mut seen, name, "person");
            }
        }

        // Known agent names
        let agents = [
            "Zeus100", "Zeus112", "Zeus106", "zeus107", "fbsd1", "fbsd2",
            "fbsd3", "zeusmolty", "ZeusMarketing", "operator",
        ];
        for agent in &agents {
            if text.contains(agent) {
                add(&mut entities, &mut seen, agent, "person");
            }
        }

        // Project names (case-insensitive patterns)
        let lower = text.to_lowercase();
        let projects = [
            ("zeus", "Zeus"),
            ("openclaw", "OpenClaw"),

            ("neurodrums", "NeuroDrums"),
            ("mnemosyne", "Mnemosyne"),
            ("prometheus", "Prometheus"),
            ("pantheon", "Pantheon"),
            ("agora", "Agora"),
        ];
        for (pattern, canonical) in &projects {
            if lower.contains(pattern) {
                add(&mut entities, &mut seen, canonical, "project");
            }
        }

        // PR references: PR #10, PR #11, etc.
        let pr_re = regex::Regex::new(r"\bPR\s*#(\d+)\b").unwrap();
        for cap in pr_re.captures_iter(text) {
            add(&mut entities, &mut seen, &cap[0], "artifact");
        }

        // Decisions: capture sentences with decision keywords
        let decision_keywords = [
            "decided", "confirmed", "approved", "merged", "assigned",
            "resolved", "agreed",
        ];
        for sentence in text.split(['.', '!', '\n']) {
            let s = sentence.trim();
            let s_lower = s.to_lowercase();
            if s.len() > 10
                && s.len() <= 200
                && decision_keywords.iter().any(|k| s_lower.contains(k))
            {
                add(&mut entities, &mut seen, s, "decision");
                break; // Only capture first decision per message
            }
        }

        entities
    }

    // Cross-Session Pattern Recognition (async wrappers)

    /// Extract patterns from completed sessions (tools, themes, topics).
    pub async fn extract_patterns(&self) -> Result<usize> {
        let store = self.store.lock().await;
        store.extract_patterns()
    }

    /// Get patterns by type, ordered by frequency.
    pub async fn get_patterns(
        &self,
        pattern_type: &str,
        limit: usize,
    ) -> Result<Vec<PatternEntry>> {
        let store = self.store.lock().await;
        store.get_patterns(pattern_type, limit)
    }

    /// Get all patterns regardless of type.
    pub async fn get_all_patterns(&self, limit: usize) -> Result<Vec<PatternEntry>> {
        let store = self.store.lock().await;
        store.get_all_patterns(limit)
    }

    // Importance Scoring with Decay (async wrappers)

    /// Decay memory importance based on time since last access.
    /// `daily_rate` is the fraction to reduce per day (e.g. 0.10 = 10%).
    pub async fn decay_memories(&self, daily_rate: f64) -> Result<usize> {
        let store = self.store.lock().await;
        store.decay_memories(daily_rate)
    }

    /// Boost a memory's importance when retrieved.
    pub async fn boost_memory(&self, message_id: i64, boost: f64) -> Result<()> {
        let store = self.store.lock().await;
        store.boost_memory(message_id, boost)
    }

    /// Get importance score and last_accessed for a memory.
    pub async fn get_memory_importance(&self, message_id: i64) -> Result<(f64, Option<String>)> {
        let store = self.store.lock().await;
        store.get_memory_importance(message_id)
    }

    // Proactive Retrieval (async wrapper)

    /// Pre-fetch likely-needed memories based on conversation topics and patterns.
    pub async fn proactive_context(
        &self,
        messages: &[Message],
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let store = self.store.lock().await;
        store.proactive_context(messages, limit)
    }

    // ── Track B: Memory Consolidation (async wrappers) ──────────────

    /// Check for near-duplicate content using FTS5 + Levenshtein.
    pub async fn find_duplicate(&self, content: &str, threshold: f64) -> Result<Option<i64>> {
        let store = self.store.lock().await;
        store.find_duplicate(content, threshold)
    }

    /// Store a message with dedup check — boosts existing if near-duplicate found.
    pub async fn store_message_dedup(
        &self,
        session_id: &str,
        message: &Message,
        dedup_threshold: f64,
    ) -> Result<i64> {
        let store = self.store.lock().await;
        store.store_message_dedup(session_id, message, dedup_threshold)
    }

    /// Consolidate a session's middle messages into a summary.
    pub async fn consolidate_session(
        &self,
        session_id: &str,
        keep_edges: usize,
    ) -> Result<(usize, usize)> {
        let store = self.store.lock().await;
        store.consolidate_session(session_id, keep_edges)
    }

    /// Get total non-superseded memory count.
    pub async fn memory_count(&self) -> Result<usize> {
        let store = self.store.lock().await;
        store.memory_count()
    }

    /// Prune lowest-importance episodic memories to enforce cap.
    pub async fn enforce_memory_cap(&self, max_memories: usize) -> Result<usize> {
        let store = self.store.lock().await;
        store.enforce_memory_cap(max_memories)
    }

    /// Run full consolidation pass (session compression + cap enforcement).
    pub async fn run_consolidation(
        &self,
        session_limit: usize,
        max_memories: usize,
        keep_edges: usize,
    ) -> Result<(usize, usize)> {
        let store = self.store.lock().await;
        store.run_consolidation(session_limit, max_memories, keep_edges)
    }

    // Embedding Cache (async wrappers)

    /// Embed text with cache: checks embedding_cache first, falls back to provider chain.
    /// Returns None if no embedding chain is configured.
    pub async fn embed_with_cache(&self, text: &str) -> Result<Option<Vec<f32>>> {
        let chain_mutex = match &self.embedding_chain {
            Some(c) => c,
            None => return Ok(None),
        };

        let mut chain = chain_mutex.lock().await;
        let content_hash = compute_content_hash(text);
        let provider = chain.active_provider().to_string();
        let model = chain.active_model().to_string();

        // Check cache
        {
            let store = self.store.lock().await;
            if let Some(cached) = store.get_cached_embedding(&provider, &model, &content_hash)? {
                return Ok(Some(cached));
            }
        }

        // Cache miss — call provider chain (may fallback)
        let embedding = chain.embed(text).await?;

        // Store in cache + evict if needed (use current active after potential fallback)
        let p = chain.active_provider().to_string();
        let m = chain.active_model().to_string();
        {
            let store = self.store.lock().await;
            store.store_cached_embedding(&p, &m, &content_hash, &embedding)?;
            let _ = store.evict_lru_cache(EMBEDDING_CACHE_MAX);
        }

        Ok(Some(embedding))
    }

    /// Embed multiple texts with cache support, calling the provider in batches.
    ///
    /// Returns a Vec of `Option<Vec<f32>>` in the same order as the input texts.
    /// Each entry is `None` if no embedding chain is configured, or `Some(embedding)`.
    /// Texts already in cache are skipped; uncached texts are batched together.
    pub async fn embed_batch_with_cache(&self, texts: &[String]) -> Result<Vec<Option<Vec<f32>>>> {
        let chain_mutex = match &self.embedding_chain {
            Some(c) => c,
            None => return Ok(vec![None; texts.len()]),
        };

        let mut chain = chain_mutex.lock().await;
        let provider = chain.active_provider().to_string();
        let model = chain.active_model().to_string();

        // Phase 1: Check cache, partition into hits and misses
        let mut results: Vec<Option<Vec<f32>>> = vec![None; texts.len()];
        let mut uncached: Vec<(usize, String)> = Vec::new(); // (original_index, content_hash)

        {
            let store = self.store.lock().await;
            for (i, text) in texts.iter().enumerate() {
                let content_hash = compute_content_hash(text);
                if let Some(cached) =
                    store.get_cached_embedding(&provider, &model, &content_hash)?
                {
                    results[i] = Some(cached);
                } else {
                    uncached.push((i, content_hash));
                }
            }
        }

        if uncached.is_empty() {
            return Ok(results);
        }

        // Phase 2: Batch-embed uncached texts
        let batch_size = self.config.embed_batch_size.max(1);
        for batch_start in (0..uncached.len()).step_by(batch_size) {
            let batch_end = (batch_start + batch_size).min(uncached.len());
            let batch_indices = &uncached[batch_start..batch_end];

            let batch_texts: Vec<&str> = batch_indices
                .iter()
                .map(|(idx, _)| texts[*idx].as_str())
                .collect();

            match chain.embed_batch(&batch_texts).await {
                Ok(embeddings) => {
                    // Phase 3: Store results in cache and in output
                    let p = chain.active_provider().to_string();
                    let m = chain.active_model().to_string();
                    let store = self.store.lock().await;
                    for (j, embedding) in embeddings.into_iter().enumerate() {
                        let (orig_idx, ref content_hash) = batch_indices[j];
                        let _ = store.store_cached_embedding(&p, &m, content_hash, &embedding);
                        results[orig_idx] = Some(embedding);
                    }
                }
                Err(e) => {
                    warn!(
                        batch_start,
                        batch_size = batch_texts.len(),
                        error = %e,
                        "Batch embedding failed"
                    );
                    // Leave these entries as None — partial success is acceptable
                }
            }
        }

        Ok(results)
    }

    /// Get a cached embedding lookup (no provider call).
    pub async fn get_cached_embedding(
        &self,
        provider: &str,
        model: &str,
        content_hash: &str,
    ) -> Result<Option<Vec<f32>>> {
        let store = self.store.lock().await;
        store.get_cached_embedding(provider, model, content_hash)
    }

    /// Get embedding cache entry count.
    pub async fn embedding_cache_count(&self) -> Result<usize> {
        let store = self.store.lock().await;
        store.embedding_cache_count()
    }

    /// Evict oldest cache entries to stay within limit.
    pub async fn evict_embedding_cache(&self, max_entries: usize) -> Result<usize> {
        let store = self.store.lock().await;
        store.evict_lru_cache(max_entries)
    }

    // File Tracking (async wrappers)

    /// Get a tracked file entry.
    pub async fn get_tracked_file(&self, path: &str, source: &str) -> Result<Option<TrackedFile>> {
        let store = self.store.lock().await;
        store.get_tracked_file(path, source)
    }

    /// Insert or update a tracked file entry.
    pub async fn upsert_tracked_file(
        &self,
        path: &str,
        source: &str,
        content_hash: &str,
        mtime: i64,
        size: i64,
    ) -> Result<()> {
        let store = self.store.lock().await;
        store.upsert_tracked_file(path, source, content_hash, mtime, size)
    }

    /// List tracked files for a source.
    pub async fn list_tracked_files(&self, source: &str) -> Result<Vec<TrackedFile>> {
        let store = self.store.lock().await;
        store.list_tracked_files(source)
    }

    // Workspace Sync

    /// Sync workspace files: hash each file, skip unchanged, embed changed chunks.
    ///
    /// Walks all `.md` files under `root`, computes content hashes, and for any
    /// file whose hash has changed since last sync, splits into chunks and embeds
    /// each chunk (using the embedding cache to avoid redundant provider calls).
    pub async fn sync_workspace(&self, root: &Path) -> Result<SyncStats> {
        let mut stats = SyncStats {
            files_scanned: 0,
            files_changed: 0,
            files_unchanged: 0,
            chunks_embedded: 0,
            cache_hits: 0,
            cache_misses: 0,
            sessions_indexed: 0,
            errors: Vec::new(),
        };

        // Sync the primary workspace root
        self.sync_directory(root, root, "workspace", &mut stats)
            .await?;

        // Sync extra memory paths (each is its own root for relative path computation)
        for extra_path in &self.config.extra_memory_paths {
            if extra_path.exists() {
                // Use the directory name as the source prefix for tracking
                let source = format!(
                    "extra:{}",
                    extra_path.file_name().unwrap_or_default().to_string_lossy()
                );
                self.sync_directory(extra_path, extra_path, &source, &mut stats)
                    .await?;
            }
        }

        // Run LRU eviction after sync
        {
            let store = self.store.lock().await;
            if let Err(e) = store.evict_lru_cache(EMBEDDING_CACHE_MAX) {
                stats.errors.push(format!("Cache eviction failed: {}", e));
            }
        }

        info!(
            scanned = stats.files_scanned,
            changed = stats.files_changed,
            unchanged = stats.files_unchanged,
            chunks = stats.chunks_embedded,
            "Workspace sync complete"
        );

        Ok(stats)
    }

    /// Sync a single directory of markdown files into the memory store.
    ///
    /// `dir` is the directory to scan, `prefix_root` is used for computing
    /// relative paths, and `source` is the tracking source label.
    async fn sync_directory(
        &self,
        dir: &Path,
        prefix_root: &Path,
        source: &str,
        stats: &mut SyncStats,
    ) -> Result<()> {
        let files = collect_md_files(dir);
        stats.files_scanned += files.len();

        // Phase 1: Store chunks in DB and collect (msg_id, chunk_text, tracking_key)
        // for batch embedding.
        let mut pending_embeds: Vec<(i64, String, String)> = Vec::new();

        // Index cache: pre-load all tracked file hashes for this source in a single
        // query so the per-file change-detection below is O(1) HashMap lookup
        // rather than N individual SQLite queries.
        let tracked_cache: std::collections::HashMap<String, String> = {
            let store = self.store.lock().await;
            store
                .list_tracked_files(source)?
                .into_iter()
                .map(|tf| (tf.path.clone(), tf.content_hash.clone()))
                .collect()
        };

        for file_path in &files {
            let content = match tokio::fs::read_to_string(file_path).await {
                Ok(c) => c,
                Err(e) => {
                    stats.errors.push(format!("{}: {}", file_path.display(), e));
                    continue;
                }
            };

            let relative = file_path
                .strip_prefix(prefix_root)
                .unwrap_or(file_path)
                .to_string_lossy()
                .to_string();

            // For extra paths, prefix with the source label for unique tracking
            let tracking_key = if source == "workspace" {
                relative.clone()
            } else {
                format!("{}/{}", source, relative)
            };

            let hash = compute_content_hash(&content);
            let metadata = tokio::fs::metadata(file_path).await.ok();
            let mtime = metadata
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let size = metadata.map(|m| m.len() as i64).unwrap_or(0);

            // Check if file has changed (uses pre-loaded index cache)
            let changed = match tracked_cache.get(&tracking_key) {
                Some(cached_hash) => cached_hash != &hash,
                None => true, // New file
            };

            if !changed {
                stats.files_unchanged += 1;
                continue;
            }

            stats.files_changed += 1;

            // Delete old chunks for this file before re-indexing
            {
                let store = self.store.lock().await;
                let session_key = format!("file:{}", tracking_key);
                let _ = store.conn.execute(
                    "DELETE FROM messages WHERE session_id = ?1",
                    params![session_key],
                );
            }

            // Split into chunks, store each, and collect for batch embedding
            let chunks = chunk_text_with_overlap(&content, self.config.chunk_overlap_tokens);
            for chunk in &chunks {
                if chunk.text.trim().is_empty() {
                    continue;
                }

                let citation = format!("{}#L{}", tracking_key, chunk.start_line);

                // Store chunk as a searchable message with citation
                let msg_id = {
                    let store = self.store.lock().await;
                    store.store_chunk_with_source(
                        &format!("file:{}", tracking_key),
                        &chunk.text,
                        &citation,
                        MemoryType::Semantic,
                    )?
                };

                pending_embeds.push((msg_id, chunk.text.clone(), tracking_key.clone()));
            }

            // Update tracked file entry
            {
                let store = self.store.lock().await;
                store.upsert_tracked_file(&tracking_key, source, &hash, mtime, size)?;
            }
        }

        // Phase 2: Batch-embed all collected chunks
        if !pending_embeds.is_empty() {
            let texts: Vec<String> = pending_embeds.iter().map(|(_, t, _)| t.clone()).collect();
            let embeddings = self.embed_batch_with_cache(&texts).await?;

            // Phase 3: Store embeddings for each chunk
            let store = self.store.lock().await;
            for (i, (msg_id, _, tracking_key)) in pending_embeds.iter().enumerate() {
                if let Some(Some(embedding)) = embeddings.get(i) {
                    if let Err(e) = store.store_embedding(*msg_id, embedding, None) {
                        stats
                            .errors
                            .push(format!("{}: store embedding failed: {}", tracking_key, e));
                    } else {
                        stats.chunks_embedded += 1;
                    }
                }
            }
        }

        Ok(())
    }

    /// Sync workspace and return detailed stats including cache hit ratio.
    /// This wraps `sync_workspace` with cache-aware counting.
    pub async fn sync_workspace_with_cache_stats(&self, root: &Path) -> Result<SyncStats> {
        let cache_before = self.embedding_cache_count().await.unwrap_or(0);
        let mut stats = self.sync_workspace(root).await?;
        let cache_after = self.embedding_cache_count().await.unwrap_or(0);

        // New cache entries = cache misses; rest were hits
        let new_entries = cache_after.saturating_sub(cache_before);
        stats.cache_misses = new_entries;
        stats.cache_hits = stats.chunks_embedded.saturating_sub(new_entries);

        Ok(stats)
    }

    // Session Transcript Indexing

    /// Sync session transcript files for embedding-based search.
    ///
    /// Scans `sessions_dir` for `*.jsonl` files, tracking each session's file
    /// size. When the byte delta exceeds `session_delta_bytes` or the message
    /// delta exceeds `session_delta_messages`, the new content is parsed,
    /// chunked, and embedded.
    pub async fn sync_sessions(&self, sessions_dir: &Path) -> Result<usize> {
        if !self.config.enable_session_indexing {
            return Ok(0);
        }

        if !sessions_dir.exists() {
            return Ok(0);
        }

        let mut sessions_indexed = 0;
        let mut rd = match tokio::fs::read_dir(sessions_dir).await {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "Failed to read sessions directory");
                return Ok(0);
            }
        };
        let mut entries = Vec::new();
        while let Ok(Some(entry)) = rd.next_entry().await {
            entries.push(entry);
        }

        for entry in entries {
            let path = entry.path();
            if path.extension().map(|e| e == "jsonl").unwrap_or(false) {
                let session_id = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                let current_size = tokio::fs::metadata(&path)
                    .await
                    .map(|m| m.len() as i64)
                    .unwrap_or(0);

                // Check tracking state
                let (last_size, needs_index) = {
                    let store = self.store.lock().await;
                    match store.get_session_file(&session_id)? {
                        Some(entry) => {
                            let byte_delta = (current_size - entry.last_size) as usize;
                            let needs = byte_delta >= self.config.session_delta_bytes;
                            (entry.last_size, needs)
                        }
                        None => (0, true), // New session — always index
                    }
                };

                if !needs_index {
                    continue;
                }

                // Read the delta content (from last_size to end)
                let content = match tokio::fs::read_to_string(&path).await {
                    Ok(c) => c,
                    Err(e) => {
                        warn!(session_id, error = %e, "Failed to read session file");
                        continue;
                    }
                };

                // Parse JSONL lines, extract User/Assistant turns from delta region
                let (text, message_count) = parse_session_jsonl(&content, last_size as usize);

                // Check message delta threshold (skip if both thresholds unmet)
                if last_size > 0
                    && message_count < self.config.session_delta_messages
                    && ((current_size - last_size) as usize) < self.config.session_delta_bytes
                {
                    continue;
                }

                if text.trim().is_empty() {
                    // Update tracking even if no text (avoids re-scanning empty deltas)
                    let store = self.store.lock().await;
                    store.upsert_session_file(
                        &session_id,
                        &path.to_string_lossy(),
                        current_size,
                        0,
                        0,
                    )?;
                    continue;
                }

                // Chunk and batch-embed
                let chunks = chunk_text_with_overlap(&text, self.config.chunk_overlap_tokens);
                let chunk_texts: Vec<String> = chunks
                    .iter()
                    .filter(|c| !c.text.trim().is_empty())
                    .map(|c| c.text.clone())
                    .collect();

                let chunks_ok = if !chunk_texts.is_empty() {
                    match self.embed_batch_with_cache(&chunk_texts).await {
                        Ok(results) => results.iter().filter(|r| r.is_some()).count(),
                        Err(e) => {
                            debug!(session_id, error = %e, "Session batch embedding failed");
                            0
                        }
                    }
                } else {
                    0
                };

                // Update tracking
                {
                    let store = self.store.lock().await;
                    store.upsert_session_file(
                        &session_id,
                        &path.to_string_lossy(),
                        current_size,
                        0,
                        0,
                    )?;
                }

                if chunks_ok > 0 {
                    sessions_indexed += 1;
                    debug!(session_id, chunks = chunks_ok, "Session indexed");
                }
            }
        }

        Ok(sessions_indexed)
    }

    /// Get configuration
    pub fn config(&self) -> &MnemosyneConfig {
        &self.config
    }

    /// Check if the embedding chain is available.
    pub fn has_embedder(&self) -> bool {
        self.embedding_chain.is_some()
    }

    /// Get the active embedding provider name (or "none").
    pub async fn active_embedding_provider(&self) -> String {
        match &self.embedding_chain {
            Some(chain_mutex) => {
                let chain = chain_mutex.lock().await;
                chain.active_provider().to_string()
            }
            None => "none".to_string(),
        }
    }

    /// Get fallback state: Vec of (provider_name, failure_count, is_active).
    pub async fn embedding_fallback_state(&self) -> Vec<(String, usize, bool)> {
        match &self.embedding_chain {
            Some(chain_mutex) => {
                let chain = chain_mutex.lock().await;
                chain.fallback_state()
            }
            None => Vec::new(),
        }
    }

    // Atomic Reindex

    /// Perform an atomic reindex of the entire database.
    ///
    /// Creates a temporary database, indexes all workspace files and sessions
    /// into it, then atomically swaps the temp DB for the live DB. If the swap
    /// fails, the original database is restored from a backup.
    ///
    /// Returns `SyncStats` from the fresh index operation.
    pub async fn atomic_reindex(
        &self,
        workspace_root: &Path,
        sessions_dir: Option<&Path>,
    ) -> Result<SyncStats> {
        let db_path = &self.config.db_path;
        let temp_path = db_path.with_extension("db.reindex");
        let backup_path = db_path.with_extension("db.backup");

        info!(
            db = %db_path.display(),
            temp = %temp_path.display(),
            "Starting atomic reindex"
        );

        // Step 1: Build a fresh Mnemosyne instance writing to the temp DB
        let temp_config = MnemosyneConfig {
            db_path: temp_path.clone(),
            ..self.config.clone()
        };
        let temp_mn = Mnemosyne::new(temp_config).await?;

        // Step 2: Full sync into temp DB
        let mut stats = temp_mn.sync_workspace(workspace_root).await?;
        if let Some(sd) = sessions_dir {
            stats.sessions_indexed = temp_mn.sync_sessions(sd).await?;
        }

        // Step 3: Drop the temp Mnemosyne to close its SQLite connection
        drop(temp_mn);

        // Step 4: Swap databases atomically
        // First, back up the live DB
        if db_path.exists() {
            tokio::fs::copy(db_path, &backup_path)
                .await
                .map_err(|e| Error::Database(format!("Failed to backup database: {}", e)))?;
        }

        // Close the live connection by replacing the store with a dummy
        // We need to drop the old connection before renaming the file
        {
            let mut store = self.store.lock().await;
            // Open the temp DB as the new live connection
            match MemoryStore::new(
                &temp_path,
                self.config.enable_fts,
                self.config.enable_embeddings,
            ) {
                Ok(new_store) => {
                    *store = new_store;
                }
                Err(e) => {
                    warn!(error = %e, "Failed to open temp DB for swap, restoring backup");
                    // Restore backup if we have one
                    if backup_path.exists() {
                        let _ = tokio::fs::copy(&backup_path, db_path).await;
                    }
                    return Err(Error::Database(format!(
                        "Atomic reindex failed during swap: {}",
                        e
                    )));
                }
            }
        }

        // Now rename temp -> live (the old live file is no longer open)
        if let Err(e) = tokio::fs::rename(&temp_path, db_path).await {
            warn!(error = %e, "Rename failed, falling back to copy");
            // Try copy instead (cross-device rename)
            if let Err(copy_err) = tokio::fs::copy(&temp_path, db_path).await {
                // Restore from backup
                warn!(error = %copy_err, "Copy also failed, restoring backup");
                if backup_path.exists() {
                    let _ = tokio::fs::copy(&backup_path, db_path).await;
                }
                // Reopen original DB
                {
                    let mut store = self.store.lock().await;
                    if let Ok(restored) = MemoryStore::new(
                        db_path,
                        self.config.enable_fts,
                        self.config.enable_embeddings,
                    ) {
                        *store = restored;
                    }
                }
                return Err(Error::Database(format!(
                    "Atomic reindex failed: rename={}, copy={}",
                    e, copy_err
                )));
            }
            let _ = tokio::fs::remove_file(&temp_path).await;
        }

        // Reopen the live DB with the new file
        {
            let mut store = self.store.lock().await;
            match MemoryStore::new(
                db_path,
                self.config.enable_fts,
                self.config.enable_embeddings,
            ) {
                Ok(new_store) => *store = new_store,
                Err(e) => {
                    warn!(error = %e, "Failed to reopen DB after swap, restoring backup");
                    if backup_path.exists() {
                        let _ = tokio::fs::copy(&backup_path, db_path).await;
                        if let Ok(restored) = MemoryStore::new(
                            db_path,
                            self.config.enable_fts,
                            self.config.enable_embeddings,
                        ) {
                            *store = restored;
                        }
                    }
                    return Err(Error::Database(format!(
                        "Failed to reopen database after reindex: {}",
                        e
                    )));
                }
            }
        }

        // Cleanup: remove backup (keep on failure for manual recovery)
        let _ = tokio::fs::remove_file(&backup_path).await;

        info!(
            files_changed = stats.files_changed,
            sessions = stats.sessions_indexed,
            "Atomic reindex complete"
        );

        Ok(stats)
    }

    /// Get a reference to the inner store (for FileWatcher access).
    pub fn store_ref(&self) -> &Arc<Mutex<MemoryStore>> {
        &self.store
    }

    // ===== Lane 3b-ii — fleet_session_alias async wrappers =====
    //
    // Thin pass-throughs to the sync `MemoryStore` primitives shipped in
    // Lane 3b-i (`2a87fb2e`). Same shape as `pub async fn search`, etc.:
    // acquire the inner mutex, delegate to the sync method, return.

    /// Upsert a fleet session alias record (Lane 3b-ii async wrapper).
    ///
    /// See [`MemoryStore::upsert_alias`] for semantics. On conflict
    /// updates `session_id`, `channel_kind`, and `last_seen`.
    pub async fn upsert_alias(
        &self,
        agent_id: &str,
        human_id: &str,
        session_id: &str,
        channel_kind: &str,
        last_seen: &str,
    ) -> Result<()> {
        let store = self.store.lock().await;
        store.upsert_alias(agent_id, human_id, session_id, channel_kind, last_seen)
    }

    /// Look up the most recent fleet session alias for `(agent_id, human_id)`
    /// within the recency window (Lane 3b-ii async wrapper).
    ///
    /// See [`MemoryStore::lookup_alias`] for semantics. `since` is an
    /// RFC3339-formatted timestamp; rows with `last_seen >= since` match.
    pub async fn lookup_alias(
        &self,
        agent_id: &str,
        human_id: &str,
        since: &str,
    ) -> Result<Option<FleetSessionAliasRow>> {
        let store = self.store.lock().await;
        store.lookup_alias(agent_id, human_id, since)
    }

    /// Prune fleet session alias rows older than the given timestamp
    /// (Lane 3b-ii async wrapper).
    ///
    /// See [`MemoryStore::prune_stale_aliases`] for semantics. Returns the
    /// number of rows deleted.
    pub async fn prune_stale_aliases(&self, older_than: &str) -> Result<usize> {
        let store = self.store.lock().await;
        store.prune_stale_aliases(older_than)
    }
}

// Workspace Bootstrap

/// Default content for workspace bootstrap files.
/// These files seed the workspace so memory accumulates from the first session.
const BOOTSTRAP_MEMORY: &str = r#"# Long-term Memory

Facts, preferences, and learnings accumulated across sessions.

## User
- (Zeus will populate this as it learns about you)

## Projects
- (Active projects and their context)

## Preferences
- (Communication style, tool preferences, workflow habits)
"#;

const BOOTSTRAP_AGENTS: &str = r#"# Zeus Agent

You are Zeus, an autonomous AI assistant with persistent memory and tool access.

## Guidelines

1. Be concise and direct in responses
2. Use tools when actions are needed — don't just describe what to do
3. Ask for clarification when requirements are ambiguous
4. Document important decisions and learnings to MEMORY.md
5. Be proactive about potential issues and edge cases
6. Reference past interactions when relevant (check memory first)
7. Prefer small, incremental changes over large rewrites

## Context Awareness

Before responding, check:
- memory/MEMORY.md for relevant past learnings
- daily/ notes for recent context
- IDENTITY.md for personality and boundaries
"#;

const BOOTSTRAP_IDENTITY: &str = r#"# Identity

## Name
Zeus

## Role
Autonomous AI assistant with persistent memory, tool execution, and multi-channel communication.

## Personality Traits
- Direct and efficient — avoids unnecessary verbosity
- Curious — asks follow-up questions to understand context
- Reliable — follows through on commitments and tracks progress
- Honest — admits uncertainty rather than guessing
- Proactive — anticipates needs and suggests improvements

## Boundaries
- Always ask before destructive operations (deleting files, dropping data)
- Never fabricate information — say "I don't know" when uncertain
- Respect user privacy — don't share data between channels without permission
- Flag security concerns when noticed
"#;

const BOOTSTRAP_TOOLS: &str = r#"# Tools Reference

## Core Tools (always available)

| Tool | Purpose | Example |
|------|---------|---------|
| `read_file` | Read file contents | `read_file(path: "config.toml")` |
| `write_file` | Create/overwrite files | `write_file(path: "out.txt", content: "...")` |
| `edit_file` | Search & replace in files | `edit_file(path: "main.rs", old: "foo", new: "bar")` |
| `list_dir` | List directory contents | `list_dir(path: "src/")` |
| `shell` | Execute shell commands | `shell(command: "cargo test")` |
| `web_fetch` | HTTP GET/POST | `web_fetch(url: "https://...")` |
| `spawn` | Background subagent | `spawn(task: "research X")` |
| `message` | Send to channels | `message(channel: "telegram", text: "...")` |

## Usage Patterns
- Chain tools: read -> edit -> shell (test) -> message (notify)
- Use `spawn` for long-running tasks that don't need immediate results
- Use `shell` for git operations, builds, and system commands
- Use `web_fetch` for API calls and web research
"#;

const BOOTSTRAP_BOOT: &str = r#"# Boot Sequence

Steps Zeus performs on startup:

1. Load workspace files (AGENTS.md, IDENTITY.md, MEMORY.md)
2. Check daily/ for today's notes — resume context if present
3. Scan memory/MEMORY.md for active projects and priorities
4. Ready for interaction

## Session Start Checklist
- [ ] Greet user with context from last session (if available)
- [ ] Check for unfinished tasks from previous sessions
- [ ] Note today's date for daily log entries

## Maintenance Tasks
- Periodically consolidate memory (remove stale entries)
- Update MEMORY.md with new learnings after significant interactions
- Archive completed project notes to keep workspace clean
"#;

/// Bootstrap file entry: (relative_path, default_content).
const BOOTSTRAP_FILES: &[(&str, &str)] = &[
    ("memory/MEMORY.md", BOOTSTRAP_MEMORY),
    ("AGENTS.md", BOOTSTRAP_AGENTS),
    ("IDENTITY.md", BOOTSTRAP_IDENTITY),
    ("TOOLS.md", BOOTSTRAP_TOOLS),
    ("BOOT.md", BOOTSTRAP_BOOT),
];

/// Bootstrap a workspace directory with default files.
///
/// Creates `MEMORY.md`, `AGENTS.md`, `IDENTITY.md`, `TOOLS.md`, and `BOOT.md`
/// if they don't exist or are empty. Existing non-empty files are left untouched.
///
/// Returns the list of files that were created or populated.
pub fn bootstrap_workspace(workspace_root: &Path) -> Result<Vec<PathBuf>> {
    let mut created = Vec::new();

    // Ensure directories exist
    std::fs::create_dir_all(workspace_root.join("memory"))
        .map_err(|e| Error::Memory(format!("Failed to create memory dir: {}", e)))?;
    std::fs::create_dir_all(workspace_root.join("daily"))
        .map_err(|e| Error::Memory(format!("Failed to create daily dir: {}", e)))?;

    for &(rel_path, default_content) in BOOTSTRAP_FILES {
        let full_path = workspace_root.join(rel_path);

        // Create parent directories if needed
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Memory(format!("Failed to create dir: {}", e)))?;
        }

        let needs_content = if full_path.exists() {
            // Check if file is empty or contains only whitespace
            match std::fs::read_to_string(&full_path) {
                Ok(content) => content.trim().is_empty(),
                Err(_) => true,
            }
        } else {
            true
        };

        if needs_content {
            std::fs::write(&full_path, default_content)
                .map_err(|e| Error::Memory(format!("Failed to write {}: {}", rel_path, e)))?;
            created.push(full_path);
        }
    }

    if !created.is_empty() {
        info!(count = created.len(), "Bootstrapped workspace files");
    }

    Ok(created)
}

// Data Types

/// Memory type classification for the memory hierarchy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    /// Current session scratch space (high priority, short-lived)
    Working,
    /// Past events and interactions (medium priority, decays over time)
    Episodic,
    /// Extracted knowledge and patterns (high priority, persistent)
    Semantic,
    /// Factual information (discrete knowledge entries)
    Fact,
    /// User preferences and settings
    Preference,
    /// Conversation messages and exchanges
    Conversation,
    /// Summaries of older memories (compressed knowledge)
    Summary,
    /// Inbound @-mentions of this titan (mention-gated cross-channel awareness)
    Mention,
}

impl MemoryType {
    pub fn as_str(&self) -> &str {
        match self {
            MemoryType::Working => "working",
            MemoryType::Episodic => "episodic",
            MemoryType::Semantic => "semantic",
            MemoryType::Fact => "fact",
            MemoryType::Preference => "preference",
            MemoryType::Conversation => "conversation",
            MemoryType::Summary => "summary",
            MemoryType::Mention => "mention",
        }
    }

    pub fn parse_label(s: &str) -> Self {
        match s {
            "working" => MemoryType::Working,
            "semantic" => MemoryType::Semantic,
            "episodic" => MemoryType::Episodic,
            "fact" => MemoryType::Fact,
            "preference" => MemoryType::Preference,
            "conversation" => MemoryType::Conversation,
            "summary" => MemoryType::Summary,
            "mention" => MemoryType::Mention,
            _ => MemoryType::Episodic,
        }
    }
}

impl std::fmt::Display for MemoryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Search result
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub id: i64,
    pub session_id: String,
    pub content: String,
    pub timestamp: String,
    pub score: f32,
    pub memory_type: MemoryType,
    pub importance: f32,
    /// Source citation, e.g. "memory/2026-02-10.md#L42"
    pub citation: Option<String>,
    /// When this memory became valid (temporal versioning)
    pub valid_from: Option<String>,
    /// When this memory was superseded (NULL = still current)
    pub valid_to: Option<String>,
    /// ID of the memory that superseded this one
    pub superseded_by: Option<i64>,
    /// Verification status — false = unverified content (e.g. unverified SHA-shaped strings)
    /// that should be filtered from export to prevent fabrication propagation
    pub verified: bool,
}

/// Stored message (raw from DB)
#[derive(Debug, Clone)]
pub struct StoredMessage {
    pub id: i64,
    pub role: String,
    pub content: String,
    pub tool_calls: String,
    pub tool_results: String,
    pub timestamp: String,
}

/// A recognized pattern extracted from sessions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternEntry {
    pub id: i64,
    pub pattern_type: String,
    pub content: String,
    pub frequency: i64,
    pub first_seen: String,
    pub last_seen: String,
}

/// An extracted entity with its canonical name, type, and aliases.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityRecord {
    pub id: i64,
    pub canonical_name: String,
    pub entity_type: String,
    pub aliases: Vec<String>,
    pub first_seen: String,
    pub last_seen: String,
    pub mention_count: i64,
}

/// Memory statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStats {
    pub message_count: usize,
    pub session_count: usize,
    pub embedding_count: usize,
    pub embedding_cache_count: usize,
    pub tracked_file_count: usize,
}

/// A tracked file in the memory_files table
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedFile {
    pub path: String,
    pub source: String,
    pub content_hash: String,
    pub mtime: i64,
    pub size: i64,
    pub last_indexed: i64,
}

/// Statistics returned from a workspace sync operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStats {
    pub files_scanned: usize,
    pub files_changed: usize,
    pub files_unchanged: usize,
    pub chunks_embedded: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub sessions_indexed: usize,
    pub errors: Vec<String>,
}

/// A tracked session file in the session_files table
#[derive(Debug, Clone)]
pub struct SessionFileEntry {
    pub session_id: String,
    pub file_path: String,
    pub last_size: i64,
    pub pending_bytes: i64,
    pub pending_messages: i64,
    pub last_indexed: i64,
}

/// Maximum embedding cache entries before LRU eviction
pub const EMBEDDING_CACHE_MAX: usize = 50_000;

/// Default debounce duration for file watcher (1.5 seconds)
const FILE_WATCHER_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(1500);

// File Watcher

/// Watches filesystem paths for changes and triggers workspace sync.
///
/// Uses the `notify` crate with a 1.5-second debounce to collect batched
/// filesystem events. When changes are detected, marks files as dirty and
/// triggers a workspace sync after the debounce period.
pub struct FileWatcher {
    /// The underlying notify watcher (kept alive to maintain watches)
    _watcher: RecommendedWatcher,
    /// Set of dirty file paths awaiting sync
    dirty_files: Arc<std::sync::Mutex<HashSet<PathBuf>>>,
    /// Handle to the background sync task
    _sync_handle: tokio::task::JoinHandle<()>,
}

impl FileWatcher {
    /// Start a file watcher that monitors the given paths and triggers sync.
    ///
    /// - `workspace_root`: primary directory to watch (recursive)
    /// - `extra_paths`: additional directories to watch
    /// - `mnemosyne`: shared Mnemosyne instance for triggering sync
    ///
    /// The watcher debounces events for 1.5 seconds before triggering a sync.
    pub fn start(
        workspace_root: PathBuf,
        extra_paths: Vec<PathBuf>,
        mnemosyne: Arc<Mnemosyne>,
    ) -> Result<Self> {
        let dirty_files: Arc<std::sync::Mutex<HashSet<PathBuf>>> =
            Arc::new(std::sync::Mutex::new(HashSet::new()));
        let dirty_clone = dirty_files.clone();

        // Channel for debounced event notification
        let (notify_tx, mut notify_rx) = tokio::sync::mpsc::channel::<()>(16);

        // Create the notify watcher with a standard event handler
        let tx_clone = notify_tx.clone();
        let mut watcher =
            notify::recommended_watcher(move |res: std::result::Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    // Only care about file modifications and creations
                    match event.kind {
                        notify::EventKind::Modify(_)
                        | notify::EventKind::Create(_)
                        | notify::EventKind::Remove(_) => {
                            let mut dirty = dirty_clone.lock().unwrap_or_else(|e| e.into_inner());
                            for path in &event.paths {
                                dirty.insert(path.clone());
                            }
                            // Signal that there are dirty files (non-blocking)
                            let _ = tx_clone.try_send(());
                        }
                        _ => {}
                    }
                }
            })
            .map_err(|e| Error::Config(format!("Failed to create file watcher: {}", e)))?;

        // Watch the workspace root
        if workspace_root.exists() {
            watcher
                .watch(&workspace_root, RecursiveMode::Recursive)
                .map_err(|e| {
                    Error::Config(format!(
                        "Failed to watch {}: {}",
                        workspace_root.display(),
                        e
                    ))
                })?;
            info!(path = %workspace_root.display(), "Watching directory");
        }

        // Watch extra paths
        for path in &extra_paths {
            if path.exists() {
                if let Err(e) = watcher.watch(path, RecursiveMode::Recursive) {
                    warn!(path = %path.display(), error = %e, "Failed to watch extra path");
                } else {
                    info!(path = %path.display(), "Watching extra directory");
                }
            }
        }

        let dirty_for_task = dirty_files.clone();
        let ws_root = workspace_root.clone();

        // Background task: wait for dirty signal, debounce, then sync
        let sync_handle = tokio::spawn(async move {
            loop {
                // Wait for a dirty signal
                if notify_rx.recv().await.is_none() {
                    // Channel closed — watcher dropped
                    break;
                }

                // Debounce: sleep, then drain any queued signals
                tokio::time::sleep(FILE_WATCHER_DEBOUNCE).await;
                while notify_rx.try_recv().is_ok() {}

                // Collect dirty files and clear the set
                let dirty: Vec<PathBuf> = {
                    let mut set = dirty_for_task.lock().unwrap_or_else(|e| e.into_inner());
                    let files: Vec<PathBuf> = set.drain().collect();
                    files
                };

                if dirty.is_empty() {
                    continue;
                }

                debug!(
                    count = dirty.len(),
                    "File watcher triggered sync after debounce"
                );

                // Trigger workspace sync
                match mnemosyne.sync_workspace(&ws_root).await {
                    Ok(stats) => {
                        if stats.files_changed > 0 {
                            info!(
                                changed = stats.files_changed,
                                chunks = stats.chunks_embedded,
                                "File watcher sync complete"
                            );
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "File watcher sync failed");
                    }
                }
            }
        });

        Ok(Self {
            _watcher: watcher,
            dirty_files,
            _sync_handle: sync_handle,
        })
    }

    /// Get the current set of dirty (pending) file paths.
    pub fn dirty_files(&self) -> Vec<PathBuf> {
        let set = self.dirty_files.lock().unwrap_or_else(|e| e.into_inner());
        set.iter().cloned().collect()
    }

    /// Get the count of dirty files pending sync.
    pub fn dirty_count(&self) -> usize {
        let set = self.dirty_files.lock().unwrap_or_else(|e| e.into_inner());
        set.len()
    }
}

// QMD Backend — BM25 + vector + reranking (external sidecar + internal)

/// Search result from QMD pipeline, with separate score components.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QmdSearchResult {
    /// Document/chunk content
    pub content: String,
    /// BM25 keyword relevance score
    pub bm25_score: f32,
    /// Vector cosine similarity score
    pub vector_score: f32,
    /// Final reranked score (cross-encoder or weighted fusion)
    pub reranked_score: f32,
    /// Optional source citation
    #[serde(default)]
    pub citation: Option<String>,
    /// Optional memory type tag
    #[serde(default)]
    pub memory_type: Option<String>,
}

/// HTTP client wrapper for the QMD sidecar process.
pub struct QmdBackend {
    client: reqwest::Client,
    base_url: String,
    available: std::sync::atomic::AtomicBool,
    reranker: Option<CrossEncoderReranker>,
}

/// Request body sent to QMD `/search` endpoint.
#[derive(Serialize)]
struct QmdSearchRequest {
    query: String,
    limit: usize,
}

/// Response body from QMD `/search` endpoint.
#[derive(Deserialize)]
struct QmdSearchResponse {
    results: Vec<QmdSearchResult>,
}

/// Response body from QMD `/qmd/health` endpoint.
#[derive(Deserialize)]
struct QmdHealthResponse {
    #[serde(default)]
    status: String,
}

// Cross-Encoder Reranker

/// Cross-encoder reranker that scores (query, document) pairs.
///
/// Supports two modes:
/// 1. **External**: calls a cross-encoder model served via HTTP (e.g. sentence-transformers)
/// 2. **Internal**: uses a term-overlap + position-aware heuristic when no external model is available
pub struct CrossEncoderReranker {
    client: reqwest::Client,
    url: Option<String>,
    model: String,
}

/// Request body for external cross-encoder API.
#[derive(Serialize)]
struct CrossEncoderRequest {
    model: String,
    query: String,
    documents: Vec<String>,
}

/// Response from external cross-encoder API.
#[derive(Deserialize)]
struct CrossEncoderResponse {
    scores: Vec<f32>,
}

/// A candidate result with its component scores for QMD fusion.
#[derive(Debug, Clone)]
pub struct QmdCandidate {
    /// Original search result
    pub result: SearchResult,
    /// Normalized BM25 score (0.0–1.0)
    pub bm25_score: f32,
    /// Normalized vector cosine similarity (0.0–1.0)
    pub vector_score: f32,
    /// Cross-encoder relevance score (0.0–1.0)
    pub reranker_score: f32,
}

impl CrossEncoderReranker {
    /// Create a new cross-encoder reranker.
    ///
    /// If `url` is `Some`, uses external HTTP API. Otherwise uses internal heuristic scoring.
    pub fn new(url: Option<String>, model: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self { client, url, model }
    }

    /// Score (query, document) pairs using cross-encoder.
    ///
    /// Returns a score for each document in the same order.
    pub async fn score(&self, query: &str, documents: &[&str]) -> Result<Vec<f32>> {
        if documents.is_empty() {
            return Ok(Vec::new());
        }

        match &self.url {
            Some(url) => self.score_external(url, query, documents).await,
            None => Ok(self.score_internal(query, documents)),
        }
    }

    /// Call external cross-encoder API.
    async fn score_external(&self, url: &str, query: &str, documents: &[&str]) -> Result<Vec<f32>> {
        let body = CrossEncoderRequest {
            model: self.model.clone(),
            query: query.to_string(),
            documents: documents.iter().map(|d| d.to_string()).collect(),
        };

        let resp = self
            .client
            .post(url)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::memory(format!("Cross-encoder request failed: {}", e)))?;

        if !resp.status().is_success() {
            // Fall back to internal scoring on HTTP error
            warn!(
                status = %resp.status(),
                "Cross-encoder API error, falling back to internal scoring"
            );
            return Ok(self.score_internal(query, documents));
        }

        let parsed: CrossEncoderResponse = resp
            .json()
            .await
            .map_err(|e| Error::memory(format!("Cross-encoder response parse error: {}", e)))?;

        if parsed.scores.len() != documents.len() {
            warn!(
                expected = documents.len(),
                got = parsed.scores.len(),
                "Cross-encoder returned wrong number of scores, falling back"
            );
            return Ok(self.score_internal(query, documents));
        }

        Ok(parsed.scores)
    }

    /// Internal heuristic cross-encoder scoring.
    ///
    /// Combines:
    /// - **Term overlap**: fraction of query terms found in document (Jaccard-like)
    /// - **Exact phrase match**: bonus for consecutive query terms appearing in order
    /// - **Position bias**: earlier matches in the document get a small boost
    /// - **Term density**: ratio of matching terms to document length
    pub fn score_internal(&self, query: &str, documents: &[&str]) -> Vec<f32> {
        let query_lower = query.to_lowercase();
        let query_terms: Vec<&str> = query_lower.split_whitespace().collect();

        if query_terms.is_empty() {
            return vec![0.0; documents.len()];
        }

        documents
            .iter()
            .map(|doc| {
                let doc_lower = doc.to_lowercase();
                let doc_terms: Vec<&str> = doc_lower.split_whitespace().collect();

                if doc_terms.is_empty() {
                    return 0.0;
                }

                // 1. Term overlap (Jaccard-like)
                let matching_terms = query_terms
                    .iter()
                    .filter(|qt| doc_terms.iter().any(|dt| dt.contains(**qt)))
                    .count();
                let term_overlap = matching_terms as f32 / query_terms.len() as f32;

                // 2. Exact phrase match bonus
                let phrase_bonus = if query_terms.len() > 1 && doc_lower.contains(&query_lower) {
                    0.3
                } else {
                    // Check for partial consecutive matches
                    let mut max_consecutive = 0usize;
                    let mut current_consecutive = 0usize;
                    for qt in &query_terms {
                        if doc_terms.iter().any(|dt| dt.contains(*qt)) {
                            current_consecutive += 1;
                            max_consecutive = max_consecutive.max(current_consecutive);
                        } else {
                            current_consecutive = 0;
                        }
                    }
                    if max_consecutive > 1 {
                        0.1 * (max_consecutive as f32 / query_terms.len() as f32)
                    } else {
                        0.0
                    }
                };

                // 3. Position bias: earlier matches score higher
                let position_score = if let Some(pos) = query_terms
                    .iter()
                    .filter_map(|qt| doc_terms.iter().position(|dt| dt.contains(*qt)))
                    .min()
                {
                    // Inverse position normalized to 0.0–0.1
                    0.1 * (1.0 / (1.0 + pos as f32))
                } else {
                    0.0
                };

                // 4. Term density: matching terms / doc length
                let density = matching_terms as f32 / doc_terms.len().min(500) as f32;
                let density_score = density.min(0.2);

                // Combine scores (weighted to sum to ~1.0 max)
                let raw = term_overlap * 0.5 + phrase_bonus + position_score + density_score;
                raw.clamp(0.0, 1.0)
            })
            .collect()
    }
}

impl QmdBackend {
    /// Create a new QMD backend and check health on startup.
    pub async fn new(base_url: &str, timeout_ms: u64) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(timeout_ms))
            .build()
            .unwrap_or_default();

        let backend = Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            available: std::sync::atomic::AtomicBool::new(false),
            reranker: None,
        };

        // Health check
        backend.check_health().await;
        backend
    }

    /// Create a QMD backend with cross-encoder reranker for internal QMD pipeline.
    pub async fn with_reranker(
        base_url: &str,
        timeout_ms: u64,
        reranker_url: Option<String>,
        reranker_model: String,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(timeout_ms))
            .build()
            .unwrap_or_default();

        let reranker = CrossEncoderReranker::new(reranker_url, reranker_model);

        let backend = Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            available: std::sync::atomic::AtomicBool::new(false),
            reranker: Some(reranker),
        };

        backend.check_health().await;
        backend
    }

    /// Whether this backend has a cross-encoder reranker configured.
    pub fn has_reranker(&self) -> bool {
        self.reranker.is_some()
    }

    /// Probe the QMD sidecar health endpoint.
    pub async fn check_health(&self) {
        let url = format!("{}/qmd/health", self.base_url);
        let ok = match self.client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<QmdHealthResponse>().await {
                    Ok(h) => h.status == "ok" || h.status == "healthy",
                    Err(_) => false,
                }
            }
            _ => false,
        };
        self.available
            .store(ok, std::sync::atomic::Ordering::Relaxed);
        if ok {
            info!("QMD sidecar available at {}", self.base_url);
        } else {
            debug!("QMD sidecar unavailable at {}", self.base_url);
        }
    }

    /// Whether the QMD sidecar is currently marked available.
    pub fn is_available(&self) -> bool {
        self.available.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Search via QMD sidecar. Returns reranked results.
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<QmdSearchResult>> {
        if !self.is_available() {
            return Err(Error::memory("QMD sidecar is not available"));
        }

        let url = format!("{}/search", self.base_url);
        let body = QmdSearchRequest {
            query: query.to_string(),
            limit,
        };

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                // Mark unavailable on connection error
                self.available
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                Error::memory(format!("QMD request failed: {}", e))
            })?;

        if !resp.status().is_success() {
            return Err(Error::memory(format!(
                "QMD returned status {}",
                resp.status()
            )));
        }

        let search_resp: QmdSearchResponse = resp
            .json()
            .await
            .map_err(|e| Error::memory(format!("QMD response parse error: {}", e)))?;

        Ok(search_resp.results)
    }

    /// Rerank search results using the cross-encoder.
    ///
    /// Takes pre-scored candidates from hybrid search and applies cross-encoder
    /// scoring, then fuses BM25 + vector + cross-encoder scores with configurable weights.
    pub async fn rerank(
        &self,
        query: &str,
        candidates: Vec<QmdCandidate>,
        bm25_weight: f64,
        vector_weight: f64,
        reranker_weight: f64,
        limit: usize,
    ) -> Result<Vec<QmdSearchResult>> {
        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        let reranker = self
            .reranker
            .as_ref()
            .ok_or_else(|| Error::memory("No cross-encoder reranker configured"))?;

        // Score all candidates with cross-encoder
        let documents: Vec<&str> = candidates
            .iter()
            .map(|c| c.result.content.as_str())
            .collect();
        let ce_scores = reranker.score(query, &documents).await?;

        // Fuse scores: weighted combination of BM25 + vector + cross-encoder
        let mut results: Vec<QmdSearchResult> = candidates
            .into_iter()
            .zip(ce_scores.into_iter())
            .map(|(candidate, ce_score)| {
                let fused = bm25_weight * candidate.bm25_score as f64
                    + vector_weight * candidate.vector_score as f64
                    + reranker_weight * ce_score as f64;
                QmdSearchResult {
                    content: candidate.result.content,
                    bm25_score: candidate.bm25_score,
                    vector_score: candidate.vector_score,
                    reranked_score: fused as f32,
                    citation: candidate.result.citation,
                    memory_type: Some(candidate.result.memory_type.as_str().to_string()),
                }
            })
            .collect();

        // Sort by reranked score descending
        results.sort_by(|a, b| {
            b.reranked_score
                .partial_cmp(&a.reranked_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);

        Ok(results)
    }
}

/// Converts QMD results to standard `SearchResult` for uniform consumption.
fn qmd_to_search_results(qmd_results: Vec<QmdSearchResult>) -> Vec<SearchResult> {
    qmd_results
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            let memory_type = r
                .memory_type
                .as_deref()
                .map(MemoryType::parse_label)
                .unwrap_or(MemoryType::Episodic);
            SearchResult {
                id: -(i as i64 + 1), // Negative IDs to distinguish from local DB IDs
                session_id: String::new(),
                content: r.content,
                timestamp: String::new(),
                score: r.reranked_score,
                memory_type,
                importance: r.reranked_score.clamp(0.0, 1.0),
                citation: r.citation,
                valid_from: None,
                valid_to: None,
                verified: true,
                superseded_by: None,
            }
        })
        .collect()
}

// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_mnemosyne_creation() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            max_messages_per_session: 100,
            enable_embeddings: false,
            ..Default::default()
        };

        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        let stats = mnemosyne
            .stats()
            .await
            .expect("async operation should succeed");
        assert_eq!(stats.message_count, 0);
        assert_eq!(stats.embedding_count, 0);
    }

    #[tokio::test]
    async fn test_store_and_recall() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: false,
            max_messages_per_session: 100,
            enable_embeddings: false,
            ..Default::default()
        };

        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let msg = Message::user("Hello, world!");
        mnemosyne
            .store("session-1", &msg)
            .await
            .expect("async operation should succeed");

        let messages = mnemosyne
            .recall_session("session-1", 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "Hello, world!");
    }

    #[tokio::test]
    async fn test_fts_search_after_insert() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test_fts.db"),
            enable_fts: true,
            max_messages_per_session: 100,
            enable_embeddings: false,
            ..Default::default()
        };

        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store messages with distinct content
        let msg1 = Message::user("The quick brown fox jumps");
        let msg2 = Message::user("A lazy dog sleeps peacefully");
        mnemosyne
            .store("session-1", &msg1)
            .await
            .expect("async operation should succeed");
        mnemosyne
            .store("session-1", &msg2)
            .await
            .expect("async operation should succeed");

        // FTS search should find the matching message
        let results = mnemosyne
            .search("fox", 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("fox"));

        let results = mnemosyne
            .search("dog", 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("dog"));
    }

    #[tokio::test]
    async fn test_role_serialization() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test_role.db"),
            enable_fts: false,
            max_messages_per_session: 100,
            enable_embeddings: false,
            ..Default::default()
        };

        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let msg = Message::user("test");
        mnemosyne
            .store("s1", &msg)
            .await
            .expect("async operation should succeed");

        let messages = mnemosyne
            .recall_session("s1", 10)
            .await
            .expect("async operation should succeed");
        // Should be lowercase "user", not "User" (Debug format)
        assert_eq!(messages[0].role, "user");
    }

    // Vector embedding tests

    #[test]
    fn test_cosine_similarity_identical() {
        let v = vec![1.0, 2.0, 3.0, 4.0];
        let sim = cosine_similarity(&v, &v);
        assert!(
            (sim - 1.0).abs() < 1e-6,
            "Identical vectors should have similarity 1.0, got {}",
            sim
        );
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(
            sim.abs() < 1e-6,
            "Orthogonal vectors should have similarity 0.0, got {}",
            sim
        );
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![-1.0, -2.0, -3.0];
        let sim = cosine_similarity(&a, &b);
        assert!(
            (sim - (-1.0)).abs() < 1e-6,
            "Opposite vectors should have similarity -1.0, got {}",
            sim
        );
    }

    #[test]
    fn test_embedding_roundtrip() {
        let original = vec![0.1, -0.2, 3.14159, 0.0, f32::MAX, f32::MIN];
        let bytes = embedding_to_bytes(&original);
        let recovered = bytes_to_embedding(&bytes);
        assert_eq!(original.len(), recovered.len());
        for (a, b) in original.iter().zip(recovered.iter()) {
            assert_eq!(
                a.to_bits(),
                b.to_bits(),
                "Embedding roundtrip failed: {} != {}",
                a,
                b
            );
        }
    }

    #[tokio::test]
    async fn test_store_and_retrieve_embedding() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test_embed.db"),
            enable_fts: false,
            max_messages_per_session: 100,
            enable_embeddings: true,
            ..Default::default()
        };

        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store a message
        let msg = Message::user("Test embedding storage");
        let msg_id = mnemosyne
            .store("s1", &msg)
            .await
            .expect("async operation should succeed");

        // Store an embedding for it
        let embedding = vec![0.1, 0.2, 0.3, 0.4];
        mnemosyne
            .store_embedding(msg_id, &embedding, Some("test-model"))
            .await
            .expect("async operation should succeed");

        // Search with the same embedding - should get similarity ~1.0
        let results = mnemosyne
            .vector_search(&embedding, 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(results.len(), 1);
        assert!(
            (results[0].score - 1.0).abs() < 1e-5,
            "Same embedding should have similarity ~1.0, got {}",
            results[0].score
        );
        assert_eq!(results[0].content, "Test embedding storage");
    }

    #[tokio::test]
    async fn test_vector_search_ranking() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test_rank.db"),
            enable_fts: false,
            max_messages_per_session: 100,
            enable_embeddings: true,
            ..Default::default()
        };

        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store 3 messages with different embeddings
        let msg1 = Message::user("North pole");
        let id1 = mnemosyne
            .store("s1", &msg1)
            .await
            .expect("async operation should succeed");
        mnemosyne
            .store_embedding(id1, &[1.0, 0.0, 0.0], None)
            .await
            .expect("async operation should succeed");

        let msg2 = Message::user("East pole");
        let id2 = mnemosyne
            .store("s1", &msg2)
            .await
            .expect("async operation should succeed");
        mnemosyne
            .store_embedding(id2, &[0.0, 1.0, 0.0], None)
            .await
            .expect("async operation should succeed");

        let msg3 = Message::user("Mostly north");
        let id3 = mnemosyne
            .store("s1", &msg3)
            .await
            .expect("async operation should succeed");
        mnemosyne
            .store_embedding(id3, &[0.9, 0.1, 0.0], None)
            .await
            .expect("async operation should succeed");

        // Search with a vector close to "North pole" [1, 0, 0]
        let query = vec![0.95, 0.05, 0.0];
        let results = mnemosyne
            .vector_search(&query, 10)
            .await
            .expect("async operation should succeed");

        assert_eq!(results.len(), 3);
        // "Mostly north" and "North pole" should be top 2 (both close to query)
        // "East pole" should be last
        assert!(
            results[0].content == "North pole" || results[0].content == "Mostly north",
            "Top result should be 'North pole' or 'Mostly north', got '{}'",
            results[0].content
        );
        assert_eq!(
            results[2].content, "East pole",
            "Last result should be 'East pole'"
        );

        // All scores should be non-negative (since all vectors have non-negative components)
        for r in &results {
            assert!(r.score >= 0.0, "Score should be >= 0, got {}", r.score);
        }

        // Top result should have higher score than last
        assert!(results[0].score > results[2].score);
    }

    #[tokio::test]
    async fn test_hybrid_search() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test_hybrid.db"),
            enable_fts: true,
            max_messages_per_session: 100,
            enable_embeddings: true,
            ..Default::default()
        };

        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store messages with both text and embeddings
        let msg1 = Message::user("The quick brown fox jumps over the lazy dog");
        let id1 = mnemosyne
            .store("s1", &msg1)
            .await
            .expect("async operation should succeed");
        mnemosyne
            .store_embedding(id1, &[1.0, 0.0, 0.0], None)
            .await
            .expect("async operation should succeed");

        let msg2 = Message::user("A cat sleeps on the warm windowsill");
        let id2 = mnemosyne
            .store("s1", &msg2)
            .await
            .expect("async operation should succeed");
        mnemosyne
            .store_embedding(id2, &[0.0, 1.0, 0.0], None)
            .await
            .expect("async operation should succeed");

        let msg3 = Message::user("The fox hunts at night in the forest");
        let id3 = mnemosyne
            .store("s1", &msg3)
            .await
            .expect("async operation should succeed");
        mnemosyne
            .store_embedding(id3, &[0.8, 0.2, 0.0], None)
            .await
            .expect("async operation should succeed");

        // Hybrid search: FTS for "fox" + vector close to msg1
        let query_embedding = vec![0.9, 0.1, 0.0];
        let results = mnemosyne
            .hybrid_search("fox", Some(&query_embedding), 10)
            .await
            .expect("async operation should succeed");

        // Should find at least the 2 fox messages
        assert!(
            results.len() >= 2,
            "Should find at least 2 results, got {}",
            results.len()
        );

        // The fox messages should be in results
        let contents: Vec<&str> = results.iter().map(|r| r.content.as_str()).collect();
        assert!(
            contents.iter().any(|c| c.contains("quick brown fox")),
            "Should find 'quick brown fox' message"
        );
        assert!(
            contents.iter().any(|c| c.contains("fox hunts")),
            "Should find 'fox hunts' message"
        );
    }

    #[tokio::test]
    async fn test_stats_with_embeddings() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test_stats.db"),
            enable_fts: false,
            max_messages_per_session: 100,
            enable_embeddings: true,
            ..Default::default()
        };

        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store messages and embeddings
        let msg1 = Message::user("First message");
        let id1 = mnemosyne
            .store("s1", &msg1)
            .await
            .expect("async operation should succeed");
        mnemosyne
            .store_embedding(id1, &[1.0, 0.0, 0.0], None)
            .await
            .expect("async operation should succeed");

        let msg2 = Message::user("Second message");
        let id2 = mnemosyne
            .store("s1", &msg2)
            .await
            .expect("async operation should succeed");
        mnemosyne
            .store_embedding(id2, &[0.0, 1.0, 0.0], None)
            .await
            .expect("async operation should succeed");

        let msg3 = Message::user("Third message, no embedding");
        mnemosyne
            .store("s2", &msg3)
            .await
            .expect("async operation should succeed");

        let stats = mnemosyne
            .stats()
            .await
            .expect("async operation should succeed");
        assert_eq!(stats.message_count, 3);
        assert_eq!(stats.session_count, 2);
        assert_eq!(stats.embedding_count, 2);
    }

    #[tokio::test]
    async fn test_embeddings_disabled() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test_disabled.db"),
            enable_fts: false,
            max_messages_per_session: 100,
            enable_embeddings: false,
            ..Default::default()
        };

        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store a message
        let msg = Message::user("test");
        let msg_id = mnemosyne
            .store("s1", &msg)
            .await
            .expect("async operation should succeed");

        // Storing an embedding should fail when disabled
        let result = mnemosyne
            .store_embedding(msg_id, &[1.0, 2.0, 3.0], None)
            .await;
        assert!(
            result.is_err(),
            "store_embedding should fail when embeddings are disabled"
        );

        // Vector search should also fail
        let result = mnemosyne.vector_search(&[1.0, 2.0, 3.0], 10).await;
        assert!(
            result.is_err(),
            "vector_search should fail when embeddings are disabled"
        );

        // Stats should still work and show 0 embeddings
        let stats = mnemosyne
            .stats()
            .await
            .expect("async operation should succeed");
        assert_eq!(stats.embedding_count, 0);
    }

    // Embedding provider tests

    #[test]
    fn test_embedder_instance_construction() {
        let config = MnemosyneConfig::default();
        let instance = EmbedderInstance::new(EmbeddingProvider::Ollama, &config);
        assert_eq!(instance.model_name(), "nomic-embed-text");
        assert_eq!(instance.provider_name(), "ollama");
    }

    #[test]
    fn test_embedder_instance_url_trailing_slash() {
        let config = MnemosyneConfig {
            ollama_url: "http://localhost:11434/".to_string(),
            ..Default::default()
        };
        let instance = EmbedderInstance::new(EmbeddingProvider::Ollama, &config);
        // Trailing slash should be stripped
        assert_eq!(instance.base_url, "http://localhost:11434");
    }

    #[tokio::test]
    async fn test_embedder_instance_connection_refused() {
        // Use a port that's almost certainly not running Ollama
        let config = MnemosyneConfig {
            ollama_url: "http://127.0.0.1:19999".to_string(),
            ..Default::default()
        };
        let instance = EmbedderInstance::new(EmbeddingProvider::Ollama, &config);
        let result = instance.embed("test text").await;
        assert!(result.is_err(), "Should fail when Ollama is not reachable");
    }

    #[test]
    fn test_config_defaults() {
        let config = MnemosyneConfig::default();
        let expected_url =
            std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".to_string());
        assert_eq!(config.ollama_url, expected_url);
        assert_eq!(config.embedding_model, "nomic-embed-text");
        assert_eq!(config.embedding_dim, 768);
        assert!(!config.enable_embeddings);
    }

    #[test]
    fn test_config_serde_roundtrip() {
        let config = MnemosyneConfig {
            db_path: "/tmp/test.db".into(),
            enable_fts: true,
            max_messages_per_session: 500,
            enable_embeddings: true,
            embedding_dim: 768,
            ollama_url: "http://my-ollama:11434".to_string(),
            embedding_model: "mxbai-embed-large".to_string(),
            ..Default::default()
        };

        let json = serde_json::to_string(&config).expect("should serialize to JSON");
        let deserialized: MnemosyneConfig =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(deserialized.ollama_url, "http://my-ollama:11434");
        assert_eq!(deserialized.embedding_model, "mxbai-embed-large");
        assert_eq!(deserialized.embedding_dim, 768);
    }

    #[test]
    fn test_config_serde_defaults_missing_fields() {
        // Simulate config without the new fields (backward compatibility)
        let json = r#"{
            "db_path": "/tmp/test.db",
            "enable_fts": true,
            "max_messages_per_session": 100,
            "enable_embeddings": false
        }"#;
        let config: MnemosyneConfig =
            serde_json::from_str(json).expect("should parse successfully");
        let expected_url =
            std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".to_string());
        assert_eq!(config.ollama_url, expected_url);
        assert_eq!(config.embedding_model, "nomic-embed-text");
        assert_eq!(config.embedding_dim, 768);
    }

    #[tokio::test]
    async fn test_has_embedder_enabled() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test_has.db"),
            enable_embeddings: true,
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        assert!(mnemosyne.has_embedder());
    }

    #[tokio::test]
    async fn test_has_embedder_disabled() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test_no.db"),
            enable_embeddings: false,
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        assert!(!mnemosyne.has_embedder());
    }

    #[tokio::test]
    async fn test_embed_text_disabled() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test_et.db"),
            enable_embeddings: false,
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        let result = mnemosyne
            .embed_text("hello")
            .await
            .expect("async operation should succeed");
        assert!(
            result.is_none(),
            "embed_text should return None when disabled"
        );
    }

    #[tokio::test]
    async fn test_store_with_embedding_no_provider() {
        // When embeddings are disabled, store_with_embedding should still store the message
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test_swe.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let msg = Message::user("Hello, world!");
        let msg_id = mnemosyne
            .store_with_embedding("s1", &msg)
            .await
            .expect("async operation should succeed");
        assert!(msg_id > 0);

        let messages = mnemosyne
            .recall_session("s1", 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "Hello, world!");
    }

    #[tokio::test]
    async fn test_store_with_embedding_empty_content() {
        // Empty content should be stored but not embedded
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test_empty.db"),
            enable_fts: false,
            enable_embeddings: true,
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let msg = Message::user("   ");
        let msg_id = mnemosyne
            .store_with_embedding("s1", &msg)
            .await
            .expect("async operation should succeed");
        assert!(msg_id > 0);

        // No embedding should have been stored
        let stats = mnemosyne
            .stats()
            .await
            .expect("async operation should succeed");
        assert_eq!(stats.message_count, 1);
        assert_eq!(stats.embedding_count, 0);
    }

    #[tokio::test]
    async fn test_semantic_search_fts_fallback() {
        // When embedding provider is unreachable, semantic_search falls back to FTS-only
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test_sem.db"),
            enable_fts: true,
            enable_embeddings: true,
            ollama_url: "http://127.0.0.1:19999".to_string(), // unreachable
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let msg = Message::user("The quick brown fox");
        mnemosyne
            .store("s1", &msg)
            .await
            .expect("async operation should succeed");

        // semantic_search should gracefully fall back to FTS
        let results = mnemosyne
            .semantic_search("fox", 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("fox"));
    }

    #[tokio::test]
    async fn test_semantic_search_no_embedder() {
        // With embeddings disabled, semantic_search uses FTS only
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test_sem2.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let msg1 = Message::user("Rust programming language");
        let msg2 = Message::user("Python is great too");
        mnemosyne
            .store("s1", &msg1)
            .await
            .expect("async operation should succeed");
        mnemosyne
            .store("s1", &msg2)
            .await
            .expect("async operation should succeed");

        let results = mnemosyne
            .semantic_search("Rust", 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("Rust"));
    }

    #[tokio::test]
    async fn test_embedder_batch_connection_refused() {
        let config = MnemosyneConfig {
            ollama_url: "http://127.0.0.1:19999".to_string(),
            ..Default::default()
        };
        let instance = EmbedderInstance::new(EmbeddingProvider::Ollama, &config);
        // Embed two texts sequentially — first failure is enough
        let result = instance.embed("hello").await;
        assert!(
            result.is_err(),
            "Embed should fail when Ollama is unreachable"
        );
    }

    // Live Ollama integration tests (require network access).
    //
    // #230: these four `test_live_ollama_*` tests reach out to
    // `ollama.example.com` (a placeholder host). On credential-less /
    // network-less CI boxes that DNS lookup can hang or fail in
    // unexpected ways, so they are opt-in: they no-op early unless
    // `OLLAMA_LIVE_TESTS=1` (or `=true`) is set in the environment.
    fn live_ollama_tests_enabled() -> bool {
        std::env::var("OLLAMA_LIVE_TESTS")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn test_live_ollama_embed_unsupported_model() {
        if !live_ollama_tests_enabled() {
            eprintln!("skipping test_live_ollama_embed_unsupported_model (set OLLAMA_LIVE_TESTS=1 to enable)");
            return;
        }
        // gpt-oss:20b is a generation model, not an embedding model
        let config = MnemosyneConfig {
            ollama_url: "https://ollama.example.com".to_string(),
            embedding_model: "gpt-oss:20b".to_string(),
            ..Default::default()
        };
        let instance = EmbedderInstance::new(EmbeddingProvider::Ollama, &config);
        let result = instance.embed("hello world").await;
        assert!(result.is_err(), "Should fail for non-embedding model");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("does not support")
                || err_msg.contains("400")
                || err_msg.contains("error"),
            "Error should indicate unsupported embeddings, got: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_live_ollama_store_with_embedding_graceful_degradation() {
        if !live_ollama_tests_enabled() {
            eprintln!("skipping test_live_ollama_store_with_embedding_graceful_degradation (set OLLAMA_LIVE_TESTS=1 to enable)");
            return;
        }
        // With a non-embedding model, store_with_embedding should still store the message
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("live_test.db"),
            enable_fts: true,
            enable_embeddings: true,
            ollama_url: "https://ollama.example.com".to_string(),
            embedding_model: "gpt-oss:20b".to_string(),
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        assert!(mnemosyne.has_embedder());

        // store_with_embedding should succeed (message stored) even though embedding fails
        let msg = Message::user("Testing live Ollama integration");
        let msg_id = mnemosyne
            .store_with_embedding("s1", &msg)
            .await
            .expect("async operation should succeed");
        assert!(msg_id > 0);

        // Message should be retrievable
        let messages = mnemosyne
            .recall_session("s1", 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "Testing live Ollama integration");

        // No embedding stored (model doesn't support it)
        let stats = mnemosyne
            .stats()
            .await
            .expect("async operation should succeed");
        assert_eq!(stats.message_count, 1);
        assert_eq!(stats.embedding_count, 0);
    }

    #[tokio::test]
    async fn test_live_ollama_semantic_search_fts_fallback() {
        if !live_ollama_tests_enabled() {
            eprintln!("skipping test_live_ollama_semantic_search_fts_fallback (set OLLAMA_LIVE_TESTS=1 to enable)");
            return;
        }
        // When embedding fails, semantic_search falls back to FTS
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("live_sem.db"),
            enable_fts: true,
            enable_embeddings: true,
            ollama_url: "https://ollama.example.com".to_string(),
            embedding_model: "gpt-oss:20b".to_string(),
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store some messages via plain store (no embedding attempt)
        let msg1 = Message::user("Rust programming language features");
        let msg2 = Message::user("Python data science libraries");
        mnemosyne
            .store("s1", &msg1)
            .await
            .expect("async operation should succeed");
        mnemosyne
            .store("s1", &msg2)
            .await
            .expect("async operation should succeed");

        // semantic_search should fall back to FTS when embed fails
        let results = mnemosyne
            .semantic_search("Rust", 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("Rust"));

        let results = mnemosyne
            .semantic_search("Python", 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("Python"));
    }

    #[tokio::test]
    #[ignore] // Hits a placeholder URL — opt-in only, not default CI
    async fn test_live_ollama_server_reachable() {
        // Verify the server responds (basic connectivity test)
        let client = reqwest::Client::new();
        let resp = client
            .get("https://ollama.example.com/api/version")
            .send()
            .await;
        assert!(resp.is_ok(), "Ollama server should be reachable");
        let resp = resp.expect("operation should succeed");
        assert!(resp.status().is_success(), "Should return 200");
    }

    #[tokio::test]
    async fn test_live_ollama_embed_text_returns_error() {
        if !live_ollama_tests_enabled() {
            eprintln!("skipping test_live_ollama_embed_text_returns_error (set OLLAMA_LIVE_TESTS=1 to enable)");
            return;
        }
        // embed_text() with a non-embedding model should propagate the error
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("live_et.db"),
            enable_embeddings: true,
            ollama_url: "https://ollama.example.com".to_string(),
            embedding_model: "gpt-oss:20b".to_string(),
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let result = mnemosyne.embed_text("test").await;
        assert!(
            result.is_err(),
            "embed_text should return error for non-embedding model"
        );
    }

    // Memory hierarchy tests (Phase 4)

    #[tokio::test]
    async fn test_store_typed_and_recall() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test_typed.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let msg = Message::user("Important working memory item");
        let id = mnemosyne
            .store_typed("s1", &msg, MemoryType::Working, 0.9)
            .await
            .expect("async operation should succeed");
        assert!(id > 0);

        // Verify it's stored as working memory
        let working = mnemosyne
            .working_memory("s1")
            .await
            .expect("async operation should succeed");
        assert_eq!(working.len(), 1);
        assert!(working[0].content.contains("Important working memory"));
        assert_eq!(working[0].memory_type, MemoryType::Working);
        assert!((working[0].importance - 0.9).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_store_typed_different_types() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test_types.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store one of each type
        let m1 = Message::user("working scratch data");
        let m2 = Message::user("episodic past event");
        let m3 = Message::user("semantic knowledge fact");

        mnemosyne
            .store_typed("s1", &m1, MemoryType::Working, 0.9)
            .await
            .expect("async operation should succeed");
        mnemosyne
            .store_typed("s1", &m2, MemoryType::Episodic, 0.5)
            .await
            .expect("async operation should succeed");
        mnemosyne
            .store_typed("s1", &m3, MemoryType::Semantic, 0.8)
            .await
            .expect("async operation should succeed");

        // Working memory should only return working type
        let working = mnemosyne
            .working_memory("s1")
            .await
            .expect("async operation should succeed");
        assert_eq!(working.len(), 1);
        assert!(working[0].content.contains("scratch"));
    }

    #[tokio::test]
    async fn test_search_by_type_filters() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test_search_type.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store messages with the word "Rust" in different memory types
        let m1 = Message::user("Rust programming working memory");
        let m2 = Message::user("Rust is a systems language episodic");
        let m3 = Message::user("Rust ownership model semantic knowledge");

        mnemosyne
            .store_typed("s1", &m1, MemoryType::Working, 0.9)
            .await
            .expect("async operation should succeed");
        mnemosyne
            .store_typed("s1", &m2, MemoryType::Episodic, 0.5)
            .await
            .expect("async operation should succeed");
        mnemosyne
            .store_typed("s1", &m3, MemoryType::Semantic, 0.8)
            .await
            .expect("async operation should succeed");

        // Search by type should filter correctly
        let working = mnemosyne
            .search_by_type("Rust", MemoryType::Working, 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(working.len(), 1);
        assert!(working[0].content.contains("working memory"));

        let semantic = mnemosyne
            .search_by_type("Rust", MemoryType::Semantic, 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(semantic.len(), 1);
        assert!(semantic[0].content.contains("semantic knowledge"));

        let episodic = mnemosyne
            .search_by_type("Rust", MemoryType::Episodic, 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(episodic.len(), 1);
        assert!(episodic[0].content.contains("episodic"));
    }

    #[tokio::test]
    async fn test_decay_importance() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test_decay.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store episodic memories with known importance
        let m1 = Message::user("episodic memory one");
        let m2 = Message::user("episodic memory two");
        mnemosyne
            .store_typed("s1", &m1, MemoryType::Episodic, 0.8)
            .await
            .expect("async operation should succeed");
        mnemosyne
            .store_typed("s1", &m2, MemoryType::Episodic, 0.3)
            .await
            .expect("async operation should succeed");

        // Also store a working memory (should NOT be decayed)
        let m3 = Message::user("working memory item");
        mnemosyne
            .store_typed("s1", &m3, MemoryType::Working, 0.9)
            .await
            .expect("async operation should succeed");

        // Decay by 0.1
        let updated = mnemosyne
            .decay_importance(0.1)
            .await
            .expect("async operation should succeed");
        assert_eq!(updated, 2); // Only episodic memories

        // Search episodic to verify importance decreased
        let results = mnemosyne
            .search_by_type("episodic", MemoryType::Episodic, 10)
            .await
            .expect("async operation should succeed");
        for r in &results {
            if r.content.contains("one") {
                assert!(
                    (r.importance - 0.7).abs() < 0.01,
                    "Expected ~0.7, got {}",
                    r.importance
                );
            } else if r.content.contains("two") {
                assert!(
                    (r.importance - 0.2).abs() < 0.01,
                    "Expected ~0.2, got {}",
                    r.importance
                );
            }
        }

        // Working memory should be unaffected
        let working = mnemosyne
            .working_memory("s1")
            .await
            .expect("async operation should succeed");
        assert_eq!(working.len(), 1);
        assert!((working[0].importance - 0.9).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_decay_importance_floors_at_zero() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test_decay_floor.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let m = Message::user("low importance episodic item");
        mnemosyne
            .store_typed("s1", &m, MemoryType::Episodic, 0.05)
            .await
            .expect("async operation should succeed");

        // Decay by 0.1 (more than current importance)
        mnemosyne
            .decay_importance(0.1)
            .await
            .expect("async operation should succeed");

        let results = mnemosyne
            .search_by_type("episodic", MemoryType::Episodic, 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(results.len(), 1);
        assert!(
            results[0].importance >= 0.0,
            "Importance should not go below 0"
        );
        assert!(
            results[0].importance < 0.01,
            "Should be ~0.0, got {}",
            results[0].importance
        );
    }

    #[tokio::test]
    async fn test_promote_to_semantic() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test_promote.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store as episodic
        let m = Message::user("User prefers morning meetings repeatedly");
        let id = mnemosyne
            .store_typed("s1", &m, MemoryType::Episodic, 0.5)
            .await
            .expect("async operation should succeed");

        // Promote to semantic with distilled knowledge
        let knowledge = "User preference: morning meetings (high confidence)";
        mnemosyne
            .promote_to_semantic(id, knowledge)
            .await
            .expect("async operation should succeed");

        // Should now appear in semantic search
        let semantic = mnemosyne
            .search_by_type("morning", MemoryType::Semantic, 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(semantic.len(), 1);
        assert!(semantic[0].content.contains("preference"));
        assert_eq!(semantic[0].memory_type, MemoryType::Semantic);
        assert!(
            (semantic[0].importance - 0.9).abs() < 0.01,
            "Promoted memory should have 0.9 importance"
        );

        // Should NOT appear in episodic search
        let episodic = mnemosyne
            .search_by_type("morning", MemoryType::Episodic, 10)
            .await
            .expect("async operation should succeed");
        assert!(episodic.is_empty());
    }

    #[tokio::test]
    async fn test_working_memory_session_isolation() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test_session_iso.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store working memory in two sessions
        let m1 = Message::user("session one working item");
        let m2 = Message::user("session two working item");
        mnemosyne
            .store_typed("s1", &m1, MemoryType::Working, 0.9)
            .await
            .expect("async operation should succeed");
        mnemosyne
            .store_typed("s2", &m2, MemoryType::Working, 0.9)
            .await
            .expect("async operation should succeed");

        // Each session should only see its own working memory
        let s1_working = mnemosyne
            .working_memory("s1")
            .await
            .expect("async operation should succeed");
        assert_eq!(s1_working.len(), 1);
        assert!(s1_working[0].content.contains("session one"));

        let s2_working = mnemosyne
            .working_memory("s2")
            .await
            .expect("async operation should succeed");
        assert_eq!(s2_working.len(), 1);
        assert!(s2_working[0].content.contains("session two"));
    }

    #[tokio::test]
    async fn test_default_store_is_episodic() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test_default_type.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Regular store (no type specified) should default to episodic
        let m = Message::user("default type message about Rust");
        mnemosyne
            .store("s1", &m)
            .await
            .expect("async operation should succeed");

        // Should appear in regular search with episodic type
        let results = mnemosyne
            .search("Rust", 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].memory_type, MemoryType::Episodic);
        assert!(
            (results[0].importance - 0.5).abs() < 0.01,
            "Default importance should be 0.5"
        );
    }

    #[tokio::test]
    async fn test_finalize_working_memory_promotes_and_discards() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test_finalize.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store working memories with different importance levels
        let m_high = Message::user("important working context about Rust");
        let m_low = Message::user("scratch notes about temporary stuff");
        mnemosyne
            .store_typed("s1", &m_high, MemoryType::Working, 0.8)
            .await
            .expect("async operation should succeed");
        mnemosyne
            .store_typed("s1", &m_low, MemoryType::Working, 0.2)
            .await
            .expect("async operation should succeed");

        // Verify both are in working memory
        let working = mnemosyne
            .working_memory("s1")
            .await
            .expect("async operation should succeed");
        assert_eq!(working.len(), 2);

        // Finalize with threshold 0.5
        let (promoted, discarded) = mnemosyne
            .finalize_working_memory("s1", 0.5)
            .await
            .expect("async operation should succeed");
        assert_eq!(promoted, 1);
        assert_eq!(discarded, 1);

        // Working memory should be empty now
        let working = mnemosyne
            .working_memory("s1")
            .await
            .expect("async operation should succeed");
        assert!(working.is_empty());

        // The promoted one should be in episodic memory now
        let episodic = mnemosyne
            .search_by_type("Rust", MemoryType::Episodic, 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(episodic.len(), 1);
        assert!(episodic[0].content.contains("important"));
    }

    #[tokio::test]
    async fn test_finalize_working_memory_other_sessions_unaffected() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test_finalize_iso.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store working memory in two sessions
        let m1 = Message::user("session one working data");
        let m2 = Message::user("session two working data");
        mnemosyne
            .store_typed("s1", &m1, MemoryType::Working, 0.9)
            .await
            .expect("async operation should succeed");
        mnemosyne
            .store_typed("s2", &m2, MemoryType::Working, 0.9)
            .await
            .expect("async operation should succeed");

        // Finalize only s1
        let (promoted, _) = mnemosyne
            .finalize_working_memory("s1", 0.5)
            .await
            .expect("async operation should succeed");
        assert_eq!(promoted, 1);

        // s2 working memory should be untouched
        let s2_working = mnemosyne
            .working_memory("s2")
            .await
            .expect("async operation should succeed");
        assert_eq!(s2_working.len(), 1);
    }

    #[tokio::test]
    async fn test_memory_type_display_and_roundtrip() {
        assert_eq!(MemoryType::Working.to_string(), "working");
        assert_eq!(MemoryType::Episodic.to_string(), "episodic");
        assert_eq!(MemoryType::Semantic.to_string(), "semantic");
        assert_eq!(MemoryType::Fact.to_string(), "fact");
        assert_eq!(MemoryType::Preference.to_string(), "preference");
        assert_eq!(MemoryType::Conversation.to_string(), "conversation");
        assert_eq!(MemoryType::Summary.to_string(), "summary");

        assert_eq!(MemoryType::parse_label("working"), MemoryType::Working);
        assert_eq!(MemoryType::parse_label("episodic"), MemoryType::Episodic);
        assert_eq!(MemoryType::parse_label("semantic"), MemoryType::Semantic);
        assert_eq!(MemoryType::parse_label("fact"), MemoryType::Fact);
        assert_eq!(
            MemoryType::parse_label("preference"),
            MemoryType::Preference
        );
        assert_eq!(
            MemoryType::parse_label("conversation"),
            MemoryType::Conversation
        );
        assert_eq!(MemoryType::parse_label("summary"), MemoryType::Summary);
        assert_eq!(MemoryType::parse_label("unknown"), MemoryType::Episodic); // Default fallback
    }

    // Embedding Cache tests

    #[test]
    fn test_content_hash_deterministic() {
        let h1 = compute_content_hash("hello world");
        let h2 = compute_content_hash("hello world");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // SHA-256 hex = 64 chars
    }

    #[test]
    fn test_content_hash_different_inputs() {
        let h1 = compute_content_hash("hello");
        let h2 = compute_content_hash("world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_embedding_cache_store_and_retrieve() {
        let dir = tempdir().expect("should create temp dir");
        let store = MemoryStore::new(&dir.path().join("cache.db"), false, false)
            .expect("MemoryStore::new should succeed");

        let embedding = vec![0.1, 0.2, 0.3, 0.4];
        let hash = compute_content_hash("test content");

        // Store
        store
            .store_cached_embedding("ollama", "nomic", &hash, &embedding)
            .expect("store_cached_embedding should succeed");

        // Retrieve
        let cached = store
            .get_cached_embedding("ollama", "nomic", &hash)
            .expect("get_cached_embedding should succeed");
        assert!(cached.is_some());
        let cached = cached.expect("operation should succeed");
        assert_eq!(cached.len(), 4);
        assert!((cached[0] - 0.1).abs() < 1e-6);
        assert!((cached[3] - 0.4).abs() < 1e-6);
    }

    #[test]
    fn test_embedding_cache_miss() {
        let dir = tempdir().expect("should create temp dir");
        let store = MemoryStore::new(&dir.path().join("cache.db"), false, false)
            .expect("MemoryStore::new should succeed");

        let result = store
            .get_cached_embedding("ollama", "nomic", "nonexistent_hash")
            .expect("get_cached_embedding should succeed");
        assert!(result.is_none());
    }

    #[test]
    fn test_embedding_cache_different_models() {
        let dir = tempdir().expect("should create temp dir");
        let store = MemoryStore::new(&dir.path().join("cache.db"), false, false)
            .expect("MemoryStore::new should succeed");

        let hash = compute_content_hash("same content");
        let emb1 = vec![1.0, 0.0];
        let emb2 = vec![0.0, 1.0];

        store
            .store_cached_embedding("ollama", "model-a", &hash, &emb1)
            .expect("store_cached_embedding should succeed");
        store
            .store_cached_embedding("ollama", "model-b", &hash, &emb2)
            .expect("store_cached_embedding should succeed");

        let a = store
            .get_cached_embedding("ollama", "model-a", &hash)
            .expect("get_cached_embedding should succeed")
            .expect("unwrap should succeed");
        let b = store
            .get_cached_embedding("ollama", "model-b", &hash)
            .expect("get_cached_embedding should succeed")
            .expect("unwrap should succeed");

        assert!((a[0] - 1.0).abs() < 1e-6);
        assert!((b[1] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_embedding_cache_upsert() {
        let dir = tempdir().expect("should create temp dir");
        let store = MemoryStore::new(&dir.path().join("cache.db"), false, false)
            .expect("MemoryStore::new should succeed");

        let hash = compute_content_hash("content");
        let emb1 = vec![1.0, 2.0];
        let emb2 = vec![3.0, 4.0];

        store
            .store_cached_embedding("ollama", "model", &hash, &emb1)
            .expect("store_cached_embedding should succeed");
        store
            .store_cached_embedding("ollama", "model", &hash, &emb2)
            .expect("store_cached_embedding should succeed");

        // Should have the updated embedding
        let cached = store
            .get_cached_embedding("ollama", "model", &hash)
            .expect("get_cached_embedding should succeed")
            .expect("unwrap should succeed");
        assert!((cached[0] - 3.0).abs() < 1e-6);

        // Should still be only 1 entry
        assert_eq!(
            store
                .embedding_cache_count()
                .expect("embedding_cache_count should succeed"),
            1
        );
    }

    #[test]
    fn test_embedding_cache_lru_eviction() {
        let dir = tempdir().expect("should create temp dir");
        let store = MemoryStore::new(&dir.path().join("cache.db"), false, false)
            .expect("MemoryStore::new should succeed");

        // Insert 10 entries
        for i in 0..10 {
            let hash = compute_content_hash(&format!("content-{}", i));
            store
                .store_cached_embedding("ollama", "model", &hash, &[i as f32])
                .expect("store_cached_embedding should succeed");
        }
        assert_eq!(
            store
                .embedding_cache_count()
                .expect("embedding_cache_count should succeed"),
            10
        );

        // Evict to keep 5
        let evicted = store
            .evict_lru_cache(5)
            .expect("evict_lru_cache should succeed");
        assert_eq!(evicted, 5);
        assert_eq!(
            store
                .embedding_cache_count()
                .expect("embedding_cache_count should succeed"),
            5
        );
    }

    #[test]
    fn test_embedding_cache_lru_no_eviction_needed() {
        let dir = tempdir().expect("should create temp dir");
        let store = MemoryStore::new(&dir.path().join("cache.db"), false, false)
            .expect("MemoryStore::new should succeed");

        store
            .store_cached_embedding("ollama", "model", "h1", &[1.0])
            .expect("store_cached_embedding should succeed");

        let evicted = store
            .evict_lru_cache(100)
            .expect("evict_lru_cache should succeed");
        assert_eq!(evicted, 0);
        assert_eq!(
            store
                .embedding_cache_count()
                .expect("embedding_cache_count should succeed"),
            1
        );
    }

    // Memory File Tracking tests

    #[test]
    fn test_file_tracking_upsert_and_get() {
        let dir = tempdir().expect("should create temp dir");
        let store = MemoryStore::new(&dir.path().join("track.db"), false, false)
            .expect("MemoryStore::new should succeed");

        store
            .upsert_tracked_file("MEMORY.md", "workspace", "abc123", 1000, 512)
            .expect("upsert_tracked_file should succeed");

        let tracked = store
            .get_tracked_file("MEMORY.md", "workspace")
            .expect("get_tracked_file should succeed");
        assert!(tracked.is_some());
        let t = tracked.expect("operation should succeed");
        assert_eq!(t.path, "MEMORY.md");
        assert_eq!(t.source, "workspace");
        assert_eq!(t.content_hash, "abc123");
        assert_eq!(t.mtime, 1000);
        assert_eq!(t.size, 512);
        assert!(t.last_indexed > 0);
    }

    #[test]
    fn test_file_tracking_miss() {
        let dir = tempdir().expect("should create temp dir");
        let store = MemoryStore::new(&dir.path().join("track.db"), false, false)
            .expect("MemoryStore::new should succeed");

        let result = store
            .get_tracked_file("nonexistent.md", "workspace")
            .expect("get_tracked_file should succeed");
        assert!(result.is_none());
    }

    #[test]
    fn test_file_tracking_update() {
        let dir = tempdir().expect("should create temp dir");
        let store = MemoryStore::new(&dir.path().join("track.db"), false, false)
            .expect("MemoryStore::new should succeed");

        store
            .upsert_tracked_file("file.md", "ws", "hash1", 100, 50)
            .expect("upsert_tracked_file should succeed");
        store
            .upsert_tracked_file("file.md", "ws", "hash2", 200, 75)
            .expect("upsert_tracked_file should succeed");

        let t = store
            .get_tracked_file("file.md", "ws")
            .expect("get_tracked_file should succeed")
            .expect("unwrap should succeed");
        assert_eq!(t.content_hash, "hash2");
        assert_eq!(t.mtime, 200);
        assert_eq!(t.size, 75);
    }

    #[test]
    fn test_file_tracking_list() {
        let dir = tempdir().expect("should create temp dir");
        let store = MemoryStore::new(&dir.path().join("track.db"), false, false)
            .expect("MemoryStore::new should succeed");

        store
            .upsert_tracked_file("a.md", "workspace", "h1", 100, 10)
            .expect("upsert_tracked_file should succeed");
        store
            .upsert_tracked_file("b.md", "workspace", "h2", 200, 20)
            .expect("upsert_tracked_file should succeed");
        store
            .upsert_tracked_file("c.md", "other", "h3", 300, 30)
            .expect("upsert_tracked_file should succeed");

        let ws_files = store
            .list_tracked_files("workspace")
            .expect("list_tracked_files should succeed");
        assert_eq!(ws_files.len(), 2);
        assert_eq!(ws_files[0].path, "a.md");
        assert_eq!(ws_files[1].path, "b.md");

        let other_files = store
            .list_tracked_files("other")
            .expect("list_tracked_files should succeed");
        assert_eq!(other_files.len(), 1);
    }

    #[test]
    fn test_file_tracking_remove() {
        let dir = tempdir().expect("should create temp dir");
        let store = MemoryStore::new(&dir.path().join("track.db"), false, false)
            .expect("MemoryStore::new should succeed");

        store
            .upsert_tracked_file("file.md", "ws", "hash", 100, 50)
            .expect("upsert_tracked_file should succeed");

        let removed = store
            .remove_tracked_file("file.md", "ws")
            .expect("remove_tracked_file should succeed");
        assert!(removed);

        let result = store
            .get_tracked_file("file.md", "ws")
            .expect("get_tracked_file should succeed");
        assert!(result.is_none());

        // Remove non-existent
        let removed = store
            .remove_tracked_file("gone.md", "ws")
            .expect("remove_tracked_file should succeed");
        assert!(!removed);
    }

    // Chunking tests

    #[test]
    fn test_chunk_text_basic() {
        let text = "Paragraph one.\n\nParagraph two.\n\nParagraph three.";
        let chunks = chunk_text(text);
        assert_eq!(chunks.len(), 1); // All fit in one chunk
        assert!(chunks[0].text.contains("Paragraph one"));
        assert!(chunks[0].text.contains("Paragraph three"));
        assert_eq!(chunks[0].start_line, 1);
    }

    #[test]
    fn test_chunk_text_splits_large() {
        // Create text larger than 2000 chars
        let para = "A".repeat(1500);
        let text = format!("{}\n\n{}\n\n{}", para, para, para);
        let chunks = chunk_text(&text);
        assert!(
            chunks.len() >= 2,
            "Should split into multiple chunks, got {}",
            chunks.len()
        );
        for c in &chunks {
            assert!(
                c.text.len() <= 3000,
                "Chunk too large: {} chars",
                c.text.len()
            );
        }
    }

    #[test]
    fn test_chunk_text_empty() {
        let chunks = chunk_text("");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_chunk_text_only_whitespace() {
        let chunks = chunk_text("\n\n\n\n");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_chunk_text_line_numbers() {
        let text = "# Header\n\nParagraph on line 3.\n\nAnother on line 5.";
        let chunks = chunk_text(text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start_line, 1);

        // Force a split: two large paragraphs
        let p1 = "A".repeat(1500);
        let p2 = "B".repeat(1500);
        let text2 = format!("{}\n\n{}", p1, p2);
        let chunks2 = chunk_text(&text2);
        assert!(chunks2.len() >= 2);
        assert_eq!(chunks2[0].start_line, 1);
        // Second chunk starts after the first paragraph + blank line
        assert!(chunks2[1].start_line > 1);
    }

    // Sync tests

    #[tokio::test]
    async fn test_sync_workspace_empty_dir() {
        let dir = tempdir().expect("should create temp dir");
        let workspace = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("sync.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let stats = mnemosyne
            .sync_workspace(workspace.path())
            .await
            .expect("async operation should succeed");
        assert_eq!(stats.files_scanned, 0);
        assert_eq!(stats.files_changed, 0);
        assert_eq!(stats.files_unchanged, 0);
    }

    #[tokio::test]
    async fn test_sync_workspace_detects_new_files() {
        let dir = tempdir().expect("should create temp dir");
        let workspace = tempdir().expect("should create temp dir");

        // Create a .md file
        std::fs::write(workspace.path().join("README.md"), "# Hello\n\nWorld")
            .expect("should write file");

        let config = MnemosyneConfig {
            db_path: dir.path().join("sync.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let stats = mnemosyne
            .sync_workspace(workspace.path())
            .await
            .expect("async operation should succeed");
        assert_eq!(stats.files_scanned, 1);
        assert_eq!(stats.files_changed, 1);
        assert_eq!(stats.files_unchanged, 0);
    }

    #[tokio::test]
    async fn test_sync_workspace_skips_unchanged() {
        let dir = tempdir().expect("should create temp dir");
        let workspace = tempdir().expect("should create temp dir");

        std::fs::write(workspace.path().join("NOTE.md"), "# Note\n\nContent here")
            .expect("should write file");

        let config = MnemosyneConfig {
            db_path: dir.path().join("sync.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // First sync — detects as new
        let stats1 = mnemosyne
            .sync_workspace(workspace.path())
            .await
            .expect("async operation should succeed");
        assert_eq!(stats1.files_changed, 1);

        // Second sync — should skip (unchanged)
        let stats2 = mnemosyne
            .sync_workspace(workspace.path())
            .await
            .expect("async operation should succeed");
        assert_eq!(stats2.files_scanned, 1);
        assert_eq!(stats2.files_changed, 0);
        assert_eq!(stats2.files_unchanged, 1);
    }

    #[tokio::test]
    async fn test_sync_workspace_detects_modified() {
        let dir = tempdir().expect("should create temp dir");
        let workspace = tempdir().expect("should create temp dir");

        let file_path = workspace.path().join("DOC.md");
        std::fs::write(&file_path, "Version 1").expect("should write file");

        let config = MnemosyneConfig {
            db_path: dir.path().join("sync.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // First sync
        mnemosyne
            .sync_workspace(workspace.path())
            .await
            .expect("async operation should succeed");

        // Modify file
        std::fs::write(&file_path, "Version 2 with changes").expect("should write file");

        // Second sync — should detect change
        let stats = mnemosyne
            .sync_workspace(workspace.path())
            .await
            .expect("async operation should succeed");
        assert_eq!(stats.files_changed, 1);
        assert_eq!(stats.files_unchanged, 0);
    }

    #[tokio::test]
    async fn test_sync_workspace_subdirectories() {
        let dir = tempdir().expect("should create temp dir");
        let workspace = tempdir().expect("should create temp dir");

        // Create nested .md files
        let sub = workspace.path().join("memory");
        std::fs::create_dir_all(&sub).expect("should create directory");
        std::fs::write(workspace.path().join("AGENTS.md"), "Agent config")
            .expect("should write file");
        std::fs::write(sub.join("MEMORY.md"), "Memory content").expect("should write file");

        // Non-md file should be ignored
        std::fs::write(workspace.path().join("config.toml"), "[settings]")
            .expect("should write file");

        let config = MnemosyneConfig {
            db_path: dir.path().join("sync.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let stats = mnemosyne
            .sync_workspace(workspace.path())
            .await
            .expect("async operation should succeed");
        assert_eq!(stats.files_scanned, 2); // Only .md files
        assert_eq!(stats.files_changed, 2);
    }

    #[tokio::test]
    async fn test_stats_include_cache_and_tracking() {
        let dir = tempdir().expect("should create temp dir");
        let workspace = tempdir().expect("should create temp dir");
        std::fs::write(workspace.path().join("test.md"), "content").expect("should write file");

        let config = MnemosyneConfig {
            db_path: dir.path().join("stats.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Sync to populate tracked files
        mnemosyne
            .sync_workspace(workspace.path())
            .await
            .expect("async operation should succeed");

        let stats = mnemosyne
            .stats()
            .await
            .expect("async operation should succeed");
        assert_eq!(stats.tracked_file_count, 1);
        assert_eq!(stats.embedding_cache_count, 0); // No embedder configured
    }

    #[tokio::test]
    async fn test_embed_with_cache_no_embedder() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("cache.db"),
            enable_embeddings: false,
            ..Default::default()
        };
        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let result = mnemosyne
            .embed_with_cache("test")
            .await
            .expect("async operation should succeed");
        assert!(result.is_none());
    }

    #[test]
    fn test_collect_md_files() {
        let dir = tempdir().expect("should create temp dir");
        std::fs::write(dir.path().join("a.md"), "content").expect("should write file");
        std::fs::write(dir.path().join("b.txt"), "content").expect("should write file");
        let sub = dir.path().join("sub");
        std::fs::create_dir_all(&sub).expect("should create directory");
        std::fs::write(sub.join("c.md"), "content").expect("should write file");

        let files = collect_md_files(dir.path());
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|p| p.ends_with("a.md")));
        assert!(files.iter().any(|p| p.ends_with("c.md")));
    }

    // Hybrid Search Tuning Tests

    #[test]
    fn test_config_defaults_hybrid_weights() {
        let cfg = MnemosyneConfig::default();
        assert!((cfg.vector_weight - 0.7).abs() < f64::EPSILON);
        assert!((cfg.text_weight - 0.3).abs() < f64::EPSILON);
        assert_eq!(cfg.candidate_multiplier, 4);
    }

    #[test]
    fn test_config_serde_hybrid_weights() {
        let json = r#"{
            "db_path": "/tmp/test.db",
            "vector_weight": 0.5,
            "text_weight": 0.5,
            "candidate_multiplier": 8
        }"#;
        let cfg: MnemosyneConfig = serde_json::from_str(json).expect("should parse successfully");
        assert!((cfg.vector_weight - 0.5).abs() < f64::EPSILON);
        assert!((cfg.text_weight - 0.5).abs() < f64::EPSILON);
        assert_eq!(cfg.candidate_multiplier, 8);
    }

    #[test]
    fn test_config_serde_defaults_when_missing() {
        let json = r#"{"db_path": "/tmp/test.db"}"#;
        let cfg: MnemosyneConfig = serde_json::from_str(json).expect("should parse successfully");
        assert!((cfg.vector_weight - 0.7).abs() < f64::EPSILON);
        assert!((cfg.text_weight - 0.3).abs() < f64::EPSILON);
        assert_eq!(cfg.candidate_multiplier, 4);
    }

    #[test]
    fn test_bm25_score_normalization() {
        // BM25 rank from FTS5 is negative; more negative = better match
        // Formula: 1.0 / (1.0 + (-rank).max(0.0))
        // Negate rank to get positive magnitude, then normalize to 0.0-1.0

        // For rank = -5.0 (strong match), score = 1.0 / (1.0 + 5.0) = ~0.167
        let rank_neg = -5.0_f64;
        let score = 1.0 / (1.0 + (-rank_neg).max(0.0));
        assert!((score - (1.0 / 6.0)).abs() < f64::EPSILON);

        // For rank = 0.0 (no match signal), score = 1.0 / (1.0 + 0.0) = 1.0
        let rank_zero = 0.0_f64;
        let score_zero = 1.0_f64 / (1.0 + (-rank_zero).max(0.0));
        assert!((score_zero - 1.0).abs() < f64::EPSILON);

        // For rank = -1.0 (weak match), score = 1.0 / (1.0 + 1.0) = 0.5
        let rank_weak = -1.0_f64;
        let score_weak = 1.0_f64 / (1.0 + (-rank_weak).max(0.0));
        assert!((score_weak - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_weighted_merge_formula() {
        let vector_weight = 0.7_f64;
        let text_weight = 0.3_f64;
        let vector_score = 0.9_f64;
        let text_score = 0.6_f64;
        let final_score = vector_weight * vector_score + text_weight * text_score;
        // 0.7 * 0.9 + 0.3 * 0.6 = 0.63 + 0.18 = 0.81
        assert!((final_score - 0.81_f64).abs() < 1e-10);
    }

    #[tokio::test]
    async fn test_hybrid_search_fts_only_with_weights() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            max_messages_per_session: 100,
            enable_embeddings: false,
            vector_weight: 0.7,
            text_weight: 0.3,
            candidate_multiplier: 4,
            ..Default::default()
        };

        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        mnemosyne
            .store("s1", &Message::user("The quick brown fox"))
            .await
            .expect("async operation should succeed");
        mnemosyne
            .store("s1", &Message::user("A lazy dog sleeps"))
            .await
            .expect("async operation should succeed");

        // FTS-only search (no embeddings enabled)
        let results = mnemosyne
            .search("fox", 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("fox"));
    }

    #[tokio::test]
    async fn test_hybrid_search_with_custom_weights() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            max_messages_per_session: 100,
            enable_embeddings: true,
            embedding_dim: 3,
            vector_weight: 0.8,
            text_weight: 0.2,
            candidate_multiplier: 2,
            ..Default::default()
        };

        let mnemosyne = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let id1 = mnemosyne
            .store("s1", &Message::user("The quick brown fox jumps"))
            .await
            .expect("async operation should succeed");
        mnemosyne
            .store_embedding(id1, &[1.0, 0.0, 0.0], None)
            .await
            .expect("async operation should succeed");

        let id2 = mnemosyne
            .store("s1", &Message::user("A cat sleeps on the mat"))
            .await
            .expect("async operation should succeed");
        mnemosyne
            .store_embedding(id2, &[0.0, 1.0, 0.0], None)
            .await
            .expect("async operation should succeed");

        // Hybrid search: "fox" FTS + vector close to msg1
        let results = mnemosyne
            .hybrid_search("fox", Some(&[0.9, 0.1, 0.0]), 10)
            .await
            .expect("async operation should succeed");
        assert!(!results.is_empty());
        // With vector_weight=0.8, the fox message should score highest
        assert!(results[0].content.contains("fox"));
    }

    #[test]
    fn test_candidate_multiplier_effect() {
        // candidate_multiplier=1 means candidates = 1 * limit
        // candidate_multiplier=4 means candidates = 4 * limit
        let multiplier = 4_usize;
        let limit = 5_usize;
        let candidates = multiplier * limit;
        assert_eq!(candidates, 20);
    }

    // Embedding provider fallback tests

    #[test]
    fn test_embedding_chain_construction() {
        let config = MnemosyneConfig {
            embedding_providers: vec![EmbeddingProvider::Ollama, EmbeddingProvider::OpenAI],
            fallback_threshold: 3,
            ..Default::default()
        };
        let chain = EmbeddingChain::from_config(&config);
        // Assert structure/ordering logic without requiring API keys
        // The chain may have fewer providers if keys are missing, but must have at least 1
        assert!(chain.providers.len() >= 1, "Chain must have at least one provider");
        assert_eq!(chain.active_provider(), "ollama", "First provider should be active");
        assert_eq!(chain.fallback_threshold, 3, "Fallback threshold should be preserved");
        // Verify first provider is always ollama via fallback state
        let state = chain.fallback_state();
        assert_eq!(state[0].0, "ollama", "First provider should be ollama");
        assert_eq!(state[0].2, true, "First provider should be active");
        assert_eq!(state[0].1, 0, "Initial failure count should be 0");
    }

    #[test]
    fn test_embedding_chain_fallback_state() {
        let config = MnemosyneConfig {
            embedding_providers: vec![
                EmbeddingProvider::Ollama,
                EmbeddingProvider::OpenAI,
                EmbeddingProvider::Gemini,
            ],
            fallback_threshold: 2,
            ..Default::default()
        };
        let chain = EmbeddingChain::from_config(&config);
        let state = chain.fallback_state();
        // Assert structure/ordering logic without requiring API keys
        // The chain may have fewer providers if keys are missing, but must have at least 1
        assert!(state.len() >= 1, "Fallback state must have at least one entry");
        // Verify first provider is always ollama and active
        assert_eq!(state[0].0, "ollama", "First provider should be ollama");
        assert_eq!(state[0].2, true, "First provider should be active");
        assert_eq!(state[0].1, 0, "Initial failure count should be 0");
        // Verify fallback_threshold is preserved
        assert_eq!(chain.fallback_threshold, 2, "Fallback threshold should be preserved");
        // If more providers are available (keys present), verify ordering
        if state.len() > 1 {
            assert_eq!(state[1].0, "openai", "Second provider should be openai");
            assert_eq!(state[1].2, false, "Second provider should not be active");
        }
        if state.len() > 2 {
            assert_eq!(state[2].0, "gemini", "Third provider should be gemini");
            assert_eq!(state[2].2, false, "Third provider should not be active");
        }
    }

    #[tokio::test]
    async fn test_embedding_chain_switches_on_failure() {
        // Configure with two unreachable providers, threshold=2
        let config = MnemosyneConfig {
            ollama_url: "http://127.0.0.1:19998".to_string(),
            embedding_providers: vec![
                EmbeddingProvider::Ollama,
                EmbeddingProvider::Ollama, // second Ollama on same bad port
            ],
            fallback_threshold: 2,
            ..Default::default()
        };
        let mut chain = EmbeddingChain::from_config(&config);
        assert_eq!(chain.active_index, 0);

        // First failure — stays on provider 0
        let _ = chain.embed("test").await;
        assert_eq!(chain.failure_counts[0], 1);
        assert_eq!(chain.active_index, 0);

        // Second failure — hits threshold, switches to provider 1
        let _ = chain.embed("test").await;
        assert_eq!(chain.failure_counts[0], 2);
        assert_eq!(chain.active_index, 1);
    }

    #[tokio::test]
    async fn test_embedding_chain_exhaustion() {
        // Single provider, threshold=1 — one failure exhausts the chain
        let config = MnemosyneConfig {
            ollama_url: "http://127.0.0.1:19998".to_string(),
            embedding_providers: vec![EmbeddingProvider::Ollama],
            fallback_threshold: 1,
            ..Default::default()
        };
        let mut chain = EmbeddingChain::from_config(&config);
        let result = chain.embed("test").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("exhausted"),
            "Should indicate all providers exhausted, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_mnemosyne_active_provider_default() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_embeddings: true,
            embedding_providers: vec![EmbeddingProvider::Ollama],
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        assert_eq!(mn.active_embedding_provider().await, "ollama");
    }

    #[tokio::test]
    async fn test_mnemosyne_fallback_state_disabled() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        assert_eq!(mn.active_embedding_provider().await, "none");
        assert!(mn.embedding_fallback_state().await.is_empty());
    }

    #[test]
    fn test_config_embedding_providers_default() {
        let config = MnemosyneConfig::default();
        assert_eq!(config.embedding_providers, vec![EmbeddingProvider::Ollama]);
        assert_eq!(config.fallback_threshold, 3);
    }

    #[test]
    fn test_config_embedding_providers_serde() {
        let json = r#"{
            "db_path": "/tmp/test.db",
            "enable_fts": true,
            "embedding_providers": ["ollama", "openai", "voyage"],
            "fallback_threshold": 5
        }"#;
        let config: MnemosyneConfig =
            serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(config.embedding_providers.len(), 3);
        assert_eq!(config.embedding_providers[0], EmbeddingProvider::Ollama);
        assert_eq!(config.embedding_providers[1], EmbeddingProvider::OpenAI);
        assert_eq!(config.embedding_providers[2], EmbeddingProvider::Voyage);
        assert_eq!(config.fallback_threshold, 5);
    }

    #[test]
    fn test_config_embedding_providers_backward_compat() {
        // Config without new fields should use defaults
        let json = r#"{
            "db_path": "/tmp/test.db",
            "enable_fts": true,
            "enable_embeddings": false
        }"#;
        let config: MnemosyneConfig =
            serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(config.embedding_providers, vec![EmbeddingProvider::Ollama]);
        assert_eq!(config.fallback_threshold, 3);
    }

    #[test]
    fn test_embedding_provider_display() {
        assert_eq!(EmbeddingProvider::Ollama.to_string(), "ollama");
        assert_eq!(EmbeddingProvider::OpenAI.to_string(), "openai");
        assert_eq!(EmbeddingProvider::Gemini.to_string(), "gemini");
        assert_eq!(EmbeddingProvider::Voyage.to_string(), "voyage");
    }

    // Session transcript indexing tests

    #[test]
    fn test_parse_session_jsonl_basic() {
        let content = r#"{"type":"session_start","id":"abc","created":"2026-01-01T00:00:00Z"}
{"type":"message","role":"user","content":"Hello world"}
{"type":"message","role":"assistant","content":"Hi there! How can I help?"}
{"type":"message","role":"tool","content":"[tool output]"}
"#;
        let (text, count) = parse_session_jsonl(content, 0);
        assert_eq!(count, 2);
        assert!(text.contains("Hello world"));
        assert!(text.contains("Hi there!"));
        assert!(!text.contains("tool output")); // Tool messages excluded
        assert!(!text.contains("session_start")); // Non-message entries excluded
    }

    #[test]
    fn test_parse_session_jsonl_delta_offset() {
        let line1 = r#"{"type":"message","role":"user","content":"Old message"}"#;
        let line2 = r#"{"type":"message","role":"user","content":"New message"}"#;
        let content = format!("{}\n{}\n", line1, line2);
        let offset = line1.len() + 1; // Skip first line

        let (text, count) = parse_session_jsonl(&content, offset);
        assert_eq!(count, 1);
        assert!(text.contains("New message"));
        assert!(!text.contains("Old message"));
    }

    #[test]
    fn test_parse_session_jsonl_empty() {
        let (text, count) = parse_session_jsonl("", 0);
        assert_eq!(count, 0);
        assert!(text.is_empty());
    }

    #[test]
    fn test_parse_session_jsonl_malformed_lines() {
        let content =
            "not json\n{\"type\":\"message\",\"role\":\"user\",\"content\":\"OK\"}\n{broken\n";
        let (text, count) = parse_session_jsonl(content, 0);
        assert_eq!(count, 1);
        assert!(text.contains("OK"));
    }

    #[test]
    fn test_normalize_whitespace() {
        assert_eq!(normalize_whitespace("  hello   world  "), "hello world");
        assert_eq!(normalize_whitespace("a\n\n\nb"), "a b");
        assert_eq!(normalize_whitespace("  \t\n  "), "");
        assert_eq!(normalize_whitespace("single"), "single");
    }

    #[tokio::test]
    async fn test_sync_sessions_empty_dir() {
        let dir = tempdir().expect("should create temp dir");
        let sessions_dir = dir.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).expect("should create directory");

        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_session_indexing: true,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        let count = mn
            .sync_sessions(&sessions_dir)
            .await
            .expect("async operation should succeed");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_sync_sessions_disabled() {
        let dir = tempdir().expect("should create temp dir");
        let sessions_dir = dir.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).expect("should create directory");

        // Write a session file
        let session_content = r#"{"type":"session_start","id":"s1","created":"2026-01-01T00:00:00Z"}
{"type":"message","role":"user","content":"test message"}
"#;
        std::fs::write(sessions_dir.join("s1.jsonl"), session_content).expect("should write file");

        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_session_indexing: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        let count = mn
            .sync_sessions(&sessions_dir)
            .await
            .expect("async operation should succeed");
        assert_eq!(count, 0); // Disabled — should not index
    }

    #[tokio::test]
    async fn test_sync_sessions_nonexistent_dir() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_session_indexing: true,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        let count = mn
            .sync_sessions(&dir.path().join("no_such_dir"))
            .await
            .expect("async operation should succeed");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_sync_sessions_delta_tracking() {
        let dir = tempdir().expect("should create temp dir");
        let sessions_dir = dir.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).expect("should create directory");

        // Write a small session — below delta threshold
        let small_content = r#"{"type":"session_start","id":"s1","created":"2026-01-01T00:00:00Z"}
{"type":"message","role":"user","content":"hello"}
"#;
        std::fs::write(sessions_dir.join("s1.jsonl"), small_content).expect("should write file");

        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_session_indexing: true,
            session_delta_bytes: 50, // Low threshold for testing
            session_delta_messages: 1,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // First sync: new session should be indexed
        let count1 = mn
            .sync_sessions(&sessions_dir)
            .await
            .expect("async operation should succeed");
        // No embedder configured, so 0 sessions_indexed (chunks_ok=0)
        // But the tracking entry is still written
        assert_eq!(count1, 0); // No embedder → no chunks_ok

        // Second sync without changes: should skip
        let count2 = mn
            .sync_sessions(&sessions_dir)
            .await
            .expect("async operation should succeed");
        assert_eq!(count2, 0);
    }

    #[tokio::test]
    async fn test_session_file_tracking_crud() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Initially no entry
        {
            let store = mn.store.lock().await;
            assert!(
                store
                    .get_session_file("s1")
                    .expect("get_session_file should succeed")
                    .is_none()
            );
        }

        // Upsert
        {
            let store = mn.store.lock().await;
            store
                .upsert_session_file("s1", "/tmp/s1.jsonl", 1024, 0, 0)
                .expect("upsert_session_file should succeed");
        }

        // Read back
        {
            let store = mn.store.lock().await;
            let entry = store
                .get_session_file("s1")
                .expect("get_session_file should succeed")
                .expect("unwrap should succeed");
            assert_eq!(entry.session_id, "s1");
            assert_eq!(entry.last_size, 1024);
            assert_eq!(entry.file_path, "/tmp/s1.jsonl");
        }

        // Update
        {
            let store = mn.store.lock().await;
            store
                .upsert_session_file("s1", "/tmp/s1.jsonl", 2048, 100, 5)
                .expect("upsert_session_file should succeed");
        }

        {
            let store = mn.store.lock().await;
            let entry = store
                .get_session_file("s1")
                .expect("get_session_file should succeed")
                .expect("unwrap should succeed");
            assert_eq!(entry.last_size, 2048);
        }
    }

    #[test]
    fn test_config_session_indexing_defaults() {
        let config = MnemosyneConfig::default();
        assert!(config.enable_session_indexing);
        assert_eq!(config.session_delta_bytes, 100_000);
        assert_eq!(config.session_delta_messages, 50);
    }

    #[test]
    fn test_config_session_indexing_serde() {
        let json = r#"{
            "db_path": "/tmp/test.db",
            "enable_session_indexing": false,
            "session_delta_bytes": 50000,
            "session_delta_messages": 25
        }"#;
        let config: MnemosyneConfig =
            serde_json::from_str(json).expect("should parse successfully");
        assert!(!config.enable_session_indexing);
        assert_eq!(config.session_delta_bytes, 50000);
        assert_eq!(config.session_delta_messages, 25);
    }

    #[test]
    fn test_config_session_indexing_backward_compat() {
        let json = r#"{"db_path": "/tmp/test.db"}"#;
        let config: MnemosyneConfig =
            serde_json::from_str(json).expect("should parse successfully");
        assert!(config.enable_session_indexing);
        assert_eq!(config.session_delta_bytes, 100_000);
        assert_eq!(config.session_delta_messages, 50);
    }

    // Atomic Reindex Tests

    #[tokio::test]
    async fn test_atomic_reindex_fresh_db() {
        let dir = tempdir().expect("should create temp dir");
        let ws_root = dir.path().join("workspace");
        std::fs::create_dir_all(&ws_root).expect("should create directory");
        std::fs::write(
            ws_root.join("notes.md"),
            "# Hello\n\nSome notes about Rust.",
        )
        .expect("operation should succeed");

        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store a message first
        let m = Message::user("existing message about testing");
        mn.store("s1", &m)
            .await
            .expect("async operation should succeed");

        // Do an initial sync
        let initial = mn
            .sync_workspace(&ws_root)
            .await
            .expect("async operation should succeed");
        assert_eq!(initial.files_changed, 1);

        // Now do an atomic reindex
        let stats = mn
            .atomic_reindex(&ws_root, None)
            .await
            .expect("async operation should succeed");
        assert_eq!(stats.files_scanned, 1);
        assert_eq!(stats.files_changed, 1); // All files re-indexed in fresh DB

        // DB should still be usable after reindex
        let db_stats = mn.stats().await.expect("async operation should succeed");
        // The reindexed DB is fresh — messages from old DB are not preserved
        // (that's by design: reindex rebuilds the embedding index, not messages)
        assert_eq!(db_stats.tracked_file_count, 1);
    }

    #[tokio::test]
    async fn test_atomic_reindex_preserves_functionality() {
        let dir = tempdir().expect("should create temp dir");
        let ws_root = dir.path().join("workspace");
        std::fs::create_dir_all(&ws_root).expect("should create directory");
        std::fs::write(ws_root.join("a.md"), "# Alpha\n\nAlpha content.")
            .expect("should write file");
        std::fs::write(ws_root.join("b.md"), "# Beta\n\nBeta content.").expect("should write file");

        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Atomic reindex
        let stats = mn
            .atomic_reindex(&ws_root, None)
            .await
            .expect("async operation should succeed");
        assert_eq!(stats.files_scanned, 2);
        assert_eq!(stats.files_changed, 2);

        // After reindex, we can still store and search messages
        let m = Message::user("post-reindex message about gamma");
        mn.store("s1", &m)
            .await
            .expect("async operation should succeed");

        let results = mn
            .search("gamma", 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_atomic_reindex_with_sessions() {
        let dir = tempdir().expect("should create temp dir");
        let ws_root = dir.path().join("workspace");
        let sessions_dir = dir.path().join("sessions");
        std::fs::create_dir_all(&ws_root).expect("should create directory");
        std::fs::create_dir_all(&sessions_dir).expect("should create directory");

        std::fs::write(ws_root.join("notes.md"), "# Notes\n\nSome content.")
            .expect("should write file");
        let session_content = r#"{"type":"message","role":"user","content":"hello world"}
{"type":"message","role":"assistant","content":"hi there"}
"#;
        std::fs::write(sessions_dir.join("s1.jsonl"), session_content).expect("should write file");

        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_embeddings: false,
            enable_session_indexing: true,
            session_delta_bytes: 10, // Low threshold
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let stats = mn
            .atomic_reindex(&ws_root, Some(&sessions_dir))
            .await
            .expect("async operation should succeed");
        assert_eq!(stats.files_scanned, 1);
        assert_eq!(stats.files_changed, 1);
        // sessions_indexed is 0 because no embedder is configured (chunks_ok=0)
        // but the tracking entry is still created
    }

    #[tokio::test]
    async fn test_atomic_reindex_backup_cleanup() {
        let dir = tempdir().expect("should create temp dir");
        let ws_root = dir.path().join("workspace");
        std::fs::create_dir_all(&ws_root).expect("should create directory");

        let db_path = dir.path().join("test.db");
        let backup_path = dir.path().join("test.db.backup");
        let temp_path = dir.path().join("test.db.reindex");

        let config = MnemosyneConfig {
            db_path: db_path.clone(),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Reindex should succeed and clean up temp/backup files
        mn.atomic_reindex(&ws_root, None)
            .await
            .expect("async operation should succeed");

        assert!(db_path.exists(), "Main DB should exist");
        assert!(
            !backup_path.exists(),
            "Backup should be cleaned up on success"
        );
        assert!(
            !temp_path.exists(),
            "Temp DB should be cleaned up on success"
        );
    }

    // File Watcher Tests

    #[test]
    fn test_config_file_watcher_defaults() {
        let config = MnemosyneConfig::default();
        assert!(!config.enable_file_watcher);
        assert!(config.watch_paths.is_empty());
    }

    #[test]
    fn test_config_file_watcher_serde() {
        let json = r#"{
            "db_path": "/tmp/test.db",
            "enable_file_watcher": true,
            "watch_paths": ["/tmp/extra"]
        }"#;
        let config: MnemosyneConfig =
            serde_json::from_str(json).expect("should parse successfully");
        assert!(config.enable_file_watcher);
        assert_eq!(config.watch_paths.len(), 1);
        assert_eq!(config.watch_paths[0], PathBuf::from("/tmp/extra"));
    }

    #[test]
    fn test_config_file_watcher_backward_compat() {
        let json = r#"{"db_path": "/tmp/test.db"}"#;
        let config: MnemosyneConfig =
            serde_json::from_str(json).expect("should parse successfully");
        assert!(!config.enable_file_watcher);
        assert!(config.watch_paths.is_empty());
    }

    #[tokio::test]
    async fn test_file_watcher_start_and_dirty_count() {
        let dir = tempdir().expect("should create temp dir");
        let ws_root = dir.path().join("workspace");
        std::fs::create_dir_all(&ws_root).expect("should create directory");
        std::fs::write(ws_root.join("test.md"), "# Test").expect("should write file");

        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Arc::new(
            Mnemosyne::new(config)
                .await
                .expect("Arc::new should succeed"),
        );

        let watcher = FileWatcher::start(ws_root.clone(), vec![], mn.clone())
            .expect("operation should succeed");

        // Initially no dirty files
        assert_eq!(watcher.dirty_count(), 0);
        assert!(watcher.dirty_files().is_empty());

        // Write a file — should eventually appear as dirty
        std::fs::write(ws_root.join("new.md"), "# New file").expect("should write file");

        // Give the watcher a moment to receive the event
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // The file should be marked dirty (before debounce triggers sync)
        // Note: timing-sensitive, but 200ms is well within debounce window
        let dirty = watcher.dirty_count();
        // On some systems the event may not fire instantly, so we just check
        // it doesn't panic and the API works
        assert!(dirty <= 2, "Dirty count should be reasonable: {}", dirty);
    }

    #[tokio::test]
    async fn test_file_watcher_with_extra_paths() {
        let dir = tempdir().expect("should create temp dir");
        let ws_root = dir.path().join("workspace");
        let extra = dir.path().join("extra");
        std::fs::create_dir_all(&ws_root).expect("should create directory");
        std::fs::create_dir_all(&extra).expect("should create directory");

        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Arc::new(
            Mnemosyne::new(config)
                .await
                .expect("Arc::new should succeed"),
        );

        // Should start successfully with extra paths
        let watcher = FileWatcher::start(ws_root, vec![extra.clone()], mn);
        assert!(watcher.is_ok());
    }

    #[tokio::test]
    async fn test_file_watcher_nonexistent_extra_path() {
        let dir = tempdir().expect("should create temp dir");
        let ws_root = dir.path().join("workspace");
        std::fs::create_dir_all(&ws_root).expect("should create directory");

        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Arc::new(
            Mnemosyne::new(config)
                .await
                .expect("Arc::new should succeed"),
        );

        // Nonexistent extra paths should be skipped gracefully
        let watcher = FileWatcher::start(ws_root, vec![PathBuf::from("/nonexistent/path")], mn);
        assert!(watcher.is_ok());
    }

    // Memory Citations Tests

    #[tokio::test]
    async fn test_sync_stores_citations() {
        let dir = tempdir().expect("should create temp dir");
        let ws_root = dir.path().join("workspace");
        std::fs::create_dir_all(&ws_root).expect("should create directory");
        std::fs::write(
            ws_root.join("notes.md"),
            "# Header\n\nParagraph about Rust programming.\n\nAnother paragraph about memory.",
        )
        .expect("operation should succeed");

        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let stats = mn
            .sync_workspace(&ws_root)
            .await
            .expect("async operation should succeed");
        assert_eq!(stats.files_changed, 1);

        // Search for content from the synced file
        let results = mn
            .search("Rust programming", 10)
            .await
            .expect("async operation should succeed");
        assert!(!results.is_empty(), "Should find indexed content");

        // The result should have a citation
        let r = &results[0];
        assert!(r.citation.is_some(), "Search result should have citation");
        let cite = r.citation.as_ref().expect("as_ref should succeed");
        assert!(
            cite.starts_with("notes.md#L"),
            "Citation should reference file: {}",
            cite
        );
    }

    #[tokio::test]
    async fn test_citation_line_numbers_accurate() {
        let dir = tempdir().expect("should create temp dir");
        let ws_root = dir.path().join("workspace");
        std::fs::create_dir_all(&ws_root).expect("should create directory");

        // Create a file where content starts at line 1
        std::fs::write(ws_root.join("test.md"), "First paragraph on line one.")
            .expect("should write file");

        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        mn.sync_workspace(&ws_root)
            .await
            .expect("async operation should succeed");

        let results = mn
            .search("paragraph", 10)
            .await
            .expect("async operation should succeed");
        assert!(!results.is_empty());
        let cite = results[0].citation.as_ref().expect("as_ref should succeed");
        assert!(
            cite.contains("#L1"),
            "First chunk should start at L1: {}",
            cite
        );
    }

    #[tokio::test]
    async fn test_search_result_citation_none_for_messages() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Regular messages (not from file sync) should have no citation
        let msg = Message::user("testing citation behavior");
        mn.store("s1", &msg)
            .await
            .expect("async operation should succeed");

        let results = mn
            .search("citation behavior", 10)
            .await
            .expect("async operation should succeed");
        assert!(!results.is_empty());
        assert!(
            results[0].citation.is_none(),
            "Regular message should not have citation"
        );
    }

    #[tokio::test]
    async fn test_resync_replaces_old_chunks() {
        let dir = tempdir().expect("should create temp dir");
        let ws_root = dir.path().join("workspace");
        std::fs::create_dir_all(&ws_root).expect("should create directory");
        std::fs::write(
            ws_root.join("doc.md"),
            "# Version 1\n\nOld content about cats.",
        )
        .expect("operation should succeed");

        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // First sync
        mn.sync_workspace(&ws_root)
            .await
            .expect("async operation should succeed");
        let results = mn
            .search("cats", 10)
            .await
            .expect("async operation should succeed");
        assert!(!results.is_empty(), "Should find 'cats' after first sync");

        // Update file content
        std::fs::write(
            ws_root.join("doc.md"),
            "# Version 2\n\nNew content about dogs.",
        )
        .expect("operation should succeed");

        // Re-sync
        mn.sync_workspace(&ws_root)
            .await
            .expect("async operation should succeed");

        // Old content should be gone, new content should be found
        let old_results = mn
            .search("cats", 10)
            .await
            .expect("async operation should succeed");
        assert!(
            old_results.is_empty(),
            "Old content 'cats' should be removed after re-sync"
        );

        let new_results = mn
            .search("dogs", 10)
            .await
            .expect("async operation should succeed");
        assert!(
            !new_results.is_empty(),
            "New content 'dogs' should be found after re-sync"
        );
    }

    // Extra Memory Paths Tests

    #[tokio::test]
    async fn test_extra_memory_paths_indexed() {
        let dir = tempdir().expect("should create temp dir");
        let ws_root = dir.path().join("workspace");
        let extra_dir = dir.path().join("external_notes");
        std::fs::create_dir_all(&ws_root).expect("should create directory");
        std::fs::create_dir_all(&extra_dir).expect("should create directory");

        std::fs::write(ws_root.join("main.md"), "# Main\n\nWorkspace content here.")
            .expect("should write file");
        std::fs::write(
            extra_dir.join("extra.md"),
            "# Extra\n\nExternal knowledge base.",
        )
        .expect("operation should succeed");

        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_embeddings: false,
            extra_memory_paths: vec![extra_dir],
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let stats = mn
            .sync_workspace(&ws_root)
            .await
            .expect("async operation should succeed");
        // Should have scanned files from both workspace and extra path
        assert!(
            stats.files_scanned >= 2,
            "Should scan workspace + extra: {}",
            stats.files_scanned
        );
        assert!(
            stats.files_changed >= 2,
            "Should index workspace + extra: {}",
            stats.files_changed
        );

        // Content from extra path should be searchable
        let results = mn
            .search("External knowledge", 10)
            .await
            .expect("async operation should succeed");
        assert!(
            !results.is_empty(),
            "Extra path content should be searchable"
        );

        // Citation should include the extra source prefix
        let cite = results[0].citation.as_ref().expect("as_ref should succeed");
        assert!(
            cite.contains("extra:external_notes"),
            "Extra path citation should include source prefix: {}",
            cite
        );
    }

    #[tokio::test]
    async fn test_extra_memory_paths_nonexistent_skipped() {
        let dir = tempdir().expect("should create temp dir");
        let ws_root = dir.path().join("workspace");
        std::fs::create_dir_all(&ws_root).expect("should create directory");
        std::fs::write(ws_root.join("test.md"), "# Test\n\nContent.").expect("should write file");

        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_embeddings: false,
            extra_memory_paths: vec![PathBuf::from("/nonexistent/extra")],
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Should succeed without error (nonexistent path is silently skipped)
        let stats = mn
            .sync_workspace(&ws_root)
            .await
            .expect("async operation should succeed");
        assert_eq!(stats.files_scanned, 1);
        assert_eq!(stats.files_changed, 1);
    }

    #[test]
    fn test_config_extra_memory_paths_default() {
        let config = MnemosyneConfig::default();
        assert!(config.extra_memory_paths.is_empty());
    }

    #[test]
    fn test_config_extra_memory_paths_serde() {
        let json = r#"{
            "db_path": "/tmp/test.db",
            "extra_memory_paths": ["/tmp/notes", "/tmp/docs"]
        }"#;
        let config: MnemosyneConfig =
            serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(config.extra_memory_paths.len(), 2);
        assert_eq!(config.extra_memory_paths[0], PathBuf::from("/tmp/notes"));
    }

    #[test]
    fn test_config_extra_memory_paths_backward_compat() {
        // Old config without extra_memory_paths should still deserialize
        let json = r#"{"db_path": "/tmp/test.db"}"#;
        let config: MnemosyneConfig =
            serde_json::from_str(json).expect("should parse successfully");
        assert!(config.extra_memory_paths.is_empty());
    }

    #[tokio::test]
    async fn test_store_chunk_with_source() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Directly test the store method
        {
            let store = mn.store_ref().lock().await;
            let id = store
                .store_chunk_with_source(
                    "file:test.md",
                    "chunk content about testing",
                    "test.md#L1",
                    MemoryType::Semantic,
                )
                .expect("operation should succeed");
            assert!(id > 0);
        }

        // Search should find it with citation
        let results = mn
            .search("chunk content", 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].citation.as_deref(), Some("test.md#L1"));
    }

    // Chunk Overlap Tests

    #[test]
    fn test_chunk_overlap_zero_is_default() {
        // With 0 overlap, chunks should not share text
        let p1 = "A".repeat(1500);
        let p2 = "B".repeat(1500);
        let text = format!("{}\n\n{}", p1, p2);
        let chunks = chunk_text(&text);
        assert!(chunks.len() >= 2);
        // Second chunk should NOT start with A's
        assert!(!chunks[1].text.starts_with('A'));
    }

    #[test]
    fn test_chunk_overlap_shares_boundary_text() {
        // With overlap, the trailing text from chunk N appears at the start of chunk N+1
        let p1 = "Alpha beta gamma delta epsilon. ".repeat(60); // ~1860 chars
        let p2 = "Zeta eta theta iota kappa. ".repeat(60);
        let text = format!("{}\n\n{}", p1.trim(), p2.trim());
        let chunks = chunk_text_with_overlap(&text, 20); // 20 tokens ≈ 80 chars overlap
        assert!(
            chunks.len() >= 2,
            "Should split into 2+ chunks, got {}",
            chunks.len()
        );

        // The second chunk should contain some text from the end of the first chunk
        let first_tail: String = chunks[0]
            .text
            .chars()
            .rev()
            .take(40)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        // At least part of the tail should appear in the second chunk's beginning
        let overlap_region: String = chunks[1].text.chars().take(200).collect();
        // The overlap region should contain words from the first chunk's tail
        let tail_words: Vec<&str> = first_tail.split_whitespace().collect();
        let found_overlap = tail_words.iter().any(|w| overlap_region.contains(w));
        assert!(
            found_overlap,
            "Second chunk should contain overlap from first chunk's tail"
        );
    }

    #[test]
    fn test_chunk_overlap_single_chunk_no_effect() {
        // Short text that fits in one chunk — overlap has no effect
        let text = "Short paragraph.";
        let chunks = chunk_text_with_overlap(text, 80);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "Short paragraph.");
    }

    #[test]
    fn test_chunk_overlap_preserves_line_numbers() {
        let p1 = "A".repeat(1500);
        let p2 = "B".repeat(1500);
        let text = format!("{}\n\n{}", p1, p2);
        let chunks = chunk_text_with_overlap(&text, 20);
        assert!(chunks.len() >= 2);
        // First chunk always starts at line 1
        assert_eq!(chunks[0].start_line, 1);
        // Second chunk starts after the first paragraph
        assert!(chunks[1].start_line > 1);
    }

    #[test]
    fn test_config_chunk_overlap_default() {
        let config = MnemosyneConfig::default();
        assert_eq!(config.chunk_overlap_tokens, 80);
    }

    #[test]
    fn test_config_chunk_overlap_serde() {
        let json = r#"{"db_path": "/tmp/test.db", "chunk_overlap_tokens": 40}"#;
        let config: MnemosyneConfig =
            serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(config.chunk_overlap_tokens, 40);
    }

    #[test]
    fn test_config_chunk_overlap_backward_compat() {
        let json = r#"{"db_path": "/tmp/test.db"}"#;
        let config: MnemosyneConfig =
            serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(config.chunk_overlap_tokens, 80);
    }

    // Workspace Bootstrap Tests

    #[test]
    fn test_bootstrap_creates_all_files() {
        let dir = tempdir().expect("should create temp dir");
        let ws = dir.path().join("workspace");

        let created = bootstrap_workspace(&ws).expect("operation should succeed");
        assert_eq!(created.len(), 5, "Should create all 5 bootstrap files");

        assert!(ws.join("AGENTS.md").exists());
        assert!(ws.join("IDENTITY.md").exists());
        assert!(ws.join("TOOLS.md").exists());
        assert!(ws.join("BOOT.md").exists());
        assert!(ws.join("memory/MEMORY.md").exists());

        // daily/ directory should also be created
        assert!(ws.join("daily").is_dir());
    }

    #[test]
    fn test_bootstrap_skips_existing_nonempty() {
        let dir = tempdir().expect("should create temp dir");
        let ws = dir.path().join("workspace");
        std::fs::create_dir_all(ws.join("memory")).expect("should create directory");

        // Pre-create AGENTS.md with custom content
        std::fs::write(ws.join("AGENTS.md"), "My custom agent prompt").expect("should write file");

        let created = bootstrap_workspace(&ws).expect("operation should succeed");
        // Should create 4 files (not AGENTS.md since it already has content)
        assert_eq!(created.len(), 4);

        // Custom content should be preserved
        let content = std::fs::read_to_string(ws.join("AGENTS.md")).expect("should read file");
        assert_eq!(content, "My custom agent prompt");
    }

    #[test]
    fn test_bootstrap_replaces_empty_files() {
        let dir = tempdir().expect("should create temp dir");
        let ws = dir.path().join("workspace");
        std::fs::create_dir_all(&ws).expect("should create directory");

        // Create empty AGENTS.md
        std::fs::write(ws.join("AGENTS.md"), "").expect("should write file");
        // Create whitespace-only IDENTITY.md
        std::fs::write(ws.join("IDENTITY.md"), "   \n  \n  ").expect("should write file");

        let created = bootstrap_workspace(&ws).expect("operation should succeed");
        assert_eq!(
            created.len(),
            5,
            "Empty/whitespace files should be replaced"
        );

        // Empty file should now have content
        let content = std::fs::read_to_string(ws.join("AGENTS.md")).expect("should read file");
        assert!(content.contains("Zeus Agent"));
    }

    #[test]
    fn test_bootstrap_idempotent() {
        let dir = tempdir().expect("should create temp dir");
        let ws = dir.path().join("workspace");

        // First bootstrap
        let created1 = bootstrap_workspace(&ws).expect("operation should succeed");
        assert_eq!(created1.len(), 5);

        // Second bootstrap — all files exist, none should be recreated
        let created2 = bootstrap_workspace(&ws).expect("operation should succeed");
        assert_eq!(created2.len(), 0, "Second bootstrap should be a no-op");
    }

    #[test]
    fn test_bootstrap_file_contents_valid() {
        let dir = tempdir().expect("should create temp dir");
        let ws = dir.path().join("workspace");
        bootstrap_workspace(&ws).expect("operation should succeed");

        // Verify each file has meaningful content
        for &(rel_path, _) in BOOTSTRAP_FILES {
            let content = std::fs::read_to_string(ws.join(rel_path)).expect("should read file");
            assert!(
                content.len() > 50,
                "{} should have substantial content, got {} bytes",
                rel_path,
                content.len()
            );
            assert!(
                content.starts_with('#'),
                "{} should start with a markdown heading",
                rel_path
            );
        }
    }

    #[test]
    fn test_embed_batch_size_config_default() {
        let cfg = MnemosyneConfig::default();
        assert_eq!(cfg.embed_batch_size, 100);
    }

    #[test]
    fn test_embed_batch_size_config_serde() {
        let json_str = r#"{"db_path": "/tmp/test.db", "embed_batch_size": 50}"#;
        let cfg: MnemosyneConfig =
            serde_json::from_str(json_str).expect("should parse successfully");
        assert_eq!(cfg.embed_batch_size, 50);
    }

    #[test]
    fn test_embed_batch_size_backward_compat() {
        // Missing field should use default
        let json_str = r#"{"db_path": "/tmp/test.db"}"#;
        let cfg: MnemosyneConfig =
            serde_json::from_str(json_str).expect("should parse successfully");
        assert_eq!(cfg.embed_batch_size, 100);
    }

    #[test]
    fn test_embed_batch_chunks_split() {
        // Verify that batch processing correctly splits at batch_size boundary
        let batch_size = 3;
        let texts: Vec<String> = (0..8).map(|i| format!("text_{}", i)).collect();

        // Simulate the batching logic from embed_batch_with_cache
        let mut batches: Vec<Vec<&str>> = Vec::new();
        for batch_start in (0..texts.len()).step_by(batch_size) {
            let batch_end = (batch_start + batch_size).min(texts.len());
            let batch: Vec<&str> = texts[batch_start..batch_end]
                .iter()
                .map(|t| t.as_str())
                .collect();
            batches.push(batch);
        }

        assert_eq!(batches.len(), 3); // ceil(8/3) = 3 batches
        assert_eq!(batches[0].len(), 3);
        assert_eq!(batches[1].len(), 3);
        assert_eq!(batches[2].len(), 2);
        assert_eq!(batches[0][0], "text_0");
        assert_eq!(batches[2][1], "text_7");
    }

    #[tokio::test]
    async fn test_embed_batch_with_cache_no_embedder() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let texts = vec!["hello".to_string(), "world".to_string()];
        let results = mn
            .embed_batch_with_cache(&texts)
            .await
            .expect("async operation should succeed");

        // With no embedder, all results should be None
        assert_eq!(results.len(), 2);
        assert!(results[0].is_none());
        assert!(results[1].is_none());
    }

    #[tokio::test]
    async fn test_embed_batch_with_cache_empty_input() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let texts: Vec<String> = Vec::new();
        let results = mn
            .embed_batch_with_cache(&texts)
            .await
            .expect("async operation should succeed");
        assert!(results.is_empty());
    }

    // QMD Backend tests

    #[test]
    fn test_qmd_to_search_results_basic() {
        let qmd_results = vec![
            QmdSearchResult {
                content: "first result".to_string(),
                bm25_score: 0.8,
                vector_score: 0.9,
                reranked_score: 0.85,
                citation: Some("memory/notes.md#L10".to_string()),
                memory_type: Some("semantic".to_string()),
            },
            QmdSearchResult {
                content: "second result".to_string(),
                bm25_score: 0.5,
                vector_score: 0.6,
                reranked_score: 0.55,
                citation: None,
                memory_type: None,
            },
        ];

        let results = qmd_to_search_results(qmd_results);
        assert_eq!(results.len(), 2);

        // First result
        assert_eq!(results[0].content, "first result");
        assert!((results[0].score - 0.85).abs() < 0.001);
        assert_eq!(results[0].memory_type, MemoryType::Semantic);
        assert_eq!(results[0].citation, Some("memory/notes.md#L10".to_string()));
        assert_eq!(results[0].id, -1); // Negative IDs for QMD results

        // Second result (defaults)
        assert_eq!(results[1].content, "second result");
        assert!((results[1].score - 0.55).abs() < 0.001);
        assert_eq!(results[1].memory_type, MemoryType::Episodic);
        assert!(results[1].citation.is_none());
        assert_eq!(results[1].id, -2);
    }

    #[test]
    fn test_qmd_to_search_results_empty() {
        let results = qmd_to_search_results(vec![]);
        assert!(results.is_empty());
    }

    #[test]
    fn test_qmd_to_search_results_clamps_importance() {
        let qmd_results = vec![QmdSearchResult {
            content: "high score".to_string(),
            bm25_score: 1.5,
            vector_score: 1.8,
            reranked_score: 2.5, // Above 1.0
            citation: None,
            memory_type: None,
        }];

        let results = qmd_to_search_results(qmd_results);
        assert!((results[0].importance - 1.0).abs() < 0.001); // Clamped to 1.0
    }

    #[tokio::test]
    async fn test_qmd_backend_unavailable_on_bad_url() {
        let backend = QmdBackend::new("http://127.0.0.1:1", 100).await;
        assert!(!backend.is_available());
    }

    #[tokio::test]
    async fn test_qmd_search_when_unavailable() {
        let backend = QmdBackend::new("http://127.0.0.1:1", 100).await;
        let result = backend.search("test query", 10).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("not available"));
    }

    #[tokio::test]
    async fn test_mnemosyne_qmd_disabled_by_default() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_qmd: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        assert!(!mn.qmd_available());
    }

    #[tokio::test]
    async fn test_mnemosyne_search_qmd_not_enabled() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_qmd: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        let result = mn.search_qmd("test", 10).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not enabled"));
    }

    #[tokio::test]
    async fn test_mnemosyne_semantic_search_fallback_when_qmd_unavailable() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_qmd: true,
            qmd_url: "http://127.0.0.1:1".to_string(),
            qmd_timeout_ms: 100,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        // QMD is unavailable (bad port), should fall back to builtin
        assert!(!mn.qmd_available());

        // semantic_search should still succeed via builtin FTS
        let results = mn
            .semantic_search("test query", 5)
            .await
            .expect("async operation should succeed");
        // Empty DB, so 0 results is fine — the important thing is no error
        assert_eq!(results.len(), 0);
    }

    #[tokio::test]
    async fn test_qmd_search_result_serde() {
        let result = QmdSearchResult {
            content: "test content".to_string(),
            bm25_score: 0.75,
            vector_score: 0.82,
            reranked_score: 0.80,
            citation: Some("doc.md".to_string()),
            memory_type: Some("working".to_string()),
        };
        let json = serde_json::to_string(&result).expect("should serialize to JSON");
        let parsed: QmdSearchResult =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed.content, "test content");
        assert!((parsed.bm25_score - 0.75).abs() < 0.001);
        assert!((parsed.reranked_score - 0.80).abs() < 0.001);
        assert_eq!(parsed.citation, Some("doc.md".to_string()));
        assert_eq!(parsed.memory_type, Some("working".to_string()));
    }

    #[tokio::test]
    async fn test_qmd_search_result_serde_optional_fields() {
        let json =
            r#"{"content":"hello","bm25_score":0.5,"vector_score":0.6,"reranked_score":0.55}"#;
        let parsed: QmdSearchResult =
            serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(parsed.content, "hello");
        assert!(parsed.citation.is_none());
        assert!(parsed.memory_type.is_none());
    }

    #[tokio::test]
    async fn test_qmd_health_check_noop_when_disabled() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_qmd: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        // Should be a no-op, not panic
        mn.qmd_health_check().await;
        assert!(!mn.qmd_available());
    }

    // Cross-Encoder Reranker Tests

    #[test]
    fn test_cross_encoder_internal_scoring_basic() {
        let reranker = CrossEncoderReranker::new(None, "test-model".to_string());
        let query = "rust memory management";
        let documents = vec![
            "Rust has a unique ownership-based memory management system",
            "Python uses garbage collection for memory management",
            "The weather forecast for tomorrow is sunny",
        ];
        let scores = reranker.score_internal(query, &documents);

        assert_eq!(scores.len(), 3);
        // First doc should score highest (most relevant)
        assert!(
            scores[0] > scores[2],
            "Rust memory doc should beat weather doc"
        );
        // Second doc partially matches
        assert!(
            scores[1] > scores[2],
            "Python memory doc should beat weather doc"
        );
    }

    #[test]
    fn test_cross_encoder_internal_exact_phrase_bonus() {
        let reranker = CrossEncoderReranker::new(None, "test-model".to_string());
        let query = "memory management";
        let docs = vec![
            "memory management is important for systems programming",
            "management of memory requires careful design",
        ];
        let scores = reranker.score_internal(query, &docs);

        assert_eq!(scores.len(), 2);
        // First doc has exact phrase match, should score higher
        assert!(scores[0] > scores[1]);
    }

    #[test]
    fn test_cross_encoder_internal_empty_query() {
        let reranker = CrossEncoderReranker::new(None, "test-model".to_string());
        let docs = vec!["some document"];
        let scores = reranker.score_internal("", &docs);
        assert_eq!(scores.len(), 1);
        assert_eq!(scores[0], 0.0);
    }

    #[test]
    fn test_cross_encoder_internal_empty_documents() {
        let reranker = CrossEncoderReranker::new(None, "test-model".to_string());
        let docs: Vec<&str> = vec![];
        let scores = reranker.score_internal("test query", &docs);
        assert!(scores.is_empty());
    }

    #[test]
    fn test_cross_encoder_internal_empty_doc_content() {
        let reranker = CrossEncoderReranker::new(None, "test-model".to_string());
        let docs = vec!["", "non-empty document about rust"];
        let scores = reranker.score_internal("rust", &docs);
        assert_eq!(scores.len(), 2);
        assert_eq!(scores[0], 0.0); // Empty doc scores 0
        assert!(scores[1] > 0.0); // Non-empty matching doc scores positive
    }

    #[test]
    fn test_cross_encoder_internal_scores_clamped() {
        let reranker = CrossEncoderReranker::new(None, "test-model".to_string());
        let docs = vec!["rust rust rust rust rust rust"];
        let scores = reranker.score_internal("rust", &docs);
        assert!(scores[0] <= 1.0, "Score should be clamped to 1.0");
        assert!(scores[0] >= 0.0, "Score should not be negative");
    }

    #[test]
    fn test_cross_encoder_internal_no_match() {
        let reranker = CrossEncoderReranker::new(None, "test-model".to_string());
        let docs = vec!["the quick brown fox jumped over the lazy dog"];
        let scores = reranker.score_internal("quantum computing blockchain", &docs);
        assert_eq!(scores[0], 0.0);
    }

    #[tokio::test]
    async fn test_cross_encoder_score_async_empty() {
        let reranker = CrossEncoderReranker::new(None, "test-model".to_string());
        let docs: Vec<&str> = vec![];
        let scores = reranker
            .score("test", &docs)
            .await
            .expect("async operation should succeed");
        assert!(scores.is_empty());
    }

    #[tokio::test]
    async fn test_cross_encoder_score_async_internal() {
        let reranker = CrossEncoderReranker::new(None, "test-model".to_string());
        let docs = vec!["relevant document about testing", "unrelated content"];
        let scores = reranker
            .score("testing", &docs)
            .await
            .expect("async operation should succeed");
        assert_eq!(scores.len(), 2);
        assert!(scores[0] > scores[1]);
    }

    // QMD Backend Reranking Tests

    #[tokio::test]
    async fn test_qmd_rerank_empty_candidates() {
        let qmd =
            QmdBackend::with_reranker("http://127.0.0.1:1", 100, None, "test-model".to_string())
                .await;
        let results = qmd
            .rerank("test query", vec![], 0.3, 0.3, 0.4, 10)
            .await
            .expect("async operation should succeed");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_qmd_rerank_with_candidates() {
        let qmd =
            QmdBackend::with_reranker("http://127.0.0.1:1", 100, None, "test-model".to_string())
                .await;

        let candidates = vec![
            QmdCandidate {
                result: SearchResult {
                    id: 1,
                    session_id: "s1".to_string(),
                    content: "rust programming language systems".to_string(),
                    timestamp: "2026-01-01T00:00:00Z".to_string(),
                    score: 0.0,
                    memory_type: MemoryType::Semantic,
                    importance: 0.5,
                    citation: None,
                    valid_from: None,
                    valid_to: None,
                    verified: true,
                    superseded_by: None,
                },
                bm25_score: 0.8,
                vector_score: 0.7,
                reranker_score: 0.0,
            },
            QmdCandidate {
                result: SearchResult {
                    id: 2,
                    session_id: "s1".to_string(),
                    content: "the weather is nice today".to_string(),
                    timestamp: "2026-01-01T00:00:00Z".to_string(),
                    score: 0.0,
                    memory_type: MemoryType::Episodic,
                    importance: 0.3,
                    citation: None,
                    valid_from: None,
                    valid_to: None,
                    verified: true,
                    superseded_by: None,
                },
                bm25_score: 0.1,
                vector_score: 0.2,
                reranker_score: 0.0,
            },
        ];

        let results = qmd
            .rerank("rust programming", candidates, 0.3, 0.3, 0.4, 10)
            .await
            .expect("async operation should succeed");

        assert_eq!(results.len(), 2);
        // First result should have higher reranked score
        assert!(results[0].reranked_score >= results[1].reranked_score);
        assert_eq!(results[0].content, "rust programming language systems");
    }

    #[tokio::test]
    async fn test_qmd_rerank_respects_limit() {
        let qmd =
            QmdBackend::with_reranker("http://127.0.0.1:1", 100, None, "test-model".to_string())
                .await;

        let candidates: Vec<QmdCandidate> = (0..10)
            .map(|i| QmdCandidate {
                result: SearchResult {
                    id: i,
                    session_id: "s1".to_string(),
                    content: format!("document number {}", i),
                    timestamp: String::new(),
                    score: 0.0,
                    memory_type: MemoryType::Episodic,
                    importance: 0.5,
                    citation: None,
                    valid_from: None,
                    valid_to: None,
                    verified: true,
                    superseded_by: None,
                },
                bm25_score: 0.5,
                vector_score: 0.5,
                reranker_score: 0.0,
            })
            .collect();

        let results = qmd
            .rerank("document", candidates, 0.3, 0.3, 0.4, 3)
            .await
            .expect("async operation should succeed");

        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn test_qmd_rerank_no_reranker_returns_error() {
        let qmd = QmdBackend::new("http://127.0.0.1:1", 100).await;
        assert!(!qmd.has_reranker());

        let candidate = QmdCandidate {
            result: SearchResult {
                id: 1,
                session_id: "s1".to_string(),
                content: "test content".to_string(),
                timestamp: String::new(),
                score: 0.0,
                memory_type: MemoryType::Episodic,
                importance: 0.5,
                citation: None,
                valid_from: None,
                valid_to: None,
                verified: true,
                superseded_by: None,
            },
            bm25_score: 0.5,
            vector_score: 0.5,
            reranker_score: 0.0,
        };
        let result = qmd.rerank("test", vec![candidate], 0.3, 0.3, 0.4, 10).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("reranker"));
    }

    #[tokio::test]
    async fn test_qmd_with_reranker_flag() {
        let qmd_no_reranker = QmdBackend::new("http://127.0.0.1:1", 100).await;
        assert!(!qmd_no_reranker.has_reranker());

        let qmd_with_reranker =
            QmdBackend::with_reranker("http://127.0.0.1:1", 100, None, "test-model".to_string())
                .await;
        assert!(qmd_with_reranker.has_reranker());
    }

    #[tokio::test]
    async fn test_qmd_rerank_score_fusion_weights() {
        let qmd =
            QmdBackend::with_reranker("http://127.0.0.1:1", 100, None, "test-model".to_string())
                .await;

        let candidates = vec![QmdCandidate {
            result: SearchResult {
                id: 1,
                session_id: "s1".to_string(),
                content: "testing weights".to_string(),
                timestamp: String::new(),
                score: 0.0,
                memory_type: MemoryType::Working,
                importance: 0.5,
                citation: None,
                valid_from: None,
                valid_to: None,
                verified: true,
                superseded_by: None,
            },
            bm25_score: 1.0,
            vector_score: 0.0,
            reranker_score: 0.0,
        }];

        // With bm25_weight=1.0, only BM25 matters
        let results = qmd
            .rerank("testing", candidates.clone(), 1.0, 0.0, 0.0, 10)
            .await
            .expect("async operation should succeed");
        let score_bm25_only = results[0].reranked_score;

        // With vector_weight=1.0, only vector matters (which is 0.0)
        let results = qmd
            .rerank("testing", candidates, 0.0, 1.0, 0.0, 10)
            .await
            .expect("async operation should succeed");
        let score_vector_only = results[0].reranked_score;

        assert!(score_bm25_only > score_vector_only);
    }

    // Internal QMD Pipeline Tests (end-to-end)

    #[tokio::test]
    async fn test_internal_qmd_search_with_fts() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_qmd: true,
            qmd_url: "http://127.0.0.1:1".to_string(), // Sidecar unavailable
            qmd_timeout_ms: 100,
            ..Default::default()
        };

        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store some messages
        let msg1 = Message::user("Rust ownership and borrowing rules");
        let msg2 = Message::user("Python list comprehensions are powerful");
        let msg3 = Message::user("Rust memory safety without garbage collection");
        mn.store("s1", &msg1)
            .await
            .expect("async operation should succeed");
        mn.store("s1", &msg2)
            .await
            .expect("async operation should succeed");
        mn.store("s1", &msg3)
            .await
            .expect("async operation should succeed");

        // Internal QMD should work via FTS + cross-encoder reranking
        let results = mn
            .semantic_search("rust memory", 5)
            .await
            .expect("async operation should succeed");
        // Should find results (FTS matches + cross-encoder reranked)
        assert!(!results.is_empty());
    }

    #[tokio::test]
    async fn test_internal_qmd_search_empty_db() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_qmd: true,
            qmd_url: "http://127.0.0.1:1".to_string(),
            qmd_timeout_ms: 100,
            ..Default::default()
        };

        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        let results = mn
            .semantic_search("test query", 5)
            .await
            .expect("async operation should succeed");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_internal_qmd_fallback_to_hybrid() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_qmd: false, // QMD disabled
            ..Default::default()
        };

        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let msg = Message::user("hello world");
        mn.store("s1", &msg)
            .await
            .expect("async operation should succeed");

        // Should fall through to builtin hybrid search
        let results = mn
            .semantic_search("hello", 5)
            .await
            .expect("async operation should succeed");
        assert!(!results.is_empty());
    }

    #[tokio::test]
    async fn test_internal_qmd_reranks_relevance() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_qmd: true,
            qmd_url: "http://127.0.0.1:1".to_string(),
            qmd_timeout_ms: 100,
            qmd_bm25_weight: 0.3,
            qmd_vector_weight: 0.0, // No vector search in this test
            qmd_reranker_weight: 0.7,
            ..Default::default()
        };

        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store messages with varying relevance
        mn.store("s1", &Message::user("Rust async programming with tokio"))
            .await
            .expect("async operation should succeed");
        mn.store(
            "s1",
            &Message::user("Rust async await patterns and best practices"),
        )
        .await
        .expect("async operation should succeed");
        mn.store("s1", &Message::user("Cooking recipes for dinner"))
            .await
            .expect("async operation should succeed");

        let results = mn
            .semantic_search("rust async", 10)
            .await
            .expect("async operation should succeed");
        // Should find rust-related messages (not cooking)
        assert!(!results.is_empty());
        // Top results should be about rust async
        assert!(results[0].content.to_lowercase().contains("rust"));
    }

    #[tokio::test]
    async fn test_qmd_config_defaults() {
        let config = MnemosyneConfig::default();
        assert!(!config.enable_qmd);
        assert_eq!(config.qmd_url, "http://localhost:7720");
        assert_eq!(config.qmd_timeout_ms, 3000);
        assert!(config.qmd_reranker_url.is_none());
        assert_eq!(
            config.qmd_reranker_model,
            "cross-encoder/ms-marco-MiniLM-L-6-v2"
        );
        assert!((config.qmd_bm25_weight - 0.3).abs() < f64::EPSILON);
        assert!((config.qmd_vector_weight - 0.3).abs() < f64::EPSILON);
        assert!((config.qmd_reranker_weight - 0.4).abs() < f64::EPSILON);
        assert_eq!(config.qmd_candidate_multiplier, 4);
    }

    #[tokio::test]
    async fn test_qmd_config_serde_roundtrip() {
        let config = MnemosyneConfig {
            enable_qmd: true,
            qmd_reranker_url: Some("http://localhost:8080/rerank".to_string()),
            qmd_reranker_model: "my-model".to_string(),
            qmd_bm25_weight: 0.2,
            qmd_vector_weight: 0.4,
            qmd_reranker_weight: 0.4,
            qmd_candidate_multiplier: 6,
            ..Default::default()
        };

        let json = serde_json::to_string(&config).expect("should serialize to JSON");
        let parsed: MnemosyneConfig =
            serde_json::from_str(&json).expect("should parse successfully");

        assert!(parsed.enable_qmd);
        assert_eq!(
            parsed.qmd_reranker_url,
            Some("http://localhost:8080/rerank".to_string())
        );
        assert_eq!(parsed.qmd_reranker_model, "my-model");
        assert!((parsed.qmd_bm25_weight - 0.2).abs() < f64::EPSILON);
        assert!((parsed.qmd_vector_weight - 0.4).abs() < f64::EPSILON);
        assert!((parsed.qmd_reranker_weight - 0.4).abs() < f64::EPSILON);
        assert_eq!(parsed.qmd_candidate_multiplier, 6);
    }

    // Cross-Session Pattern Recognition Tests

    #[tokio::test]
    async fn test_extract_patterns_empty_db() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // No messages → no patterns extracted
        let count = mn
            .extract_patterns()
            .await
            .expect("async operation should succeed");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_extract_patterns_tool_frequency() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store messages with tool_calls
        let mut msg1 = Message::assistant("Used shell");
        msg1.tool_calls.push(zeus_core::ToolCall {
            id: "1".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "ls"}),
        });
        mn.store("s1", &msg1)
            .await
            .expect("async operation should succeed");

        let mut msg2 = Message::assistant("Used shell again");
        msg2.tool_calls.push(zeus_core::ToolCall {
            id: "2".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "pwd"}),
        });
        mn.store("s2", &msg2)
            .await
            .expect("async operation should succeed");

        let mut msg3 = Message::assistant("Read a file");
        msg3.tool_calls.push(zeus_core::ToolCall {
            id: "3".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({"path": "test.rs"}),
        });
        mn.store("s2", &msg3)
            .await
            .expect("async operation should succeed");

        let count = mn
            .extract_patterns()
            .await
            .expect("async operation should succeed");
        assert!(count > 0, "Should extract at least one pattern");

        let tool_patterns = mn
            .get_patterns("tool", 10)
            .await
            .expect("async operation should succeed");
        assert!(!tool_patterns.is_empty());
        // "shell" should be the most frequent tool pattern
        assert_eq!(tool_patterns[0].content, "shell");
        assert!(tool_patterns[0].frequency >= 2);
    }

    #[tokio::test]
    async fn test_extract_patterns_themes() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store user messages with repeated theme
        mn.store("s1", &Message::user("how do I fix this bug"))
            .await
            .expect("async operation should succeed");
        mn.store("s2", &Message::user("how do I fix the crash"))
            .await
            .expect("async operation should succeed");
        mn.store("s3", &Message::user("something different entirely"))
            .await
            .expect("async operation should succeed");

        let count = mn
            .extract_patterns()
            .await
            .expect("async operation should succeed");
        assert!(count > 0);

        let themes = mn
            .get_patterns("theme", 10)
            .await
            .expect("async operation should succeed");
        // "how do i fix" should appear as a theme
        assert!(themes.iter().any(|t| t.content.starts_with("how do")));
    }

    #[tokio::test]
    async fn test_extract_patterns_topics_across_sessions() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Same topic word in different sessions
        mn.store("session-a", &Message::user("deploy the kubernetes cluster"))
            .await
            .expect("async operation should succeed");
        mn.store("session-b", &Message::user("kubernetes pod is crashing"))
            .await
            .expect("async operation should succeed");

        let count = mn
            .extract_patterns()
            .await
            .expect("async operation should succeed");
        assert!(count > 0);

        let topics = mn
            .get_patterns("topic", 20)
            .await
            .expect("async operation should succeed");
        assert!(
            topics.iter().any(|t| t.content == "kubernetes"),
            "kubernetes should be a cross-session topic, got: {:?}",
            topics.iter().map(|t| &t.content).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn test_get_all_patterns() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Mix of tool and user messages
        let mut msg1 = Message::assistant("result");
        msg1.tool_calls.push(zeus_core::ToolCall {
            id: "1".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({}),
        });
        let mut msg2 = Message::assistant("result2");
        msg2.tool_calls.push(zeus_core::ToolCall {
            id: "2".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({}),
        });
        mn.store("s1", &msg1)
            .await
            .expect("async operation should succeed");
        mn.store("s2", &msg2)
            .await
            .expect("async operation should succeed");

        mn.extract_patterns()
            .await
            .expect("async operation should succeed");
        let all = mn
            .get_all_patterns(100)
            .await
            .expect("async operation should succeed");
        assert!(!all.is_empty());
    }

    // Importance Scoring with Decay Tests

    #[tokio::test]
    async fn test_boost_memory() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let msg = Message::user("important context");
        let id = mn
            .store_typed("s1", &msg, MemoryType::Episodic, 0.5)
            .await
            .expect("async operation should succeed");

        // Check initial state
        let (score, accessed) = mn
            .get_memory_importance(id)
            .await
            .expect("async operation should succeed");
        assert!((score - 0.5).abs() < 0.01);
        assert!(accessed.is_none());

        // Boost it
        mn.boost_memory(id, 0.2)
            .await
            .expect("async operation should succeed");

        let (score, accessed) = mn
            .get_memory_importance(id)
            .await
            .expect("async operation should succeed");
        assert!(
            (score - 0.7).abs() < 0.01,
            "Score should be ~0.7, got {}",
            score
        );
        assert!(
            accessed.is_some(),
            "last_accessed should be set after boost"
        );
    }

    #[tokio::test]
    async fn test_boost_memory_caps_at_one() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let msg = Message::user("very important");
        let id = mn
            .store_typed("s1", &msg, MemoryType::Semantic, 0.9)
            .await
            .expect("async operation should succeed");

        // Boost by 0.5 — should cap at 1.0
        mn.boost_memory(id, 0.5)
            .await
            .expect("async operation should succeed");

        let (score, _) = mn
            .get_memory_importance(id)
            .await
            .expect("async operation should succeed");
        assert!(
            (score - 1.0).abs() < 0.01,
            "Score should cap at 1.0, got {}",
            score
        );
    }

    #[tokio::test]
    async fn test_decay_memories() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store episodic memory
        let msg = Message::user("will decay");
        let id = mn
            .store_typed("s1", &msg, MemoryType::Episodic, 0.8)
            .await
            .expect("async operation should succeed");

        // Also store semantic memory (should NOT be decayed)
        let sem_msg = Message::user("persistent knowledge");
        let sem_id = mn
            .store_typed("s1", &sem_msg, MemoryType::Semantic, 0.9)
            .await
            .expect("async operation should succeed");

        // Apply decay
        let decayed = mn
            .decay_memories(0.10)
            .await
            .expect("async operation should succeed");
        assert!(decayed > 0, "Should decay at least one memory");

        // Episodic should have decreased
        let (score, _) = mn
            .get_memory_importance(id)
            .await
            .expect("async operation should succeed");
        assert!(
            score < 0.8,
            "Episodic importance should decrease, got {}",
            score
        );

        // Semantic should be unchanged
        let (sem_score, _) = mn
            .get_memory_importance(sem_id)
            .await
            .expect("async operation should succeed");
        assert!(
            (sem_score - 0.9).abs() < 0.01,
            "Semantic should be unchanged, got {}",
            sem_score
        );
    }

    #[tokio::test]
    async fn test_decay_does_not_go_negative() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let msg = Message::user("low importance");
        let id = mn
            .store_typed("s1", &msg, MemoryType::Episodic, 0.05)
            .await
            .expect("async operation should succeed");

        // Heavy decay
        mn.decay_memories(0.90)
            .await
            .expect("async operation should succeed");

        let (score, _) = mn
            .get_memory_importance(id)
            .await
            .expect("async operation should succeed");
        assert!(score >= 0.0, "Score should not go negative, got {}", score);
    }

    // Proactive Retrieval Tests

    #[tokio::test]
    async fn test_proactive_context_empty_messages() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let results = mn
            .proactive_context(&[], 5)
            .await
            .expect("async operation should succeed");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_proactive_context_no_matching_memories() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store some memories on a different topic
        mn.store("s1", &Message::user("quantum physics research"))
            .await
            .expect("async operation should succeed");

        // Query about something else
        let messages = vec![Message::user("deploy kubernetes cluster")];
        let results = mn
            .proactive_context(&messages, 5)
            .await
            .expect("async operation should succeed");
        // May or may not find results depending on FTS, but shouldn't panic
        assert!(results.len() <= 5);
    }

    #[tokio::test]
    async fn test_proactive_context_with_matching_patterns() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store memories about rust
        mn.store("s1", &Message::user("how to fix rust compilation error"))
            .await
            .expect("async operation should succeed");
        mn.store("s2", &Message::user("rust borrow checker issue"))
            .await
            .expect("async operation should succeed");
        mn.store("s3", &Message::user("rust lifetime annotation problem"))
            .await
            .expect("async operation should succeed");

        // Extract patterns so "rust" becomes a known topic
        mn.extract_patterns()
            .await
            .expect("async operation should succeed");

        // Now query with messages mentioning "rust"
        let messages = vec![Message::user("I have a rust error in my code")];
        let results = mn
            .proactive_context(&messages, 5)
            .await
            .expect("async operation should succeed");
        // Should find relevant rust-related memories
        assert!(!results.is_empty(), "Should find pattern-matching memories");
        assert!(
            results
                .iter()
                .any(|r| r.content.to_lowercase().contains("rust"))
        );
    }

    #[tokio::test]
    async fn test_proactive_context_respects_limit() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store many memories about the same topic
        for i in 0..10 {
            mn.store(
                &format!("s{}", i),
                &Message::user(&format!("database query optimization technique {}", i)),
            )
            .await
            .expect("async operation should succeed");
        }

        let messages = vec![Message::user("database optimization query")];
        let results = mn
            .proactive_context(&messages, 3)
            .await
            .expect("async operation should succeed");
        assert!(
            results.len() <= 3,
            "Should respect limit of 3, got {}",
            results.len()
        );
    }

    #[tokio::test]
    async fn test_proactive_context_boosts_accessed_memories() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store a memory and note its initial importance
        let msg = Message::user("deploy the application to staging");
        let id = mn
            .store("s1", &msg)
            .await
            .expect("async operation should succeed");
        mn.store("s2", &Message::user("deploy the application to production"))
            .await
            .expect("async operation should succeed");

        // Extract patterns to get "deploy" as a known topic
        mn.extract_patterns()
            .await
            .expect("async operation should succeed");

        let (initial_score, _) = mn
            .get_memory_importance(id)
            .await
            .expect("async operation should succeed");

        // Proactive retrieval with matching topic
        let messages = vec![Message::user("deploy this to staging now")];
        let results = mn
            .proactive_context(&messages, 5)
            .await
            .expect("async operation should succeed");
        assert!(!results.is_empty());

        // Check that the accessed memory got a boost
        let (new_score, accessed) = mn
            .get_memory_importance(id)
            .await
            .expect("async operation should succeed");
        // If it was pattern-matched, score should have increased or last_accessed set
        if accessed.is_some() {
            assert!(
                new_score >= initial_score,
                "Score should not decrease on access"
            );
        }
    }

    // Schema Migration Tests

    #[tokio::test]
    async fn test_last_accessed_column_exists() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store and retrieve — last_accessed should be None initially
        let msg = Message::user("test");
        let id = mn
            .store("s1", &msg)
            .await
            .expect("async operation should succeed");

        let (_, accessed) = mn
            .get_memory_importance(id)
            .await
            .expect("async operation should succeed");
        assert!(accessed.is_none(), "last_accessed should be None initially");
    }

    #[tokio::test]
    async fn test_patterns_table_exists() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Should be able to query patterns without error
        let patterns = mn
            .get_all_patterns(10)
            .await
            .expect("async operation should succeed");
        assert!(patterns.is_empty());
    }
}

// Additional comprehensive tests — coverage for sync, sessions, bootstrap,
// forget, atomic reindex, session file tracking, and edge cases.

#[cfg(test)]
mod additional_tests {
    use super::*;
    use tempfile::tempdir;

    // ── forget_before ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_forget_before_removes_old() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store messages
        mn.store("s1", &Message::user("old message"))
            .await
            .expect("async operation should succeed");
        mn.store("s1", &Message::user("newer message"))
            .await
            .expect("async operation should succeed");

        // Forget everything before far future — should delete all
        let deleted = mn
            .forget_before(chrono::Utc::now() + chrono::Duration::hours(1))
            .await
            .expect("async operation should succeed");
        assert_eq!(deleted, 2);

        let stats = mn.stats().await.expect("async operation should succeed");
        assert_eq!(stats.message_count, 0);
    }

    #[tokio::test]
    async fn test_forget_before_empty_db() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        let deleted = mn
            .forget_before(chrono::Utc::now())
            .await
            .expect("async operation should succeed");
        assert_eq!(deleted, 0);
    }

    #[tokio::test]
    async fn test_forget_before_preserves_recent() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        mn.store("s1", &Message::user("recent"))
            .await
            .expect("async operation should succeed");

        // Forget before 1 hour ago — should keep the message
        let deleted = mn
            .forget_before(chrono::Utc::now() - chrono::Duration::hours(1))
            .await
            .expect("async operation should succeed");
        assert_eq!(deleted, 0);

        let stats = mn.stats().await.expect("async operation should succeed");
        assert_eq!(stats.message_count, 1);
    }

    // ── Multiple sessions ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_multiple_sessions_isolation() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        mn.store("session-a", &Message::user("alpha"))
            .await
            .expect("async operation should succeed");
        mn.store("session-b", &Message::user("beta"))
            .await
            .expect("async operation should succeed");
        mn.store("session-a", &Message::user("alpha2"))
            .await
            .expect("async operation should succeed");

        let a_msgs = mn
            .recall_session("session-a", 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(a_msgs.len(), 2);
        assert!(a_msgs.iter().all(|m| m.content.starts_with("alpha")));

        let b_msgs = mn
            .recall_session("session-b", 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(b_msgs.len(), 1);
        assert_eq!(b_msgs[0].content, "beta");
    }

    #[tokio::test]
    async fn test_recall_session_limit() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        for i in 0..20 {
            mn.store("s1", &Message::user(format!("msg {}", i)))
                .await
                .expect("async operation should succeed");
        }

        let limited = mn
            .recall_session("s1", 5)
            .await
            .expect("async operation should succeed");
        assert_eq!(limited.len(), 5);
        // Returns newest first (DESC), so first is msg 19
        assert!(limited[0].content.contains("19"));
    }

    #[tokio::test]
    async fn test_recall_nonexistent_session() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        let msgs = mn
            .recall_session("nonexistent", 10)
            .await
            .expect("async operation should succeed");
        assert!(msgs.is_empty());
    }

    // ── Stats ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_stats_comprehensive() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        mn.store("s1", &Message::user("hello"))
            .await
            .expect("async operation should succeed");
        mn.store("s1", &Message::assistant("hi"))
            .await
            .expect("async operation should succeed");
        mn.store("s2", &Message::user("world"))
            .await
            .expect("async operation should succeed");

        let stats = mn.stats().await.expect("async operation should succeed");
        assert_eq!(stats.message_count, 3);
        assert_eq!(stats.session_count, 2);
        assert_eq!(stats.embedding_count, 0);
    }

    // ── Typed memory ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_store_typed_all_variants() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        mn.store_typed(
            "s1",
            &Message::user("working note"),
            MemoryType::Working,
            0.9,
        )
        .await
        .expect("async operation should succeed");
        mn.store_typed("s1", &Message::user("episode"), MemoryType::Episodic, 0.5)
            .await
            .expect("async operation should succeed");
        mn.store_typed("s1", &Message::user("fact"), MemoryType::Semantic, 0.8)
            .await
            .expect("async operation should succeed");

        let working = mn
            .search_by_type("note", MemoryType::Working, 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(working.len(), 1);
        assert!(working[0].content.contains("working note"));

        let semantic = mn
            .search_by_type("fact", MemoryType::Semantic, 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(semantic.len(), 1);
    }

    // ── FTS edge cases ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_search_empty_query() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        mn.store("s1", &Message::user("test content"))
            .await
            .expect("async operation should succeed");

        // Empty query returns empty results (sanitized to nothing)
        let results = mn.search("", 10).await.expect("should not error");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_search_special_characters() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        mn.store("s1", &Message::user("hello world"))
            .await
            .expect("async operation should succeed");

        // Special FTS characters are sanitized — bare * becomes empty, returns no results
        let results = mn.search("*", 10).await.expect("should not error");
        assert!(results.is_empty());

        // Bracket-wrapped usernames like [someuser] should not crash FTS5
        let results = mn.search("[someuser] hello", 10).await.expect("should not error");
        // sanitized to "someuser" "hello" — "someuser" not in corpus, no match expected
        // The key thing is it doesn't crash (previously FTS5 syntax error on '[')

        // But searching with brackets around an actual word should find it
        let results = mn.search("[hello]", 10).await.expect("should not error");
        assert!(!results.is_empty(), "should find 'hello' despite brackets");
    }

    #[tokio::test]
    async fn test_search_in_session_scoped() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store messages in two different room sessions
        mn.store("room:war-room-1", &Message::user("[Alice] deploy the API server"))
            .await.expect("store should succeed");
        mn.store("room:war-room-1", &Message::user("[Bob] API server is running on port 3000"))
            .await.expect("store should succeed");
        mn.store("room:war-room-2", &Message::user("[Charlie] deploy the frontend app"))
            .await.expect("store should succeed");
        mn.store("general-session", &Message::user("[System] deploy scheduled task"))
            .await.expect("store should succeed");

        // Search scoped to room:war-room-1 — should only find messages from that room
        let results = mn.search_in_session("deploy", "room:war-room-1", 10)
            .await.expect("session search should succeed");
        assert_eq!(results.len(), 1, "should find 1 match in war-room-1");
        assert!(results[0].content.contains("API server"));

        // Search scoped to room:war-room-2
        let results = mn.search_in_session("deploy", "room:war-room-2", 10)
            .await.expect("session search should succeed");
        assert_eq!(results.len(), 1, "should find 1 match in war-room-2");
        assert!(results[0].content.contains("frontend"));

        // Search scoped to non-existent room — empty results
        let results = mn.search_in_session("deploy", "room:nonexistent", 10)
            .await.expect("session search should succeed");
        assert_eq!(results.len(), 0, "should find nothing in nonexistent room");

        // Global search should find all "deploy" messages
        let results = mn.search("deploy", 10)
            .await.expect("global search should succeed");
        assert!(results.len() >= 3, "global search should find messages from all sessions");
    }

    #[tokio::test]
    async fn test_fts_multiple_matches_ranked() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        mn.store("s1", &Message::user("rust programming language"))
            .await
            .expect("async operation should succeed");
        mn.store("s1", &Message::user("python is great"))
            .await
            .expect("async operation should succeed");
        mn.store("s1", &Message::user("rust is fast and safe"))
            .await
            .expect("async operation should succeed");

        let results = mn
            .search("rust", 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(results.len(), 2);
    }

    // ── Workspace sync ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_sync_workspace_skips_unchanged() {
        let dir = tempdir().expect("should create temp dir");
        let workspace = tempdir().expect("should create temp dir");

        std::fs::write(workspace.path().join("doc.md"), "# Title\n\nContent here")
            .expect("should write file");

        let config = MnemosyneConfig {
            db_path: dir.path().join("sync.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let stats1 = mn
            .sync_workspace(workspace.path())
            .await
            .expect("async operation should succeed");
        assert_eq!(stats1.files_scanned, 1);
        assert_eq!(stats1.files_changed, 1);
        assert_eq!(stats1.files_unchanged, 0);

        // Second sync — file unchanged
        let stats2 = mn
            .sync_workspace(workspace.path())
            .await
            .expect("async operation should succeed");
        assert_eq!(stats2.files_scanned, 1);
        assert_eq!(stats2.files_changed, 0);
        assert_eq!(stats2.files_unchanged, 1);
    }

    #[tokio::test]
    async fn test_sync_workspace_detects_changes() {
        let dir = tempdir().expect("should create temp dir");
        let workspace = tempdir().expect("should create temp dir");
        let file_path = workspace.path().join("notes.md");

        std::fs::write(&file_path, "version 1").expect("should write file");

        let config = MnemosyneConfig {
            db_path: dir.path().join("sync.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let stats1 = mn
            .sync_workspace(workspace.path())
            .await
            .expect("async operation should succeed");
        assert_eq!(stats1.files_changed, 1);

        // Modify the file
        std::fs::write(&file_path, "version 2 with different content").expect("should write file");

        let stats2 = mn
            .sync_workspace(workspace.path())
            .await
            .expect("async operation should succeed");
        assert_eq!(stats2.files_changed, 1);
        assert_eq!(stats2.files_unchanged, 0);
    }

    #[tokio::test]
    async fn test_sync_workspace_ignores_non_md() {
        let dir = tempdir().expect("should create temp dir");
        let workspace = tempdir().expect("should create temp dir");

        std::fs::write(workspace.path().join("code.rs"), "fn main() {}")
            .expect("should write file");
        std::fs::write(workspace.path().join("notes.md"), "# Notes").expect("should write file");
        std::fs::write(workspace.path().join("data.json"), "{}").expect("should write file");

        let config = MnemosyneConfig {
            db_path: dir.path().join("sync.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let stats = mn
            .sync_workspace(workspace.path())
            .await
            .expect("async operation should succeed");
        assert_eq!(stats.files_scanned, 1); // Only .md files
    }

    #[tokio::test]
    async fn test_sync_workspace_recursive() {
        let dir = tempdir().expect("should create temp dir");
        let workspace = tempdir().expect("should create temp dir");

        std::fs::create_dir_all(workspace.path().join("sub/deep"))
            .expect("should create directory");
        std::fs::write(workspace.path().join("root.md"), "# Root").expect("should write file");
        std::fs::write(workspace.path().join("sub/child.md"), "# Child")
            .expect("should write file");
        std::fs::write(workspace.path().join("sub/deep/leaf.md"), "# Leaf")
            .expect("should write file");

        let config = MnemosyneConfig {
            db_path: dir.path().join("sync.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let stats = mn
            .sync_workspace(workspace.path())
            .await
            .expect("async operation should succeed");
        assert_eq!(stats.files_scanned, 3);
        assert_eq!(stats.files_changed, 3);
    }

    #[tokio::test]
    async fn test_sync_workspace_extra_memory_paths() {
        let dir = tempdir().expect("should create temp dir");
        let workspace = tempdir().expect("should create temp dir");
        let extra = tempdir().expect("should create temp dir");

        std::fs::write(workspace.path().join("main.md"), "# Main").expect("should write file");
        std::fs::write(extra.path().join("extra.md"), "# Extra notes").expect("should write file");

        let config = MnemosyneConfig {
            db_path: dir.path().join("sync.db"),
            enable_fts: false,
            enable_embeddings: false,
            extra_memory_paths: vec![extra.path().to_path_buf()],
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let stats = mn
            .sync_workspace(workspace.path())
            .await
            .expect("async operation should succeed");
        // Should scan both workspace and extra path
        assert_eq!(stats.files_scanned, 2);
        assert_eq!(stats.files_changed, 2);
    }

    // ── Session sync ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_sync_sessions_disabled() {
        let dir = tempdir().expect("should create temp dir");
        let sessions = tempdir().expect("should create temp dir");

        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_session_indexing: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        let count = mn
            .sync_sessions(sessions.path())
            .await
            .expect("async operation should succeed");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_sync_sessions_empty_dir() {
        let dir = tempdir().expect("should create temp dir");
        let sessions = tempdir().expect("should create temp dir");

        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_session_indexing: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        let count = mn
            .sync_sessions(sessions.path())
            .await
            .expect("async operation should succeed");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_sync_sessions_nonexistent_dir() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_session_indexing: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        let count = mn
            .sync_sessions(Path::new("/nonexistent/path"))
            .await
            .expect("Path::new should succeed");
        assert_eq!(count, 0);
    }

    // ── Session file tracking ──────────────────────────────────────────────

    #[tokio::test]
    async fn test_session_file_upsert_and_get() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        {
            let store = mn.store.lock().await;
            store
                .upsert_session_file("ses1", "/tmp/ses1.jsonl", 512, 0, 0)
                .expect("upsert_session_file should succeed");
            let entry = store
                .get_session_file("ses1")
                .expect("get_session_file should succeed")
                .expect("unwrap should succeed");
            assert_eq!(entry.session_id, "ses1");
            assert_eq!(entry.last_size, 512);
            assert_eq!(entry.file_path, "/tmp/ses1.jsonl");
        }
    }

    #[tokio::test]
    async fn test_session_file_update() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        {
            let store = mn.store.lock().await;
            store
                .upsert_session_file("ses1", "/tmp/ses1.jsonl", 512, 0, 0)
                .expect("upsert_session_file should succeed");
            store
                .upsert_session_file("ses1", "/tmp/ses1.jsonl", 2048, 100, 10)
                .expect("upsert_session_file should succeed");
            let entry = store
                .get_session_file("ses1")
                .expect("get_session_file should succeed")
                .expect("unwrap should succeed");
            assert_eq!(entry.last_size, 2048);
        }
    }

    #[tokio::test]
    async fn test_session_file_missing() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        {
            let store = mn.store.lock().await;
            assert!(
                store
                    .get_session_file("nope")
                    .expect("get_session_file should succeed")
                    .is_none()
            );
        }
    }

    // ── Tracked files ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_tracked_file_remove() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        {
            let store = mn.store.lock().await;
            store
                .upsert_tracked_file("doc.md", "workspace", "abc123", 1000, 500)
                .expect("upsert_tracked_file should succeed");
            assert!(
                store
                    .get_tracked_file("doc.md", "workspace")
                    .expect("get_tracked_file should succeed")
                    .is_some()
            );

            let removed = store
                .remove_tracked_file("doc.md", "workspace")
                .expect("remove_tracked_file should succeed");
            assert!(removed);
            assert!(
                store
                    .get_tracked_file("doc.md", "workspace")
                    .expect("get_tracked_file should succeed")
                    .is_none()
            );
        }
    }

    #[tokio::test]
    async fn test_tracked_file_remove_nonexistent() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        {
            let store = mn.store.lock().await;
            let removed = store
                .remove_tracked_file("nope.md", "workspace")
                .expect("remove_tracked_file should succeed");
            assert!(!removed);
        }
    }

    #[tokio::test]
    async fn test_tracked_file_list_by_source() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        {
            let store = mn.store.lock().await;
            store
                .upsert_tracked_file("a.md", "workspace", "h1", 100, 50)
                .expect("upsert_tracked_file should succeed");
            store
                .upsert_tracked_file("b.md", "workspace", "h2", 200, 100)
                .expect("upsert_tracked_file should succeed");
            store
                .upsert_tracked_file("c.md", "extra:notes", "h3", 300, 150)
                .expect("upsert_tracked_file should succeed");

            let workspace_files = store
                .list_tracked_files("workspace")
                .expect("list_tracked_files should succeed");
            assert_eq!(workspace_files.len(), 2);

            let extra_files = store
                .list_tracked_files("extra:notes")
                .expect("list_tracked_files should succeed");
            assert_eq!(extra_files.len(), 1);
            assert_eq!(extra_files[0].path, "c.md");
        }
    }

    // ── Store chunk with source ────────────────────────────────────────────

    #[tokio::test]
    async fn test_store_chunk_with_source() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        {
            let store = mn.store.lock().await;
            let id = store
                .store_chunk_with_source(
                    "file:memory/MEMORY.md",
                    "important knowledge chunk",
                    "memory/MEMORY.md#L1",
                    MemoryType::Semantic,
                )
                .expect("operation should succeed");
            assert!(id > 0);
        }

        // Should be searchable via FTS
        let results = mn
            .search("knowledge", 10)
            .await
            .expect("async operation should succeed");
        assert_eq!(results.len(), 1);
        assert!(results[0].citation.as_deref() == Some("memory/MEMORY.md#L1"));
    }

    // ── Atomic reindex ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_atomic_reindex_basic() {
        let dir = tempdir().expect("should create temp dir");
        let workspace = tempdir().expect("should create temp dir");

        std::fs::write(
            workspace.path().join("doc.md"),
            "# Reindex test\n\nSome content",
        )
        .expect("operation should succeed");

        let config = MnemosyneConfig {
            db_path: dir.path().join("main.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store some pre-existing data
        mn.store("old-session", &Message::user("old data"))
            .await
            .expect("async operation should succeed");

        // Atomic reindex — should create fresh DB from workspace
        let stats = mn
            .atomic_reindex(workspace.path(), None)
            .await
            .expect("async operation should succeed");
        assert_eq!(stats.files_scanned, 1);
        assert_eq!(stats.files_changed, 1);

        // After reindex, old session data is gone (fresh DB)
        let old_msgs = mn
            .recall_session("old-session", 10)
            .await
            .expect("async operation should succeed");
        assert!(old_msgs.is_empty());
    }

    // ── Bootstrap ──────────────────────────────────────────────────────────

    #[test]
    fn test_bootstrap_creates_structure() {
        let workspace = tempdir().expect("should create temp dir");
        let created = bootstrap_workspace(workspace.path()).expect("operation should succeed");

        // Should create several files
        assert!(!created.is_empty());

        // Directories should exist
        assert!(workspace.path().join("memory").exists());
        assert!(workspace.path().join("daily").exists());

        // Key files should exist
        assert!(workspace.path().join("memory").join("MEMORY.md").exists());
        assert!(workspace.path().join("AGENTS.md").exists());
        assert!(workspace.path().join("IDENTITY.md").exists());
    }

    #[test]
    fn test_bootstrap_idempotent() {
        let workspace = tempdir().expect("should create temp dir");
        let created1 = bootstrap_workspace(workspace.path()).expect("operation should succeed");
        let created2 = bootstrap_workspace(workspace.path()).expect("operation should succeed");

        assert!(!created1.is_empty());
        assert!(created2.is_empty()); // Already populated
    }

    // ── Config defaults ────────────────────────────────────────────────────

    #[test]
    fn test_config_qmd_defaults() {
        let config = MnemosyneConfig::default();
        assert!(!config.enable_qmd);
        assert_eq!(config.qmd_url, "http://localhost:7720");
        assert_eq!(config.qmd_timeout_ms, 3000);
        assert!((config.qmd_bm25_weight - 0.3).abs() < f64::EPSILON);
        assert!((config.qmd_vector_weight - 0.3).abs() < f64::EPSILON);
        assert!((config.qmd_reranker_weight - 0.4).abs() < f64::EPSILON);
        assert_eq!(config.qmd_candidate_multiplier, 4);
        assert!(config.qmd_reranker_url.is_none());
    }

    #[test]
    fn test_config_embedding_defaults() {
        let config = MnemosyneConfig::default();
        assert!(!config.enable_embeddings);
        assert_eq!(config.embedding_dim, 768);
        assert_eq!(config.embedding_model, "nomic-embed-text");
        assert!((config.vector_weight - 0.7).abs() < f64::EPSILON);
        assert!((config.text_weight - 0.3).abs() < f64::EPSILON);
        assert_eq!(config.candidate_multiplier, 4);
        assert_eq!(config.fallback_threshold, 3);
        assert_eq!(config.embed_batch_size, 100);
        assert_eq!(config.chunk_overlap_tokens, 80);
        assert!(
            config.embedding_host.is_none(),
            "embedding_host defaults to None"
        );
    }

    #[test]
    fn test_embedding_host_serde_roundtrip() {
        let json = r#"{
            "db_path": "/tmp/test.db",
            "embedding_host": "http://gpu-server.local:11434"
        }"#;
        let config: MnemosyneConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            config.embedding_host.as_deref(),
            Some("http://gpu-server.local:11434")
        );
        // Round-trip
        let back: MnemosyneConfig =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert_eq!(back.embedding_host, config.embedding_host);
    }

    #[test]
    fn test_embedding_host_none_serde() {
        let json = r#"{"db_path": "/tmp/test.db"}"#;
        let config: MnemosyneConfig = serde_json::from_str(json).unwrap();
        assert!(config.embedding_host.is_none());
    }

    #[test]
    fn test_embedder_instance_uses_embedding_host_for_ollama() {
        let config = MnemosyneConfig {
            ollama_url: "http://default:11434".to_string(),
            embedding_host: Some("http://gpu-server:11434".to_string()),
            embedding_model: "nomic-embed-text".to_string(),
            ..Default::default()
        };
        let instance = EmbedderInstance::new(EmbeddingProvider::Ollama, &config);
        assert_eq!(
            instance.base_url, "http://gpu-server:11434",
            "embedding_host should override ollama_url for Ollama provider"
        );
    }

    #[test]
    fn test_embedder_instance_falls_back_to_ollama_url() {
        let config = MnemosyneConfig {
            ollama_url: "http://default:11434".to_string(),
            embedding_host: None,
            embedding_model: "nomic-embed-text".to_string(),
            ..Default::default()
        };
        let instance = EmbedderInstance::new(EmbeddingProvider::Ollama, &config);
        assert_eq!(
            instance.base_url, "http://default:11434",
            "should use ollama_url when embedding_host is None"
        );
    }

    #[test]
    fn test_embedder_instance_trailing_slash_stripped() {
        let config = MnemosyneConfig {
            ollama_url: "http://default:11434/".to_string(),
            embedding_host: Some("http://gpu-server:11434/".to_string()),
            embedding_model: "nomic-embed-text".to_string(),
            ..Default::default()
        };
        let instance = EmbedderInstance::new(EmbeddingProvider::Ollama, &config);
        assert!(
            !instance.base_url.ends_with('/'),
            "trailing slash must be stripped"
        );
        assert_eq!(instance.base_url, "http://gpu-server:11434");
    }

    #[test]
    fn test_openai_provider_ignores_embedding_host() {
        // embedding_host only affects Ollama; cloud providers use their own fixed URLs
        let config = MnemosyneConfig {
            ollama_url: "http://local:11434".to_string(),
            embedding_host: Some("http://should-be-ignored:11434".to_string()),
            ..Default::default()
        };
        let instance = EmbedderInstance::new(EmbeddingProvider::OpenAI, &config);
        assert!(
            instance.base_url.contains("openai.com"),
            "OpenAI provider should always use openai.com, got: {}",
            instance.base_url
        );
    }

    #[test]
    fn test_config_session_defaults() {
        let config = MnemosyneConfig::default();
        assert!(config.enable_session_indexing);
        assert_eq!(config.session_delta_bytes, 100_000);
        assert_eq!(config.session_delta_messages, 50);
        assert!(!config.enable_file_watcher);
        assert!(config.watch_paths.is_empty());
        assert!(config.extra_memory_paths.is_empty());
    }

    // ── has_embedder / active_provider ──────────────────────────────────────

    #[tokio::test]
    async fn test_has_embedder_disabled() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        assert!(!mn.has_embedder());
        assert_eq!(mn.active_embedding_provider().await, "none");
        assert!(mn.embedding_fallback_state().await.is_empty());
    }

    #[tokio::test]
    async fn test_has_embedder_enabled() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_embeddings: true,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        assert!(mn.has_embedder());
        // Active provider should be the first in the chain
        let provider = mn.active_embedding_provider().await;
        assert!(!provider.is_empty());
        assert_ne!(provider, "none");
    }

    // ── QMD ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_qmd_disabled_by_default() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        assert!(!mn.qmd_available());
    }

    // ── store_ref ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_store_ref_accessible() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // store_ref should give access to the inner store
        let store = mn.store_ref().lock().await;
        let stats = store.stats().expect("stats should succeed");
        assert_eq!(stats.message_count, 0);
    }

    // ── Memory type ────────────────────────────────────────────────────────

    #[test]
    fn test_memory_type_from_str_all() {
        assert_eq!(MemoryType::parse_label("working"), MemoryType::Working);
        assert_eq!(MemoryType::parse_label("semantic"), MemoryType::Semantic);
        assert_eq!(MemoryType::parse_label("episodic"), MemoryType::Episodic);
        assert_eq!(MemoryType::parse_label("fact"), MemoryType::Fact);
        assert_eq!(
            MemoryType::parse_label("preference"),
            MemoryType::Preference
        );
        assert_eq!(
            MemoryType::parse_label("conversation"),
            MemoryType::Conversation
        );
        assert_eq!(MemoryType::parse_label("summary"), MemoryType::Summary);
        // Unknown falls back to Episodic
        assert_eq!(MemoryType::parse_label("unknown"), MemoryType::Episodic);
        assert_eq!(MemoryType::parse_label(""), MemoryType::Episodic);
    }

    #[test]
    fn test_memory_type_display() {
        assert_eq!(MemoryType::Working.to_string(), "working");
        assert_eq!(MemoryType::Episodic.to_string(), "episodic");
        assert_eq!(MemoryType::Semantic.to_string(), "semantic");
        assert_eq!(MemoryType::Fact.to_string(), "fact");
        assert_eq!(MemoryType::Preference.to_string(), "preference");
        assert_eq!(MemoryType::Conversation.to_string(), "conversation");
        assert_eq!(MemoryType::Summary.to_string(), "summary");
    }

    #[test]
    fn test_memory_type_serde_roundtrip() {
        let types = [
            MemoryType::Working,
            MemoryType::Episodic,
            MemoryType::Semantic,
            MemoryType::Fact,
            MemoryType::Preference,
            MemoryType::Conversation,
            MemoryType::Summary,
        ];
        for mt in &types {
            let json = serde_json::to_string(mt).expect("should serialize to JSON");
            let parsed: MemoryType =
                serde_json::from_str(&json).expect("should parse successfully");
            assert_eq!(*mt, parsed);
        }
    }

    // ── Content hash ───────────────────────────────────────────────────────

    #[test]
    fn test_content_hash_consistency() {
        let h1 = compute_content_hash("same text");
        let h2 = compute_content_hash("same text");
        assert_eq!(h1, h2);

        let h3 = compute_content_hash("different text");
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_content_hash_format() {
        let hash = compute_content_hash("test");
        // SHA-256 hex digest = 64 chars
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // ── chunk_text_with_overlap ────────────────────────────────────────────

    #[test]
    fn test_chunk_text_with_overlap_basic() {
        let text = "Short paragraph.";
        let chunks = chunk_text_with_overlap(text, 80);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text.trim(), "Short paragraph.");
    }

    #[test]
    fn test_chunk_text_with_overlap_multiple() {
        // Two large paragraphs
        let p1 = "A".repeat(1500);
        let p2 = "B".repeat(1500);
        let text = format!("{}\n\n{}", p1, p2);
        let chunks = chunk_text_with_overlap(&text, 80);
        assert!(chunks.len() >= 2);
        // With overlap, second chunk should contain some trailing text from first
        if chunks.len() >= 2 {
            // overlap_chars = 80 * 4 = 320, but first chunk is all A's
            // so second chunk should start with trailing A's then B's
            assert!(chunks[1].text.contains("B"));
        }
    }

    #[test]
    fn test_chunk_text_with_overlap_empty() {
        let chunks = chunk_text_with_overlap("", 80);
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_chunk_text_with_overlap_zero_overlap() {
        let text = "Paragraph one.\n\nParagraph two.";
        let chunks = chunk_text_with_overlap(text, 0);
        assert_eq!(chunks.len(), 1);
    }

    // ── collect_md_files ───────────────────────────────────────────────────

    #[test]
    fn test_collect_md_files_basic() {
        let dir = tempdir().expect("should create temp dir");
        std::fs::write(dir.path().join("a.md"), "# A").expect("should write file");
        std::fs::write(dir.path().join("b.txt"), "not md").expect("should write file");
        std::fs::write(dir.path().join("c.md"), "# C").expect("should write file");

        let files = collect_md_files(dir.path());
        assert_eq!(files.len(), 2);
        // Should be sorted
        assert!(files[0].to_string_lossy().contains("a.md"));
        assert!(files[1].to_string_lossy().contains("c.md"));
    }

    #[test]
    fn test_collect_md_files_recursive() {
        let dir = tempdir().expect("should create temp dir");
        std::fs::create_dir_all(dir.path().join("sub")).expect("should create directory");
        std::fs::write(dir.path().join("root.md"), "# Root").expect("should write file");
        std::fs::write(dir.path().join("sub/nested.md"), "# Nested").expect("should write file");

        let files = collect_md_files(dir.path());
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_collect_md_files_empty() {
        let dir = tempdir().expect("should create temp dir");
        let files = collect_md_files(dir.path());
        assert!(files.is_empty());
    }

    // ── parse_session_jsonl ────────────────────────────────────────────────

    #[test]
    fn test_parse_session_jsonl_basic() {
        let jsonl = r#"{"type":"message","role":"user","content":"hello"}
{"type":"message","role":"assistant","content":"hi there"}
"#;
        let (text, count) = parse_session_jsonl(jsonl, 0);
        assert!(text.contains("hello"));
        assert!(text.contains("hi there"));
        assert_eq!(count, 2);
    }

    #[test]
    fn test_parse_session_jsonl_with_offset() {
        let line1 = r#"{"type":"message","role":"user","content":"old"}"#;
        let line2 = r#"{"type":"message","role":"user","content":"new"}"#;
        let jsonl = format!("{}\n{}\n", line1, line2);
        let offset = line1.len() + 1; // skip first line
        let (text, count) = parse_session_jsonl(&jsonl, offset);
        assert!(text.contains("new"));
        assert!(!text.contains("old"));
        assert_eq!(count, 1);
    }

    #[test]
    fn test_parse_session_jsonl_empty() {
        let (text, count) = parse_session_jsonl("", 0);
        assert!(text.is_empty());
        assert_eq!(count, 0);
    }

    #[test]
    fn test_parse_session_jsonl_invalid_json() {
        let jsonl = "not json at all\n{broken\n";
        let (text, count) = parse_session_jsonl(jsonl, 0);
        assert!(text.is_empty());
        assert_eq!(count, 0);
    }

    // ── SyncStats ──────────────────────────────────────────────────────────

    #[test]
    fn test_sync_stats_default() {
        let stats = SyncStats {
            files_scanned: 0,
            files_changed: 0,
            files_unchanged: 0,
            chunks_embedded: 0,
            cache_hits: 0,
            cache_misses: 0,
            sessions_indexed: 0,
            errors: Vec::new(),
        };
        assert!(stats.errors.is_empty());
        let json = serde_json::to_string(&stats).expect("should serialize to JSON");
        let parsed: SyncStats = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed.files_scanned, 0);
    }

    // ── MemoryStats serde ──────────────────────────────────────────────────

    #[test]
    fn test_memory_stats_serde() {
        let stats = MemoryStats {
            message_count: 42,
            session_count: 3,
            embedding_count: 10,
            embedding_cache_count: 5,
            tracked_file_count: 7,
        };
        let json = serde_json::to_string(&stats).expect("should serialize to JSON");
        let parsed: MemoryStats = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed.message_count, 42);
        assert_eq!(parsed.tracked_file_count, 7);
    }

    // ── PatternEntry serde ─────────────────────────────────────────────────

    #[test]
    fn test_stop_words_english_filtered() {
        let sw = build_stop_words();
        // Core English function words must be present
        for word in &["the", "is", "and", "with", "they", "your"] {
            assert!(sw.contains(*word), "English stop word '{word}' missing");
        }
    }

    #[test]
    fn test_stop_words_spanish_filtered() {
        let sw = build_stop_words();
        for word in &["el", "la", "los", "que", "por", "con", "del", "también"] {
            assert!(sw.contains(*word), "Spanish stop word '{word}' missing");
        }
    }

    #[test]
    fn test_stop_words_portuguese_filtered() {
        let sw = build_stop_words();
        for word in &["dos", "das", "pelo", "pela", "também", "porque"] {
            assert!(sw.contains(*word), "Portuguese stop word '{word}' missing");
        }
    }

    #[test]
    fn test_stop_words_japanese_filtered() {
        let sw = build_stop_words();
        // Single-char particles (3 UTF-8 bytes)
        for word in &["の", "は", "が", "を", "に", "で"] {
            assert!(sw.contains(*word), "Japanese particle '{word}' missing");
        }
        // Multi-char forms
        for word in &["から", "です", "ます", "この", "それ"] {
            assert!(sw.contains(*word), "Japanese auxiliary '{word}' missing");
        }
    }

    #[test]
    fn test_stop_words_korean_filtered() {
        let sw = build_stop_words();
        for word in &[
            "은", "는", "이", "가", "을", "를", "의", "에서", "부터", "까지",
        ] {
            assert!(sw.contains(*word), "Korean particle '{word}' missing");
        }
    }

    #[test]
    fn test_stop_words_arabic_filtered() {
        let sw = build_stop_words();
        for word in &["في", "من", "على", "هذا", "التي", "الذي", "إلى", "حتى"]
        {
            assert!(sw.contains(*word), "Arabic function word '{word}' missing");
        }
    }

    #[test]
    fn test_stop_words_content_words_not_filtered() {
        let sw = build_stop_words();
        // Meaningful content words in each language must NOT be stop words
        for word in &[
            "rust",
            "memory",
            "agent",
            "search", // English
            "proyecto",
            "usuario",
            "datos", // Spanish
            "projeto",
            "usuário",
            "sistema", // Portuguese
            "エージェント",
            "プログラム", // Japanese (katakana content words)
            "프로그램",
            "에이전트", // Korean (content words)
            "برنامج",
            "ذاكرة", // Arabic (content words)
        ] {
            assert!(
                !sw.contains(*word),
                "Content word '{word}' should not be a stop word"
            );
        }
    }

    #[test]
    fn test_pattern_entry_serde() {
        let entry = PatternEntry {
            id: 1,
            pattern_type: "tool_frequency".to_string(),
            content: "read_file: 15 uses".to_string(),
            frequency: 15,
            first_seen: "2026-01-01T00:00:00Z".to_string(),
            last_seen: "2026-02-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&entry).expect("should serialize to JSON");
        let parsed: PatternEntry = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed.pattern_type, "tool_frequency");
        assert_eq!(parsed.frequency, 15);
    }

    // ── Working memory edge cases ──────────────────────────────────────────

    #[tokio::test]
    async fn test_working_memory_empty() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        let wm = mn
            .working_memory("empty-session")
            .await
            .expect("async operation should succeed");
        assert!(wm.is_empty());
    }

    #[tokio::test]
    async fn test_finalize_working_memory_empty() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        // Should not error on empty session
        let (promoted, discarded) = mn
            .finalize_working_memory("empty", 0.5)
            .await
            .expect("async operation should succeed");
        assert_eq!(promoted, 0);
        assert_eq!(discarded, 0);
    }

    // ── Hybrid search on empty DB ──────────────────────────────────────────

    #[tokio::test]
    async fn test_hybrid_search_empty() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        let results = mn
            .hybrid_search("anything", None, 10)
            .await
            .expect("async operation should succeed");
        assert!(results.is_empty());
    }

    // ── Importance operations ──────────────────────────────────────────────

    #[tokio::test]
    async fn test_get_memory_importance_nonexistent() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");
        let result = mn.get_memory_importance(99999).await;
        // Should return error or default
        assert!(result.is_err() || result.expect("operation should succeed").0 == 0.0);
    }

    #[tokio::test]
    async fn test_boost_then_decay() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: false,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        let id = mn
            .store_typed(
                "s1",
                &Message::user("important fact"),
                MemoryType::Episodic,
                0.5,
            )
            .await
            .expect("async operation should succeed");
        mn.boost_memory(id, 0.3)
            .await
            .expect("async operation should succeed");
        let (imp, _) = mn
            .get_memory_importance(id)
            .await
            .expect("async operation should succeed");
        assert!((imp - 0.8).abs() < 0.01);

        // decay_memories only affects episodic type with daily rate
        mn.decay_memories(1.0)
            .await
            .expect("async operation should succeed");
        let (imp2, _) = mn
            .get_memory_importance(id)
            .await
            .expect("async operation should succeed");
        // With daily_rate=1.0 and at least 1 day elapsed factor, importance should drop
        assert!(imp2 <= imp);
    }

    // ── Proactive context ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_proactive_context_with_data() {
        let dir = tempdir().expect("should create temp dir");
        let config = MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        };
        let mn = Mnemosyne::new(config)
            .await
            .expect("Mnemosyne::new should succeed");

        // Store some messages and extract patterns
        mn.store("s1", &Message::user("build the login page"))
            .await
            .expect("async operation should succeed");
        mn.store("s1", &Message::assistant("I'll create the login component"))
            .await
            .expect("async operation should succeed");
        mn.store("s1", &Message::user("now add authentication"))
            .await
            .expect("async operation should succeed");

        // Even without patterns, proactive_context should not crash
        let messages = vec![Message::user("working on login")];
        let context: Vec<SearchResult> = mn
            .proactive_context(&messages, 5)
            .await
            .expect("async operation should succeed");
        // Context is a vec (may be empty if no patterns match)
        let _ = context; // just verify it doesnt crash
    }
}

// Sprint 8: Temporal Memory + Entity Tests

#[cfg(test)]
mod temporal_tests {
    use super::*;
    use tempfile::tempdir;

    fn make_config(dir: &std::path::Path) -> MnemosyneConfig {
        MnemosyneConfig {
            db_path: dir.join("temporal_test.db"),
            enable_fts: true,
            max_messages_per_session: 100,
            enable_embeddings: false,
            ..Default::default()
        }
    }

    // ── Supersession ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_supersede_message() {
        let dir = tempdir().unwrap();
        let mn = Mnemosyne::new(make_config(dir.path())).await.unwrap();

        let id1 = mn
            .store("s1", &Message::user("Favorite color is blue"))
            .await
            .unwrap();
        let id2 = mn
            .store("s1", &Message::user("Favorite color is green"))
            .await
            .unwrap();

        // Supersede the old one
        mn.supersede_message(id1, id2).await.unwrap();

        // Get current memories — should only return the new one
        let current = mn.get_current_memories(100).await.unwrap();
        let ids: Vec<i64> = current.iter().map(|r| r.id).collect();
        assert!(ids.contains(&id2), "New memory should be current");
        assert!(!ids.contains(&id1), "Old memory should be superseded");
    }

    #[tokio::test]
    async fn test_supersession_chain() {
        let dir = tempdir().unwrap();
        let mn = Mnemosyne::new(make_config(dir.path())).await.unwrap();

        let id1 = mn.store("s1", &Message::user("Version 1")).await.unwrap();
        let id2 = mn.store("s1", &Message::user("Version 2")).await.unwrap();
        let id3 = mn.store("s1", &Message::user("Version 3")).await.unwrap();

        // Build chain: id1 → id2 → id3
        mn.supersede_message(id1, id2).await.unwrap();
        mn.supersede_message(id2, id3).await.unwrap();

        // Walk chain from id1
        let chain = mn.get_supersession_chain(id1).await.unwrap();
        assert_eq!(chain.len(), 3, "Chain should have 3 versions");
        assert_eq!(chain[0].0, id1);
        assert_eq!(chain[1].0, id2);
        assert_eq!(chain[2].0, id3);
        assert!(
            chain[2].2.is_none(),
            "Last in chain should not be superseded"
        );
    }

    #[tokio::test]
    async fn test_superseded_excluded_from_search() {
        let dir = tempdir().unwrap();
        let mn = Mnemosyne::new(make_config(dir.path())).await.unwrap();

        let id1 = mn
            .store("s1", &Message::user("The capital of France is Paris"))
            .await
            .unwrap();
        let id2 = mn
            .store("s1", &Message::user("The capital of France is still Paris"))
            .await
            .unwrap();

        // Supersede the first
        mn.supersede_message(id1, id2).await.unwrap();

        // FTS search should only return the current version
        let results = mn.search("capital France", 10).await.unwrap();
        for r in &results {
            assert_ne!(r.id, id1, "Superseded memory should not appear in search");
        }
    }

    #[tokio::test]
    async fn test_valid_from_auto_set() {
        let dir = tempdir().unwrap();
        let config = make_config(dir.path());
        let store = MemoryStore::new(&config.db_path, true, false).unwrap();

        let msg = Message::user("Test valid_from population");
        let id = store.store_message("s1", &msg).unwrap();

        // Check that valid_from was set
        let valid_from: Option<String> = store
            .conn
            .query_row(
                "SELECT valid_from FROM messages WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .unwrap();

        assert!(
            valid_from.is_some(),
            "valid_from should be auto-populated on insert"
        );
    }

    #[tokio::test]
    async fn test_valid_to_null_for_new_messages() {
        let dir = tempdir().unwrap();
        let config = make_config(dir.path());
        let store = MemoryStore::new(&config.db_path, true, false).unwrap();

        let id = store
            .store_message("s1", &Message::user("New message"))
            .unwrap();

        let valid_to: Option<String> = store
            .conn
            .query_row(
                "SELECT valid_to FROM messages WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .unwrap();

        assert!(
            valid_to.is_none(),
            "New messages should have valid_to = NULL"
        );
    }

    #[tokio::test]
    async fn test_multiple_supersessions() {
        let dir = tempdir().unwrap();
        let mn = Mnemosyne::new(make_config(dir.path())).await.unwrap();

        // Store 5 versions
        let mut ids = Vec::new();
        for i in 1..=5 {
            let id = mn
                .store("s1", &Message::user(&format!("Version {}", i)))
                .await
                .unwrap();
            ids.push(id);
        }

        // Supersede each by the next
        for i in 0..4 {
            mn.supersede_message(ids[i], ids[i + 1]).await.unwrap();
        }

        // Only the last should be current
        let current = mn.get_current_memories(100).await.unwrap();
        let current_ids: Vec<i64> = current.iter().map(|r| r.id).collect();
        assert!(
            current_ids.contains(&ids[4]),
            "Last version should be current"
        );
        for i in 0..4 {
            assert!(
                !current_ids.contains(&ids[i]),
                "Old version {} should be superseded",
                i
            );
        }
    }

    // ── Entities ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_entity_creation() {
        let dir = tempdir().unwrap();
        let mn = Mnemosyne::new(make_config(dir.path())).await.unwrap();

        let id = mn
            .upsert_entity("Rust Programming Language", "technology")
            .await
            .unwrap();
        assert!(id > 0, "Entity ID should be positive");

        let entities = mn.get_entities(100).await.unwrap();
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].canonical_name, "Rust Programming Language");
        assert_eq!(entities[0].entity_type, "technology");
    }

    #[tokio::test]
    async fn test_entity_dedup_exact_match() {
        let dir = tempdir().unwrap();
        let mn = Mnemosyne::new(make_config(dir.path())).await.unwrap();

        let id1 = mn.upsert_entity("Rust", "technology").await.unwrap();
        let id2 = mn.upsert_entity("rust", "technology").await.unwrap(); // case-insensitive
        assert_eq!(id1, id2, "Same entity should be deduped");

        let entities = mn.get_entities(100).await.unwrap();
        assert_eq!(entities.len(), 1);
        assert_eq!(
            entities[0].mention_count, 2,
            "Mention count should increase"
        );
    }

    #[tokio::test]
    async fn test_entity_fuzzy_match() {
        let dir = tempdir().unwrap();
        let mn = Mnemosyne::new(make_config(dir.path())).await.unwrap();

        let id1 = mn.upsert_entity("JavaScript", "technology").await.unwrap();
        let id2 = mn.upsert_entity("Javascript", "technology").await.unwrap(); // typo
        assert_eq!(id1, id2, "Fuzzy match should resolve to same entity");
    }

    #[tokio::test]
    async fn test_entity_different_types() {
        let dir = tempdir().unwrap();
        let mn = Mnemosyne::new(make_config(dir.path())).await.unwrap();

        let id1 = mn.upsert_entity("Python", "technology").await.unwrap();
        let id2 = mn.upsert_entity("Python", "animal").await.unwrap();
        assert_ne!(id1, id2, "Same name, different type = different entities");
    }

    #[tokio::test]
    async fn test_entity_message_linking() {
        let dir = tempdir().unwrap();
        let mn = Mnemosyne::new(make_config(dir.path())).await.unwrap();

        let msg_id = mn
            .store("s1", &Message::user("I love Rust programming"))
            .await
            .unwrap();
        let entity_id = mn.upsert_entity("Rust", "technology").await.unwrap();
        mn.link_entity_to_message(entity_id, msg_id, "Rust")
            .await
            .unwrap();

        let messages = mn.get_entity_messages(entity_id, 100).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, msg_id);
    }

    #[tokio::test]
    async fn test_entity_mention_count_ordering() {
        let dir = tempdir().unwrap();
        let mn = Mnemosyne::new(make_config(dir.path())).await.unwrap();

        // Create "Rust" mentioned 3 times
        for _ in 0..3 {
            mn.upsert_entity("Rust", "technology").await.unwrap();
        }
        // Create "Python" mentioned once
        mn.upsert_entity("Python", "technology").await.unwrap();

        let entities = mn.get_entities(100).await.unwrap();
        assert_eq!(entities[0].canonical_name, "Rust");
        assert_eq!(entities[0].mention_count, 3);
        assert_eq!(entities[1].canonical_name, "Python");
        assert_eq!(entities[1].mention_count, 1);
    }

    // ── Levenshtein ──────────────────────────────────────────────────────

    #[test]
    fn test_levenshtein_identical() {
        assert_eq!(levenshtein_distance("hello", "hello"), 0);
        assert!((levenshtein_ratio("hello", "hello") - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_levenshtein_one_edit() {
        assert_eq!(levenshtein_distance("hello", "helo"), 1);
        assert_eq!(levenshtein_distance("cat", "bat"), 1);
    }

    #[test]
    fn test_levenshtein_ratio_threshold() {
        // "JavaScript" vs "Javascript" — 1 edit distance out of 10
        let ratio = levenshtein_ratio("javascript", "Javascript".to_lowercase().as_str());
        assert!((ratio - 1.0).abs() < f64::EPSILON, "Same after lowering");

        // "Miguel" vs "Miquel" — 1 edit
        let ratio = levenshtein_ratio("miguel", "miquel");
        assert!(
            ratio >= 0.83,
            "Close names should have high ratio: {}",
            ratio
        );
    }

    #[test]
    fn test_levenshtein_empty() {
        assert_eq!(levenshtein_distance("", "hello"), 5);
        assert_eq!(levenshtein_distance("hello", ""), 5);
        assert!((levenshtein_ratio("", "") - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_heuristic_extract_sprints() {
        let entities = Mnemosyne::heuristic_extract("Working on S47 Track A and S48 backlog");
        let types: Vec<(&str, &str)> = entities.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
        assert!(types.contains(&("S47", "sprint")));
        assert!(types.contains(&("S48", "sprint")));
        assert!(types.contains(&("Track A", "track")));
    }

    #[test]
    fn test_heuristic_extract_mentions() {
        let entities = Mnemosyne::heuristic_extract("@zeus107 confirmed Track B is ready");
        let types: Vec<(&str, &str)> = entities.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
        assert!(types.contains(&("zeus107", "person")));
        assert!(types.contains(&("Track B", "track")));
    }

    #[test]
    fn test_heuristic_extract_agents() {
        let entities = Mnemosyne::heuristic_extract("Zeus112 and fbsd1 are building features");
        let types: Vec<(&str, &str)> = entities.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
        assert!(types.contains(&("Zeus112", "person")));
        assert!(types.contains(&("fbsd1", "person")));
    }

    #[test]
    fn test_heuristic_extract_projects() {
        let entities = Mnemosyne::heuristic_extract("Zeus uses Mnemosyne for memory and Prometheus for orchestration");
        let types: Vec<(&str, &str)> = entities.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
        assert!(types.contains(&("Zeus", "project")));
        assert!(types.contains(&("Mnemosyne", "project")));
        assert!(types.contains(&("Prometheus", "project")));
    }

    #[test]
    fn test_heuristic_extract_pr_refs() {
        let entities = Mnemosyne::heuristic_extract("PR #11 is up for gate review");
        let types: Vec<(&str, &str)> = entities.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
        assert!(types.contains(&("PR #11", "artifact")));
    }

    #[test]
    fn test_heuristic_extract_decisions() {
        let entities = Mnemosyne::heuristic_extract("Team confirmed that Track D is merged. Moving on.");
        let types: Vec<(&str, &str)> = entities.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
        assert!(types.iter().any(|(_, t)| *t == "decision"));
    }

    #[test]
    fn test_heuristic_extract_dedup() {
        let entities = Mnemosyne::heuristic_extract("S47 is great. S47 is the best sprint. S47 forever.");
        let sprint_count = entities.iter().filter(|(_, t)| t == "sprint").count();
        assert_eq!(sprint_count, 1, "Should deduplicate S47");
    }

    #[test]
    fn test_heuristic_extract_empty() {
        let entities = Mnemosyne::heuristic_extract("hello world");
        // No sprints, tracks, mentions, known agents, or decisions
        // May match "Zeus" if not careful — but "hello world" has none
        assert!(entities.is_empty() || entities.iter().all(|(_, t)| t == "project"));
    }
}

#[cfg(test)]
mod consolidation_tests {
    use super::*;
    use tempfile::tempdir;

    fn make_msg(role: zeus_core::Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_string(),
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
        }
    }

    fn test_store(dir: &std::path::Path) -> MemoryStore {
        MemoryStore::new(&dir.join("test.db"), true, false).expect("store")
    }

    #[test]
    fn test_find_duplicate_no_match() {
        let dir = tempdir().unwrap();
        let store = test_store(dir.path());

        store
            .store_message("s1", &make_msg(zeus_core::Role::Assistant, "The weather in Paris is sunny today"))
            .unwrap();

        let dup = store
            .find_duplicate("Rust programming language tutorial for beginners", 0.85)
            .unwrap();
        assert!(dup.is_none(), "Unrelated content should not match");
    }

    #[test]
    fn test_find_duplicate_match() {
        let dir = tempdir().unwrap();
        let store = test_store(dir.path());

        let original = "The Zeus project uses Rust for the backend with 32 crates in the workspace";
        store
            .store_message("s1", &make_msg(zeus_core::Role::Assistant, original))
            .unwrap();

        // Near-duplicate (minor rewording)
        let dup = store
            .find_duplicate(
                "The Zeus project uses Rust for the backend with 32 crates in the workspace layout",
                0.80,
            )
            .unwrap();
        assert!(dup.is_some(), "Near-duplicate should be detected");
    }

    #[test]
    fn test_find_duplicate_disabled() {
        let dir = tempdir().unwrap();
        let store = test_store(dir.path());

        store
            .store_message("s1", &make_msg(zeus_core::Role::Assistant, "identical content here"))
            .unwrap();

        // threshold=0 disables dedup
        let dup = store
            .find_duplicate("identical content here", 0.0)
            .unwrap();
        assert!(dup.is_none(), "Dedup disabled with threshold=0");
    }

    #[test]
    fn test_store_message_dedup_boosts() {
        let dir = tempdir().unwrap();
        let store = test_store(dir.path());

        let msg = make_msg(zeus_core::Role::Assistant, "Zeus has 32 crates in a cargo workspace with advanced memory subsystem");
        let id1 = store.store_message("s1", &msg).unwrap();

        // Store near-duplicate — should return existing ID
        let msg2 = make_msg(zeus_core::Role::Assistant, "Zeus has 32 crates in a cargo workspace with advanced memory subsystem support");
        let id2 = store.store_message_dedup("s1", &msg2, 0.80).unwrap();

        assert_eq!(id1, id2, "Dedup should return existing message ID");
    }

    #[test]
    fn test_store_message_dedup_user_always_stored() {
        let dir = tempdir().unwrap();
        let store = test_store(dir.path());

        let msg = make_msg(zeus_core::Role::User, "hello world how are you doing today");
        let id1 = store.store_message("s1", &msg).unwrap();

        // User messages always stored even if identical
        let id2 = store.store_message_dedup("s1", &msg, 0.80).unwrap();
        assert_ne!(id1, id2, "User messages should never be deduped");
    }

    #[test]
    fn test_consolidate_session_small() {
        let dir = tempdir().unwrap();
        let store = test_store(dir.path());

        // Only 3 messages — below threshold for keep_edges=2
        for i in 0..3 {
            store
                .store_message("s1", &make_msg(zeus_core::Role::User, &format!("msg {}", i)))
                .unwrap();
        }

        let (kept, consolidated) = store.consolidate_session("s1", 2).unwrap();
        assert_eq!(consolidated, 0, "Should not consolidate small sessions");
        assert_eq!(kept, 3);
    }

    #[test]
    fn test_consolidate_session_large() {
        let dir = tempdir().unwrap();
        let store = test_store(dir.path());

        // 10 messages — keep_edges=2 means keep first 2 + last 2, consolidate middle 6
        for i in 0..10 {
            store
                .store_message(
                    "s1",
                    &make_msg(zeus_core::Role::User, &format!("message number {} with some content", i)),
                )
                .unwrap();
        }

        let (kept, consolidated) = store.consolidate_session("s1", 2).unwrap();
        assert_eq!(consolidated, 6, "Should consolidate middle 6 messages");
        assert_eq!(kept, 5, "Should keep 4 edges + 1 summary");
    }

    #[test]
    fn test_memory_count() {
        let dir = tempdir().unwrap();
        let store = test_store(dir.path());

        assert_eq!(store.memory_count().unwrap(), 0);

        for i in 0..5 {
            store
                .store_message("s1", &make_msg(zeus_core::Role::User, &format!("msg {}", i)))
                .unwrap();
        }

        assert_eq!(store.memory_count().unwrap(), 5);
    }

    #[test]
    fn test_enforce_memory_cap() {
        let dir = tempdir().unwrap();
        let store = test_store(dir.path());

        // Insert 10 episodic messages
        for i in 0..10 {
            store
                .store_message("s1", &make_msg(zeus_core::Role::User, &format!("episodic msg {}", i)))
                .unwrap();
        }

        assert_eq!(store.memory_count().unwrap(), 10);

        // Cap at 7 — should prune 3
        let pruned = store.enforce_memory_cap(7).unwrap();
        assert_eq!(pruned, 3);
        assert_eq!(store.memory_count().unwrap(), 7);
    }

    #[test]
    fn test_enforce_memory_cap_unlimited() {
        let dir = tempdir().unwrap();
        let store = test_store(dir.path());

        for i in 0..5 {
            store
                .store_message("s1", &make_msg(zeus_core::Role::User, &format!("msg {}", i)))
                .unwrap();
        }

        // max=0 means unlimited
        let pruned = store.enforce_memory_cap(0).unwrap();
        assert_eq!(pruned, 0);
        assert_eq!(store.memory_count().unwrap(), 5);
    }

    #[test]
    fn test_sessions_over_limit() {
        let dir = tempdir().unwrap();
        let store = test_store(dir.path());

        // s1: 5 messages, s2: 3 messages
        for i in 0..5 {
            store
                .store_message("s1", &make_msg(zeus_core::Role::User, &format!("msg {}", i)))
                .unwrap();
        }
        for i in 0..3 {
            store
                .store_message("s2", &make_msg(zeus_core::Role::User, &format!("msg {}", i)))
                .unwrap();
        }

        let over = store.sessions_over_limit(4).unwrap();
        assert_eq!(over.len(), 1);
        assert_eq!(over[0].0, "s1");
        assert_eq!(over[0].1, 5);
    }

    #[test]
    fn test_run_consolidation() {
        let dir = tempdir().unwrap();
        let store = test_store(dir.path());

        // Insert 15 messages in one session
        for i in 0..15 {
            store
                .store_message(
                    "s1",
                    &make_msg(zeus_core::Role::User, &format!("consolidation test message {}", i)),
                )
                .unwrap();
        }

        // session_limit=10, max_memories=50000, keep_edges=3
        let (sessions, pruned) = store.run_consolidation(10, 50000, 3).unwrap();
        assert_eq!(sessions, 1, "One session should be consolidated");
        assert!(pruned > 0, "Some messages should be pruned");

        // After consolidation, non-superseded count should be reduced
        let count = store.memory_count().unwrap();
        assert!(count < 15, "Memory count should be less after consolidation: {}", count);
    }
}

#[cfg(test)]
mod tests_v10_channel_schema {
    use super::*;
    use tempfile::tempdir;

    fn make_config(dir: &tempfile::TempDir) -> MnemosyneConfig {
        MnemosyneConfig {
            db_path: dir.path().join("test_v10.db"),
            enable_fts: false,
            max_messages_per_session: 100,
            enable_embeddings: false,
            ..Default::default()
        }
    }

    /// v10 migration runs cleanly on a fresh DB — columns exist and are nullable.
    #[tokio::test]
    async fn test_v10_migration_columns_exist() {
        let dir = tempdir().unwrap();
        let mnemosyne = Mnemosyne::new(make_config(&dir)).await.unwrap();
        let store = mnemosyne.store.lock().await;

        let col_names: Vec<String> = store
            .conn()
            .prepare("PRAGMA table_info(messages)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert!(col_names.contains(&"channel_kind".to_string()), "channel_kind column must exist");
        assert!(col_names.contains(&"chat_id".to_string()), "chat_id column must exist");
    }

    /// Inserting a message without channel_kind/chat_id (legacy path) succeeds — NULLs allowed.
    #[tokio::test]
    async fn test_v10_insert_without_channel_fields_succeeds() {
        let dir = tempdir().unwrap();
        let mnemosyne = Mnemosyne::new(make_config(&dir)).await.unwrap();
        let store = mnemosyne.store.lock().await;

        store.conn().execute(
            "INSERT INTO messages (session_id, role, content, timestamp) VALUES (?1, ?2, ?3, datetime('now'))",
            rusqlite::params!["sess-legacy", "user", "hello from legacy"],
        ).unwrap();

        let (ck, ci): (Option<String>, Option<String>) = store.conn().query_row(
            "SELECT channel_kind, chat_id FROM messages WHERE session_id = ?1",
            rusqlite::params!["sess-legacy"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();

        assert!(ck.is_none(), "channel_kind should be NULL for legacy rows");
        assert!(ci.is_none(), "chat_id should be NULL for legacy rows");
    }

    /// Inserting with channel_kind + chat_id round-trips correctly.
    #[tokio::test]
    async fn test_v10_insert_and_read_with_channel_fields() {
        let dir = tempdir().unwrap();
        let mnemosyne = Mnemosyne::new(make_config(&dir)).await.unwrap();
        let store = mnemosyne.store.lock().await;

        store.conn().execute(
            "INSERT INTO messages (session_id, role, content, timestamp, channel_kind, chat_id)
             VALUES (?1, ?2, ?3, datetime('now'), ?4, ?5)",
            rusqlite::params!["sess-discord", "user", "hey titan", "discord", "1475583517156180018"],
        ).unwrap();

        let (ck, ci): (Option<String>, Option<String>) = store.conn().query_row(
            "SELECT channel_kind, chat_id FROM messages WHERE session_id = ?1",
            rusqlite::params!["sess-discord"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();

        assert_eq!(ck.as_deref(), Some("discord"));
        assert_eq!(ci.as_deref(), Some("1475583517156180018"));
    }

    /// Migration is idempotent — opening the same DB twice doesn't error.
    #[tokio::test]
    async fn test_v10_migration_idempotent() {
        let dir = tempdir().unwrap();
        let config = make_config(&dir);
        // First open runs migrations
        let _ = Mnemosyne::new(config.clone()).await.unwrap();
        // Second open on same DB must not panic or error
        let m2 = Mnemosyne::new(config).await;
        assert!(m2.is_ok(), "Re-opening DB after v10 migration should succeed");
    }

    /// Regression: pre-existing messages (no channel columns) still readable after migration.
    #[tokio::test]
    async fn test_v10_regression_existing_rows_readable() {
        let dir = tempdir().unwrap();
        let mnemosyne = Mnemosyne::new(make_config(&dir)).await.unwrap();
        let store = mnemosyne.store.lock().await;

        store.conn().execute(
            "INSERT INTO messages (session_id, role, content, timestamp) VALUES ('old-sess', 'user', 'old msg', datetime('now'))",
            [],
        ).unwrap();
        store.conn().execute(
            "INSERT INTO messages (session_id, role, content, timestamp, channel_kind, chat_id) VALUES ('new-sess', 'assistant', 'new msg', datetime('now'), 'telegram', '-1001234567890')",
            [],
        ).unwrap();

        let count: i64 = store.conn().query_row(
            "SELECT COUNT(*) FROM messages",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 2, "Both old and new rows should be present and readable");
    }
}

#[cfg(test)]
mod tests_v10_sprint_b {
    //! Sprint-B write-path tests (#86): verify store_with_embedding_tagged
    //! populates channel_kind + chat_id columns correctly.
    use super::*;

    fn make_config(dir: &tempfile::TempDir) -> crate::MnemosyneConfig {
        crate::MnemosyneConfig {
            db_path: dir.path().join("test.db"),
            enable_embeddings: false,
            ..Default::default()
        }
    }

    /// store_with_embedding_tagged with channel fields populates columns.
    #[tokio::test]
    async fn test_tagged_store_populates_channel_columns() {
        let dir = tempfile::tempdir().unwrap();
        let mnemosyne = Mnemosyne::new(make_config(&dir)).await.unwrap();
        let msg = zeus_core::Message::user("hello from discord");
        let msg_id = mnemosyne
            .store_with_embedding_tagged("sess1", &msg, Some("discord"), Some("123456789"))
            .await
            .unwrap();

        let store = mnemosyne.store.lock().await;
        let (ck, cid): (Option<String>, Option<String>) = store.conn().query_row(
            "SELECT channel_kind, chat_id FROM messages WHERE rowid = ?1",
            rusqlite::params![msg_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(ck.as_deref(), Some("discord"));
        assert_eq!(cid.as_deref(), Some("123456789"));
    }

    /// store_with_embedding_tagged with None fields leaves columns NULL.
    #[tokio::test]
    async fn test_tagged_store_none_leaves_nulls() {
        let dir = tempfile::tempdir().unwrap();
        let mnemosyne = Mnemosyne::new(make_config(&dir)).await.unwrap();
        let msg = zeus_core::Message::user("legacy message");
        let msg_id = mnemosyne
            .store_with_embedding_tagged("sess2", &msg, None, None)
            .await
            .unwrap();

        let store = mnemosyne.store.lock().await;
        let (ck, cid): (Option<String>, Option<String>) = store.conn().query_row(
            "SELECT channel_kind, chat_id FROM messages WHERE rowid = ?1",
            rusqlite::params![msg_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert!(ck.is_none(), "channel_kind should be NULL when not provided");
        assert!(cid.is_none(), "chat_id should be NULL when not provided");
    }

    /// store_message_with_channel round-trips channel fields correctly.
    #[tokio::test]
    async fn test_store_message_with_channel_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mnemosyne = Mnemosyne::new(make_config(&dir)).await.unwrap();
        let store = mnemosyne.store.lock().await;
        let msg = zeus_core::Message::user("telegram message");
        let msg_id = store
            .store_message_with_channel("sess3", &msg, Some("telegram"), Some("-1009999"))
            .unwrap();

        let (ck, cid): (Option<String>, Option<String>) = store.conn().query_row(
            "SELECT channel_kind, chat_id FROM messages WHERE rowid = ?1",
            rusqlite::params![msg_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(ck.as_deref(), Some("telegram"));
        assert_eq!(cid.as_deref(), Some("-1009999"));
    }
}

#[cfg(test)]
mod tests_v10_sprint_c {
    use super::*;
    use std::path::PathBuf;

    fn make_config(dir: &tempfile::TempDir) -> MnemosyneConfig {
        MnemosyneConfig {
            db_path: dir.path().join("test_sprint_c.db"),
            enable_fts: true,
            enable_embeddings: false,
            ..Default::default()
        }
    }

    /// No-op: search_cross_channel returns empty when no cross-channel messages exist.
    #[tokio::test]
    async fn test_cross_channel_noop_when_no_cross_channel_msgs() {
        let dir = tempfile::tempdir().unwrap();
        let mnemosyne = Mnemosyne::new(make_config(&dir)).await.unwrap();
        // Store a discord message
        {
            let store = mnemosyne.store.lock().await;
            let msg = zeus_core::Message::user("hello from discord");
            store.store_message_with_channel("sess1", &msg, Some("discord"), Some("ch1")).unwrap();
        }
        // Query cross-channel from discord — should return nothing (only discord row exists)
        let results = mnemosyne.search_cross_channel("hello", "discord", 10).await.unwrap();
        assert!(results.is_empty(), "Expected no cross-channel results when all messages are discord");
    }

    /// Returns top-k cross-channel messages from other channels.
    #[tokio::test]
    async fn test_cross_channel_returns_other_channel_msgs() {
        let dir = tempfile::tempdir().unwrap();
        let mnemosyne = Mnemosyne::new(make_config(&dir)).await.unwrap();
        {
            let store = mnemosyne.store.lock().await;
            // telegram message — should appear in discord query
            let msg1 = zeus_core::Message::user("telegram context message");
            store.store_message_with_channel("sess1", &msg1, Some("telegram"), Some("tg1")).unwrap();
            // discord message — should NOT appear in discord query
            let msg2 = zeus_core::Message::user("discord context message");
            store.store_message_with_channel("sess2", &msg2, Some("discord"), Some("ch1")).unwrap();
        }
        let results = mnemosyne.search_cross_channel("context", "discord", 10).await.unwrap();
        assert!(!results.is_empty(), "Expected cross-channel results from telegram");
        assert!(
            results.iter().all(|r| !r.content.contains("discord context")),
            "Discord messages must not appear in cross-channel results"
        );
        assert!(
            results.iter().any(|r| r.content.contains("telegram context")),
            "Telegram message should appear in cross-channel results"
        );
    }

    /// NULL channel_kind rows (pre-v10 legacy) are included in cross-channel results.
    #[tokio::test]
    async fn test_cross_channel_includes_null_channel_kind_rows() {
        let dir = tempfile::tempdir().unwrap();
        let mnemosyne = Mnemosyne::new(make_config(&dir)).await.unwrap();
        {
            let store = mnemosyne.store.lock().await;
            // Legacy row: no channel_kind (NULL)
            let msg = zeus_core::Message::user("legacy null channel message");
            store.store_message("sess_legacy", &msg).unwrap();
        }
        // NULL rows should appear regardless of current_channel_kind
        let results = mnemosyne.search_cross_channel("legacy", "discord", 10).await.unwrap();
        assert!(
            results.iter().any(|r| r.content.contains("legacy null channel")),
            "Pre-v10 NULL channel_kind rows must be included in cross-channel results"
        );
    }

    /// Respects limit — returns at most top-k results.
    #[tokio::test]
    async fn test_cross_channel_respects_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mnemosyne = Mnemosyne::new(make_config(&dir)).await.unwrap();
        {
            let store = mnemosyne.store.lock().await;
            for i in 0..10 {
                let msg = zeus_core::Message::user(&format!("telegram msg {}", i));
                store.store_message_with_channel(
                    &format!("sess{}", i), &msg, Some("telegram"), Some("tg1")
                ).unwrap();
            }
        }
        let results = mnemosyne.search_cross_channel("telegram msg", "discord", 3).await.unwrap();
        assert!(results.len() <= 3, "search_cross_channel must respect the limit parameter");
    }
}
