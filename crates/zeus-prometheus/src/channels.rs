//! Channel-kind taxonomy for fleet session resolution.
//!
//! This module defines [`ChannelKind`], the canonical enumeration of ingress
//! surfaces used by the fleet session resolver to group `(agent, human,
//! channel_kind)` tuples into coherent fleet sessions.
//!
//! # Design notes
//!
//! - **Surface vs modality.** Variants represent ingress *surfaces*, not
//!   modalities. Voice, text, media, and history are modalities of an
//!   underlying surface; they share the same fleet session because they share
//!   the same `(agent, human, channel_kind)` grouping key. This is why
//!   `Telegram` covers both `telegram_bot.rs` and `telegram_voice.rs`,
//!   `Slack` covers `slack_relay.rs` and `slack_history.rs`, and `Discord`
//!   covers both text and voice surfaces. Modality differentiation happens
//!   *downstream* of session resolution, not upstream of it.
//!
//! - **`#[non_exhaustive]` + `Other(String)`.** New ingress surfaces can be
//!   added without breaking match-coverage at downstream call sites; unknown
//!   surfaces (e.g. third-party plugins, future channels not yet in the
//!   workspace) round-trip through `Other` rather than being lossy-coerced.
//!
//! - **Canonical lowercase strings.** [`Display`] emits a flat lowercase
//!   string per filesystem convention (matching the file naming under
//!   `crates/zeus-channels/src/`). [`FromStr`] is infallible and lowercases
//!   its input before matching, falling back to `Other(canonical_lowercased)`
//!   for unknown surfaces. This guarantees `FromStr ↔ Display` round-trips
//!   for every value.
//!
//! - **Hash + Eq derives.** Required because the resolver groups sessions by
//!   channel_kind in hash-keyed storage per PRD §3.2.
//!
//! # Upstream artifacts
//!
//! - PRD §3.2 — `docs/sprints/backlog-prepare-cook-context-extraction-2026-05-06.md`
//! - Sprint plan `f9cabf52` — Lane 3 spec
//! - Signature lock `8414027e` — Lane 3a commit (a)

use std::convert::Infallible;
use std::fmt;
use std::str::FromStr;

/// Canonical enumeration of fleet ingress surfaces.
///
/// See module-level docs for the surface-vs-modality principle and the
/// `#[non_exhaustive]` + `Other(String)` escape hatch contract.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ChannelKind {
    BlueBubbles,
    Discord,
    Email,
    Feishu,
    GoogleChat,
    IMessage,
    Instagram,
    Irc,
    Line,
    Matrix,
    Mattermost,
    McpBridge,
    Mqtt,
    Nextcloud,
    Nostr,
    Signal,
    Slack,
    Sms,
    Teams,
    Telegram,
    Tui,
    Twilio,
    Twitch,
    Webchat,
    WhatsApp,
    X,
    Zalo,
    /// Escape hatch for unknown / future ingress surfaces. Always carries
    /// the canonical-lowercased form of the input.
    Other(String),
}

impl ChannelKind {
    /// Returns the canonical lowercase string for this variant.
    ///
    /// Equivalent to `format!("{}", self)`, but allocation-free for the
    /// non-`Other` variants.
    pub fn as_canonical(&self) -> &str {
        match self {
            ChannelKind::BlueBubbles => "bluebubbles",
            ChannelKind::Discord => "discord",
            ChannelKind::Email => "email",
            ChannelKind::Feishu => "feishu",
            ChannelKind::GoogleChat => "googlechat",
            ChannelKind::IMessage => "imessage",
            ChannelKind::Instagram => "instagram",
            ChannelKind::Irc => "irc",
            ChannelKind::Line => "line",
            ChannelKind::Matrix => "matrix",
            ChannelKind::Mattermost => "mattermost",
            ChannelKind::McpBridge => "mcpbridge",
            ChannelKind::Mqtt => "mqtt",
            ChannelKind::Nextcloud => "nextcloud",
            ChannelKind::Nostr => "nostr",
            ChannelKind::Signal => "signal",
            ChannelKind::Slack => "slack",
            ChannelKind::Sms => "sms",
            ChannelKind::Teams => "teams",
            ChannelKind::Telegram => "telegram",
            ChannelKind::Tui => "tui",
            ChannelKind::Twilio => "twilio",
            ChannelKind::Twitch => "twitch",
            ChannelKind::Webchat => "webchat",
            ChannelKind::WhatsApp => "whatsapp",
            ChannelKind::X => "x",
            ChannelKind::Zalo => "zalo",
            ChannelKind::Other(s) => s.as_str(),
        }
    }
}

impl fmt::Display for ChannelKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_canonical())
    }
}

