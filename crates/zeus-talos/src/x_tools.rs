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
use zeus_channels::x::XDmOptions;
use zeus_channels::{
    CreateTweetOptions, ThreadOptions, XAdapter, XConfig, XListOptions, XReadOptions,
};
use zeus_core::{Error, Result, ToolSchema};

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
        let id = adapter
            .upload_media(&data, mime_for(path), alt_text)
            .await?;
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

fn require_string_array(args: &Value, key: &str) -> Result<Vec<String>> {
    let values = args
        .get(key)
        .and_then(|v| v.as_array())
        .ok_or_else(|| Error::Tool(format!("Missing required '{}' parameter", key)))?;
    if values.is_empty() {
        return Err(Error::Tool(format!("{} must not be empty", key)));
    }

    Ok(values
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect())
}

fn optional_string(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn optional_u32(args: &Value, key: &str) -> Result<Option<u32>> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    if let Some(n) = value.as_u64() {
        return u32::try_from(n)
            .map(Some)
            .map_err(|_| Error::Tool(format!("'{}' must fit in u32", key)));
    }
    if let Some(s) = value.as_str().map(str::trim).filter(|s| !s.is_empty()) {
        return s
            .parse::<u32>()
            .map(Some)
            .map_err(|_| Error::Tool(format!("'{}' must be an integer", key)));
    }
    Err(Error::Tool(format!("'{}' must be an integer", key)))
}

fn x_read_options(args: &Value) -> Result<XReadOptions> {
    Ok(XReadOptions {
        since_id: optional_string(args, "since_id"),
        until_id: optional_string(args, "until_id"),
        pagination_token: optional_string(args, "pagination_token"),
        max_results: optional_u32(args, "max_results")?,
    })
}

fn x_dm_options(args: &Value) -> Result<XDmOptions> {
    Ok(XDmOptions {
        pagination_token: optional_string(args, "pagination_token"),
        max_results: optional_u32(args, "max_results")?,
    })
}

fn optional_bool(args: &Value, key: &str) -> Result<Option<bool>> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    if let Some(b) = value.as_bool() {
        return Ok(Some(b));
    }
    if let Some(s) = value.as_str().map(str::trim).filter(|s| !s.is_empty()) {
        return match s.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Ok(Some(true)),
            "false" | "0" | "no" => Ok(Some(false)),
            _ => Err(Error::Tool(format!("'{}' must be a boolean", key))),
        };
    }
    Err(Error::Tool(format!("'{}' must be a boolean", key)))
}

fn x_list_options(args: &Value) -> Result<XListOptions> {
    Ok(XListOptions {
        pagination_token: optional_string(args, "pagination_token"),
        max_results: optional_u32(args, "max_results")?,
    })
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
            .with_param(
                "alt_text",
                "string",
                "Alt text applied to attached media",
                false,
            )
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
            .with_param(
                "alt_text",
                "string",
                "Alt text applied to attached media",
                false,
            )
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
        Ok(format!(
            "Reply posted (id: {}, in reply to {})",
            tweet.id, reply_to
        ))
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
        ToolSchema::new(self.name(), self.description()).with_param(
            "tweets",
            "array",
            "Tweet texts, in thread order",
            true,
        )
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
        Ok(format!(
            "Thread posted ({} tweets: {})",
            tweets.len(),
            ids.join(", ")
        ))
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
        ToolSchema::new(self.name(), self.description()).with_param(
            "tweet_id",
            "string",
            "ID of the tweet to delete",
            true,
        )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let tweet_id = require_str(&args, "tweet_id")?;
        let adapter = x_adapter().await?;
        adapter.delete_tweet(tweet_id).await?;
        Ok(format!("Tweet {} deleted", tweet_id))
    }
}

// ---------------------------------------------------------------------------
// 4b. XDeletePostTool — x_delete_post
// ---------------------------------------------------------------------------

/// Delete a single X post/tweet by ID, returning a structured per-item result
pub struct XDeletePostTool;

#[async_trait]
impl TalosTool for XDeletePostTool {
    fn name(&self) -> &'static str {
        "x_delete_post"
    }
    fn description(&self) -> &'static str {
        "Delete a single X post/tweet by ID, returning a structured per-item result"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "post_id",
            "string",
            "ID of the X post/tweet to delete",
            true,
        )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let post_id = require_str(&args, "post_id")?;
        let adapter = x_adapter().await?;
        let result = adapter.delete_tweet_result(post_id).await;
        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| format!("{:?}", result)))
    }
}

// ---------------------------------------------------------------------------
// 4c. XBatchDeleteTool — x_batch_delete
// ---------------------------------------------------------------------------

/// Delete multiple X posts/tweets sequentially with per-item results
pub struct XBatchDeleteTool;

