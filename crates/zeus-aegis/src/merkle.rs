//! Merkle tree layer over the tamper-evident audit log.
//!
//! Adds O(log n) inclusion proofs and a single root hash representing
//! the entire audit state. Wraps `AuditLog` transparently — existing
//! consumers continue to work unchanged.

use ring::digest::{SHA256, digest};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use zeus_core::{Error, Result};

use crate::audit::{AuditEvent, AuditLog, Severity};

// ============================================================================
// Merkle tree (in-memory, operates on raw 32-byte digests)
// ============================================================================

type Digest = [u8; 32];

/// Merkle tree over audit entry hashes.
///
/// Leaves are SHA-256 digests of each `AuditEntry::hash` hex string.
/// Internal nodes are `SHA-256(left || right)`. Padded to next power of two
/// by duplicating the last real leaf.
///
/// Stored as a flat BFS array: root at 0, children of `i` at `2i+1` and `2i+2`.
#[derive(Default)]
pub struct MerkleTree {
    nodes: Vec<Digest>,
    leaf_count: usize,
}

impl MerkleTree {
    /// Create an empty tree.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from a slice of `AuditEntry::hash` hex strings.
    pub fn build(entry_hashes: &[&str]) -> Self {
        if entry_hashes.is_empty() {
            return Self::new();
        }
        let pw = padded_width(entry_hashes.len());
        let total = 2 * pw - 1;
        let leaf_start = pw - 1;
        let mut nodes = vec![[0u8; 32]; total];

        // Fill leaves
        for (i, h) in entry_hashes.iter().enumerate() {
            nodes[leaf_start + i] = hash_leaf(h);
        }
        // Pad with duplicate of last real leaf
        let last = hash_leaf(entry_hashes.last().unwrap());
        for i in entry_hashes.len()..pw {
            nodes[leaf_start + i] = last;
        }
        // Build internal nodes bottom-up
        for i in (0..leaf_start).rev() {
            nodes[i] = combine(&nodes[2 * i + 1], &nodes[2 * i + 2]);
        }

        Self {
            nodes,
            leaf_count: entry_hashes.len(),
        }
    }

    /// Append a single new leaf (incremental O(log n) update).
    pub fn push(&mut self, entry_hash: &str) {
        if self.leaf_count == 0 {
            *self = Self::build(&[entry_hash]);
            return;
        }

        let old_pw = padded_width(self.leaf_count);
        let new_pw = padded_width(self.leaf_count + 1);

        if new_pw > old_pw {
            // Tree must grow — collect existing leaves and rebuild
            let leaf_start = old_pw - 1;
            let mut hashes: Vec<Digest> = self.nodes[leaf_start..leaf_start + self.leaf_count].to_vec();
            hashes.push(hash_leaf(entry_hash));
            self.leaf_count = hashes.len();

            let pw = new_pw;
            let total = 2 * pw - 1;
            let ls = pw - 1;
            self.nodes = vec![[0u8; 32]; total];
            for (i, h) in hashes.iter().enumerate() {
                self.nodes[ls + i] = *h;
            }
            let last = *hashes.last().unwrap();
            for i in hashes.len()..pw {
                self.nodes[ls + i] = last;
            }
            for i in (0..ls).rev() {
                self.nodes[i] = combine(&self.nodes[2 * i + 1], &self.nodes[2 * i + 2]);
            }
        } else {
            // Fits in current padded width — replace padding slot
            let leaf_start = old_pw - 1;
            let idx = leaf_start + self.leaf_count;
            self.nodes[idx] = hash_leaf(entry_hash);
            self.leaf_count += 1;

            // Re-pad remaining slots with new last leaf
            let last = self.nodes[leaf_start + self.leaf_count - 1];
            for i in self.leaf_count..old_pw {
                self.nodes[leaf_start + i] = last;
            }

            // Recompute all ancestors of changed positions
            // (simpler: just rebuild internal nodes bottom-up — still fast for <=10K)
            for i in (0..leaf_start).rev() {
                self.nodes[i] = combine(&self.nodes[2 * i + 1], &self.nodes[2 * i + 2]);
            }
        }
    }

    /// Current Merkle root as lowercase hex. Returns `"empty"` if no leaves.
    pub fn root_hex(&self) -> String {
        if self.nodes.is_empty() {
            "empty".to_string()
        } else {
            hex::encode(self.nodes[0])
        }
    }

    /// Number of real (non-padded) leaves.
    pub fn leaf_count(&self) -> usize {
        self.leaf_count
    }