impl FromStr for ChannelKind {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lowered = s.trim().to_ascii_lowercase();
        Ok(match lowered.as_str() {
            "bluebubbles" => ChannelKind::BlueBubbles,
            "discord" => ChannelKind::Discord,
            "email" => ChannelKind::Email,
            "feishu" => ChannelKind::Feishu,
            "googlechat" => ChannelKind::GoogleChat,
            "imessage" => ChannelKind::IMessage,
            "instagram" => ChannelKind::Instagram,
            "irc" => ChannelKind::Irc,
            "line" => ChannelKind::Line,
            "matrix" => ChannelKind::Matrix,
            "mattermost" => ChannelKind::Mattermost,
            "mcpbridge" => ChannelKind::McpBridge,
            "mqtt" => ChannelKind::Mqtt,
            "nextcloud" => ChannelKind::Nextcloud,
            "nostr" => ChannelKind::Nostr,
            "signal" => ChannelKind::Signal,
            "slack" => ChannelKind::Slack,
            "sms" => ChannelKind::Sms,
            "teams" => ChannelKind::Teams,
            "telegram" => ChannelKind::Telegram,
            "tui" => ChannelKind::Tui,
            "twilio" => ChannelKind::Twilio,
            "twitch" => ChannelKind::Twitch,
            "webchat" => ChannelKind::Webchat,
            "whatsapp" => ChannelKind::WhatsApp,
            "x" => ChannelKind::X,
            "zalo" => ChannelKind::Zalo,
            _ => ChannelKind::Other(lowered),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All canonical (non-`Other`) variants in declaration order.
    fn all_canonical_variants() -> Vec<ChannelKind> {
        vec![
            ChannelKind::BlueBubbles,
            ChannelKind::Discord,
            ChannelKind::Email,
            ChannelKind::Feishu,
            ChannelKind::GoogleChat,
            ChannelKind::IMessage,
            ChannelKind::Instagram,
            ChannelKind::Irc,
            ChannelKind::Line,
            ChannelKind::Matrix,
            ChannelKind::Mattermost,
            ChannelKind::McpBridge,
            ChannelKind::Mqtt,
            ChannelKind::Nextcloud,
            ChannelKind::Nostr,
            ChannelKind::Signal,
            ChannelKind::Slack,
            ChannelKind::Sms,
            ChannelKind::Teams,
            ChannelKind::Telegram,
            ChannelKind::Tui,
            ChannelKind::Twilio,
            ChannelKind::Twitch,
            ChannelKind::Webchat,
            ChannelKind::WhatsApp,
            ChannelKind::X,
            ChannelKind::Zalo,
        ]
    }

    #[test]
    fn variant_count_locked_at_27() {
        // Guard against silent variant-set drift. Adding/removing a variant
        // should be a deliberate decision tracked in the PRD, not an
        // accidental edit.
        assert_eq!(all_canonical_variants().len(), 27);
    }

    #[test]
    fn display_round_trips_through_from_str_for_every_variant() {
        for variant in all_canonical_variants() {
            let s = variant.to_string();
            let parsed = ChannelKind::from_str(&s).expect("FromStr is Infallible");
            assert_eq!(parsed, variant, "round-trip failed for {variant:?}");
        }
    }

    #[test]
    fn from_str_canonicalizes_mixed_case() {
        assert_eq!(
            ChannelKind::from_str("Discord").unwrap(),
            ChannelKind::Discord
        );
        assert_eq!(
            ChannelKind::from_str("DISCORD").unwrap(),
            ChannelKind::Discord
        );
        assert_eq!(
            ChannelKind::from_str("WhatsApp").unwrap(),
            ChannelKind::WhatsApp
        );
        assert_eq!(
            ChannelKind::from_str("BlueBubbles").unwrap(),
            ChannelKind::BlueBubbles
        );
    }

    #[test]
    fn from_str_trims_whitespace() {
        assert_eq!(
            ChannelKind::from_str("  discord  ").unwrap(),
            ChannelKind::Discord
        );
        assert_eq!(
            ChannelKind::from_str("\tslack\n").unwrap(),
            ChannelKind::Slack
        );
    }

    #[test]
    fn unknown_surface_falls_back_to_other_with_canonical_form() {
        // Unknown surface preserves canonical-lowercased form, NOT raw input.
        let parsed = ChannelKind::from_str("FutureChannelXYZ").unwrap();
        assert_eq!(parsed, ChannelKind::Other("futurechannelxyz".to_string()));
    }

    #[test]
    fn other_round_trips_through_display_and_from_str() {
        let original = ChannelKind::Other("plugin_foo".to_string());
        let s = original.to_string();
        assert_eq!(s, "plugin_foo");
        let parsed = ChannelKind::from_str(&s).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn other_lowercases_on_construction_via_from_str() {
        let parsed = ChannelKind::from_str("PluginFoo").unwrap();
        assert_eq!(parsed, ChannelKind::Other("pluginfoo".to_string()));
    }

    #[test]
    fn equal_variants_have_equal_hashes() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        fn hash_of(k: &ChannelKind) -> u64 {
            let mut h = DefaultHasher::new();
            k.hash(&mut h);
            h.finish()
        }

        // Hash + Eq consistency: equal values must hash equal.
        // (Required for use in HashMap keys per PRD §3.2.)
        assert_eq!(hash_of(&ChannelKind::Discord), hash_of(&ChannelKind::Discord));
        assert_eq!(
            hash_of(&ChannelKind::Other("foo".to_string())),
            hash_of(&ChannelKind::Other("foo".to_string()))
        );
    }

    #[test]
    fn as_canonical_matches_display() {
        for variant in all_canonical_variants() {
            assert_eq!(variant.as_canonical(), variant.to_string());
        }
        let other = ChannelKind::Other("xyz".to_string());
        assert_eq!(other.as_canonical(), other.to_string());
    }
}