#[async_trait]
impl TalosTool for XBatchDeleteTool {
    fn name(&self) -> &'static str {
        "x_batch_delete"
    }
    fn description(&self) -> &'static str {
        "Delete multiple X posts/tweets sequentially, returning deleted/failed/skipped per item"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "post_ids",
            "array",
            "IDs of X posts/tweets to delete",
            true,
        )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let post_ids = require_string_array(&args, "post_ids")?;
        let adapter = x_adapter().await?;
        let result = adapter.batch_delete_tweets(&post_ids).await;
        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| format!("{:?}", result)))
    }
}

// ---------------------------------------------------------------------------
// 5. XSearchRecentTool — x_search_recent
// ---------------------------------------------------------------------------

/// Search recent public X posts by keyword/query, with polling cursor support.
pub struct XSearchRecentTool;

#[async_trait]
impl TalosTool for XSearchRecentTool {
    fn name(&self) -> &'static str {
        "x_search_recent"
    }
    fn description(&self) -> &'static str {
        "Search recent public X posts by keyword/query. Supports since_id, until_id, pagination_token, and max_results for polling."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("query", "string", "X recent-search query", true)
            .with_param(
                "since_id",
                "string",
                "Only return posts newer than this tweet ID",
                false,
            )
            .with_param(
                "until_id",
                "string",
                "Only return posts older than this tweet ID",
                false,
            )
            .with_param(
                "pagination_token",
                "string",
                "Pagination token from a previous response",
                false,
            )
            .with_param("max_results", "integer", "Maximum results to return", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let query = require_str(&args, "query")?;
        let opts = x_read_options(&args)?;
        let adapter = x_adapter().await?;
        let results = adapter.search_recent(query, &opts).await?;
        Ok(serde_json::to_string_pretty(&results).unwrap_or_else(|_| format!("{:?}", results)))
    }
}

// ---------------------------------------------------------------------------
// 6. XGetMentionsTool — x_get_mentions
// ---------------------------------------------------------------------------

/// Get recent mentions for an X user ID, with polling cursor support.
pub struct XGetMentionsTool;

#[async_trait]
impl TalosTool for XGetMentionsTool {
    fn name(&self) -> &'static str {
        "x_get_mentions"
    }
    fn description(&self) -> &'static str {
        "Get recent mentions for an X user ID. Supports since_id, until_id, pagination_token, and max_results for polling."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "user_id",
                "string",
                "X user ID whose mentions should be fetched",
                true,
            )
            .with_param(
                "since_id",
                "string",
                "Only return posts newer than this tweet ID",
                false,
            )
            .with_param(
                "until_id",
                "string",
                "Only return posts older than this tweet ID",
                false,
            )
            .with_param(
                "pagination_token",
                "string",
                "Pagination token from a previous response",
                false,
            )
            .with_param("max_results", "integer", "Maximum results to return", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let user_id = require_str(&args, "user_id")?;
        let opts = x_read_options(&args)?;
        let adapter = x_adapter().await?;
        let mentions = adapter.get_mentions(user_id, &opts).await?;
        Ok(serde_json::to_string_pretty(&mentions).unwrap_or_else(|_| format!("{:?}", mentions)))
    }
}

// ---------------------------------------------------------------------------
// 7. XGetTweetTool — x_get_tweet
// ---------------------------------------------------------------------------

/// Get one X post/tweet by ID.
pub struct XGetTweetTool;

#[async_trait]
impl TalosTool for XGetTweetTool {
    fn name(&self) -> &'static str {
        "x_get_tweet"
    }
    fn description(&self) -> &'static str {
        "Get one X post/tweet by ID, including author, metrics, conversation, reply, and media metadata when available."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "tweet_id",
            "string",
            "ID of the X post/tweet to fetch",
            true,
        )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let tweet_id = require_str(&args, "tweet_id")?;
        let adapter = x_adapter().await?;
        let tweet = adapter.get_tweet(tweet_id).await?;
        Ok(serde_json::to_string_pretty(&tweet).unwrap_or_else(|_| format!("{:?}", tweet)))
    }
}

// ---------------------------------------------------------------------------
// 8. XGetUserTimelineTool — x_get_user_timeline
// ---------------------------------------------------------------------------

/// Get recent posts for an X user ID, with polling cursor support.
pub struct XGetUserTimelineTool;

#[async_trait]
impl TalosTool for XGetUserTimelineTool {
    fn name(&self) -> &'static str {
        "x_get_user_timeline"
    }
    fn description(&self) -> &'static str {
        "Get recent posts for an X user ID. Supports since_id, until_id, pagination_token, and max_results for polling."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "user_id",
                "string",
                "X user ID whose timeline should be fetched",
                true,
            )
            .with_param(
                "since_id",
                "string",
                "Only return posts newer than this tweet ID",
                false,
            )
            .with_param(
                "until_id",
                "string",
                "Only return posts older than this tweet ID",
                false,
            )
            .with_param(
                "pagination_token",
                "string",
                "Pagination token from a previous response",
                false,
            )
            .with_param("max_results", "integer", "Maximum results to return", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let user_id = require_str(&args, "user_id")?;
        let opts = x_read_options(&args)?;
        let adapter = x_adapter().await?;
        let timeline = adapter.get_user_timeline(user_id, &opts).await?;
        Ok(serde_json::to_string_pretty(&timeline).unwrap_or_else(|_| format!("{:?}", timeline)))
    }
}