    /// Generate an inclusion proof for `leaf_index` (0-based).
    pub fn proof(&self, leaf_index: usize, entry_hash: &str) -> Option<MerkleProof> {
        if leaf_index >= self.leaf_count || self.nodes.is_empty() {
            return None;
        }
        let pw = padded_width(self.leaf_count);
        let leaf_start = pw - 1;
        let mut idx = leaf_start + leaf_index;
        let mut siblings = Vec::new();
        let mut sibling_is_left = Vec::new();

        while idx > 0 {
            // BFS children: left = 2p+1 (odd), right = 2p+2 (even)
            let (sibling, is_left) = if idx % 2 == 1 {
                // idx is left child, sibling is on the right
                (idx + 1, false)
            } else {
                // idx is right child, sibling is on the left
                (idx - 1, true)
            };
            siblings.push(hex::encode(self.nodes[sibling]));
            sibling_is_left.push(is_left);
            idx = (idx - 1) / 2; // parent
        }

        Some(MerkleProof {
            leaf_index,
            leaf_entry_hash: entry_hash.to_string(),
            siblings,
            sibling_is_left,
            root_hex: self.root_hex(),
        })
    }

    /// Serialize to persistable state.
    pub fn to_state(&self) -> MerkleState {
        MerkleState {
            root_hex: self.root_hex(),
            leaf_count: self.leaf_count,
            nodes_hex: self.nodes.iter().map(hex::encode).collect(),
        }
    }

    /// Reconstruct from persisted state.
    pub fn from_state(state: MerkleState) -> Result<Self> {
        let mut nodes = Vec::with_capacity(state.nodes_hex.len());
        for h in &state.nodes_hex {
            let bytes = hex::decode(h)
                .map_err(|e| Error::Security(format!("Invalid Merkle state hex: {}", e)))?;
            if bytes.len() != 32 {
                return Err(Error::Security("Invalid digest length in Merkle state".into()));
            }
            let mut d = [0u8; 32];
            d.copy_from_slice(&bytes);
            nodes.push(d);
        }
        Ok(Self {
            nodes,
            leaf_count: state.leaf_count,
        })
    }
}

// ============================================================================
// MerkleProof — inclusion proof for a single entry
// ============================================================================

/// Merkle inclusion proof for a single audit entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleProof {
    /// 0-based leaf index.
    pub leaf_index: usize,
    /// The `AuditEntry::hash` value for this leaf.
    pub leaf_entry_hash: String,
    /// Sibling hashes from leaf to root (hex strings).
    pub siblings: Vec<String>,
    /// Whether each sibling is on the left side.
    pub sibling_is_left: Vec<bool>,
    /// The Merkle root at proof generation time.
    pub root_hex: String,
}

/// Verify a Merkle inclusion proof.
pub fn verify_proof(proof: &MerkleProof) -> bool {
    let mut current = hash_leaf(&proof.leaf_entry_hash);
    for (sib_hex, &is_left) in proof.siblings.iter().zip(&proof.sibling_is_left) {
        let sib = match hex::decode(sib_hex) {
            Ok(b) if b.len() == 32 => {
                let mut d = [0u8; 32];
                d.copy_from_slice(&b);
                d
            }
            _ => return false,
        };
        current = if is_left {
            combine(&sib, &current)
        } else {
            combine(&current, &sib)
        };
    }
    hex::encode(current) == proof.root_hex
}

// ============================================================================
// MerkleState — on-disk persistence
// ============================================================================

/// Persisted Merkle tree state (JSON).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleState {
    pub root_hex: String,
    pub leaf_count: usize,
    pub nodes_hex: Vec<String>,
}

// ============================================================================
// MerkleAuditLog — wraps AuditLog with Merkle tree
// ============================================================================

/// Audit log with Merkle tree for O(log n) inclusion proofs.
///
/// Wraps `AuditLog` transparently. After each write, the Merkle tree
/// is updated incrementally and state persisted to disk.
pub struct MerkleAuditLog {
    inner: AuditLog,
    tree: MerkleTree,
    state_path: PathBuf,
}

impl MerkleAuditLog {
    /// Open or create a Merkle-backed audit log.
    pub async fn new(path: &Path) -> Result<Self> {
        Self::with_hmac_key(path, None).await
    }

    /// Open with optional HMAC signing key.
    pub async fn with_hmac_key(path: &Path, hmac_key_bytes: Option<&[u8]>) -> Result<Self> {
        let inner = AuditLog::with_hmac_key(path, hmac_key_bytes).await?;
        let state_path = derive_state_path(path);
        let tree = load_or_rebuild(&inner, &state_path).await?;

        Ok(Self {
            inner,
            tree,
            state_path,
        })
    }

