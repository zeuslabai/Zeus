//! X (Twitter) posting tools — #315
//!
//! First-class tool wrappers over the existing `zeus_channels::XAdapter`
//! (API v2: create_tweet / delete / metrics, v2 chunked media upload).
//! Credentials come from `[channels.x_twitter]` in config.toml, with the
//! standard env fallbacks (`X_BEARER_TOKEN`, `X_CONSUMER_KEY`, …) that the
//! config layer already honors. No hand-scripted OAuth1 required.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
use zeus_core::{Error, Result, ToolSchema};
use zeus_channels::{CreateTweetOptions, ThreadOptions, XAdapter, XConfig};

/// Build an XAdapter from `[channels.x_twitter]` config (env fallbacks
/// included via the config layer's serde defaults). Mirrors the field
/// mapping in zeus-agent's channel_builder so tool and adapter behavior
/// can never drift apart.
async fn x_adapter() -> Result<XAdapter> {
    let config = zeus_core::Config::load()?;
    let xt = config.channels.and_then(|c| c.x_twitter).ok_or_else(|| {
        Error::Tool(
            "X is not configured: add [channels.x_twitter] to config.toml \
             (bearer_token / consumer_key + secrets / oauth2 credentials)"
                .to_string(),
        )
    })?;
    let x_config = XConfig {
        bearer_token: xt.bearer_token.clone(),
        api_key: xt.consumer_key.clone(),
        api_secret: xt.consumer_key_secret.clone(),
        access_token: xt.access_token.clone(),
        access_token_secret: xt.access_token_secret.clone(),
        client_id: xt.client_id.clone(),
        client_secret: xt.client_secret.clone(),
        poll_interval_secs: xt.poll_interval_secs,
        auto_reply: xt.auto_reply,
        ..Default::default()
    };
    XAdapter::new(x_config).await
}

/// Guess a MIME type from a media file extension (X accepts jpg/png/gif/webp/mp4).
fn mime_for(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    match lower.rsplit('.').next().unwrap_or("") {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        _ => "application/octet-stream",
    }
}

/// Upload each media path and return the media IDs.
async fn upload_media_paths(adapter: &XAdapter, args: &Value) -> Result<Vec<String>> {
    let Some(paths) = args.get("media").and_then(|v| v.as_array()) else {
        return Ok(Vec::new());
    };
    let alt_text = args.get("alt_text").and_then(|v| v.as_str());
    let mut ids = Vec::new();
    for p in paths {
        let Some(path) = p.as_str() else { continue };
        let data = std::fs::read(path)
            .map_err(|e| Error::Tool(format!("Cannot read media file '{}': {}", path, e)))?;
        let id = adapter.upload_media(&data, mime_for(path), alt_text).await?;
        ids.push(id);
    }
    Ok(ids)
}

fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| Error::Tool(format!("Missing required '{}' parameter", key)))
}

// ---------------------------------------------------------------------------
// 1. XPostTool — x_post
// ---------------------------------------------------------------------------

/// Post a tweet (optionally with media attachments or as a quote tweet)
pub struct XPostTool;

#[async_trait]
impl TalosTool for XPostTool {
    fn name(&self) -> &'static str {
        "x_post"
    }
    fn description(&self) -> &'static str {
        "Post a tweet to X (Twitter) via API v2. Supports media attachments \
         (local file paths, uploaded via v2 chunked upload) and quote tweets. \
         Credentials come from [channels.x_twitter] in config.toml."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("text", "string", "Tweet text (280 chars standard)", true)
            .with_param(
                "media",
                "array",
                "Local file paths of images/video to attach (jpg/png/gif/webp/mp4)",
                false,
            )
            .with_param("alt_text", "string", "Alt text applied to attached media", false)
            .with_param("quote_tweet_id", "string", "Tweet ID to quote", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let text = require_str(&args, "text")?;
        let adapter = x_adapter().await?;
        let media_ids = upload_media_paths(&adapter, &args).await?;
        let opts = CreateTweetOptions {
            text: text.to_string(),
            quote_tweet_id: args
                .get("quote_tweet_id")
                .and_then(|v| v.as_str())
                .map(String::from),
            media_ids,
            ..Default::default()
        };
        let tweet = adapter.post_tweet(&opts).await?;
        Ok(format!("Tweet posted (id: {})", tweet.id))
    }
}

// ---------------------------------------------------------------------------
// 2. XReplyTool — x_reply
// ---------------------------------------------------------------------------

/// Reply to a tweet (threading via in_reply_to)
pub struct XReplyTool;