// ---------------------------------------------------------------------------
// 9. X engagement/media P2 tools — #339
// ---------------------------------------------------------------------------

macro_rules! x_tweet_action_tool {
    ($struct_name:ident, $tool_name:expr, $description:expr, $method:ident) => {
        pub struct $struct_name;

        #[async_trait]
        impl TalosTool for $struct_name {
            fn name(&self) -> &'static str {
                $tool_name
            }
            fn description(&self) -> &'static str {
                $description
            }
            fn schema(&self) -> ToolSchema {
                ToolSchema::new(self.name(), self.description())
                    .with_param("user_id", "string", "Acting/authenticated X user ID", true)
                    .with_param("tweet_id", "string", "Target tweet/post ID", true)
            }
            async fn execute(&self, args: Value) -> Result<String> {
                let user_id = require_str(&args, "user_id")?;
                let tweet_id = require_str(&args, "tweet_id")?;
                let adapter = x_adapter().await?;
                let result = adapter.$method(user_id, tweet_id).await?;
                Ok(serde_json::to_string_pretty(&result)
                    .unwrap_or_else(|_| format!("{:?}", result)))
            }
        }
    };
}

x_tweet_action_tool!(
    XLikeTool,
    "x_like",
    "Like an X tweet/post as the authenticated user via API v2.",
    like_tweet
);
x_tweet_action_tool!(
    XUnlikeTool,
    "x_unlike",
    "Remove the authenticated user's like from an X tweet/post via API v2.",
    unlike_tweet
);
x_tweet_action_tool!(
    XRetweetTool,
    "x_retweet",
    "Retweet an X tweet/post as the authenticated user via API v2.",
    retweet
);
x_tweet_action_tool!(
    XUnretweetTool,
    "x_unretweet",
    "Remove the authenticated user's retweet via API v2.",
    unretweet
);
x_tweet_action_tool!(
    XBookmarkTool,
    "x_bookmark",
    "Bookmark an X tweet/post as the authenticated user via API v2.",
    bookmark_tweet
);
x_tweet_action_tool!(
    XUnbookmarkTool,
    "x_unbookmark",
    "Remove the authenticated user's bookmark from an X tweet/post via API v2.",
    unbookmark_tweet
);

pub struct XFollowTool;

#[async_trait]
impl TalosTool for XFollowTool {
    fn name(&self) -> &'static str {
        "x_follow"
    }
    fn description(&self) -> &'static str {
        "Follow an X user as the authenticated user via API v2."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("user_id", "string", "Acting/authenticated X user ID", true)
            .with_param("target_user_id", "string", "X user ID to follow", true)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let user_id = require_str(&args, "user_id")?;
        let target_user_id = require_str(&args, "target_user_id")?;
        let adapter = x_adapter().await?;
        let result = adapter.follow_user(user_id, target_user_id).await?;
        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| format!("{:?}", result)))
    }
}

pub struct XUnfollowTool;

#[async_trait]
impl TalosTool for XUnfollowTool {
    fn name(&self) -> &'static str {
        "x_unfollow"
    }
    fn description(&self) -> &'static str {
        "Unfollow an X user as the authenticated user via API v2."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("user_id", "string", "Acting/authenticated X user ID", true)
            .with_param("target_user_id", "string", "X user ID to unfollow", true)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let user_id = require_str(&args, "user_id")?;
        let target_user_id = require_str(&args, "target_user_id")?;
        let adapter = x_adapter().await?;
        let result = adapter.unfollow_user(user_id, target_user_id).await?;
        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| format!("{:?}", result)))
    }
}

pub struct XQuoteTool;

#[async_trait]
impl TalosTool for XQuoteTool {
    fn name(&self) -> &'static str {
        "x_quote"
    }
    fn description(&self) -> &'static str {
        "Quote an X tweet/post by creating a new quote tweet via API v2."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("text", "string", "Quote tweet text", true)
            .with_param("quote_tweet_id", "string", "Tweet/post ID to quote", true)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let text = require_str(&args, "text")?;
        let quote_tweet_id = require_str(&args, "quote_tweet_id")?;
        let adapter = x_adapter().await?;
        let tweet = adapter.quote_tweet(text, quote_tweet_id).await?;
        Ok(serde_json::to_string_pretty(&tweet).unwrap_or_else(|_| format!("{:?}", tweet)))
    }
}

pub struct XUploadMediaTool;