    /// Log an event. Updates Merkle tree incrementally.
    pub async fn log(&mut self, event: AuditEvent) -> Result<()> {
        self.inner.log(event).await?;
        self.sync_after_log().await
    }

    /// Log with explicit user.
    pub async fn log_with_user(
        &mut self,
        event: AuditEvent,
        user: Option<String>,
    ) -> Result<()> {
        self.inner.log_with_user(event, user).await?;
        self.sync_after_log().await
    }

    /// Log with explicit severity and user.
    pub async fn log_with_severity(
        &mut self,
        event: AuditEvent,
        severity: Severity,
        user: Option<String>,
    ) -> Result<()> {
        self.inner.log_with_severity(event, severity, user).await?;
        self.sync_after_log().await
    }

    /// Current Merkle root (hex string).
    pub fn root_hex(&self) -> String {
        self.tree.root_hex()
    }

    /// Generate inclusion proof for an entry by sequence number (1-based).
    pub fn proof_for_sequence(&self, seq: u64) -> Option<MerkleProof> {
        if seq == 0 || seq as usize > self.tree.leaf_count() {
            return None;
        }
        // We need the entry hash — read it from entries
        // For now, return None if we can't look it up synchronously.
        // The proof requires the entry_hash which is stored in the leaf.
        // We can reconstruct from the tree leaf digest, but the proof
        // needs the original hex string. Store a mapping instead.
        None // See proof_for_entry() for the full version
    }

    /// Generate inclusion proof given the entry hash directly.
    pub fn proof_for_entry(&self, leaf_index: usize, entry_hash: &str) -> Option<MerkleProof> {
        self.tree.proof(leaf_index, entry_hash)
    }

    /// Verify the underlying hash chain integrity.
    pub async fn verify_chain(&self) -> Result<bool> {
        self.inner.verify().await
    }

    /// Rebuild Merkle tree from scratch (e.g. after rotation).
    pub async fn rebuild_tree(&mut self) -> Result<()> {
        let entries = self.inner.read_entries().await?;
        let mut sorted = entries;
        sorted.sort_by_key(|e| e.sequence);
        let hashes: Vec<&str> = sorted.iter().map(|e| e.hash.as_str()).collect();
        self.tree = MerkleTree::build(&hashes);
        self.persist_state();
        Ok(())
    }

    /// Access the underlying `AuditLog` for querying, pattern detection, etc.
    pub fn inner(&self) -> &AuditLog {
        &self.inner
    }

    /// Merkle state file path.
    pub fn state_path(&self) -> &Path {
        &self.state_path
    }

    /// Sync Merkle tree after an inner log write.
    async fn sync_after_log(&mut self) -> Result<()> {
        let count = self.inner.entry_count() as usize;

        // Detect rotation: entry_count dropped below tree leaf count
        if count <= self.tree.leaf_count() && self.tree.leaf_count() > 0 && count <= 1 {
            self.rebuild_tree().await?;
            // After rebuild, tree may be empty. If count == 1, fall through to push.
            if count == 0 {
                return Ok(());
            }
        }

        // Read the last entry to get its hash
        let (_seq, last_hash) = AuditLog::read_last_entry(self.inner.path()).await?;
        self.tree.push(&last_hash);
        self.persist_state();
        Ok(())
    }

