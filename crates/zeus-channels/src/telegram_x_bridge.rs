//! Telegram ↔ X bridge (T3 — social automation sprint)
//!
//! Two-way glue between the Telegram relay and the X adapter:
//!
//! 1. **Telegram → X**: intercept `/tweet <text>` (or `tweet this: <text>`)
//!    chat messages in the configured group, post via [`XAdapter`], then
//!    send a confirmation back to the same Telegram chat.
//! 2. **X → Telegram**: forward a just-posted tweet to the configured
//!    Telegram group so humans see what the social agent just shipped.
//!
//! Intentionally small (~30 LOC of logic, rest is plumbing + tests). The
//! bridge holds `Arc`-shared adapters; wire it once at gateway boot.

use std::sync::Arc;

use zeus_core::Result;

use crate::telegram::TelegramAdapter;
use crate::x::{CreateTweetOptions, Tweet, XAdapter};
use crate::{ChannelAdapter, ChannelSource};

/// Telegram ↔ X bridge.
///
/// Construct with [`TelegramXBridge::new`], then call the two relay
/// methods from the social agent's message handler / post hook.
#[derive(Clone)]
pub struct TelegramXBridge {
    telegram: Arc<TelegramAdapter>,
    x: Arc<XAdapter>,
    /// Telegram chat ID that mirrors the social agent (group or DM).
    mirror_chat_id: String,
}

impl TelegramXBridge {
    pub fn new(
        telegram: Arc<TelegramAdapter>,
        x: Arc<XAdapter>,
        mirror_chat_id: impl Into<String>,
    ) -> Self {
        Self {
            telegram,
            x,
            mirror_chat_id: mirror_chat_id.into(),
        }
    }

    /// Parse a Telegram chat message for a "tweet this" command.
    ///
    /// Returns `Some(text)` if the message is a tweet-relay request,
    /// otherwise `None`. Accepts:
    ///   - `/tweet <text>`
    ///   - `tweet this: <text>`   (case-insensitive, leading whitespace OK)
    pub fn parse_tweet_command(raw: &str) -> Option<String> {
        let trimmed = raw.trim();
        if let Some(rest) = trimmed.strip_prefix("/tweet") {
            let body = rest.trim();
            return (!body.is_empty()).then(|| body.to_string());
        }
        let lower = trimmed.to_lowercase();
        if let Some(idx) = lower.find("tweet this:") {
            let body = trimmed[idx + "tweet this:".len()..].trim();
            return (!body.is_empty()).then(|| body.to_string());
        }
        None
    }

    /// Telegram → X: post `text` as a tweet, then reply to the Telegram
    /// chat with a confirmation link. Returns the posted tweet.
    pub async fn relay_tweet_command(&self, text: &str) -> Result<Tweet> {
        let opts = CreateTweetOptions {
            text: text.to_string(),
            reply_to: None,
            quote_tweet_id: None,
            media_ids: Vec::new(),
            poll_options: Vec::new(),
            poll_duration_minutes: None,
            scheduled_at: None,
        };
        let tweet = self.x.post_tweet(&opts).await?;

        let confirm = format!(
            "✅ tweeted: {}\n{}",
            tweet.text,
            format_tweet_url(&tweet)
        );
        // Best-effort confirmation — don't fail the whole relay if TG send flakes.
        if let Err(e) = self.send_tg(&confirm).await {
            tracing::warn!("telegram_x_bridge: confirm send failed: {e:#}");
        }
        Ok(tweet)
    }

    /// X → Telegram: forward a posted tweet to the mirror chat.
    pub async fn forward_posted_tweet(&self, tweet: &Tweet) -> Result<()> {
        let msg = format!(
            "🐦 new tweet\n{}\n{}",
            tweet.text,
            format_tweet_url(tweet)
        );
        self.send_tg(&msg).await
    }

    async fn send_tg(&self, text: &str) -> Result<()> {
        // Use the standard ChannelAdapter trait; user_id is symbolic
        // ("zeus") since Telegram routes by chat_id.
        let target = ChannelSource::with_chat("telegram", "zeus", &self.mirror_chat_id);
        self.telegram.send(&target, text).await
    }
}

fn format_tweet_url(tweet: &Tweet) -> String {
    match &tweet.author_username {
        Some(u) => format!("https://x.com/{u}/status/{}", tweet.id),
        None => format!("https://x.com/i/status/{}", tweet.id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_slash_tweet() {
        assert_eq!(
            TelegramXBridge::parse_tweet_command("/tweet hello world"),
            Some("hello world".to_string())
        );
    }

    #[test]
    fn parse_tweet_this_prefix() {
        assert_eq!(
            TelegramXBridge::parse_tweet_command("tweet this: shipping ⚡"),
            Some("shipping ⚡".to_string())
        );
        assert_eq!(
            TelegramXBridge::parse_tweet_command("  TWEET THIS:  yo  "),
            Some("yo".to_string())
        );
    }

    #[test]
    fn parse_ignores_plain_chat() {
        assert!(TelegramXBridge::parse_tweet_command("just chatting").is_none());
        assert!(TelegramXBridge::parse_tweet_command("/tweet   ").is_none());
        assert!(TelegramXBridge::parse_tweet_command("tweet this:").is_none());
    }

    #[test]
    fn format_url_with_username() {
        let t = Tweet {
            id: "123".into(),
            text: "hi".into(),
            author_username: Some("zeus".into()),
            author_name: None,
            created_at: None,
            metrics: None,
            media: Vec::new(),
            conversation_id: None,
            in_reply_to: None,
        };
        assert_eq!(format_tweet_url(&t), "https://x.com/zeus/status/123");
    }

    #[test]
    fn format_url_without_username() {
        let t = Tweet {
            id: "456".into(),
            text: "hi".into(),
            author_username: None,
            author_name: None,
            created_at: None,
            metrics: None,
            media: Vec::new(),
            conversation_id: None,
            in_reply_to: None,
        };
        assert_eq!(format_tweet_url(&t), "https://x.com/i/status/456");
    }
}