#[async_trait]
impl TalosTool for XUploadMediaTool {
    fn name(&self) -> &'static str {
        "x_upload_media"
    }
    fn description(&self) -> &'static str {
        "Upload local media to X via v2 chunked media upload and return a reusable media_id."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("path", "string", "Local image/video path to upload", true)
            .with_param(
                "mime_type",
                "string",
                "Override MIME type; inferred from path by default",
                false,
            )
            .with_param(
                "alt_text",
                "string",
                "Optional alt text for the media",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let path = require_str(&args, "path")?;
        let data = std::fs::read(path)
            .map_err(|e| Error::Tool(format!("Cannot read media file '{}': {}", path, e)))?;
        let mime_type = args
            .get("mime_type")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| mime_for(path));
        let alt_text = args.get("alt_text").and_then(|v| v.as_str());
        let adapter = x_adapter().await?;
        let media_id = adapter.upload_media(&data, mime_type, alt_text).await?;
        Ok(serde_json::json!({ "media_id": media_id }).to_string())
    }
}

// ---------------------------------------------------------------------------
// 10. XMetricsTool — x_metrics
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
        ToolSchema::new(self.name(), self.description()).with_param(
            "tweet_id",
            "string",
            "ID of the tweet",
            true,
        )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let tweet_id = require_str(&args, "tweet_id")?;
        let adapter = x_adapter().await?;
        let m = adapter.get_tweet_metrics(tweet_id).await?;
        Ok(serde_json::to_string_pretty(&m).unwrap_or_else(|_| format!("{:?}", m)))
    }
}

/// Fetch public account metrics for an X user.
pub struct XAccountMetricsTool;

#[async_trait]
impl TalosTool for XAccountMetricsTool {
    fn name(&self) -> &'static str {
        "x_account_metrics"
    }
    fn description(&self) -> &'static str {
        "Get public account metrics (followers, following, tweets, listed count) for an X user ID."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "user_id",
            "string",
            "ID of the X user/account",
            true,
        )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let user_id = require_str(&args, "user_id")?;
        let adapter = x_adapter().await?;
        let metrics = adapter.get_account_metrics(user_id).await?;
        Ok(serde_json::to_string_pretty(&metrics).unwrap_or_else(|_| format!("{:?}", metrics)))
    }
}

// ---------------------------------------------------------------------------
// 11. X moderation P3 tools — #339
// ---------------------------------------------------------------------------

macro_rules! x_user_moderation_tool {
    ($struct_name:ident, $tool_name:expr, $description:expr, $method:ident) => {
        pub struct $struct_name;

        #[async_trait]
        impl TalosTool for $struct_name {
            fn name(&self) -> &'static str {
                $tool_name
            }
            fn description(&self) -> &'static str {
                $description
            }
            fn schema(&self) -> ToolSchema {
                ToolSchema::new(self.name(), self.description())
                    .with_param("user_id", "string", "Acting/authenticated X user ID", true)
                    .with_param("target_user_id", "string", "Target X user ID", true)
            }
            async fn execute(&self, args: Value) -> Result<String> {
                let user_id = require_str(&args, "user_id")?;
                let target_user_id = require_str(&args, "target_user_id")?;
                let adapter = x_adapter().await?;
                let result = adapter.$method(user_id, target_user_id).await?;
                Ok(serde_json::to_string_pretty(&result)
                    .unwrap_or_else(|_| format!("{:?}", result)))
            }
        }
    };
}

x_user_moderation_tool!(
    XBlockTool,
    "x_block",
    "Block an X user via API v2.",
    block_user
);
x_user_moderation_tool!(
    XUnblockTool,
    "x_unblock",
    "Unblock an X user via API v2.",
    unblock_user
);
x_user_moderation_tool!(XMuteTool, "x_mute", "Mute an X user via API v2.", mute_user);
x_user_moderation_tool!(
    XUnmuteTool,
    "x_unmute",
    "Unmute an X user via API v2.",
    unmute_user
);

macro_rules! x_reply_visibility_tool {
    ($struct_name:ident, $tool_name:expr, $description:expr, $method:ident) => {
        pub struct $struct_name;

        #[async_trait]
        impl TalosTool for $struct_name {
            fn name(&self) -> &'static str {
                $tool_name
            }
            fn description(&self) -> &'static str {
                $description
            }
            fn schema(&self) -> ToolSchema {
                ToolSchema::new(self.name(), self.description()).with_param(
                    "tweet_id",
                    "string",
                    "Reply tweet/post ID",
                    true,
                )
            }
            async fn execute(&self, args: Value) -> Result<String> {
                let tweet_id = require_str(&args, "tweet_id")?;
                let adapter = x_adapter().await?;
                let result = adapter.$method(tweet_id).await?;
                Ok(serde_json::to_string_pretty(&result)
                    .unwrap_or_else(|_| format!("{:?}", result)))
            }
        }
    };
}

x_reply_visibility_tool!(
    XHideReplyTool,
    "x_hide_reply",
    "Hide a reply tweet via API v2.",
    hide_reply
);
x_reply_visibility_tool!(
    XUnhideReplyTool,
    "x_unhide_reply",
    "Unhide a reply tweet via API v2.",
    unhide_reply
);