#[async_trait]
impl TalosTool for XReplyTool {
    fn name(&self) -> &'static str {
        "x_reply"
    }
    fn description(&self) -> &'static str {
        "Reply to a tweet on X (Twitter). Replying to your own tweet extends \
         it into a thread. Supports media attachments like x_post."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("in_reply_to", "string", "Tweet ID to reply to", true)
            .with_param("text", "string", "Reply text", true)
            .with_param(
                "media",
                "array",
                "Local file paths of images/video to attach",
                false,
            )
            .with_param("alt_text", "string", "Alt text applied to attached media", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let text = require_str(&args, "text")?;
        let reply_to = require_str(&args, "in_reply_to")?;
        let adapter = x_adapter().await?;
        let media_ids = upload_media_paths(&adapter, &args).await?;
        let opts = CreateTweetOptions {
            text: text.to_string(),
            reply_to: Some(reply_to.to_string()),
            media_ids,
            ..Default::default()
        };
        let tweet = adapter.post_tweet(&opts).await?;
        Ok(format!("Reply posted (id: {}, in reply to {})", tweet.id, reply_to))
    }
}

// ---------------------------------------------------------------------------
// 3. XThreadTool — x_thread
// ---------------------------------------------------------------------------

/// Post a multi-tweet thread
pub struct XThreadTool;

#[async_trait]
impl TalosTool for XThreadTool {
    fn name(&self) -> &'static str {
        "x_thread"
    }
    fn description(&self) -> &'static str {
        "Post a thread (chain of tweets) to X (Twitter). Each entry in \
         'tweets' becomes one tweet, replying to the previous."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("tweets", "array", "Tweet texts, in thread order", true)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let texts: Vec<String> = args
            .get("tweets")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|t| t.as_str())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();
        if texts.is_empty() {
            return Err(Error::Tool(
                "Missing required 'tweets' parameter (non-empty array of strings)".to_string(),
            ));
        }
        let count = texts.len();
        let adapter = x_adapter().await?;
        let thread = ThreadOptions {
            media_per_tweet: vec![Vec::new(); count],
            tweets: texts,
        };
        let tweets = adapter.post_thread(&thread).await?;
        let ids: Vec<&str> = tweets.iter().map(|t| t.id.as_str()).collect();
        Ok(format!("Thread posted ({} tweets: {})", tweets.len(), ids.join(", ")))
    }
}

// ---------------------------------------------------------------------------
// 4. XDeleteTool — x_delete
// ---------------------------------------------------------------------------

/// Delete a tweet by ID
pub struct XDeleteTool;

#[async_trait]
impl TalosTool for XDeleteTool {
    fn name(&self) -> &'static str {
        "x_delete"
    }
    fn description(&self) -> &'static str {
        "Delete one of your tweets on X (Twitter) by tweet ID."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("tweet_id", "string", "ID of the tweet to delete", true)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let tweet_id = require_str(&args, "tweet_id")?;
        let adapter = x_adapter().await?;
        adapter.delete_tweet(tweet_id).await?;
        Ok(format!("Tweet {} deleted", tweet_id))
    }
}

// ---------------------------------------------------------------------------
// 5. XMetricsTool — x_metrics
// ---------------------------------------------------------------------------

/// Fetch public metrics for a tweet
pub struct XMetricsTool;

#[async_trait]
impl TalosTool for XMetricsTool {
    fn name(&self) -> &'static str {
        "x_metrics"
    }
    fn description(&self) -> &'static str {
        "Get public engagement metrics (likes, retweets, replies, views) for \
         a tweet on X (Twitter)."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("tweet_id", "string", "ID of the tweet", true)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let tweet_id = require_str(&args, "tweet_id")?;
        let adapter = x_adapter().await?;
        let m = adapter.get_tweet_metrics(tweet_id).await?;
        Ok(serde_json::to_string_pretty(&m)
            .unwrap_or_else(|_| format!("{:?}", m)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mime_for() {
        assert_eq!(mime_for("a.JPG"), "image/jpeg");
        assert_eq!(mime_for("b.png"), "image/png");
        assert_eq!(mime_for("c.mp4"), "video/mp4");
        assert_eq!(mime_for("noext"), "application/octet-stream");
    }

    #[test]
    fn test_schemas_have_names() {
        assert_eq!(XPostTool.name(), "x_post");
        assert_eq!(XReplyTool.name(), "x_reply");
        assert_eq!(XThreadTool.name(), "x_thread");
        assert_eq!(XDeleteTool.name(), "x_delete");
        assert_eq!(XMetricsTool.name(), "x_metrics");
    }

    #[tokio::test]
    async fn test_x_post_requires_text() {
        let err = XPostTool
            .execute(serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("text"), "got: {err}");
    }

    #[tokio::test]
    async fn test_x_reply_requires_in_reply_to() {
        let err = XReplyTool
            .execute(serde_json::json!({"text": "hi"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("in_reply_to"), "got: {err}");
    }

    #[tokio::test]
    async fn test_x_thread_requires_tweets() {
        let err = XThreadTool
            .execute(serde_json::json!({"tweets": []}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("tweets"), "got: {err}");
    }
}