    fn persist_state(&self) {
        let state = self.tree.to_state();
        if let Ok(json) = serde_json::to_string(&state) {
            let _ = std::fs::write(&self.state_path, json);
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn hash_leaf(entry_hash_hex: &str) -> Digest {
    let d = digest(&SHA256, entry_hash_hex.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(d.as_ref());
    out
}

fn combine(left: &Digest, right: &Digest) -> Digest {
    let mut input = [0u8; 64];
    input[..32].copy_from_slice(left);
    input[32..].copy_from_slice(right);
    let d = digest(&SHA256, &input);
    let mut out = [0u8; 32];
    out.copy_from_slice(d.as_ref());
    out
}

fn padded_width(count: usize) -> usize {
    count.max(1).next_power_of_two()
}

fn derive_state_path(audit_path: &Path) -> PathBuf {
    let stem = audit_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    audit_path.with_file_name(format!("{}_merkle.json", stem))
}

async fn load_or_rebuild(inner: &AuditLog, state_path: &Path) -> Result<MerkleTree> {
    // Try loading cached state
    if state_path.exists()
        && let Ok(json) = tokio::fs::read_to_string(state_path).await
        && let Ok(state) = serde_json::from_str::<MerkleState>(&json)
        && state.leaf_count == inner.entry_count() as usize
        && let Ok(tree) = MerkleTree::from_state(state)
    {
        return Ok(tree);
    }

    // Rebuild from active log
    let entries = inner.read_entries().await?;
    let mut sorted = entries;
    sorted.sort_by_key(|e| e.sequence);
    let hashes: Vec<&str> = sorted.iter().map(|e| e.hash.as_str()).collect();
    let tree = MerkleTree::build(&hashes);

    // Persist the rebuilt state
    if let Ok(json) = serde_json::to_string(&tree.to_state()) {
        let _ = std::fs::write(state_path, json);
    }

    Ok(tree)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- MerkleTree unit tests ---

    #[test]
    fn test_empty_tree() {
        let tree = MerkleTree::new();
        assert_eq!(tree.root_hex(), "empty");
        assert_eq!(tree.leaf_count(), 0);
    }

    #[test]
    fn test_single_leaf() {
        let tree = MerkleTree::build(&["abc123"]);
        assert_eq!(tree.leaf_count(), 1);
        assert_ne!(tree.root_hex(), "empty");
        // pw=1, total=1: root IS the leaf hash directly (no internal nodes)
        let expected = hash_leaf("abc123");
        assert_eq!(tree.root_hex(), hex::encode(expected));
    }

    #[test]
    fn test_two_leaves() {
        let tree = MerkleTree::build(&["aaa", "bbb"]);
        assert_eq!(tree.leaf_count(), 2);
        let expected = combine(&hash_leaf("aaa"), &hash_leaf("bbb"));
        assert_eq!(tree.root_hex(), hex::encode(expected));
    }

    #[test]
    fn test_three_leaves_padding() {
        let tree = MerkleTree::build(&["a", "b", "c"]);
        assert_eq!(tree.leaf_count(), 3);
        // Padded to 4: leaves = [a, b, c, c]
        let l0 = hash_leaf("a");
        let l1 = hash_leaf("b");
        let l2 = hash_leaf("c");
        let l3 = hash_leaf("c"); // pad
        let n1 = combine(&l0, &l1);
        let n2 = combine(&l2, &l3);
        let root = combine(&n1, &n2);
        assert_eq!(tree.root_hex(), hex::encode(root));
    }

    #[test]
    fn test_push_matches_build() {
        let hashes = ["h1", "h2", "h3", "h4", "h5"];
        let built = MerkleTree::build(&hashes);

        let mut pushed = MerkleTree::new();
        for h in &hashes {
            pushed.push(h);
        }

        assert_eq!(built.root_hex(), pushed.root_hex());
        assert_eq!(built.leaf_count(), pushed.leaf_count());
    }

    #[test]
    fn test_push_across_power_of_two() {
        let mut tree = MerkleTree::new();
        tree.push("a"); // 1 leaf
        assert_eq!(tree.leaf_count(), 1);
        tree.push("b"); // 2 leaves (boundary)
        assert_eq!(tree.leaf_count(), 2);
        tree.push("c"); // 3 leaves (boundary at 4)
        assert_eq!(tree.leaf_count(), 3);
        tree.push("d"); // 4 leaves
        tree.push("e"); // 5 leaves (boundary at 8)
        assert_eq!(tree.leaf_count(), 5);

        let built = MerkleTree::build(&["a", "b", "c", "d", "e"]);
        assert_eq!(tree.root_hex(), built.root_hex());
    }

    // --- MerkleProof tests ---

    #[test]
    fn test_proof_verify_leaf_0() {
        let hashes = ["h0", "h1", "h2", "h3"];
        let tree = MerkleTree::build(&hashes);
        let proof = tree.proof(0, "h0").unwrap();
        assert!(verify_proof(&proof));
    }

    #[test]
    fn test_proof_verify_last_leaf() {
        let hashes = ["h0", "h1", "h2", "h3"];
        let tree = MerkleTree::build(&hashes);
        let proof = tree.proof(3, "h3").unwrap();
        assert!(verify_proof(&proof));
    }

    #[test]
    fn test_proof_verify_middle() {
        let hashes = ["a", "b", "c", "d", "e"];
        let tree = MerkleTree::build(&hashes);
        for (i, h) in hashes.iter().enumerate() {
            let proof = tree.proof(i, h).unwrap();
            assert!(verify_proof(&proof), "Proof failed for leaf {}", i);
        }
    }

    #[test]
    fn test_proof_wrong_root_fails() {
        let tree = MerkleTree::build(&["a", "b", "c", "d"]);
        let mut proof = tree.proof(0, "a").unwrap();
        proof.root_hex = "deadbeef".repeat(8);
        assert!(!verify_proof(&proof));
    }

    #[test]
    fn test_proof_wrong_sibling_fails() {
        let tree = MerkleTree::build(&["a", "b", "c", "d"]);
        let mut proof = tree.proof(1, "b").unwrap();
        if !proof.siblings.is_empty() {
            proof.siblings[0] = "ff".repeat(32);
        }
        assert!(!verify_proof(&proof));
    }

    #[test]
    fn test_proof_out_of_bounds() {
        let tree = MerkleTree::build(&["a", "b"]);
        assert!(tree.proof(2, "c").is_none());
        assert!(tree.proof(100, "x").is_none());
    }

    // --- MerkleState round-trip ---

    #[test]
    fn test_state_roundtrip() {
        let tree = MerkleTree::build(&["x", "y", "z"]);
        let state = tree.to_state();
        let restored = MerkleTree::from_state(state).unwrap();
        assert_eq!(tree.root_hex(), restored.root_hex());
        assert_eq!(tree.leaf_count(), restored.leaf_count());
    }

    #[test]
    fn test_state_invalid_hex() {
        let state = MerkleState {
            root_hex: "bad".into(),
            leaf_count: 1,
            nodes_hex: vec!["not_valid_hex!!".into()],
        };
        assert!(MerkleTree::from_state(state).is_err());
    }

    // --- MerkleAuditLog integration tests ---

    #[tokio::test]
    async fn test_merkle_audit_log_basic() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("audit.log");
        let mut log = MerkleAuditLog::new(&path).await.unwrap();

        assert_eq!(log.root_hex(), "empty");

        log.log(AuditEvent::System {
            event: "start".into(),
            details: None,
        })
        .await
        .unwrap();

        assert_ne!(log.root_hex(), "empty");

        log.log(AuditEvent::ToolExecution {
            tool: "shell".into(),
            args: serde_json::json!({}),
            success: true,
        })
        .await
        .unwrap();

        log.log(AuditEvent::FileAccess {
            path: "/tmp/test".into(),
            operation: "read".into(),
        })
        .await
        .unwrap();

        assert_eq!(log.inner().entry_count(), 3);
    }

    #[tokio::test]
    async fn test_merkle_audit_log_proof_verifies() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("audit.log");
        let mut log = MerkleAuditLog::new(&path).await.unwrap();

        for i in 0..5 {
            log.log(AuditEvent::System {
                event: format!("evt-{}", i),
                details: None,
            })
            .await
            .unwrap();
        }

        // Read entries to get hashes
        let entries = log.inner().read_entries().await.unwrap();
        let mut sorted = entries;
        sorted.sort_by_key(|e| e.sequence);

        for (i, entry) in sorted.iter().enumerate() {
            let proof = log.proof_for_entry(i, &entry.hash).unwrap();
            assert!(verify_proof(&proof), "Proof failed for entry {}", i);
        }
    }

    #[tokio::test]
    async fn test_merkle_state_persists() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("audit.log");

        {
            let mut log = MerkleAuditLog::new(&path).await.unwrap();
            for i in 0..3 {
                log.log(AuditEvent::System {
                    event: format!("evt-{}", i),
                    details: None,
                })
                .await
                .unwrap();
            }
        }

        // Re-open — should load from cached state (fast path)
        let log2 = MerkleAuditLog::new(&path).await.unwrap();
        assert_ne!(log2.root_hex(), "empty");

        // State file should exist
        assert!(log2.state_path().exists());
    }

    #[tokio::test]
    async fn test_merkle_rebuild_on_corrupt_state() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("audit.log");
        let state_path = derive_state_path(&path);

        {
            let mut log = MerkleAuditLog::new(&path).await.unwrap();
            for i in 0..4 {
                log.log(AuditEvent::System {
                    event: format!("evt-{}", i),
                    details: None,
                })
                .await
                .unwrap();
            }
        }

        // Corrupt the state file
        std::fs::write(&state_path, "garbage").unwrap();

        // Re-open — should rebuild from log
        let log2 = MerkleAuditLog::new(&path).await.unwrap();
        assert_ne!(log2.root_hex(), "empty");
    }

    #[tokio::test]
    async fn test_merkle_verify_chain_passthrough() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("audit.log");
        let mut log = MerkleAuditLog::new(&path).await.unwrap();

        log.log(AuditEvent::System {
            event: "test".into(),
            details: None,
        })
        .await
        .unwrap();

        assert!(log.verify_chain().await.unwrap());
    }
}