pub struct XReportTweetTool;

#[async_trait]
impl TalosTool for XReportTweetTool {
    fn name(&self) -> &'static str {
        "x_report_tweet"
    }
    fn description(&self) -> &'static str {
        "Return a clear capability error for report-tweet requests; public X API v2 does not expose standard app reporting."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("tweet_id", "string", "Tweet/post ID to report", true)
            .with_param("reason", "string", "Optional report reason/context", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let tweet_id = require_str(&args, "tweet_id")?;
        let adapter = x_adapter().await?;
        let result = adapter
            .report_tweet(tweet_id, optional_string(&args, "reason").as_deref())
            .await?;
        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| format!("{:?}", result)))
    }
}

// ---------------------------------------------------------------------------
// 12. X list P3 tools — #339
// ---------------------------------------------------------------------------

pub struct XGetListTool;

#[async_trait]
impl TalosTool for XGetListTool {
    fn name(&self) -> &'static str {
        "x_get_list"
    }
    fn description(&self) -> &'static str {
        "Fetch metadata for an X list by list_id via API v2."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "list_id",
            "string",
            "X list ID",
            true,
        )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let list_id = require_str(&args, "list_id")?;
        let adapter = x_adapter().await?;
        let list = adapter.get_list(list_id).await?;
        Ok(serde_json::to_string_pretty(&list).unwrap_or_else(|_| format!("{:?}", list)))
    }
}

macro_rules! x_user_list_page_tool {
    ($struct_name:ident, $tool_name:expr, $description:expr, $method:ident) => {
        pub struct $struct_name;

        #[async_trait]
        impl TalosTool for $struct_name {
            fn name(&self) -> &'static str {
                $tool_name
            }
            fn description(&self) -> &'static str {
                $description
            }
            fn schema(&self) -> ToolSchema {
                ToolSchema::new(self.name(), self.description())
                    .with_param("user_id", "string", "X user ID", true)
                    .with_param(
                        "pagination_token",
                        "string",
                        "Pagination token from a previous response",
                        false,
                    )
                    .with_param("max_results", "integer", "Maximum lists to return", false)
            }
            async fn execute(&self, args: Value) -> Result<String> {
                let user_id = require_str(&args, "user_id")?;
                let opts = x_list_options(&args)?;
                let adapter = x_adapter().await?;
                let page = adapter.$method(user_id, &opts).await?;
                Ok(serde_json::to_string_pretty(&page).unwrap_or_else(|_| format!("{:?}", page)))
            }
        }
    };
}

x_user_list_page_tool!(
    XGetOwnedListsTool,
    "x_get_owned_lists",
    "Fetch X lists owned by a user ID via API v2.",
    get_owned_lists
);
x_user_list_page_tool!(
    XGetListMembershipsTool,
    "x_get_list_memberships",
    "Fetch X lists that a user belongs to via API v2.",
    get_list_memberships
);
x_user_list_page_tool!(
    XGetFollowedListsTool,
    "x_get_followed_lists",
    "Fetch X lists followed by a user ID via API v2.",
    get_followed_lists
);

pub struct XGetListTweetsTool;

#[async_trait]
impl TalosTool for XGetListTweetsTool {
    fn name(&self) -> &'static str {
        "x_get_list_tweets"
    }
    fn description(&self) -> &'static str {
        "Fetch recent X posts from a list, with since_id/pagination_token polling support."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("list_id", "string", "X list ID", true)
            .with_param(
                "since_id",
                "string",
                "Only return posts newer than this tweet ID",
                false,
            )
            .with_param(
                "until_id",
                "string",
                "Only return posts older than this tweet ID",
                false,
            )
            .with_param(
                "pagination_token",
                "string",
                "Pagination token from a previous response",
                false,
            )
            .with_param("max_results", "integer", "Maximum posts to return", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let list_id = require_str(&args, "list_id")?;
        let opts = x_read_options(&args)?;
        let adapter = x_adapter().await?;
        let tweets = adapter.get_list_tweets(list_id, &opts).await?;
        Ok(serde_json::to_string_pretty(&tweets).unwrap_or_else(|_| format!("{:?}", tweets)))
    }
}

pub struct XCreateListTool;

#[async_trait]
impl TalosTool for XCreateListTool {
    fn name(&self) -> &'static str {
        "x_create_list"
    }
    fn description(&self) -> &'static str {
        "Create an X list via API v2."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("name", "string", "List name", true)
            .with_param("description", "string", "Optional list description", false)
            .with_param("private", "boolean", "Whether the list is private", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let name = require_str(&args, "name")?;
        let adapter = x_adapter().await?;
        let list = adapter
            .create_list(
                name,
                optional_string(&args, "description").as_deref(),
                optional_bool(&args, "private")?,
            )
            .await?;
        Ok(serde_json::to_string_pretty(&list).unwrap_or_else(|_| format!("{:?}", list)))
    }
}

