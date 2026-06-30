//! Channel Session Router — deterministic session key derivation for relay channels.
//!
//! # Problem
//!
//! Multiple chats on the same platform (e.g. two Discord channels talking to
//! the same bot) should not share a single Zeus session. Before this module
//! existed, most relays routed all inbound traffic to a single "default"
//! session per agent, causing cross-contamination of conversation history
//! and context window pollution.
//!
//! Telegram solved this in `zeus-channels::telegram_relay::SessionRouter` with
//! in-memory per-chat `zeus_session_id` tracking. This module generalizes that
//! approach with **deterministic** keys that survive gateway restarts — no
//! persistence layer needed.
//!
//! # Key Scheme
//!
//! Legacy channel session IDs are derived deterministically from channel + chat
//! identifiers:
//!
//! ```text
//! agent:{channel_type}:{chat_id}
//! ```
//!
//! Unified Titan context uses the Titan identity instead:
//!
//! ```text
//! titan:{identity}
//! ```
//!
//! Examples:
//! - `agent:discord:1488620262676238426` — Discord channel
//! - `agent:slack:C0123456789`           — Slack channel
//! - `agent:telegram:-1001234567890`     — Telegram group
//! - `agent:telegram:dm-123:456`         — Telegram DM (chat_id:user_id)
//! - `titan:zeus-titan`                  — same Titan across all surfaces
//!
//! The resulting session files live at `~/.zeus/sessions/agent:discord:<id>.jsonl`
//! and are picked up by `Session::resume_or_create()` on demand.
//!
//! # Usage
//!
//! ```rust,no_run
//! # use zeus_session::channel_router::{ChannelKey, derive_session_id};
//! let key = ChannelKey::new("discord", "1488620262676238426");
//! let session_id = derive_session_id(&key);
//! assert_eq!(session_id, "agent:discord:1488620262676238426");
//! ```
//!
//! Downstream code passes `session_id` directly to
//! `Session::resume_or_create(&sessions_dir, &session_id)`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// ============================================================================
// ChannelKey — normalized identifier for a channel+chat pair
// ============================================================================

/// A routing key that identifies a unique conversation across channels.
///
/// `channel_type` is the platform (`discord`, `slack`, `telegram`, …).
/// `chat_id` is the platform-specific chat/channel/room identifier.
/// `user_id` is only relevant for DMs on platforms where a single chat_id
/// handles multiple users (e.g. Telegram DMs use `{chat_id}:{user_id}`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChannelKey {
    pub channel_type: String,
    pub chat_id: String,
    /// Optional user_id discriminator — only used when the platform's
    /// chat_id alone isn't enough to disambiguate conversations.
    pub user_id: Option<String>,
}

/// Context scope for session routing.
///
/// `Channel` preserves legacy per-surface behavior (`agent:{surface}:{chat}`).
/// `Titan` is the unified cross-channel scope: every surface for the same Titan
/// resolves to the same deterministic session ID (`titan:{identity}`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ContextScope {
    Channel(ChannelKey),
    Titan { identity: String },
}

impl ContextScope {
    pub fn channel(key: ChannelKey) -> Self {
        Self::Channel(key)
    }

    pub fn titan(identity: impl Into<String>) -> Self {
        Self::Titan {
            identity: identity.into(),
        }
    }
}

impl ChannelKey {
    /// Construct a key from channel_type + chat_id. Most platforms use this.
    pub fn new(channel_type: impl Into<String>, chat_id: impl Into<String>) -> Self {
        Self {
            channel_type: channel_type.into(),
            chat_id: chat_id.into(),
            user_id: None,
        }
    }

    /// Construct a DM key where chat_id alone doesn't discriminate users.
    /// Used for Telegram DMs and similar platforms.
    pub fn dm(
        channel_type: impl Into<String>,
        chat_id: impl Into<String>,
        user_id: impl Into<String>,
    ) -> Self {
        Self {
            channel_type: channel_type.into(),
            chat_id: chat_id.into(),
            user_id: Some(user_id.into()),
        }
    }
}

// ============================================================================
// derive_session_id — pure function, deterministic from ChannelKey
// ============================================================================