pub struct XUpdateListTool;

#[async_trait]
impl TalosTool for XUpdateListTool {
    fn name(&self) -> &'static str {
        "x_update_list"
    }
    fn description(&self) -> &'static str {
        "Update an X list's name, description, or privacy via API v2."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("list_id", "string", "X list ID", true)
            .with_param("name", "string", "Optional new list name", false)
            .with_param("description", "string", "Optional new description", false)
            .with_param("private", "boolean", "Optional privacy flag", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let list_id = require_str(&args, "list_id")?;
        let adapter = x_adapter().await?;
        let list = adapter
            .update_list(
                list_id,
                optional_string(&args, "name").as_deref(),
                optional_string(&args, "description").as_deref(),
                optional_bool(&args, "private")?,
            )
            .await?;
        Ok(serde_json::to_string_pretty(&list).unwrap_or_else(|_| format!("{:?}", list)))
    }
}

pub struct XDeleteListTool;

#[async_trait]
impl TalosTool for XDeleteListTool {
    fn name(&self) -> &'static str {
        "x_delete_list"
    }
    fn description(&self) -> &'static str {
        "Delete an X list via API v2."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "list_id",
            "string",
            "X list ID",
            true,
        )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let list_id = require_str(&args, "list_id")?;
        let adapter = x_adapter().await?;
        let result = adapter.delete_list(list_id).await?;
        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| format!("{:?}", result)))
    }
}

macro_rules! x_list_user_mutation_tool {
    ($struct_name:ident, $tool_name:expr, $description:expr, $method:ident) => {
        pub struct $struct_name;

        #[async_trait]
        impl TalosTool for $struct_name {
            fn name(&self) -> &'static str {
                $tool_name
            }
            fn description(&self) -> &'static str {
                $description
            }
            fn schema(&self) -> ToolSchema {
                ToolSchema::new(self.name(), self.description())
                    .with_param("list_id", "string", "X list ID", true)
                    .with_param("user_id", "string", "X user ID", true)
            }
            async fn execute(&self, args: Value) -> Result<String> {
                let list_id = require_str(&args, "list_id")?;
                let user_id = require_str(&args, "user_id")?;
                let adapter = x_adapter().await?;
                let result = adapter.$method(list_id, user_id).await?;
                Ok(serde_json::to_string_pretty(&result)
                    .unwrap_or_else(|_| format!("{:?}", result)))
            }
        }
    };
}

x_list_user_mutation_tool!(
    XAddListMemberTool,
    "x_add_list_member",
    "Add a user to an X list via API v2.",
    add_list_member
);
x_list_user_mutation_tool!(
    XRemoveListMemberTool,
    "x_remove_list_member",
    "Remove a user from an X list via API v2.",
    remove_list_member
);

macro_rules! x_user_list_follow_tool {
    ($struct_name:ident, $tool_name:expr, $description:expr, $method:ident) => {
        pub struct $struct_name;

        #[async_trait]
        impl TalosTool for $struct_name {
            fn name(&self) -> &'static str {
                $tool_name
            }
            fn description(&self) -> &'static str {
                $description
            }
            fn schema(&self) -> ToolSchema {
                ToolSchema::new(self.name(), self.description())
                    .with_param("user_id", "string", "Authenticated X user ID", true)
                    .with_param("list_id", "string", "X list ID", true)
            }
            async fn execute(&self, args: Value) -> Result<String> {
                let user_id = require_str(&args, "user_id")?;
                let list_id = require_str(&args, "list_id")?;
                let adapter = x_adapter().await?;
                let result = adapter.$method(user_id, list_id).await?;
                Ok(serde_json::to_string_pretty(&result)
                    .unwrap_or_else(|_| format!("{:?}", result)))
            }
        }
    };
}

x_user_list_follow_tool!(
    XFollowListTool,
    "x_follow_list",
    "Follow an X list as the authenticated user via API v2.",
    follow_list
);
x_user_list_follow_tool!(
    XUnfollowListTool,
    "x_unfollow_list",
    "Unfollow an X list as the authenticated user via API v2.",
    unfollow_list
);

// ---------------------------------------------------------------------------
// 13. X Direct Message P3 tools — #339
// ---------------------------------------------------------------------------

pub struct XGetDmEventsTool;

#[async_trait]
impl TalosTool for XGetDmEventsTool {
    fn name(&self) -> &'static str {
        "x_get_dm_events"
    }
    fn description(&self) -> &'static str {
        "Read recent X Direct Message events for the authenticated user. Supports pagination_token and max_results for polling."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "pagination_token",
                "string",
                "Pagination token from a previous DM event page",
                false,
            )
            .with_param(
                "max_results",
                "number",
                "Maximum DM events to return, clamped to X limits",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let opts = x_dm_options(&args)?;
        let adapter = x_adapter().await?;
        let page = adapter.get_dm_events(&opts).await?;
        Ok(serde_json::to_string_pretty(&page).unwrap_or_else(|_| format!("{:?}", page)))
    }
}

pub struct XGetDmConversationEventsTool;

#[async_trait]
impl TalosTool for XGetDmConversationEventsTool {
    fn name(&self) -> &'static str {
        "x_get_dm_conversation_events"
    }
    fn description(&self) -> &'static str {
        "Read X Direct Message events for one conversation ID. Supports pagination_token and max_results for polling."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("dm_conversation_id", "string", "X DM conversation ID", true)
            .with_param(
                "pagination_token",
                "string",
                "Pagination token from a previous DM event page",
                false,
            )
            .with_param(
                "max_results",
                "number",
                "Maximum DM events to return, clamped to X limits",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let conversation_id = require_str(&args, "dm_conversation_id")?;
        let opts = x_dm_options(&args)?;
        let adapter = x_adapter().await?;
        let page = adapter
            .get_dm_conversation_events(conversation_id, &opts)
            .await?;
        Ok(serde_json::to_string_pretty(&page).unwrap_or_else(|_| format!("{:?}", page)))
    }
}

pub struct XSendDmTool;

#[async_trait]
impl TalosTool for XSendDmTool {
    fn name(&self) -> &'static str {
        "x_send_dm"
    }
    fn description(&self) -> &'static str {
        "Send an X Direct Message into an existing DM conversation via API v2."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("dm_conversation_id", "string", "X DM conversation ID", true)
            .with_param("text", "string", "DM text body", true)
            .with_param(
                "media_id",
                "string",
                "Optional uploaded X media ID to attach",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let conversation_id = require_str(&args, "dm_conversation_id")?;
        let text = require_str(&args, "text")?;
        let adapter = x_adapter().await?;
        let result = adapter
            .send_dm(
                conversation_id,
                text,
                optional_string(&args, "media_id").as_deref(),
            )
            .await?;
        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| format!("{:?}", result)))
    }
}

pub struct XSendDmToUserTool;