/// Derive a deterministic session ID from a `ChannelKey`.
///
/// Scheme: `agent:{channel_type}:{chat_id}` — with `:{user_id}` suffix when
/// user_id is present (for DM platforms that need it).
///
/// This is a pure function. Calling it twice with the same input always
/// returns the same output, which is what makes channel sessions survive
/// restarts without any persistence layer.
pub fn derive_session_id(key: &ChannelKey) -> String {
    match &key.user_id {
        Some(uid) => format!("agent:{}:{}:{}", key.channel_type, key.chat_id, uid),
        None => format!("agent:{}:{}", key.channel_type, key.chat_id),
    }
}

/// Derive a deterministic session ID from a context scope.
pub fn derive_context_session_id(scope: &ContextScope) -> String {
    match scope {
        ContextScope::Channel(key) => derive_session_id(key),
        ContextScope::Titan { identity } => derive_titan_session_id(identity),
    }
}

/// Derive a deterministic unified session ID for a Titan identity.
///
/// The returned ID intentionally ignores channel/chat/thread surface details so
/// all channel traffic for the same Titan assembles history from one session.
pub fn derive_titan_session_id(identity: impl AsRef<str>) -> String {
    format!("titan:{}", sanitize_identity(identity.as_ref()))
}

fn sanitize_identity(identity: &str) -> String {
    let mut out = String::with_capacity(identity.len());
    let mut last_dash = false;
    for ch in identity.trim().chars().flat_map(char::to_lowercase) {
        let safe = ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-');
        if safe {
            out.push(ch);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "main".to_string()
    } else {
        out
    }
}

// ============================================================================
// ChannelSessionRouter — in-memory cache over derive_session_id
// ============================================================================

/// Routes channel keys to Zeus session IDs.
///
/// This is a thin wrapper around `derive_session_id` that also tracks
/// recently-seen channels so relays can query "how many active chats do
/// I have?" without hitting the filesystem.
///
/// The mapping itself is deterministic, so the cache is purely a performance
/// and observability aid — if the process restarts, the same channel will
/// resolve to the same session ID on first call.
pub struct ChannelSessionRouter {
    /// Cached mappings. Populated on first `resolve()` call per key.
    cache: RwLock<HashMap<ChannelKey, String>>,
}

impl ChannelSessionRouter {
    pub fn new() -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Resolve a channel key to a session ID.
    ///
    /// Deterministic: same input → same output, regardless of cache state.
    /// Also records the key in the cache for `known_channels()` queries.
    pub async fn resolve(&self, key: &ChannelKey) -> String {
        // Fast path: cached
        if let Some(sid) = self.cache.read().await.get(key) {
            return sid.clone();
        }

        // Slow path: derive and cache
        let sid = derive_session_id(key);
        self.cache.write().await.insert(key.clone(), sid.clone());
        sid
    }

    /// Resolve a context scope to a session ID.
    ///
    /// Channel scopes are cached for `known_channels()` observability. Titan
    /// scopes are pure identity derivations and are not recorded as channel activity.
    pub async fn resolve_context(&self, scope: &ContextScope) -> String {
        match scope {
            ContextScope::Channel(key) => self.resolve(key).await,
            ContextScope::Titan { identity } => derive_titan_session_id(identity),
        }
    }

    /// Return the set of channels this router has resolved since startup.
    /// Useful for observability / `/status` endpoints.
    pub async fn known_channels(&self) -> Vec<ChannelKey> {
        self.cache.read().await.keys().cloned().collect()
    }

    /// Number of distinct channels this router has resolved.
    pub async fn len(&self) -> usize {
        self.cache.read().await.len()
    }

    /// True if the router has not yet resolved any channels.
    pub async fn is_empty(&self) -> bool {
        self.cache.read().await.is_empty()
    }

    /// Convenience: wrap in an `Arc` for sharing across relay tasks.
    pub fn shared() -> Arc<Self> {
        Arc::new(Self::new())
    }
}

impl Default for ChannelSessionRouter {
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

    #[test]
    fn test_derive_basic_channel() {
        let key = ChannelKey::new("discord", "1488620262676238426");
        assert_eq!(derive_session_id(&key), "agent:discord:1488620262676238426");
    }

    #[test]
    fn test_derive_slack_channel() {
        let key = ChannelKey::new("slack", "C0123456789");
        assert_eq!(derive_session_id(&key), "agent:slack:C0123456789");
    }

    #[test]
    fn test_derive_telegram_group() {
        let key = ChannelKey::new("telegram", "-1001234567890");
        assert_eq!(derive_session_id(&key), "agent:telegram:-1001234567890");
    }

    #[test]
    fn test_derive_dm_includes_user() {
        let key = ChannelKey::dm("telegram", "12345", "67890");
        assert_eq!(derive_session_id(&key), "agent:telegram:12345:67890");
    }

    #[test]
    fn test_derive_titan_identity() {
        assert_eq!(derive_titan_session_id("zeus-titan"), "titan:zeus-titan");
        assert_eq!(derive_titan_session_id("Zeus Titan"), "titan:zeus-titan");
        assert_eq!(
            derive_titan_session_id("/bad\\identity\0"),
            "titan:bad-identity"
        );
        assert_eq!(derive_titan_session_id("   "), "titan:main");
    }

    #[test]
    fn test_context_scope_channel_preserves_legacy_derivation() {
        let key = ChannelKey::new("discord", "1488620262676238426");
        let scope = ContextScope::channel(key.clone());
        assert_eq!(derive_context_session_id(&scope), derive_session_id(&key));
        assert_eq!(
            derive_context_session_id(&scope),
            "agent:discord:1488620262676238426"
        );
    }

    #[test]
    fn test_context_scope_titan_ignores_surface() {
        let discord = ContextScope::titan("zeus-titan");
        let slack = ContextScope::titan("zeus-titan");
        assert_eq!(derive_context_session_id(&discord), "titan:zeus-titan");
        assert_eq!(
            derive_context_session_id(&discord),
            derive_context_session_id(&slack)
        );
    }

    #[test]
    fn test_distinct_titans_do_not_collide() {
        assert_ne!(
            derive_titan_session_id("zeus-titan"),
            derive_titan_session_id("zeus100")
        );
    }

    #[test]
    fn test_derive_is_deterministic() {
        // The whole point — same inputs must always produce the same output.
        let k1 = ChannelKey::new("discord", "999");
        let k2 = ChannelKey::new("discord", "999");
        assert_eq!(derive_session_id(&k1), derive_session_id(&k2));
    }

    #[test]
    fn test_different_channels_different_sessions() {
        let k1 = ChannelKey::new("discord", "aaa");
        let k2 = ChannelKey::new("discord", "bbb");
        assert_ne!(derive_session_id(&k1), derive_session_id(&k2));
    }

    #[test]
    fn test_different_platforms_different_sessions() {
        let k1 = ChannelKey::new("discord", "12345");
        let k2 = ChannelKey::new("slack", "12345");
        assert_ne!(derive_session_id(&k1), derive_session_id(&k2));
    }

    #[test]
    fn test_dm_vs_group_different_sessions() {
        let group = ChannelKey::new("telegram", "-100123");
        let dm = ChannelKey::dm("telegram", "12345", "67890");
        assert_ne!(derive_session_id(&group), derive_session_id(&dm));
    }

    #[tokio::test]
    async fn test_router_resolve_caches() {
        let router = ChannelSessionRouter::new();
        let key = ChannelKey::new("discord", "42");
        assert!(router.is_empty().await);

        let sid1 = router.resolve(&key).await;
        assert_eq!(sid1, "agent:discord:42");
        assert_eq!(router.len().await, 1);

        // Second resolve hits cache, same result
        let sid2 = router.resolve(&key).await;
        assert_eq!(sid1, sid2);
        assert_eq!(router.len().await, 1);
    }

    #[tokio::test]
    async fn test_router_resolve_multiple_channels() {
        let router = ChannelSessionRouter::new();
        let k1 = ChannelKey::new("discord", "1");
        let k2 = ChannelKey::new("discord", "2");
        let k3 = ChannelKey::new("slack", "1");

        router.resolve(&k1).await;
        router.resolve(&k2).await;
        router.resolve(&k3).await;

        assert_eq!(router.len().await, 3);
        let known = router.known_channels().await;
        assert!(known.contains(&k1));
        assert!(known.contains(&k2));
        assert!(known.contains(&k3));
    }

    #[tokio::test]
    async fn test_router_survives_restart_semantically() {
        // Simulate a restart by creating a fresh router and resolving
        // the same key — we should get the same session ID back, because
        // the derivation is deterministic.
        let key = ChannelKey::new("discord", "persistent");

        let r1 = ChannelSessionRouter::new();
        let sid1 = r1.resolve(&key).await;
        drop(r1);

        let r2 = ChannelSessionRouter::new();
        let sid2 = r2.resolve(&key).await;

        assert_eq!(sid1, sid2, "session ID must survive router restart");
    }

    #[test]
    fn test_session_id_format_is_filesystem_safe() {
        // Session IDs become filenames — make sure the scheme doesn't produce
        // anything nasty (path separators, null bytes, etc.) for common inputs.
        let ids = [
            derive_session_id(&ChannelKey::new("discord", "1488620262676238426")),
            derive_session_id(&ChannelKey::new("slack", "C0123456789")),
            derive_session_id(&ChannelKey::new("telegram", "-1001234567890")),
            derive_session_id(&ChannelKey::dm("telegram", "123", "456")),
        ];
        for id in ids {
            assert!(!id.contains('/'), "session id must not contain /: {}", id);
            assert!(!id.contains('\\'), "session id must not contain \\: {}", id);
            assert!(
                !id.contains('\0'),
                "session id must not contain NUL: {}",
                id
            );
            assert!(id.starts_with("agent:"), "must use agent: prefix: {}", id);
        }
    }

    #[tokio::test]
    async fn test_router_resolve_context_titan_is_unified() {
        let router = ChannelSessionRouter::new();
        let sid_a = router
            .resolve_context(&ContextScope::titan("zeus-titan"))
            .await;
        let sid_b = router
            .resolve_context(&ContextScope::titan("Zeus Titan"))
            .await;
        assert_eq!(sid_a, "titan:zeus-titan");
        assert_eq!(sid_a, sid_b);
        assert!(
            router.is_empty().await,
            "titan scope must not pollute channel observability"
        );
    }

    /// Per-channel sessions Phase 2 verification: prove that two distinct
    /// Discord channels route to two distinct on-disk session files, and
    /// that writes to one do not contaminate the other.
    #[tokio::test]
    async fn test_per_channel_sessions_isolated_on_disk() {
        use std::fs;

        let tmp = tempfile::tempdir().expect("tempdir");
        let sessions_dir = tmp.path();

        let router = ChannelSessionRouter::new();
        let ch_a = ChannelKey::new("discord", "1111111111");
        let ch_b = ChannelKey::new("discord", "2222222222");

        let sid_a = router.resolve(&ch_a).await;
        let sid_b = router.resolve(&ch_b).await;

        // Distinct channels → distinct session IDs.
        assert_ne!(
            sid_a, sid_b,
            "different channels must produce different session IDs"
        );

        // Simulate how the agent loop persists session transcripts: one file
        // per session_id in the sessions dir.
        let path_a = sessions_dir.join(format!("{}.jsonl", sid_a));
        let path_b = sessions_dir.join(format!("{}.jsonl", sid_b));
        fs::write(&path_a, "msg-from-channel-a\n").expect("write a");
        fs::write(&path_b, "msg-from-channel-b\n").expect("write b");

        // Both files exist, at distinct paths.
        assert!(
            path_a.exists() && path_b.exists(),
            "both session files must exist"
        );
        assert_ne!(path_a, path_b, "session file paths must differ");

        // No cross-contamination: channel A's file contains only its own data.
        let a = fs::read_to_string(&path_a).expect("read a");
        let b = fs::read_to_string(&path_b).expect("read b");
        assert!(a.contains("channel-a") && !a.contains("channel-b"));
        assert!(b.contains("channel-b") && !b.contains("channel-a"));

        // Restart semantics: a fresh router resolves the same keys to the same
        // files on disk — sessions survive process restarts.
        drop(router);
        let router2 = ChannelSessionRouter::new();
        assert_eq!(router2.resolve(&ch_a).await, sid_a);
        assert_eq!(router2.resolve(&ch_b).await, sid_b);
    }
}