#[async_trait]
impl TalosTool for XSendDmToUserTool {
    fn name(&self) -> &'static str {
        "x_send_dm_to_user"
    }
    fn description(&self) -> &'static str {
        "Send an X Direct Message to a user, creating or reusing the one-to-one conversation via API v2."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("user_id", "string", "Target X user ID", true)
            .with_param("text", "string", "DM text body", true)
            .with_param(
                "media_id",
                "string",
                "Optional uploaded X media ID to attach",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let user_id = require_str(&args, "user_id")?;
        let text = require_str(&args, "text")?;
        let adapter = x_adapter().await?;
        let result = adapter
            .send_dm_to_user(user_id, text, optional_string(&args, "media_id").as_deref())
            .await?;
        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| format!("{:?}", result)))
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
        assert_eq!(XDeletePostTool.name(), "x_delete_post");
        assert_eq!(XBatchDeleteTool.name(), "x_batch_delete");
        assert_eq!(XSearchRecentTool.name(), "x_search_recent");
        assert_eq!(XGetMentionsTool.name(), "x_get_mentions");
        assert_eq!(XGetTweetTool.name(), "x_get_tweet");
        assert_eq!(XGetUserTimelineTool.name(), "x_get_user_timeline");
        assert_eq!(XLikeTool.name(), "x_like");
        assert_eq!(XUnlikeTool.name(), "x_unlike");
        assert_eq!(XRetweetTool.name(), "x_retweet");
        assert_eq!(XUnretweetTool.name(), "x_unretweet");
        assert_eq!(XQuoteTool.name(), "x_quote");
        assert_eq!(XFollowTool.name(), "x_follow");
        assert_eq!(XUnfollowTool.name(), "x_unfollow");
        assert_eq!(XBookmarkTool.name(), "x_bookmark");
        assert_eq!(XUnbookmarkTool.name(), "x_unbookmark");
        assert_eq!(XUploadMediaTool.name(), "x_upload_media");
        assert_eq!(XMetricsTool.name(), "x_metrics");
        assert_eq!(XAccountMetricsTool.name(), "x_account_metrics");
        assert_eq!(XGetListTool.name(), "x_get_list");
        assert_eq!(XGetOwnedListsTool.name(), "x_get_owned_lists");
        assert_eq!(XGetListMembershipsTool.name(), "x_get_list_memberships");
        assert_eq!(XGetFollowedListsTool.name(), "x_get_followed_lists");
        assert_eq!(XGetListTweetsTool.name(), "x_get_list_tweets");
        assert_eq!(XCreateListTool.name(), "x_create_list");
        assert_eq!(XUpdateListTool.name(), "x_update_list");
        assert_eq!(XDeleteListTool.name(), "x_delete_list");
        assert_eq!(XAddListMemberTool.name(), "x_add_list_member");
        assert_eq!(XRemoveListMemberTool.name(), "x_remove_list_member");
        assert_eq!(XFollowListTool.name(), "x_follow_list");
        assert_eq!(XUnfollowListTool.name(), "x_unfollow_list");
        assert_eq!(XGetDmEventsTool.name(), "x_get_dm_events");
        assert_eq!(
            XGetDmConversationEventsTool.name(),
            "x_get_dm_conversation_events"
        );
        assert_eq!(XSendDmTool.name(), "x_send_dm");
        assert_eq!(XSendDmToUserTool.name(), "x_send_dm_to_user");
    }

    #[tokio::test]
    async fn test_x_post_requires_text() {
        let err = XPostTool.execute(serde_json::json!({})).await.unwrap_err();
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

    #[tokio::test]
    async fn test_x_delete_post_requires_post_id() {
        let err = XDeletePostTool
            .execute(serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("post_id"), "got: {err}");
    }

    #[tokio::test]
    async fn test_x_batch_delete_requires_post_ids() {
        let err = XBatchDeleteTool
            .execute(serde_json::json!({"post_ids": []}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("post_ids"), "got: {err}");
    }

    #[tokio::test]
    async fn test_x_search_recent_requires_query() {
        let err = XSearchRecentTool
            .execute(serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("query"), "got: {err}");
    }

    #[tokio::test]
    async fn test_x_get_mentions_requires_user_id() {
        let err = XGetMentionsTool
            .execute(serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("user_id"), "got: {err}");
    }

    #[tokio::test]
    async fn test_x_get_tweet_requires_tweet_id() {
        let err = XGetTweetTool
            .execute(serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("tweet_id"), "got: {err}");
    }

    #[tokio::test]
    async fn test_x_get_user_timeline_requires_user_id() {
        let err = XGetUserTimelineTool
            .execute(serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("user_id"), "got: {err}");
    }

    #[tokio::test]
    async fn test_x_like_requires_user_id() {
        let err = XLikeTool
            .execute(serde_json::json!({ "tweet_id": "123" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("user_id"), "got: {err}");
    }

    #[tokio::test]
    async fn test_x_like_requires_tweet_id() {
        let err = XLikeTool
            .execute(serde_json::json!({ "user_id": "42" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("tweet_id"), "got: {err}");
    }

    #[tokio::test]
    async fn test_x_follow_requires_target_user_id() {
        let err = XFollowTool
            .execute(serde_json::json!({ "user_id": "42" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("target_user_id"), "got: {err}");
    }

    #[tokio::test]
    async fn test_x_quote_requires_quote_tweet_id() {
        let err = XQuoteTool
            .execute(serde_json::json!({ "text": "quote" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("quote_tweet_id"), "got: {err}");
    }

    #[tokio::test]
    async fn test_x_upload_media_requires_path() {
        let err = XUploadMediaTool
            .execute(serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("path"), "got: {err}");
    }
    #[tokio::test]
    async fn test_x_block_requires_target_user_id() {
        let err = XBlockTool
            .execute(serde_json::json!({ "user_id": "42" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("target_user_id"), "got: {err}");
    }

    #[tokio::test]
    async fn test_x_hide_reply_requires_tweet_id() {
        let err = XHideReplyTool
            .execute(serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("tweet_id"), "got: {err}");
    }

    #[tokio::test]
    async fn test_x_report_tweet_requires_tweet_id() {
        let err = XReportTweetTool
            .execute(serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("tweet_id"), "got: {err}");
    }
    #[tokio::test]
    async fn test_x_get_list_requires_list_id() {
        let err = XGetListTool
            .execute(serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("list_id"), "got: {err}");
    }

    #[tokio::test]
    async fn test_x_create_list_requires_name() {
        let err = XCreateListTool
            .execute(serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("name"), "got: {err}");
    }

    #[tokio::test]
    async fn test_x_add_list_member_requires_user_id() {
        let err = XAddListMemberTool
            .execute(serde_json::json!({"list_id": "123"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("user_id"), "got: {err}");
    }

    #[tokio::test]
    async fn test_x_follow_list_requires_list_id() {
        let err = XFollowListTool
            .execute(serde_json::json!({"user_id": "42"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("list_id"), "got: {err}");
    }
    #[tokio::test]
    async fn test_x_get_dm_conversation_events_requires_conversation_id() {
        let err = XGetDmConversationEventsTool
            .execute(serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("dm_conversation_id"), "got: {err}");
    }

    #[tokio::test]
    async fn test_x_send_dm_requires_text() {
        let err = XSendDmTool
            .execute(serde_json::json!({"dm_conversation_id": "c1"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("text"), "got: {err}");
    }

    #[tokio::test]
    async fn test_x_send_dm_to_user_requires_user_id() {
        let err = XSendDmToUserTool
            .execute(serde_json::json!({"text": "hello"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("user_id"), "got: {err}");
    }
    #[tokio::test]
    async fn test_x_account_metrics_requires_user_id() {
        let err = XAccountMetricsTool
            .execute(serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("user_id"), "got: {err}");
    }
}
