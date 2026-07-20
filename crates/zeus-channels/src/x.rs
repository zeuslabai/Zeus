//! X (Twitter) channel adapter
//!
//! Full-featured X/Twitter integration via API v2:
//! - Post tweets, threads, and quote tweets
//! - Upload and attach media (images, video, GIF)
//! - Schedule tweets for future posting
//! - Receive mentions and DMs via polling
//! - Engagement metrics (likes, retweets, replies, impressions)
//! - Delete tweets
//!
//! Authentication: OAuth 2.0 with PKCE (user context) or OAuth 1.0a (app context)
//!
//! ## API v2 notes
//!
//! **This adapter is v2-only.** The legacy v1.1 write surface
//! (`statuses/update.json`, `upload.twitter.com/1.1/media/upload.json`) is
//! deprecated by X and must not be reintroduced. Posting is
//! `POST https://api.x.com/2/tweets` (the `create_tweet` operation); media
//! upload follows the v2 OpenAPI contract: one-shot multipart
//! `POST /2/media/upload` for static images, and chunked
//! `initialize` → `{id}/append` → `{id}/finalize` subresource calls
//! for video/gif. No query parameters on any media POST.
//!
//! Replies use the v2 request shape `POST /2/tweets` with body:
//! `{"text": "...", "reply": {"in_reply_to_tweet_id": "..."}}`.
//! For Python/tweepy callers the equivalent is
//! `client.create_tweet(text="...", in_reply_to_tweet_id=TWEET_ID)` — NOT the
//! legacy v1.1 `reply={"in_reply_to_tweet_id": ...}` dict form, which v2
//! rejects. Validated end-to-end via ZeusMarketing integration test
//! (see `memory/2026-04-23.md`).

use crate::{AgentSendIdentity, ChannelAdapter, ChannelMessage, ChannelSource, MediaFile, ReceiveMode};
use async_trait::async_trait;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::{Notify, RwLock, mpsc};
use zeus_core::{Error, Result};

type HmacSha1 = Hmac<Sha1>;

const X_API_BASE: &str = "https://api.x.com/2";

/// Build the X API v2 `create_tweet` request body from options.
///
/// Pure function so the mapping from [`CreateTweetOptions`] (text, reply_to,
/// media_ids, poll, schedule) to the wire JSON is unit-testable without a
/// network call. `post_tweet` sends exactly what this returns.
fn build_tweet_body(opts: &CreateTweetOptions) -> serde_json::Value {
    let mut body = serde_json::json!({
        "text": opts.text,
    });

    if let Some(ref reply_to) = opts.reply_to {
        body["reply"] = serde_json::json!({
            "in_reply_to_tweet_id": reply_to,
        });
    }

    if let Some(ref quote_id) = opts.quote_tweet_id {
        body["quote_tweet_id"] = serde_json::json!(quote_id);
    }

    if !opts.media_ids.is_empty() {
        body["media"] = serde_json::json!({
            "media_ids": opts.media_ids,
        });
    }

    if !opts.poll_options.is_empty() {
        body["poll"] = serde_json::json!({
            "options": opts.poll_options.iter().map(|o| serde_json::json!({"label": o})).collect::<Vec<_>>(),
            "duration_minutes": opts.poll_duration_minutes.unwrap_or(1440),
        });
    }

    if let Some(ref scheduled_at) = opts.scheduled_at {
        body["scheduled_at"] = serde_json::json!(scheduled_at);
    }

    body
}

/// Infer an X-supported MIME type from a filename's extension.
///
/// X accepts png/jpg/gif/webp images and mp4 video for tweet media. Returns an
/// error for anything else so the caller surfaces a clear message rather than
/// a failed upload.
fn mime_from_filename(filename: &str) -> Result<&'static str> {
    match std::path::Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Ok("image/png"),
        Some("jpg") | Some("jpeg") => Ok("image/jpeg"),
        Some("gif") => Ok("image/gif"),
        Some("webp") => Ok("image/webp"),
        Some("mp4") => Ok("video/mp4"),
        other => Err(Error::channel(format!(
            "Unsupported media type for X: {:?} (png/jpg/gif/webp/mp4 only)",
            other
        ))),
    }
}

fn parse_retry_after_seconds(value: &str) -> Option<u64> {
    value.trim().parse::<u64>().ok()
}

// ── Types ────────────────────────────────────────────────────────────────

/// A posted tweet
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tweet {
    /// Tweet ID
    pub id: String,
    /// Tweet text content
    pub text: String,
    /// Author username
    pub author_username: Option<String>,
    /// Author display name
    pub author_name: Option<String>,
    /// Created timestamp
    pub created_at: Option<String>,
    /// Metrics (likes, retweets, replies, impressions)
    pub metrics: Option<TweetMetrics>,
    /// Media attachments
    pub media: Vec<TweetMedia>,
    /// Conversation/thread ID
    pub conversation_id: Option<String>,
    /// ID of tweet this replies to
    pub in_reply_to: Option<String>,
}

/// Tweet engagement metrics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TweetMetrics {
    pub like_count: u64,
    pub retweet_count: u64,
    pub reply_count: u64,
    pub quote_count: u64,
    pub impression_count: u64,
    pub bookmark_count: u64,
}

/// Public account metrics returned by X API v2.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct XAccountMetrics {
    pub user_id: String,
    pub username: Option<String>,
    pub name: Option<String>,
    pub followers_count: u64,
    pub following_count: u64,
    pub tweet_count: u64,
    pub listed_count: u64,
}

/// Media attached to a tweet
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TweetMedia {
    pub media_id: String,
    pub media_type: MediaType,
    pub url: Option<String>,
    pub alt_text: Option<String>,
}

/// Types of media that can be uploaded
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaType {
    Image,
    Video,
    Gif,
}

/// Options for creating a tweet
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateTweetOptions {
    /// Tweet text (up to 280 chars, or 25,000 for long-form)
    pub text: String,
    /// Reply to this tweet ID (creates a thread if own tweet)
    pub reply_to: Option<String>,
    /// Quote this tweet ID
    pub quote_tweet_id: Option<String>,
    /// Media IDs to attach (upload first via upload_media)
    pub media_ids: Vec<String>,
    /// Poll options (2-4 choices)
    pub poll_options: Vec<String>,
    /// Poll duration in minutes (5-10080)
    pub poll_duration_minutes: Option<u32>,
    /// Schedule for future posting (ISO 8601)
    pub scheduled_at: Option<String>,
}

/// Options for a thread (multiple tweets)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadOptions {
    /// List of tweet texts in order
    pub tweets: Vec<String>,
    /// Media IDs per tweet (indexed same as tweets, empty vec for no media)
    pub media_per_tweet: Vec<Vec<String>>,
}

/// Cursor and result-limit options for X read/listening endpoints.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct XReadOptions {
    /// Return posts newer than this tweet ID.
    pub since_id: Option<String>,
    /// Return posts older than this tweet ID.
    pub until_id: Option<String>,
    /// X pagination token from a previous response.
    pub pagination_token: Option<String>,
    /// Requested page size. Clamped to endpoint-supported bounds.
    pub max_results: Option<u32>,
}

/// Page of tweets returned by X read/listening endpoints.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct XTweetList {
    pub tweets: Vec<Tweet>,
    pub newest_id: Option<String>,
    pub oldest_id: Option<String>,
    pub result_count: usize,
    pub next_token: Option<String>,
}

/// Cursor and result-limit options for X Direct Message read endpoints.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct XDmOptions {
    /// X pagination token from a previous response.
    pub pagination_token: Option<String>,
    /// Requested page size. Clamped to endpoint-supported bounds.
    pub max_results: Option<u32>,
}

/// Media attachment metadata included on an X Direct Message event.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct XDmAttachment {
    pub media_key: String,
    pub media_type: Option<String>,
    pub url: Option<String>,
    pub preview_image_url: Option<String>,
}

/// X Direct Message event returned by API v2 DM read endpoints.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct XDmEvent {
    pub id: String,
    pub text: Option<String>,
    pub event_type: Option<String>,
    pub created_at: Option<String>,
    pub dm_conversation_id: Option<String>,
    pub sender_id: Option<String>,
    pub participant_ids: Vec<String>,
    pub attachments: Vec<XDmAttachment>,
}

/// Page of X Direct Message events returned by API v2.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct XDmEventPage {
    pub events: Vec<XDmEvent>,
    pub result_count: usize,
    pub next_token: Option<String>,
}

/// Result returned after sending an X Direct Message.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct XDmSendResult {
    pub dm_event_id: Option<String>,
    pub dm_conversation_id: Option<String>,
    pub participant_id: Option<String>,
    pub text: Option<String>,
}

/// Cursor and result-limit options for X list endpoints.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct XListOptions {
    /// X pagination token from a previous response.
    pub pagination_token: Option<String>,
    /// Requested page size. Clamped to endpoint-supported bounds.
    pub max_results: Option<u32>,
}

/// X list metadata returned by API v2 list endpoints.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct XListInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub private: bool,
    pub member_count: u64,
    pub follower_count: u64,
    pub owner_id: Option<String>,
    pub created_at: Option<String>,
}

/// Page of X lists returned by API v2 list endpoints.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct XListPage {
    pub lists: Vec<XListInfo>,
    pub result_count: usize,
    pub next_token: Option<String>,
}

/// Result for mutating X list membership/follow/delete state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct XListMutationResult {
    pub action: String,
    pub list_id: String,
    pub user_id: Option<String>,
    pub success: bool,
}

impl XListMutationResult {
    fn new(
        action: impl Into<String>,
        list_id: impl Into<String>,
        user_id: Option<String>,
        success: bool,
    ) -> Self {
        Self {
            action: action.into(),
            list_id: list_id.into(),
            user_id,
            success,
        }
    }
}

/// Per-item status returned by X delete operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum XDeleteStatus {
    Deleted,
    Failed,
    Skipped,
}

/// Per-item result for single and batch X delete operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XDeleteResult {
    pub tweet_id: String,
    pub status: XDeleteStatus,
    pub index: usize,
    pub attempts: u32,
    pub http_status: Option<u16>,
    pub error: Option<String>,
}

impl XDeleteResult {
    fn deleted(tweet_id: impl Into<String>, index: usize, attempts: u32, http_status: u16) -> Self {
        Self {
            tweet_id: tweet_id.into(),
            status: XDeleteStatus::Deleted,
            index,
            attempts,
            http_status: Some(http_status),
            error: None,
        }
    }

    fn failed(
        tweet_id: impl Into<String>,
        index: usize,
        attempts: u32,
        http_status: Option<u16>,
        error: impl Into<String>,
    ) -> Self {
        Self {
            tweet_id: tweet_id.into(),
            status: XDeleteStatus::Failed,
            index,
            attempts,
            http_status,
            error: Some(error.into()),
        }
    }

    fn skipped(tweet_id: impl Into<String>, index: usize, error: impl Into<String>) -> Self {
        Self {
            tweet_id: tweet_id.into(),
            status: XDeleteStatus::Skipped,
            index,
            attempts: 0,
            http_status: None,
            error: Some(error.into()),
        }
    }
}

/// Summary returned by sequential X batch deletes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XBatchDeleteResult {
    pub results: Vec<XDeleteResult>,
    pub deleted: usize,
    pub failed: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct XActionResult {
    pub action: String,
    pub user_id: String,
    pub target_id: String,
    pub success: bool,
}

impl XActionResult {
    fn new(
        action: impl Into<String>,
        user_id: impl Into<String>,
        target_id: impl Into<String>,
        success: bool,
    ) -> Self {
        Self {
            action: action.into(),
            user_id: user_id.into(),
            target_id: target_id.into(),
            success,
        }
    }
}

impl XBatchDeleteResult {
    fn from_results(results: Vec<XDeleteResult>) -> Self {
        let deleted = results
            .iter()
            .filter(|r| r.status == XDeleteStatus::Deleted)
            .count();
        let failed = results
            .iter()
            .filter(|r| r.status == XDeleteStatus::Failed)
            .count();
        let skipped = results
            .iter()
            .filter(|r| r.status == XDeleteStatus::Skipped)
            .count();
        Self {
            results,
            deleted,
            failed,
            skipped,
        }
    }
}

/// User profile from X
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XUserProfile {
    pub id: String,
    pub username: String,
    pub name: String,
    pub bio: Option<String>,
    pub followers_count: u64,
    pub following_count: u64,
    pub tweet_count: u64,
    pub verified: bool,
    pub profile_image_url: Option<String>,
}

// ── Adapter ──────────────────────────────────────────────────────────────

/// X (Twitter) channel adapter
pub struct XAdapter {
    connected: Arc<AtomicBool>,
    config: XConfig,
    client: reqwest::Client,
    api_base: String,
    shutdown: Arc<Notify>,
    task_handle: RwLock<Option<tokio::task::JoinHandle<()>>>,
    /// Track the last seen mention ID for polling
    last_mention_id: Arc<RwLock<Option<String>>>,
    /// Live OAuth 2.0 user-context access token. When non-empty, takes priority
    /// over every other auth path for both reads and writes. Refreshable at
    /// runtime via `set_oauth2_token()` without rebuilding the adapter.
    oauth2_access_token: Arc<RwLock<String>>,
}

impl XAdapter {
    /// Create a new X adapter
    pub async fn new(config: XConfig) -> Result<Self> {
        // Adapter is usable if *any* credential path is configured:
        //   1. OAuth 2.0 user-context access token (PKCE flow result)
        //   2. OAuth 1.0a (api_key + secrets)
        //   3. OAuth 2.0 App-Only bearer token
        let has_any_cred = !config.oauth2_access_token.is_empty()
            || !config.bearer_token.is_empty()
            || !config.api_key.is_empty();
        if !has_any_cred {
            return Err(Error::Config(
                "X adapter requires oauth2_access_token, bearer_token, or api_key + api_secret"
                    .into(),
            ));
        }

        let initial_oauth2 = config.oauth2_access_token.clone();

        tracing::info!("X adapter created");

        Ok(Self {
            connected: Arc::new(AtomicBool::new(false)),
            config,
            client: reqwest::Client::new(),
            api_base: X_API_BASE.to_string(),
            shutdown: Arc::new(Notify::new()),
            task_handle: RwLock::new(None),
            last_mention_id: Arc::new(RwLock::new(None)),
            oauth2_access_token: Arc::new(RwLock::new(initial_oauth2)),
        })
    }

    #[cfg(test)]
    async fn new_with_base_url(config: XConfig, api_base: impl Into<String>) -> Result<Self> {
        let mut adapter = Self::new(config).await?;
        adapter.api_base = api_base.into().trim_end_matches('/').to_string();
        Ok(adapter)
    }

    /// Replace the live OAuth 2.0 user-context access token at runtime.
    ///
    /// Call this after completing PKCE authorization, or after a successful
    /// `refresh_oauth2_token()` exchange. Thread-safe — takes effect for all
    /// subsequent requests without restarting the adapter.
    pub async fn set_oauth2_token(&self, access_token: impl Into<String>) {
        let token = access_token.into();
        let is_clearing = token.is_empty();
        *self.oauth2_access_token.write().await = token;
        if is_clearing {
            tracing::info!("X adapter OAuth 2.0 token cleared");
        } else {
            tracing::info!("X adapter OAuth 2.0 token updated");
        }
    }

    /// Snapshot the current OAuth 2.0 access token (empty string if unset).
    pub async fn current_oauth2_token(&self) -> String {
        self.oauth2_access_token.read().await.clone()
    }

    /// Non-blocking best-effort read of the live OAuth 2.0 token.
    /// Returns empty string if the lock is contended — callers should treat
    /// that as "no OAuth 2.0 token available right now" and fall back.
    fn try_oauth2_token_snapshot(&self) -> String {
        self.oauth2_access_token
            .try_read()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    /// Get authorization header for read operations.
    ///
    /// Priority order:
    ///   1. OAuth 2.0 user-context access token (PKCE) — live, runtime-updatable
    ///   2. OAuth 2.0 App-Only bearer token
    ///   3. OAuth 1.0a (fallback)
    fn read_auth_header(&self) -> Result<String> {
        let oauth2 = self.try_oauth2_token_snapshot();
        if !oauth2.is_empty() {
            Ok(format!("Bearer {}", oauth2))
        } else if !self.config.bearer_token.is_empty() {
            Ok(format!("Bearer {}", self.config.bearer_token))
        } else {
            Err(Error::Channel(
                "X reads require bearer_token or OAuth 2.0 access token; OAuth 1.0a is only used for write endpoints"
                    .into(),
            ))
        }
    }

    /// Get authorization header for write operations.
    fn has_oauth1_user_tokens(&self) -> bool {
        !self.config.api_key.is_empty()
            && !self.config.api_secret.is_empty()
            && !self.config.access_token.is_empty()
            && !self.config.access_token_secret.is_empty()
    }

    fn user_context_auth_header(
        &self,
        method: &str,
        url: &str,
        query_params: &[(String, String)],
    ) -> Result<String> {
        let oauth2 = self.try_oauth2_token_snapshot();
        if !oauth2.is_empty() {
            return Ok(format!("Bearer {}", oauth2));
        }
        if self.has_oauth1_user_tokens() {
            let pairs = query_params
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect::<Vec<_>>();
            return Ok(self.oauth1_header(method, url, &pairs));
        }
        Err(Error::Channel(
            "X DMs require OAuth 2.0 user-context access token or OAuth 1.0a user tokens with DM scopes; app-only bearer tokens cannot access DMs"
                .into(),
        ))
    }

    ///
    /// Priority order:
    ///   1. OAuth 2.0 user-context access token (PKCE) — required for posting
    ///      tweets on most API tiers; the canonical write auth in 2024+.
    ///   2. OAuth 1.0a (legacy, still works for write endpoints with full
    ///      consumer + access token pair).
    ///   3. OAuth 2.0 App-Only bearer token (last resort — often read-only,
    ///      especially on free tier).
    fn write_auth_header(&self, method: &str, url: &str) -> String {
        self.write_auth_header_with_params(method, url, &[])
    }

    /// Like [`Self::write_auth_header`], but includes request body parameters
    /// in the OAuth 1.0a signature base string.
    ///
    /// REQUIRED for `application/x-www-form-urlencoded` requests (RFC 5849
    /// §3.4.1.3): X rejects form posts whose body params are missing from the
    /// signature with 401 Unauthorized. JSON and multipart bodies must NOT be
    /// signed — use [`Self::write_auth_header`] for those. The Bearer arms
    /// ignore `extra_params` (OAuth 2.0 does not sign bodies).
    fn write_auth_header_with_params(
        &self,
        method: &str,
        url: &str,
        extra_params: &[(&str, &str)],
    ) -> String {
        let oauth2 = self.try_oauth2_token_snapshot();
        if !oauth2.is_empty() {
            return format!("Bearer {}", oauth2);
        }

        let has_oauth1 = !self.config.api_key.is_empty()
            && !self.config.api_secret.is_empty()
            && !self.config.access_token.is_empty()
            && !self.config.access_token_secret.is_empty();
        if has_oauth1 {
            self.oauth1_header(method, url, extra_params)
        } else if !self.config.bearer_token.is_empty() {
            format!("Bearer {}", self.config.bearer_token)
        } else {
            String::new()
        }
    }

    /// Build a full OAuth 1.0a Authorization header for the given method/url/params
    fn oauth1_header(&self, method: &str, url: &str, extra_params: &[(&str, &str)]) -> String {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .to_string();

        let nonce: String = rand::thread_rng()
            .sample_iter(&rand::distributions::Alphanumeric)
            .take(32)
            .map(char::from)
            .collect();

        let mut params: Vec<(&str, String)> = vec![
            ("oauth_consumer_key", self.config.api_key.clone()),
            ("oauth_nonce", nonce.clone()),
            ("oauth_signature_method", "HMAC-SHA1".to_string()),
            ("oauth_timestamp", timestamp.clone()),
            ("oauth_token", self.config.access_token.clone()),
            ("oauth_version", "1.0".to_string()),
        ];
        for (k, v) in extra_params {
            params.push((k, v.to_string()));
        }

        // Sort params lexicographically for signature base string
        params.sort_by(|a, b| a.0.cmp(b.0));

        let param_string = params
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");

        let base_string = format!(
            "{}&{}&{}",
            urlencoding::encode(method),
            urlencoding::encode(url),
            urlencoding::encode(&param_string),
        );

        let signing_key = format!(
            "{}&{}",
            urlencoding::encode(&self.config.api_secret),
            urlencoding::encode(&self.config.access_token_secret),
        );

        let mut mac =
            HmacSha1::new_from_slice(signing_key.as_bytes()).expect("HMAC accepts any key length");
        mac.update(base_string.as_bytes());
        let signature =
            base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());

        format!(
            "OAuth oauth_consumer_key=\"{}\", oauth_nonce=\"{}\", oauth_signature=\"{}\", oauth_signature_method=\"HMAC-SHA1\", oauth_timestamp=\"{}\", oauth_token=\"{}\", oauth_version=\"1.0\"",
            urlencoding::encode(&self.config.api_key),
            urlencoding::encode(&nonce),
            urlencoding::encode(&signature),
            urlencoding::encode(&timestamp),
            urlencoding::encode(&self.config.access_token),
        )
    }

    /// Post a single tweet
    pub async fn post_tweet(&self, opts: &CreateTweetOptions) -> Result<Tweet> {
        let body = build_tweet_body(opts);

        let tweet_url = format!("{}/tweets", X_API_BASE);
        let resp = self
            .client
            .post(&tweet_url)
            .header("Authorization", self.write_auth_header("POST", &tweet_url))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("X API error: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_else(|_| "unknown error".into());
            return Err(Error::Channel(format!("X API error {}: {}", status, text)));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Channel(format!("X API parse error: {}", e)))?;

        let tweet_data = data
            .get("data")
            .ok_or_else(|| Error::Channel("X API: missing data in response".into()))?;

        Ok(Tweet {
            id: tweet_data["id"].as_str().unwrap_or_default().to_string(),
            text: tweet_data["text"].as_str().unwrap_or_default().to_string(),
            author_username: None,
            author_name: None,
            created_at: None,
            metrics: None,
            media: Vec::new(),
            conversation_id: None,
            in_reply_to: opts.reply_to.clone(),
        })
    }

    /// Post a thread (multiple tweets in sequence)
    pub async fn post_thread(&self, thread: &ThreadOptions) -> Result<Vec<Tweet>> {
        if thread.tweets.is_empty() {
            return Err(Error::Channel("Thread must have at least one tweet".into()));
        }

        let mut posted: Vec<Tweet> = Vec::new();
        let mut reply_to: Option<String> = None;

        for (i, text) in thread.tweets.iter().enumerate() {
            let media_ids = thread.media_per_tweet.get(i).cloned().unwrap_or_default();

            let opts = CreateTweetOptions {
                text: text.clone(),
                reply_to: reply_to.clone(),
                media_ids,
                ..Default::default()
            };

            let tweet = self.post_tweet(&opts).await?;
            reply_to = Some(tweet.id.clone());
            posted.push(tweet);

            // Small delay between tweets to avoid rate limits
            if i < thread.tweets.len() - 1 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }

        tracing::info!(count = posted.len(), "X thread posted");
        Ok(posted)
    }

    /// Upload media for attachment to a tweet
    pub async fn upload_media(
        &self,
        data: &[u8],
        mime_type: &str,
        alt_text: Option<&str>,
    ) -> Result<String> {
        // X API v2 media upload — contract verified against X's own OpenAPI
        // spec (api.x.com/2/openapi.json). The command=INIT/APPEND/FINALIZE
        // flow from X's sample code no longer exists: POST /2/media/upload
        // accepts NO query parameters ("query parameter [x] is not one of []").
        //   images/subtitles → ONE-SHOT multipart POST /2/media/upload
        //     (parts: media, media_category, media_type)
        //   video + gif → chunked: POST /2/media/upload/initialize (JSON body)
        //     → POST /2/media/upload/{id}/append (multipart media+segment_index)
        //     → POST /2/media/upload/{id}/finalize (empty body)
        // Multipart and JSON bodies contribute nothing to the OAuth 1.0a
        // signature base string (RFC 5849 §3.4.1.3) — plain auth header,
        // the same proven path `post_tweet` uses.
        let media_category = if mime_type.starts_with("video/") {
            "tweet_video"
        } else if mime_type == "image/gif" {
            "tweet_gif"
        } else {
            "tweet_image"
        };

        let media_id = if mime_type.starts_with("image/") && mime_type != "image/gif" {
            // ONE-SHOT (static images): a single multipart request.
            let upload_url = format!("{}/media/upload", self.api_base);
            let part = reqwest::multipart::Part::bytes(data.to_vec())
                .file_name("media")
                .mime_str(mime_type)
                .map_err(|e| Error::Channel(format!("MIME error: {}", e)))?;
            let form = reqwest::multipart::Form::new()
                .text("media_category", media_category)
                .text("media_type", mime_type.to_string())
                .part("media", part);
            let resp = self
                .client
                .post(&upload_url)
                .header("Authorization", self.write_auth_header("POST", &upload_url))
                .multipart(form)
                .send()
                .await
                .map_err(|e| Error::Channel(format!("X media upload error: {}", e)))?;

            if !resp.status().is_success() {
                let text = resp.text().await.unwrap_or_else(|_| "unknown".into());
                return Err(Error::Channel(format!("X media upload failed: {}", text)));
            }

            let body: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| Error::Channel(format!("X media upload parse: {}", e)))?;

            // v2 wraps the payload in `data` and names the field `id`;
            // fall back to the legacy `media_id_string` shape defensively.
            body["data"]["id"]
                .as_str()
                .or_else(|| body["media_id_string"].as_str())
                .ok_or_else(|| {
                    Error::Channel("X media: no media id in upload response".into())
                })?
                .to_string()
        } else {
            // CHUNKED (video/gif): initialize → append(s) → finalize.
            let init_url = format!("{}/media/upload/initialize", self.api_base);
            let init_resp = self
                .client
                .post(&init_url)
                .header("Authorization", self.write_auth_header("POST", &init_url))
                .json(&serde_json::json!({
                    "media_type": mime_type,
                    "total_bytes": data.len(),
                    "media_category": media_category,
                }))
                .send()
                .await
                .map_err(|e| Error::Channel(format!("X media INIT error: {}", e)))?;

            if !init_resp.status().is_success() {
                let text = init_resp.text().await.unwrap_or_else(|_| "unknown".into());
                return Err(Error::Channel(format!("X media INIT failed: {}", text)));
            }

            let init_data: serde_json::Value = init_resp
                .json()
                .await
                .map_err(|e| Error::Channel(format!("X media INIT parse: {}", e)))?;

            let media_id = init_data["data"]["id"]
                .as_str()
                .or_else(|| init_data["media_id_string"].as_str())
                .ok_or_else(|| {
                    Error::Channel("X media: no media id in INIT response".into())
                })?
                .to_string();

            // Segments capped well under X's per-APPEND limit.
            let chunk_size = 4 * 1024 * 1024;
            for (i, chunk) in data.chunks(chunk_size).enumerate() {
                let append_url = format!("{}/media/upload/{}/append", self.api_base, media_id);
                let part = reqwest::multipart::Part::bytes(chunk.to_vec())
                    .file_name("media")
                    .mime_str("application/octet-stream")
                    .map_err(|e| Error::Channel(format!("MIME error: {}", e)))?;
                let form = reqwest::multipart::Form::new()
                    .text("segment_index", i.to_string())
                    .part("media", part);

                let append_resp = self
                    .client
                    .post(&append_url)
                    .header("Authorization", self.write_auth_header("POST", &append_url))
                    .multipart(form)
                    .send()
                    .await
                    .map_err(|e| Error::Channel(format!("X media APPEND error: {}", e)))?;

                if !append_resp.status().is_success() {
                    let text = append_resp
                        .text()
                        .await
                        .unwrap_or_else(|_| "unknown".into());
                    return Err(Error::Channel(format!(
                        "X media APPEND chunk {} failed: {}",
                        i, text
                    )));
                }
            }

            let finalize_url = format!("{}/media/upload/{}/finalize", self.api_base, media_id);
            let finalize_resp = self
                .client
                .post(&finalize_url)
                .header(
                    "Authorization",
                    self.write_auth_header("POST", &finalize_url),
                )
                .send()
                .await
                .map_err(|e| Error::Channel(format!("X media FINALIZE error: {}", e)))?;

            if !finalize_resp.status().is_success() {
                let text = finalize_resp
                    .text()
                    .await
                    .unwrap_or_else(|_| "unknown".into());
                return Err(Error::Channel(format!("X media FINALIZE failed: {}", text)));
            }

            media_id
        };

        // Step 4: Set alt text if provided (v2 metadata endpoint)
        if let Some(alt) = alt_text {
            let meta_url = format!("{}/media/metadata", self.api_base);
            let _ = self
                .client
                .post(&meta_url)
                .header("Authorization", self.write_auth_header("POST", &meta_url))
                .json(&serde_json::json!({
                    "id": media_id,
                    "metadata": {
                        "alt_text": { "text": alt }
                    }
                }))
                .send()
                .await;
        }

        tracing::info!(media_id = %media_id, "X media uploaded");
        Ok(media_id)
    }

    /// Delete a tweet, returning an error for compatibility with the legacy `x_delete` tool.
    pub async fn delete_tweet(&self, tweet_id: &str) -> Result<()> {
        let result = self.delete_tweet_result(tweet_id).await;
        if result.status == XDeleteStatus::Deleted {
            return Ok(());
        }

        Err(Error::Channel(format!(
            "X delete tweet {} failed: {}",
            tweet_id,
            result.error.unwrap_or_else(|| "unknown".into())
        )))
    }

    /// Delete a single tweet/post with rate-limit retry and a structured per-item result.
    pub async fn delete_tweet_result(&self, tweet_id: &str) -> XDeleteResult {
        self.delete_tweet_result_at(tweet_id, 1).await
    }

    /// Delete tweets/posts sequentially. Every input receives an item result; one
    /// failure never aborts the rest of the batch.
    pub async fn batch_delete_tweets(&self, tweet_ids: &[String]) -> XBatchDeleteResult {
        let mut results = Vec::with_capacity(tweet_ids.len());
        for (index, tweet_id) in tweet_ids.iter().enumerate() {
            results.push(self.delete_tweet_result_at(tweet_id, index + 1).await);
        }
        XBatchDeleteResult::from_results(results)
    }

    // X POST actions need request metadata plus response contract fields in one shared helper.
    #[allow(clippy::too_many_arguments)]
    async fn post_user_action(
        &self,
        action: &str,
        user_id: &str,
        target_id: &str,
        path: &str,
        body: serde_json::Value,
        success_field: &str,
        expected: bool,
    ) -> Result<XActionResult> {
        let user_id = user_id.trim();
        let target_id = target_id.trim();
        if user_id.is_empty() || target_id.is_empty() {
            return Err(Error::Channel(format!(
                "X {action} requires user_id and target_id"
            )));
        }

        let url = format!("{}{}", self.api_base, path);
        let resp = self
            .client
            .post(&url)
            .header("Authorization", self.write_auth_header("POST", &url))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("X {action} error: {e}")))?;

        Self::parse_action_response(resp, action, user_id, target_id, success_field, expected).await
    }

    async fn delete_user_action(
        &self,
        action: &str,
        user_id: &str,
        target_id: &str,
        path: &str,
        success_field: &str,
        expected: bool,
    ) -> Result<XActionResult> {
        let user_id = user_id.trim();
        let target_id = target_id.trim();
        if user_id.is_empty() || target_id.is_empty() {
            return Err(Error::Channel(format!(
                "X {action} requires user_id and target_id"
            )));
        }

        let url = format!("{}{}", self.api_base, path);
        let resp = self
            .client
            .delete(&url)
            .header("Authorization", self.write_auth_header("DELETE", &url))
            .send()
            .await
            .map_err(|e| Error::Channel(format!("X {action} error: {e}")))?;

        Self::parse_action_response(resp, action, user_id, target_id, success_field, expected).await
    }

    async fn parse_action_response(
        resp: reqwest::Response,
        action: &str,
        user_id: &str,
        target_id: &str,
        success_field: &str,
        expected: bool,
    ) -> Result<XActionResult> {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_else(|_| "unknown".into());
        if !status.is_success() {
            return Err(Error::Channel(format!(
                "X {action} failed: {}: {}",
                status.as_u16(),
                text
            )));
        }

        let data: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({}));
        let success = data
            .get("data")
            .and_then(|d| d.get(success_field))
            .and_then(|v| v.as_bool())
            .map(|v| v == expected)
            .unwrap_or(true);

        Ok(XActionResult::new(action, user_id, target_id, success))
    }

    pub async fn like_tweet(&self, user_id: &str, tweet_id: &str) -> Result<XActionResult> {
        let uid = user_id.trim();
        let tid = tweet_id.trim();
        self.post_user_action(
            "like",
            uid,
            tid,
            &format!("/users/{uid}/likes"),
            serde_json::json!({ "tweet_id": tid }),
            "liked",
            true,
        )
        .await
    }

    pub async fn unlike_tweet(&self, user_id: &str, tweet_id: &str) -> Result<XActionResult> {
        let uid = user_id.trim();
        let tid = tweet_id.trim();
        self.delete_user_action(
            "unlike",
            uid,
            tid,
            &format!("/users/{uid}/likes/{tid}"),
            "liked",
            false,
        )
        .await
    }

    pub async fn retweet(&self, user_id: &str, tweet_id: &str) -> Result<XActionResult> {
        let uid = user_id.trim();
        let tid = tweet_id.trim();
        self.post_user_action(
            "retweet",
            uid,
            tid,
            &format!("/users/{uid}/retweets"),
            serde_json::json!({ "tweet_id": tid }),
            "retweeted",
            true,
        )
        .await
    }

    pub async fn unretweet(&self, user_id: &str, tweet_id: &str) -> Result<XActionResult> {
        let uid = user_id.trim();
        let tid = tweet_id.trim();
        self.delete_user_action(
            "unretweet",
            uid,
            tid,
            &format!("/users/{uid}/retweets/{tid}"),
            "retweeted",
            false,
        )
        .await
    }

    pub async fn quote_tweet(&self, text: &str, quote_tweet_id: &str) -> Result<Tweet> {
        if text.trim().is_empty() || quote_tweet_id.trim().is_empty() {
            return Err(Error::Channel(
                "X quote_tweet requires text and quote_tweet_id".into(),
            ));
        }
        self.post_tweet(&CreateTweetOptions {
            text: text.to_string(),
            quote_tweet_id: Some(quote_tweet_id.trim().to_string()),
            ..Default::default()
        })
        .await
    }

    pub async fn follow_user(&self, user_id: &str, target_user_id: &str) -> Result<XActionResult> {
        let uid = user_id.trim();
        let target = target_user_id.trim();
        self.post_user_action(
            "follow",
            uid,
            target,
            &format!("/users/{uid}/following"),
            serde_json::json!({ "target_user_id": target }),
            "following",
            true,
        )
        .await
    }

    pub async fn unfollow_user(
        &self,
        user_id: &str,
        target_user_id: &str,
    ) -> Result<XActionResult> {
        let uid = user_id.trim();
        let target = target_user_id.trim();
        self.delete_user_action(
            "unfollow",
            uid,
            target,
            &format!("/users/{uid}/following/{target}"),
            "following",
            false,
        )
        .await
    }

    pub async fn bookmark_tweet(&self, user_id: &str, tweet_id: &str) -> Result<XActionResult> {
        let uid = user_id.trim();
        let tid = tweet_id.trim();
        self.post_user_action(
            "bookmark",
            uid,
            tid,
            &format!("/users/{uid}/bookmarks"),
            serde_json::json!({ "tweet_id": tid }),
            "bookmarked",
            true,
        )
        .await
    }

    pub async fn unbookmark_tweet(&self, user_id: &str, tweet_id: &str) -> Result<XActionResult> {
        let uid = user_id.trim();
        let tid = tweet_id.trim();
        self.delete_user_action(
            "unbookmark",
            uid,
            tid,
            &format!("/users/{uid}/bookmarks/{tid}"),
            "bookmarked",
            false,
        )
        .await
    }

    async fn delete_tweet_result_at(&self, tweet_id: &str, index: usize) -> XDeleteResult {
        let tweet_id = tweet_id.trim();
        if tweet_id.is_empty() {
            return XDeleteResult::skipped(tweet_id, index, "tweet_id is empty");
        }

        const MAX_ATTEMPTS: u32 = 3;
        let delete_url = format!("{}/tweets/{}", self.api_base, tweet_id);
        for attempt in 1..=MAX_ATTEMPTS {
            let resp = self
                .client
                .delete(&delete_url)
                .header(
                    "Authorization",
                    self.write_auth_header("DELETE", &delete_url),
                )
                .send()
                .await;

            let resp = match resp {
                Ok(resp) => resp,
                Err(e) => {
                    return XDeleteResult::failed(
                        tweet_id,
                        index,
                        attempt,
                        None,
                        format!("X delete error: {e}"),
                    );
                }
            };

            let status = resp.status();
            if status.is_success() {
                tracing::info!(tweet_id = %tweet_id, attempts = attempt, "X tweet deleted");
                return XDeleteResult::deleted(tweet_id, index, attempt, status.as_u16());
            }

            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(parse_retry_after_seconds);
            let text = resp.text().await.unwrap_or_else(|_| "unknown".into());

            if status == reqwest::StatusCode::TOO_MANY_REQUESTS && attempt < MAX_ATTEMPTS {
                let delay = retry_after.unwrap_or_else(|| u64::from(attempt));
                tracing::warn!(
                    tweet_id = %tweet_id,
                    attempt,
                    delay_secs = delay,
                    "X delete rate-limited; backing off before retry"
                );
                if delay > 0 {
                    tokio::time::sleep(Duration::from_secs(delay)).await;
                }
                continue;
            }

            return XDeleteResult::failed(
                tweet_id,
                index,
                attempt,
                Some(status.as_u16()),
                format!("{}: {}", status.as_u16(), text),
            );
        }

        XDeleteResult::failed(
            tweet_id,
            index,
            MAX_ATTEMPTS,
            None,
            "delete retry budget exhausted",
        )
    }

    fn read_query_params(&self, opts: &XReadOptions, min_results: u32) -> Vec<(String, String)> {
        let mut params = vec![
            (
                "tweet.fields".to_string(),
                "id,text,author_id,created_at,public_metrics,conversation_id,referenced_tweets,attachments".to_string(),
            ),
            ("expansions".to_string(), "author_id,attachments.media_keys".to_string()),
            ("user.fields".to_string(), "id,username,name".to_string()),
            ("media.fields".to_string(), "media_key,type,url,alt_text".to_string()),
        ];

        let max_results = opts.max_results.unwrap_or(25).clamp(min_results, 100);
        params.push(("max_results".to_string(), max_results.to_string()));

        if let Some(since_id) = opts.since_id.as_deref().filter(|s| !s.trim().is_empty()) {
            params.push(("since_id".to_string(), since_id.trim().to_string()));
        }
        if let Some(until_id) = opts.until_id.as_deref().filter(|s| !s.trim().is_empty()) {
            params.push(("until_id".to_string(), until_id.trim().to_string()));
        }
        if let Some(token) = opts
            .pagination_token
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            params.push(("pagination_token".to_string(), token.trim().to_string()));
        }

        params
    }

    fn read_url(&self, path: &str, params: Vec<(String, String)>) -> String {
        let query = params
            .into_iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(&k), urlencoding::encode(&v)))
            .collect::<Vec<_>>()
            .join("&");
        format!("{}{}?{}", self.api_base, path, query)
    }

    async fn get_x_json(&self, url: String, label: &str) -> Result<serde_json::Value> {
        let resp = self
            .client
            .get(&url)
            .header("Authorization", self.read_auth_header()?)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("X {label} error: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_else(|_| "unknown".into());
            return Err(Error::Channel(format!(
                "X {label} failed: {}: {}",
                status.as_u16(),
                text
            )));
        }

        resp.json()
            .await
            .map_err(|e| Error::Channel(format!("X {label} parse: {e}")))
    }

    fn parse_tweet(value: &serde_json::Value, includes: &serde_json::Value) -> Tweet {
        let author_id = value["author_id"].as_str();
        let (author_username, author_name) = author_id
            .and_then(|id| {
                includes
                    .get("users")
                    .and_then(|users| users.as_array())
                    .and_then(|users| users.iter().find(|u| u["id"].as_str() == Some(id)))
                    .map(|u| {
                        (
                            u["username"].as_str().map(|s| s.to_string()),
                            u["name"].as_str().map(|s| s.to_string()),
                        )
                    })
            })
            .unwrap_or((None, None));

        let metrics = value.get("public_metrics").map(|metrics| TweetMetrics {
            like_count: metrics["like_count"].as_u64().unwrap_or(0),
            retweet_count: metrics["retweet_count"].as_u64().unwrap_or(0),
            reply_count: metrics["reply_count"].as_u64().unwrap_or(0),
            quote_count: metrics["quote_count"].as_u64().unwrap_or(0),
            impression_count: metrics["impression_count"].as_u64().unwrap_or(0),
            bookmark_count: metrics["bookmark_count"].as_u64().unwrap_or(0),
        });

        let media_keys = value
            .get("attachments")
            .and_then(|a| a.get("media_keys"))
            .and_then(|keys| keys.as_array())
            .into_iter()
            .flatten()
            .filter_map(|key| key.as_str())
            .collect::<Vec<_>>();
        let media = media_keys
            .iter()
            .filter_map(|key| {
                includes
                    .get("media")
                    .and_then(|media| media.as_array())
                    .and_then(|media| media.iter().find(|m| m["media_key"].as_str() == Some(*key)))
                    .map(|m| TweetMedia {
                        media_id: (*key).to_string(),
                        media_type: match m["type"].as_str().unwrap_or("photo") {
                            "video" => MediaType::Video,
                            "animated_gif" => MediaType::Gif,
                            _ => MediaType::Image,
                        },
                        url: m["url"].as_str().map(|s| s.to_string()),
                        alt_text: m["alt_text"].as_str().map(|s| s.to_string()),
                    })
            })
            .collect();

        let in_reply_to = value
            .get("referenced_tweets")
            .and_then(|refs| refs.as_array())
            .and_then(|refs| {
                refs.iter()
                    .find(|r| r["type"].as_str() == Some("replied_to"))
                    .and_then(|r| r["id"].as_str())
            })
            .map(|s| s.to_string());

        Tweet {
            id: value["id"].as_str().unwrap_or_default().to_string(),
            text: value["text"].as_str().unwrap_or_default().to_string(),
            author_username,
            author_name,
            created_at: value["created_at"].as_str().map(|s| s.to_string()),
            metrics,
            media,
            conversation_id: value["conversation_id"].as_str().map(|s| s.to_string()),
            in_reply_to,
        }
    }

    fn parse_tweet_list(data: serde_json::Value) -> XTweetList {
        let includes = data.get("includes").cloned().unwrap_or_default();
        let tweets = match data.get("data") {
            Some(value) if value.is_array() => value
                .as_array()
                .into_iter()
                .flatten()
                .map(|tweet| Self::parse_tweet(tweet, &includes))
                .collect::<Vec<_>>(),
            Some(value) if value.is_object() => vec![Self::parse_tweet(value, &includes)],
            _ => Vec::new(),
        };
        let meta = &data["meta"];
        XTweetList {
            result_count: meta["result_count"].as_u64().unwrap_or(tweets.len() as u64) as usize,
            newest_id: meta["newest_id"].as_str().map(|s| s.to_string()),
            oldest_id: meta["oldest_id"].as_str().map(|s| s.to_string()),
            next_token: meta["next_token"].as_str().map(|s| s.to_string()),
            tweets,
        }
    }

    fn list_query_params(&self, opts: &XListOptions) -> Vec<(String, String)> {
        let mut params = vec![(
            "list.fields".to_string(),
            "id,name,description,private,member_count,follower_count,owner_id,created_at"
                .to_string(),
        )];

        let max_results = opts.max_results.unwrap_or(25).clamp(1, 100);
        params.push(("max_results".to_string(), max_results.to_string()));

        if let Some(token) = opts
            .pagination_token
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            params.push(("pagination_token".to_string(), token.trim().to_string()));
        }

        params
    }

    fn parse_list_info(value: &serde_json::Value) -> XListInfo {
        XListInfo {
            id: value["id"].as_str().unwrap_or_default().to_string(),
            name: value["name"].as_str().unwrap_or_default().to_string(),
            description: value["description"].as_str().map(|s| s.to_string()),
            private: value["private"].as_bool().unwrap_or(false),
            member_count: value["member_count"].as_u64().unwrap_or(0),
            follower_count: value["follower_count"].as_u64().unwrap_or(0),
            owner_id: value["owner_id"].as_str().map(|s| s.to_string()),
            created_at: value["created_at"].as_str().map(|s| s.to_string()),
        }
    }

    fn parse_list_page(data: serde_json::Value) -> XListPage {
        let lists = match data.get("data") {
            Some(value) if value.is_array() => value
                .as_array()
                .unwrap()
                .iter()
                .map(Self::parse_list_info)
                .collect::<Vec<_>>(),
            Some(value) if value.is_object() => vec![Self::parse_list_info(value)],
            _ => Vec::new(),
        };
        let meta = &data["meta"];
        XListPage {
            result_count: meta["result_count"].as_u64().unwrap_or(lists.len() as u64) as usize,
            next_token: meta["next_token"].as_str().map(|s| s.to_string()),
            lists,
        }
    }

    fn dm_query_params(&self, opts: &XDmOptions) -> Vec<(String, String)> {
        let mut params = vec![
            (
                "dm_event.fields".to_string(),
                "id,text,event_type,created_at,dm_conversation_id,sender_id,participant_ids,attachments,referenced_tweets".to_string(),
            ),
            (
                "expansions".to_string(),
                "sender_id,participant_ids,attachments.media_keys,referenced_tweets.id".to_string(),
            ),
            (
                "user.fields".to_string(),
                "id,name,username,verified,profile_image_url".to_string(),
            ),
            (
                "media.fields".to_string(),
                "media_key,type,url,preview_image_url".to_string(),
            ),
            (
                "max_results".to_string(),
                opts.max_results.unwrap_or(100).clamp(1, 100).to_string(),
            ),
        ];
        if let Some(token) = opts
            .pagination_token
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            params.push(("pagination_token".to_string(), token.to_string()));
        }
        params
    }

    fn parse_dm_event(value: &serde_json::Value, includes: &serde_json::Value) -> XDmEvent {
        let media_keys = value
            .get("attachments")
            .and_then(|a| a.get("media_keys"))
            .and_then(|keys| keys.as_array())
            .into_iter()
            .flatten()
            .filter_map(|key| key.as_str())
            .collect::<Vec<_>>();
        let attachments = media_keys
            .iter()
            .map(|key| {
                let media = includes
                    .get("media")
                    .and_then(|m| m.as_array())
                    .and_then(|items| items.iter().find(|m| m["media_key"].as_str() == Some(*key)));
                XDmAttachment {
                    media_key: (*key).to_string(),
                    media_type: media
                        .and_then(|m| m.get("type"))
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    url: media
                        .and_then(|m| m.get("url"))
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    preview_image_url: media
                        .and_then(|m| m.get("preview_image_url"))
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                }
            })
            .collect();

        XDmEvent {
            id: value["id"].as_str().unwrap_or_default().to_string(),
            text: value["text"].as_str().map(str::to_string),
            event_type: value["event_type"].as_str().map(str::to_string),
            created_at: value["created_at"].as_str().map(str::to_string),
            dm_conversation_id: value["dm_conversation_id"].as_str().map(str::to_string),
            sender_id: value["sender_id"].as_str().map(str::to_string),
            participant_ids: value
                .get("participant_ids")
                .and_then(|ids| ids.as_array())
                .into_iter()
                .flatten()
                .filter_map(|id| id.as_str().map(str::to_string))
                .collect(),
            attachments,
        }
    }

    fn parse_dm_event_page(data: serde_json::Value) -> XDmEventPage {
        let includes = data.get("includes").cloned().unwrap_or_default();
        let events = data
            .get("data")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .map(|event| Self::parse_dm_event(event, &includes))
            .collect::<Vec<_>>();
        let meta = data.get("meta").cloned().unwrap_or_default();
        XDmEventPage {
            result_count: meta["result_count"].as_u64().unwrap_or(events.len() as u64) as usize,
            next_token: meta["next_token"].as_str().map(str::to_string),
            events,
        }
    }

    fn parse_dm_send(data: serde_json::Value, participant_id: Option<String>) -> XDmSendResult {
        let event = data.get("data").unwrap_or(&data);
        XDmSendResult {
            dm_event_id: event
                .get("dm_event_id")
                .or_else(|| event.get("id"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
            dm_conversation_id: event
                .get("dm_conversation_id")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            participant_id,
            text: event
                .get("text")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        }
    }

    async fn get_dm_json(
        &self,
        action: &str,
        path: &str,
        params: Vec<(String, String)>,
    ) -> Result<serde_json::Value> {
        let query = params
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        let bare_url = format!("{}{}", self.api_base, path);
        let url = if query.is_empty() {
            bare_url.clone()
        } else {
            format!("{}?{}", bare_url, query)
        };
        let resp = self
            .client
            .get(&url)
            .header(
                "Authorization",
                self.user_context_auth_header("GET", &bare_url, &params)?,
            )
            .send()
            .await
            .map_err(|e| Error::Channel(format!("X {action} error: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_else(|_| "unknown".into());
            return Err(Error::Channel(format!(
                "X {action} failed: {}: {}",
                status.as_u16(),
                text
            )));
        }

        resp.json()
            .await
            .map_err(|e| Error::Channel(format!("X {action} parse error: {e}")))
    }

    async fn post_dm_json(
        &self,
        action: &str,
        path: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let url = format!("{}{}", self.api_base, path);
        let resp = self
            .client
            .post(&url)
            .header(
                "Authorization",
                self.user_context_auth_header("POST", &url, &[])?,
            )
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("X {action} error: {e}")))?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_else(|_| "unknown".into());
        if !status.is_success() {
            return Err(Error::Channel(format!(
                "X {action} failed: {}: {}",
                status.as_u16(),
                text
            )));
        }

        Ok(serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({})))
    }

    fn parse_list_mutation(
        data: serde_json::Value,
        action: &str,
        list_id: &str,
        user_id: Option<String>,
        success_field: &str,
        expected: bool,
    ) -> XListMutationResult {
        let success = data
            .get("data")
            .and_then(|d| d.get(success_field))
            .and_then(|v| v.as_bool())
            .map(|v| v == expected)
            .unwrap_or(true);
        XListMutationResult::new(action, list_id.trim(), user_id, success)
    }

    async fn post_list_json(
        &self,
        action: &str,
        path: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let url = format!("{}{}", self.api_base, path);
        let resp = self
            .client
            .post(&url)
            .header("Authorization", self.write_auth_header("POST", &url))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("X {action} error: {e}")))?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_else(|_| "unknown".into());
        if !status.is_success() {
            return Err(Error::Channel(format!(
                "X {action} failed: {}: {}",
                status.as_u16(),
                text
            )));
        }

        Ok(serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({})))
    }

    async fn put_list_json(
        &self,
        action: &str,
        path: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let url = format!("{}{}", self.api_base, path);
        let resp = self
            .client
            .put(&url)
            .header("Authorization", self.write_auth_header("PUT", &url))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("X {action} error: {e}")))?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_else(|_| "unknown".into());
        if !status.is_success() {
            return Err(Error::Channel(format!(
                "X {action} failed: {}: {}",
                status.as_u16(),
                text
            )));
        }

        Ok(serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({})))
    }

    async fn delete_json(&self, action: &str, path: &str) -> Result<serde_json::Value> {
        let url = format!("{}{}", self.api_base, path);
        let resp = self
            .client
            .delete(&url)
            .header("Authorization", self.write_auth_header("DELETE", &url))
            .send()
            .await
            .map_err(|e| Error::Channel(format!("X {action} error: {e}")))?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_else(|_| "unknown".into());
        if !status.is_success() {
            return Err(Error::Channel(format!(
                "X {action} failed: {}: {}",
                status.as_u16(),
                text
            )));
        }

        Ok(serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({})))
    }

    // Search recent public X posts by keyword/query. Use since_id/pagination_token for polling.
    pub async fn block_user(&self, user_id: &str, target_user_id: &str) -> Result<XActionResult> {
        let uid = user_id.trim();
        let target = target_user_id.trim();
        self.post_user_action(
            "block",
            uid,
            target,
            &format!("/users/{uid}/blocking"),
            serde_json::json!({ "target_user_id": target }),
            "blocking",
            true,
        )
        .await
    }

    pub async fn unblock_user(&self, user_id: &str, target_user_id: &str) -> Result<XActionResult> {
        let uid = user_id.trim();
        let target = target_user_id.trim();
        self.delete_user_action(
            "unblock",
            uid,
            target,
            &format!("/users/{uid}/blocking/{target}"),
            "blocking",
            false,
        )
        .await
    }

    pub async fn mute_user(&self, user_id: &str, target_user_id: &str) -> Result<XActionResult> {
        let uid = user_id.trim();
        let target = target_user_id.trim();
        self.post_user_action(
            "mute",
            uid,
            target,
            &format!("/users/{uid}/muting"),
            serde_json::json!({ "target_user_id": target }),
            "muting",
            true,
        )
        .await
    }

    pub async fn unmute_user(&self, user_id: &str, target_user_id: &str) -> Result<XActionResult> {
        let uid = user_id.trim();
        let target = target_user_id.trim();
        self.delete_user_action(
            "unmute",
            uid,
            target,
            &format!("/users/{uid}/muting/{target}"),
            "muting",
            false,
        )
        .await
    }

    pub async fn hide_reply(&self, tweet_id: &str) -> Result<XActionResult> {
        self.set_reply_hidden(tweet_id, true).await
    }

    pub async fn unhide_reply(&self, tweet_id: &str) -> Result<XActionResult> {
        self.set_reply_hidden(tweet_id, false).await
    }

    async fn set_reply_hidden(&self, tweet_id: &str, hidden: bool) -> Result<XActionResult> {
        let tweet_id = tweet_id.trim();
        if tweet_id.is_empty() {
            return Err(Error::Channel("X hide/unhide requires tweet_id".into()));
        }
        let url = format!("{}/tweets/{tweet_id}/hidden", self.api_base);
        let resp = self
            .client
            .put(&url)
            .header("Authorization", self.write_auth_header("PUT", &url))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({ "hidden": hidden }))
            .send()
            .await
            .map_err(|e| Error::Channel(format!("X hide reply error: {e}")))?;
        Self::parse_action_response(
            resp,
            if hidden { "hide_reply" } else { "unhide_reply" },
            "",
            tweet_id,
            "hidden",
            hidden,
        )
        .await
    }

    pub async fn report_tweet(
        &self,
        tweet_id: &str,
        reason: Option<&str>,
    ) -> Result<XActionResult> {
        if tweet_id.trim().is_empty() {
            return Err(Error::Channel("X report tweet requires tweet_id".into()));
        }
        let _ = reason;
        Err(Error::Channel(
            "X report_tweet is not available through the public X API v2 for standard developer apps; use the native X safety flow or an elevated enterprise endpoint"
                .into(),
        ))
    }

    pub async fn search_recent(&self, query: &str, opts: &XReadOptions) -> Result<XTweetList> {
        if query.trim().is_empty() {
            return Err(Error::Channel("X recent search query is required".into()));
        }
        let mut params = self.read_query_params(opts, 10);
        params.push(("query".to_string(), query.trim().to_string()));
        let url = self.read_url("/tweets/search/recent", params);
        let data = self.get_x_json(url, "recent search").await?;
        Ok(Self::parse_tweet_list(data))
    }

    /// Get recent mentions for a user ID. Use since_id/pagination_token for polling.
    pub async fn get_mentions(&self, user_id: &str, opts: &XReadOptions) -> Result<XTweetList> {
        if user_id.trim().is_empty() {
            return Err(Error::Channel("X mentions user_id is required".into()));
        }
        let params = self.read_query_params(opts, 5);
        let url = self.read_url(&format!("/users/{}/mentions", user_id.trim()), params);
        let data = self.get_x_json(url, "mentions").await?;
        Ok(Self::parse_tweet_list(data))
    }

    /// Get one X post/tweet by ID.
    pub async fn get_tweet(&self, tweet_id: &str) -> Result<Tweet> {
        if tweet_id.trim().is_empty() {
            return Err(Error::Channel("X tweet_id is required".into()));
        }
        let params = vec![
            (
                "tweet.fields".to_string(),
                "id,text,author_id,created_at,public_metrics,conversation_id,referenced_tweets,attachments".to_string(),
            ),
            ("expansions".to_string(), "author_id,attachments.media_keys".to_string()),
            ("user.fields".to_string(), "id,username,name".to_string()),
            ("media.fields".to_string(), "media_key,type,url,alt_text".to_string()),
        ];
        let url = self.read_url(&format!("/tweets/{}", tweet_id.trim()), params);
        let data = self.get_x_json(url, "get tweet").await?;
        Self::parse_tweet_list(data)
            .tweets
            .into_iter()
            .next()
            .ok_or_else(|| Error::Channel(format!("X tweet {} not found", tweet_id.trim())))
    }

    /// Get recent posts for a user ID. Use since_id/pagination_token for polling.
    pub async fn get_user_timeline(
        &self,
        user_id: &str,
        opts: &XReadOptions,
    ) -> Result<XTweetList> {
        if user_id.trim().is_empty() {
            return Err(Error::Channel("X timeline user_id is required".into()));
        }
        let params = self.read_query_params(opts, 5);
        let url = self.read_url(&format!("/users/{}/tweets", user_id.trim()), params);
        let data = self.get_x_json(url, "user timeline").await?;
        Ok(Self::parse_tweet_list(data))
    }

    /// Read recent Direct Message events for the authenticated user. Use pagination_token for polling.
    pub async fn get_dm_events(&self, opts: &XDmOptions) -> Result<XDmEventPage> {
        let params = self.dm_query_params(opts);
        let data = self.get_dm_json("dm events", "/dm_events", params).await?;
        Ok(Self::parse_dm_event_page(data))
    }

    /// Read Direct Message events for one conversation. Use pagination_token for polling.
    pub async fn get_dm_conversation_events(
        &self,
        dm_conversation_id: &str,
        opts: &XDmOptions,
    ) -> Result<XDmEventPage> {
        let conversation = dm_conversation_id.trim();
        if conversation.is_empty() {
            return Err(Error::Channel("X DM conversation_id is required".into()));
        }
        let params = self.dm_query_params(opts);
        let data = self
            .get_dm_json(
                "dm conversation events",
                &format!("/dm_conversations/{conversation}/dm_events"),
                params,
            )
            .await?;
        Ok(Self::parse_dm_event_page(data))
    }

    /// Send a Direct Message into an existing X DM conversation.
    pub async fn send_dm(
        &self,
        dm_conversation_id: &str,
        text: &str,
        media_id: Option<&str>,
    ) -> Result<XDmSendResult> {
        let conversation = dm_conversation_id.trim();
        let text = text.trim();
        if conversation.is_empty() || text.is_empty() {
            return Err(Error::Channel(
                "X send DM requires dm_conversation_id and text".into(),
            ));
        }
        let mut body = serde_json::json!({ "text": text });
        if let Some(media_id) = media_id.map(str::trim).filter(|s| !s.is_empty()) {
            body["attachments"] = serde_json::json!([{ "media_id": media_id }]);
        }
        let data = self
            .post_dm_json(
                "send dm",
                &format!("/dm_conversations/{conversation}/messages"),
                body,
            )
            .await?;
        Ok(Self::parse_dm_send(data, None))
    }

    /// Send a Direct Message to a user, creating/reusing the one-to-one conversation.
    pub async fn send_dm_to_user(
        &self,
        participant_id: &str,
        text: &str,
        media_id: Option<&str>,
    ) -> Result<XDmSendResult> {
        let participant = participant_id.trim();
        let text = text.trim();
        if participant.is_empty() || text.is_empty() {
            return Err(Error::Channel(
                "X send DM to user requires user_id and text".into(),
            ));
        }
        let mut body = serde_json::json!({ "text": text });
        if let Some(media_id) = media_id.map(str::trim).filter(|s| !s.is_empty()) {
            body["attachments"] = serde_json::json!([{ "media_id": media_id }]);
        }
        let data = self
            .post_dm_json(
                "send dm to user",
                &format!("/dm_conversations/with/{participant}/messages"),
                body,
            )
            .await?;
        Ok(Self::parse_dm_send(data, Some(participant.to_string())))
    }

    /// Get metadata for a single X list by ID.
    pub async fn get_list(&self, list_id: &str) -> Result<XListInfo> {
        if list_id.trim().is_empty() {
            return Err(Error::Channel("X list_id is required".into()));
        }
        let params = vec![(
            "list.fields".to_string(),
            "id,name,description,private,member_count,follower_count,owner_id,created_at"
                .to_string(),
        )];
        let url = self.read_url(&format!("/lists/{}", list_id.trim()), params);
        let data = self.get_x_json(url, "get list").await?;
        data.get("data")
            .map(Self::parse_list_info)
            .ok_or_else(|| Error::Channel(format!("X list {} not found", list_id.trim())))
    }

    /// Get lists owned by a user ID.
    pub async fn get_owned_lists(&self, user_id: &str, opts: &XListOptions) -> Result<XListPage> {
        if user_id.trim().is_empty() {
            return Err(Error::Channel("X owned lists user_id is required".into()));
        }
        let url = self.read_url(
            &format!("/users/{}/owned_lists", user_id.trim()),
            self.list_query_params(opts),
        );
        let data = self.get_x_json(url, "owned lists").await?;
        Ok(Self::parse_list_page(data))
    }

    /// Get lists a user is a member of.
    pub async fn get_list_memberships(
        &self,
        user_id: &str,
        opts: &XListOptions,
    ) -> Result<XListPage> {
        if user_id.trim().is_empty() {
            return Err(Error::Channel(
                "X list memberships user_id is required".into(),
            ));
        }
        let url = self.read_url(
            &format!("/users/{}/list_memberships", user_id.trim()),
            self.list_query_params(opts),
        );
        let data = self.get_x_json(url, "list memberships").await?;
        Ok(Self::parse_list_page(data))
    }

    /// Get lists followed by a user ID.
    pub async fn get_followed_lists(
        &self,
        user_id: &str,
        opts: &XListOptions,
    ) -> Result<XListPage> {
        if user_id.trim().is_empty() {
            return Err(Error::Channel(
                "X followed lists user_id is required".into(),
            ));
        }
        let url = self.read_url(
            &format!("/users/{}/followed_lists", user_id.trim()),
            self.list_query_params(opts),
        );
        let data = self.get_x_json(url, "followed lists").await?;
        Ok(Self::parse_list_page(data))
    }

    /// Get recent posts from a list. Use since_id/pagination_token for polling.
    pub async fn get_list_tweets(&self, list_id: &str, opts: &XReadOptions) -> Result<XTweetList> {
        if list_id.trim().is_empty() {
            return Err(Error::Channel("X list_tweets list_id is required".into()));
        }
        let params = self.read_query_params(opts, 5);
        let url = self.read_url(&format!("/lists/{}/tweets", list_id.trim()), params);
        let data = self.get_x_json(url, "list tweets").await?;
        Ok(Self::parse_tweet_list(data))
    }

    /// Create an X list.
    pub async fn create_list(
        &self,
        name: &str,
        description: Option<&str>,
        private: Option<bool>,
    ) -> Result<XListInfo> {
        if name.trim().is_empty() {
            return Err(Error::Channel("X list name is required".into()));
        }
        let mut body = serde_json::json!({ "name": name.trim() });
        if let Some(description) = description.map(str::trim).filter(|s| !s.is_empty()) {
            body["description"] = serde_json::json!(description);
        }
        if let Some(private) = private {
            body["private"] = serde_json::json!(private);
        }
        let data = self.post_list_json("create list", "/lists", body).await?;
        data.get("data")
            .map(Self::parse_list_info)
            .ok_or_else(|| Error::Channel("X create list returned no list data".into()))
    }

    /// Update an X list's metadata.
    pub async fn update_list(
        &self,
        list_id: &str,
        name: Option<&str>,
        description: Option<&str>,
        private: Option<bool>,
    ) -> Result<XListInfo> {
        if list_id.trim().is_empty() {
            return Err(Error::Channel("X list_id is required".into()));
        }
        let mut body = serde_json::json!({});
        if let Some(name) = name.map(str::trim).filter(|s| !s.is_empty()) {
            body["name"] = serde_json::json!(name);
        }
        if let Some(description) = description.map(str::trim) {
            body["description"] = serde_json::json!(description);
        }
        if let Some(private) = private {
            body["private"] = serde_json::json!(private);
        }
        if body.as_object().map(|o| o.is_empty()).unwrap_or(true) {
            return Err(Error::Channel(
                "X update list requires name, description, or private".into(),
            ));
        }
        let data = self
            .put_list_json("update list", &format!("/lists/{}", list_id.trim()), body)
            .await?;
        data.get("data")
            .map(Self::parse_list_info)
            .ok_or_else(|| Error::Channel("X update list returned no list data".into()))
    }

    /// Delete an X list.
    pub async fn delete_list(&self, list_id: &str) -> Result<XListMutationResult> {
        if list_id.trim().is_empty() {
            return Err(Error::Channel("X list_id is required".into()));
        }
        let path = format!("/lists/{}", list_id.trim());
        let data = self.delete_json("delete list", &path).await?;
        Ok(Self::parse_list_mutation(
            data,
            "delete_list",
            list_id,
            None,
            "deleted",
            true,
        ))
    }

    /// Add a user to an X list.
    pub async fn add_list_member(
        &self,
        list_id: &str,
        user_id: &str,
    ) -> Result<XListMutationResult> {
        let list = list_id.trim();
        let user = user_id.trim();
        if list.is_empty() || user.is_empty() {
            return Err(Error::Channel(
                "X add list member requires list_id and user_id".into(),
            ));
        }
        let data = self
            .post_list_json(
                "add list member",
                &format!("/lists/{list}/members"),
                serde_json::json!({ "user_id": user }),
            )
            .await?;
        Ok(Self::parse_list_mutation(
            data,
            "add_list_member",
            list,
            Some(user.to_string()),
            "is_member",
            true,
        ))
    }

    /// Remove a user from an X list.
    pub async fn remove_list_member(
        &self,
        list_id: &str,
        user_id: &str,
    ) -> Result<XListMutationResult> {
        let list = list_id.trim();
        let user = user_id.trim();
        if list.is_empty() || user.is_empty() {
            return Err(Error::Channel(
                "X remove list member requires list_id and user_id".into(),
            ));
        }
        let data = self
            .delete_json(
                "remove list member",
                &format!("/lists/{list}/members/{user}"),
            )
            .await?;
        Ok(Self::parse_list_mutation(
            data,
            "remove_list_member",
            list,
            Some(user.to_string()),
            "is_member",
            false,
        ))
    }

    /// Follow an X list as a user.
    pub async fn follow_list(&self, user_id: &str, list_id: &str) -> Result<XListMutationResult> {
        let user = user_id.trim();
        let list = list_id.trim();
        if user.is_empty() || list.is_empty() {
            return Err(Error::Channel(
                "X follow list requires user_id and list_id".into(),
            ));
        }
        let data = self
            .post_list_json(
                "follow list",
                &format!("/users/{user}/followed_lists"),
                serde_json::json!({ "list_id": list }),
            )
            .await?;
        Ok(Self::parse_list_mutation(
            data,
            "follow_list",
            list,
            Some(user.to_string()),
            "following",
            true,
        ))
    }

    /// Unfollow an X list as a user.
    pub async fn unfollow_list(&self, user_id: &str, list_id: &str) -> Result<XListMutationResult> {
        let user = user_id.trim();
        let list = list_id.trim();
        if user.is_empty() || list.is_empty() {
            return Err(Error::Channel(
                "X unfollow list requires user_id and list_id".into(),
            ));
        }
        let data = self
            .delete_json(
                "unfollow list",
                &format!("/users/{user}/followed_lists/{list}"),
            )
            .await?;
        Ok(Self::parse_list_mutation(
            data,
            "unfollow_list",
            list,
            Some(user.to_string()),
            "following",
            false,
        ))
    }

    /// Get tweet metrics
    pub async fn get_tweet_metrics(&self, tweet_id: &str) -> Result<TweetMetrics> {
        let resp = self
            .client
            .get(format!(
                "{}/tweets/{}?tweet.fields=public_metrics",
                self.api_base, tweet_id
            ))
            .header("Authorization", self.read_auth_header()?)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("X metrics error: {}", e)))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_else(|_| "unknown".into());
            return Err(Error::Channel(format!("X get metrics failed: {}", text)));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Channel(format!("X metrics parse: {}", e)))?;

        let metrics = &data["data"]["public_metrics"];
        Ok(TweetMetrics {
            like_count: metrics["like_count"].as_u64().unwrap_or(0),
            retweet_count: metrics["retweet_count"].as_u64().unwrap_or(0),
            reply_count: metrics["reply_count"].as_u64().unwrap_or(0),
            quote_count: metrics["quote_count"].as_u64().unwrap_or(0),
            impression_count: metrics["impression_count"].as_u64().unwrap_or(0),
            bookmark_count: metrics["bookmark_count"].as_u64().unwrap_or(0),
        })
    }

    /// Get public account metrics by user ID.
    pub async fn get_account_metrics(&self, user_id: &str) -> Result<XAccountMetrics> {
        let user_id = user_id.trim();
        if user_id.is_empty() {
            return Err(Error::Channel("X account metrics requires user_id".into()));
        }

        let url = format!(
            "{}/users/{}?user.fields=public_metrics",
            self.api_base, user_id
        );
        let resp = self
            .client
            .get(&url)
            .header("Authorization", self.read_auth_header()?)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("X account metrics error: {e}")))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_else(|_| "unknown".into());
            return Err(Error::Channel(format!("X account metrics failed: {text}")));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Channel(format!("X account metrics parse: {e}")))?;
        Ok(Self::parse_account_metrics(&data["data"]))
    }

    fn parse_account_metrics(user: &serde_json::Value) -> XAccountMetrics {
        let metrics = &user["public_metrics"];
        XAccountMetrics {
            user_id: user["id"].as_str().unwrap_or_default().to_string(),
            username: user["username"].as_str().map(str::to_string),
            name: user["name"].as_str().map(str::to_string),
            followers_count: metrics["followers_count"].as_u64().unwrap_or(0),
            following_count: metrics["following_count"].as_u64().unwrap_or(0),
            tweet_count: metrics["tweet_count"].as_u64().unwrap_or(0),
            listed_count: metrics["listed_count"].as_u64().unwrap_or(0),
        }
    }

    /// Get authenticated user's profile
    pub async fn get_me(&self) -> Result<XUserProfile> {
        let resp = self
            .client
            .get(format!(
                "{}/users/me?user.fields=description,public_metrics,verified,profile_image_url",
                X_API_BASE
            ))
            .header("Authorization", self.read_auth_header()?)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("X get_me error: {}", e)))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_else(|_| "unknown".into());
            return Err(Error::Channel(format!("X get_me failed: {}", text)));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Channel(format!("X get_me parse: {}", e)))?;

        let user = &data["data"];
        let metrics = &user["public_metrics"];

        Ok(XUserProfile {
            id: user["id"].as_str().unwrap_or_default().to_string(),
            username: user["username"].as_str().unwrap_or_default().to_string(),
            name: user["name"].as_str().unwrap_or_default().to_string(),
            bio: user["description"].as_str().map(|s| s.to_string()),
            followers_count: metrics["followers_count"].as_u64().unwrap_or(0),
            following_count: metrics["following_count"].as_u64().unwrap_or(0),
            tweet_count: metrics["tweet_count"].as_u64().unwrap_or(0),
            verified: user["verified"].as_bool().unwrap_or(false),
            profile_image_url: user["profile_image_url"].as_str().map(|s| s.to_string()),
        })
    }

    /// Start the mention polling loop
    async fn start_polling(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        let connected = self.connected.clone();
        let shutdown = self.shutdown.clone();
        let poll_interval =
            std::time::Duration::from_secs(self.config.poll_interval_secs.unwrap_or(60));

        // Clone what we need for the polling task
        let client = self.client.clone();
        let last_mention = self.last_mention_id.clone();
        let config = self.config.clone();
        let oauth2_token = self.oauth2_access_token.clone();

        let handle = tokio::spawn(async move {
            // Create a mini adapter for polling (avoids self-referential async)
            let adapter = XPollingState {
                client,
                oauth2_token,
                bearer_token: config.bearer_token.clone(),
                last_mention_id: last_mention,
                user_id: Arc::new(RwLock::new(config.user_id.clone())),
            };

            loop {
                tokio::select! {
                    _ = shutdown.notified() => {
                        tracing::info!("X polling shutting down");
                        break;
                    }
                    _ = tokio::time::sleep(poll_interval) => {
                        if let Err(e) = adapter.poll_mentions(&tx).await {
                            tracing::warn!(error = %e, "X mention poll failed");
                        }
                    }
                }
            }
            connected.store(false, Ordering::SeqCst);
        });

        *self.task_handle.write().await = Some(handle);
        self.connected.store(true, Ordering::SeqCst);
        tracing::info!(
            "X adapter started (polling every {}s)",
            poll_interval.as_secs()
        );

        Ok(())
    }
}

/// Internal polling state (avoids self-referential async issues)
struct XPollingState {
    client: reqwest::Client,
    /// Shared handle to the adapter's live OAuth 2.0 token. Re-read every poll
    /// so a `set_oauth2_token()` call takes effect on the next request.
    oauth2_token: Arc<RwLock<String>>,
    /// Fallback App-Only bearer token (captured at start; this one doesn't
    /// rotate at runtime).
    bearer_token: String,
    last_mention_id: Arc<RwLock<Option<String>>>,
    /// Cached user ID — fetched once on first poll, never re-fetched
    user_id: Arc<RwLock<Option<String>>>,
}

impl XPollingState {
    /// Compute the current Authorization header, preferring the live OAuth 2.0
    /// token over the static App-Only bearer.
    async fn current_auth(&self) -> String {
        let oauth2 = self.oauth2_token.read().await;
        if !oauth2.is_empty() {
            format!("Bearer {}", *oauth2)
        } else if !self.bearer_token.is_empty() {
            format!("Bearer {}", self.bearer_token)
        } else {
            // Polling without user context or bearer isn't viable — return
            // an empty header so the request fails fast with 401 and the
            // warning surfaces in logs rather than looping silently.
            String::new()
        }
    }

    async fn poll_mentions(&self, tx: &mpsc::Sender<ChannelMessage>) -> Result<()> {
        let auth = self.current_auth().await;
        // Use cached user_id; fetch once if not yet set
        let user_id = {
            let cached = self.user_id.read().await;
            cached.clone()
        };
        let user_id = match user_id {
            Some(id) => id,
            None => {
                let resp = self
                    .client
                    .get(format!("{}/users/me", X_API_BASE))
                    .header("Authorization", &auth)
                    .send()
                    .await
                    .map_err(|e| Error::Channel(format!("X poll error: {}", e)))?;

                let data: serde_json::Value = resp
                    .json()
                    .await
                    .map_err(|e| Error::Channel(format!("X poll parse: {}", e)))?;

                let id = data["data"]["id"].as_str().unwrap_or_default().to_string();
                // Cache it for all future polls
                *self.user_id.write().await = Some(id.clone());
                id
            }
        };

        let mut url = format!(
            "{}/users/{}/mentions?tweet.fields=created_at,author_id&expansions=author_id&user.fields=username",
            X_API_BASE, user_id
        );

        if let Some(ref since_id) = *self.last_mention_id.read().await {
            url.push_str(&format!("&since_id={}", since_id));
        }

        let resp = self
            .client
            .get(&url)
            .header("Authorization", &auth)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("X mentions error: {}", e)))?;

        if !resp.status().is_success() {
            return Ok(());
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Channel(format!("X mentions parse: {}", e)))?;

        let mut user_map = std::collections::HashMap::new();
        if let Some(users) = data
            .get("includes")
            .and_then(|i| i.get("users"))
            .and_then(|u| u.as_array())
        {
            for user in users {
                if let (Some(id), Some(username)) = (user["id"].as_str(), user["username"].as_str())
                {
                    user_map.insert(id.to_string(), username.to_string());
                }
            }
        }

        if let Some(tweets) = data.get("data").and_then(|d| d.as_array()) {
            let mut newest_id: Option<String> = None;

            for tweet in tweets {
                let tweet_id = tweet["id"].as_str().unwrap_or_default().to_string();
                let text = tweet["text"].as_str().unwrap_or_default().to_string();
                let author_id = tweet["author_id"].as_str().unwrap_or_default().to_string();
                let username = user_map
                    .get(&author_id)
                    .cloned()
                    .unwrap_or_else(|| author_id.clone());

                if newest_id.is_none() || tweet_id > *newest_id.as_ref().unwrap() {
                    newest_id = Some(tweet_id.clone());
                }

                let source = ChannelSource::with_chat("x_twitter", &username, &tweet_id);
                let msg = ChannelMessage::new(source, text).with_platform_message_id(&tweet_id);

                let _ = tx.send(msg).await;
            }

            if let Some(id) = newest_id {
                *self.last_mention_id.write().await = Some(id);
            }
        }

        Ok(())
    }
}

#[async_trait]
impl ChannelAdapter for XAdapter {
    fn channel_type(&self) -> &'static str {
        "x_twitter"
    }

    fn receive_mode(&self) -> ReceiveMode {
        ReceiveMode::Polling {
            interval_secs: self.config.poll_interval_secs.unwrap_or(60),
        }
    }

    async fn start(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        self.start_polling(tx).await
    }

    async fn stop(&self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);
        self.shutdown.notify_one();

        if let Some(handle) = self.task_handle.write().await.take() {
            let _ = handle.await;
        }

        tracing::info!("X adapter stopped");
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        if to.channel_type() != "x_twitter" {
            return Err(Error::channel("Invalid channel source for X"));
        }

        // On X, reply_to_id (if any) is carried in the message_id field, not chat_id.
        // chat_id on X has no meaningful "channel" concept — ignore it here.
        let opts = CreateTweetOptions {
            text: content.to_string(),
            reply_to: to.reply_to_message_id.clone(),
            ..Default::default()
        };

        self.post_tweet(&opts).await?;
        Ok(())
    }

    /// Send a file as tweet media: v2 chunked upload, then a tweet carrying
    /// the media ID with `caption` as the tweet text. Errors from the upload
    /// or the create_tweet call propagate verbatim — never a silent success.
    async fn send_file(
        &self,
        to: &ChannelSource,
        filename: &str,
        data: &[u8],
        caption: Option<&str>,
    ) -> Result<()> {
        if to.channel_type() != "x_twitter" {
            return Err(Error::channel("Invalid channel source for X"));
        }

        let mime_type = mime_from_filename(filename)?;

        let media_id = self.upload_media(data, mime_type, None).await?;

        let opts = CreateTweetOptions {
            text: caption.unwrap_or("").to_string(),
            reply_to: to.reply_to_message_id.clone(),
            media_ids: vec![media_id],
            ..Default::default()
        };

        self.post_tweet(&opts).await?;
        Ok(())
    }

    /// Attach one or more media items to a single tweet.
    ///
    /// Each file is uploaded (v2 chunked INIT/APPEND/FINALIZE) to obtain a
    /// media ID, then all IDs are attached to one `create_tweet` call. This is
    /// what powers illustrated posts/threads from the agent `x_twitter` and
    /// `message` tools — text-only `send` cannot carry media. Any upload or
    /// post error propagates verbatim (never a silent partial success).
    async fn send_media(
        &self,
        to: &ChannelSource,
        files: &[MediaFile],
        caption: Option<&str>,
        alt_text: Option<&str>,
    ) -> Result<()> {
        if to.channel_type() != "x_twitter" {
            return Err(Error::channel("Invalid channel source for X"));
        }
        if files.is_empty() {
            return Err(Error::channel("send_media requires at least one file"));
        }
        if files.len() > 4 {
            return Err(Error::channel(format!(
                "X supports at most 4 media items per tweet, got {}",
                files.len()
            )));
        }

        let mut media_ids = Vec::with_capacity(files.len());
        for (filename, data, mime_type) in files {
            // Trust the caller-supplied MIME type when present, else infer.
            let mime = if mime_type.is_empty() {
                mime_from_filename(filename)?
            } else {
                mime_type.as_str()
            };
            let id = self.upload_media(data, mime, alt_text).await?;
            media_ids.push(id);
        }

        let opts = CreateTweetOptions {
            text: caption.unwrap_or("").to_string(),
            reply_to: to.reply_to_message_id.clone(),
            media_ids,
            ..Default::default()
        };
        self.post_tweet(&opts).await?;
        Ok(())
    }

    async fn send_as(
        &self,
        to: &ChannelSource,
        content: &str,
        _identity: &AgentSendIdentity,
    ) -> Result<()> {
        // Don't prefix tweets with [name] — on a public social adapter that would
        // appear verbatim in every published tweet. Just send the content as-is.
        self.send(to, content).await
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn supports_threading(&self) -> bool {
        true
    }

    async fn send_threaded(
        &self,
        _to: &ChannelSource,
        content: &str,
        opts: &crate::threading::ThreadedReplyOptions,
    ) -> Result<()> {
        let tweet_opts = CreateTweetOptions {
            text: content.to_string(),
            reply_to: opts
                .reply_to_message_id
                .clone()
                .or_else(|| opts.thread_id.clone()),
            ..Default::default()
        };

        self.post_tweet(&tweet_opts).await?;
        Ok(())
    }
}

// ── Config ───────────────────────────────────────────────────────────────

/// X (Twitter) configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct XConfig {
    /// Bearer token (OAuth 2.0 App-Only)
    #[serde(default)]
    pub bearer_token: String,
    /// API key (OAuth 1.0a consumer key)
    #[serde(default)]
    pub api_key: String,
    /// API secret (OAuth 1.0a consumer secret)
    #[serde(default)]
    pub api_secret: String,
    /// Access token (OAuth 1.0a)
    #[serde(default)]
    pub access_token: String,
    /// Access token secret (OAuth 1.0a)
    #[serde(default)]
    pub access_token_secret: String,
    /// OAuth 2.0 Client ID (for PKCE flow)
    #[serde(default)]
    pub client_id: String,
    /// OAuth 2.0 Client Secret (for PKCE flow — optional, only for confidential clients)
    #[serde(default)]
    pub client_secret: String,
    /// OAuth 2.0 user-context access token (result of PKCE authorization).
    /// When non-empty, this is the **preferred** credential for both reads
    /// and writes — X's 2024+ API tiers require user-context OAuth 2.0 for
    /// posting tweets, and bearer tokens alone are increasingly read-only.
    #[serde(default)]
    pub oauth2_access_token: String,
    /// OAuth 2.0 refresh token (granted when the `offline.access` scope is
    /// included). Persist this alongside the access token so the adapter can
    /// silently refresh via `refresh_oauth2_token()` before expiry.
    #[serde(default)]
    pub oauth2_refresh_token: String,
    /// Unix epoch seconds at which `oauth2_access_token` expires. `0` means
    /// "unknown" — callers should treat an unknown expiry as "expired soon"
    /// and proactively refresh.
    #[serde(default)]
    pub oauth2_expires_at: u64,
    /// Pre-fetched user ID (optional, avoids extra API call)
    #[serde(default)]
    pub user_id: Option<String>,
    /// Polling interval for mentions in seconds (default: 60)
    #[serde(default)]
    pub poll_interval_secs: Option<u64>,
    /// Auto-reply to mentions (default: false)
    #[serde(default)]
    pub auto_reply: bool,
}

// ── OAuth 2.0 PKCE Flow ──────────────────────────────────────────────────
//
// X/Twitter requires OAuth 2.0 with PKCE for user-context endpoints
// (e.g. posting tweets as a user). Flow:
//   1. `generate_pkce_pair()` → (verifier, challenge)
//   2. Redirect user to `build_authorize_url(..., challenge)`
//   3. User approves; callback delivers `?code=...&state=...`
//   4. `exchange_code(code, verifier)` → access_token + refresh_token
//   5. `refresh_oauth2_token(refresh_token)` when access_token expires
//
// Endpoints:
//   Authorize: https://x.com/i/oauth2/authorize
//   Token:     https://api.x.com/2/oauth2/token
// Recommended scopes: "tweet.read tweet.write users.read offline.access"

/// X OAuth 2.0 authorize endpoint.
pub const X_OAUTH2_AUTHORIZE_URL: &str = "https://x.com/i/oauth2/authorize";
/// X OAuth 2.0 token endpoint.
pub const X_OAUTH2_TOKEN_URL: &str = "https://api.x.com/2/oauth2/token";
/// Default scopes for read+write user context with refresh support.
pub const X_OAUTH2_DEFAULT_SCOPES: &str = "tweet.read tweet.write users.read offline.access";

/// Response from a successful OAuth 2.0 token exchange or refresh.
///
/// `refresh_token` is only returned when the `offline.access` scope was granted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2TokenResponse {
    pub token_type: String,
    pub access_token: String,
    pub expires_in: u64,
    pub scope: Option<String>,
    pub refresh_token: Option<String>,
}

/// Generate a PKCE (code_verifier, code_challenge) pair.
///
/// - `code_verifier`: 64-byte URL-safe-random string (43..128 chars as required by RFC 7636).
/// - `code_challenge`: BASE64URL(SHA256(verifier)), no padding — the `S256` method.
pub fn generate_pkce_pair() -> (String, String) {
    use rand::RngCore;
    use sha2::{Digest, Sha256};

    // 64 bytes of entropy → ~86 chars base64url (well within 43..128).
    let mut raw = [0u8; 64];
    rand::thread_rng().fill_bytes(&mut raw);
    let verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw);

    let digest = Sha256::digest(verifier.as_bytes());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);

    (verifier, challenge)
}

/// Build the authorization URL the user must visit to grant consent.
///
/// Always uses `code_challenge_method=S256` and `response_type=code`.
/// Scopes are space-separated (e.g. `X_OAUTH2_DEFAULT_SCOPES`).
pub fn build_authorize_url(
    client_id: &str,
    redirect_uri: &str,
    scopes: &str,
    state: &str,
    code_challenge: &str,
) -> String {
    let enc = urlencoding::encode;
    format!(
        "{base}?response_type=code&client_id={cid}&redirect_uri={redir}&scope={scope}&state={state}&code_challenge={chal}&code_challenge_method=S256",
        base = X_OAUTH2_AUTHORIZE_URL,
        cid = enc(client_id),
        redir = enc(redirect_uri),
        scope = enc(scopes),
        state = enc(state),
        chal = enc(code_challenge),
    )
}

/// Exchange an authorization `code` for an access_token (+ refresh_token if
/// `offline.access` was requested).
///
/// For confidential clients pass both `client_id` and a non-empty `client_secret`
/// (HTTP Basic auth is added). For public clients pass an empty `client_secret`.
pub async fn exchange_code(
    client: &reqwest::Client,
    client_id: &str,
    client_secret: &str,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<OAuth2TokenResponse> {
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", code_verifier),
        ("client_id", client_id),
    ];

    let mut req = client.post(X_OAUTH2_TOKEN_URL).form(&form);
    if !client_secret.is_empty() {
        req = req.basic_auth(client_id, Some(client_secret));
    }

    let resp = req
        .send()
        .await
        .map_err(|e| Error::Channel(format!("x oauth2 exchange_code request failed: {e}")))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| Error::Channel(format!("x oauth2 exchange_code read body: {e}")))?;

    if !status.is_success() {
        return Err(Error::Channel(format!(
            "x oauth2 exchange_code HTTP {status}: {body}"
        )));
    }

    serde_json::from_str::<OAuth2TokenResponse>(&body)
        .map_err(|e| Error::Channel(format!("x oauth2 exchange_code parse: {e} — body={body}")))
}

/// Refresh an OAuth 2.0 access token using a previously-issued `refresh_token`.
///
/// The response *may* include a new `refresh_token` — callers should persist
/// whichever value they get back (X may rotate refresh tokens).
pub async fn refresh_oauth2_token(
    client: &reqwest::Client,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<OAuth2TokenResponse> {
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
    ];

    let mut req = client.post(X_OAUTH2_TOKEN_URL).form(&form);
    if !client_secret.is_empty() {
        req = req.basic_auth(client_id, Some(client_secret));
    }

    let resp = req
        .send()
        .await
        .map_err(|e| Error::Channel(format!("x oauth2 refresh request failed: {e}")))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| Error::Channel(format!("x oauth2 refresh read body: {e}")))?;

    if !status.is_success() {
        return Err(Error::Channel(format!(
            "x oauth2 refresh HTTP {status}: {body}"
        )));
    }

    serde_json::from_str::<OAuth2TokenResponse>(&body)
        .map_err(|e| Error::Channel(format!("x oauth2 refresh parse: {e} — body={body}")))
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_x_config_default() {
        let config = XConfig::default();
        assert!(config.bearer_token.is_empty());
        assert!(config.api_key.is_empty());
        assert!(config.poll_interval_secs.is_none());
        assert!(!config.auto_reply);
    }

    #[test]
    fn test_x_config_serde() {
        let config = XConfig {
            bearer_token: "test-token".to_string(),
            poll_interval_secs: Some(30),
            auto_reply: true,
            ..Default::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        let back: XConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.bearer_token, "test-token");
        assert_eq!(back.poll_interval_secs, Some(30));
        assert!(back.auto_reply);
    }

    #[tokio::test]
    async fn test_x_adapter_validation() {
        // Empty config should fail
        let config = XConfig::default();
        assert!(XAdapter::new(config).await.is_err());

        // Bearer token should work
        let config = XConfig {
            bearer_token: "test-bearer".to_string(),
            ..Default::default()
        };
        assert!(XAdapter::new(config).await.is_ok());

        // API key should work
        let config = XConfig {
            api_key: "test-key".to_string(),
            api_secret: "test-secret".to_string(),
            ..Default::default()
        };
        assert!(XAdapter::new(config).await.is_ok());
    }

    #[tokio::test]
    async fn test_x_adapter_lifecycle() {
        let config = XConfig {
            bearer_token: "test-bearer".to_string(),
            ..Default::default()
        };
        let adapter = XAdapter::new(config).await.unwrap();
        assert!(!adapter.is_connected());
        assert_eq!(adapter.channel_type(), "x_twitter");
        assert!(adapter.supports_threading());
    }

    #[test]
    fn test_api_endpoints_are_v2_only() {
        // #198 regression guard: the write surface must be X API v2.
        // v1.1 statuses/update.json and upload.twitter.com/1.1 are retired —
        // reintroducing them silently breaks posting.
        assert_eq!(X_API_BASE, "https://api.x.com/2");
        let tweet_url = format!("{}/tweets", X_API_BASE);
        assert_eq!(tweet_url, "https://api.x.com/2/tweets");
        let upload_url = format!("{}/media/upload", X_API_BASE);
        assert_eq!(upload_url, "https://api.x.com/2/media/upload");
        assert!(!tweet_url.contains("1.1"));
        assert!(!upload_url.contains("upload.twitter.com"));
    }

    #[test]
    fn test_create_tweet_options_default() {
        let opts = CreateTweetOptions::default();
        assert!(opts.text.is_empty());
        assert!(opts.reply_to.is_none());
        assert!(opts.media_ids.is_empty());
        assert!(opts.poll_options.is_empty());
    }

    #[test]
    fn test_build_tweet_body_media_ids_flow() {
        // #420: media_ids must reach the create_tweet request body under
        // `media.media_ids`, alongside text — this is the wire contract that
        // makes an illustrated tweet actually carry its images.
        let opts = CreateTweetOptions {
            text: "illustrated tweet".to_string(),
            media_ids: vec!["111".to_string(), "222".to_string()],
            ..Default::default()
        };
        let body = build_tweet_body(&opts);
        assert_eq!(body["text"], "illustrated tweet");
        assert_eq!(body["media"]["media_ids"][0], "111");
        assert_eq!(body["media"]["media_ids"][1], "222");
    }

    #[test]
    fn test_build_tweet_body_media_and_reply_together() {
        // #420: illustrated threads need media_ids + reply_to in the SAME body
        // (a reply tweet that also carries an image).
        let opts = CreateTweetOptions {
            text: "reply with image".to_string(),
            reply_to: Some("999".to_string()),
            media_ids: vec!["abc".to_string()],
            ..Default::default()
        };
        let body = build_tweet_body(&opts);
        assert_eq!(body["reply"]["in_reply_to_tweet_id"], "999");
        assert_eq!(body["media"]["media_ids"][0], "abc");
    }

    #[test]
    fn test_build_tweet_body_no_media_omits_key() {
        // Text-only tweets must NOT carry a media key (keeps text path clean).
        let opts = CreateTweetOptions {
            text: "plain".to_string(),
            ..Default::default()
        };
        let body = build_tweet_body(&opts);
        assert!(body.get("media").is_none());
    }

    #[test]
    fn test_mime_from_filename() {
        assert_eq!(mime_from_filename("a.png").unwrap(), "image/png");
        assert_eq!(mime_from_filename("b.JPG").unwrap(), "image/jpeg");
        assert_eq!(mime_from_filename("c.jpeg").unwrap(), "image/jpeg");
        assert_eq!(mime_from_filename("d.gif").unwrap(), "image/gif");
        assert_eq!(mime_from_filename("e.webp").unwrap(), "image/webp");
        assert_eq!(mime_from_filename("f.mp4").unwrap(), "video/mp4");
        assert!(mime_from_filename("g.txt").is_err());
        assert!(mime_from_filename("noext").is_err());
    }

    #[tokio::test]
    async fn test_send_media_rejects_wrong_channel() {
        let config = XConfig {
            bearer_token: "test-bearer".to_string(),
            ..Default::default()
        };
        let adapter = XAdapter::new(config).await.unwrap();
        let src = ChannelSource::new("discord", "agent");
        let files = vec![("a.png".to_string(), vec![0u8; 4], "image/png".to_string())];
        let err = adapter
            .send_media(&src, &files, Some("hi"), None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Invalid channel source"));
    }

    #[tokio::test]
    async fn test_send_media_rejects_empty_and_too_many() {
        let config = XConfig {
            bearer_token: "test-bearer".to_string(),
            ..Default::default()
        };
        let adapter = XAdapter::new(config).await.unwrap();
        let src = ChannelSource::new("x_twitter", "agent");

        // empty
        let err = adapter
            .send_media(&src, &[], Some("hi"), None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("at least one file"));

        // >4
        let files: Vec<MediaFile> = (0..5)
            .map(|i| (format!("{i}.png"), vec![0u8; 4], "image/png".to_string()))
            .collect();
        let err = adapter
            .send_media(&src, &files, Some("hi"), None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("at most 4"));
    }

    #[test]
    fn test_thread_options() {
        let thread = ThreadOptions {
            tweets: vec![
                "Tweet 1".to_string(),
                "Tweet 2".to_string(),
                "Tweet 3".to_string(),
            ],
            media_per_tweet: vec![vec![], vec!["media1".to_string()], vec![]],
        };
        assert_eq!(thread.tweets.len(), 3);
        assert_eq!(thread.media_per_tweet[1].len(), 1);
    }

    #[test]
    fn test_tweet_metrics_default() {
        let metrics = TweetMetrics::default();
        assert_eq!(metrics.like_count, 0);
        assert_eq!(metrics.retweet_count, 0);
        assert_eq!(metrics.impression_count, 0);
    }

    #[test]
    fn test_tweet_serde() {
        let tweet = Tweet {
            id: "123".to_string(),
            text: "Hello X!".to_string(),
            author_username: Some("zeus_ai".to_string()),
            author_name: Some("Zeus AI".to_string()),
            created_at: None,
            metrics: Some(TweetMetrics {
                like_count: 42,
                retweet_count: 10,
                ..Default::default()
            }),
            media: Vec::new(),
            conversation_id: None,
            in_reply_to: None,
        };
        let json = serde_json::to_string(&tweet).unwrap();
        let back: Tweet = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "123");
        assert_eq!(back.metrics.unwrap().like_count, 42);
    }

    #[test]
    fn test_media_type_serde() {
        let img = MediaType::Image;
        let json = serde_json::to_string(&img).unwrap();
        assert!(json.contains("Image"));

        let back: MediaType = serde_json::from_str(&json).unwrap();
        assert_eq!(back, MediaType::Image);
    }

    #[test]
    fn test_x_user_profile() {
        let profile = XUserProfile {
            id: "1234567".to_string(),
            username: "zeus_ai".to_string(),
            name: "Zeus AI".to_string(),
            bio: Some("Almighty AI agent runtime".to_string()),
            followers_count: 1000,
            following_count: 50,
            tweet_count: 500,
            verified: true,
            profile_image_url: None,
        };
        let json = serde_json::to_string(&profile).unwrap();
        let back: XUserProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.username, "zeus_ai");
        assert_eq!(back.followers_count, 1000);
        assert!(back.verified);
    }

    #[test]
    fn test_receive_mode() {
        let config = XConfig {
            bearer_token: "test".to_string(),
            poll_interval_secs: Some(30),
            ..Default::default()
        };
        // Can't call receive_mode without adapter, but test the config
        assert_eq!(config.poll_interval_secs, Some(30));
    }

    #[test]
    fn test_channel_source_x_twitter() {
        let source = ChannelSource::with_chat("x_twitter", "zeus_ai", "tweet_123");
        assert_eq!(source.channel_type(), "x_twitter");
        assert_eq!(source.user_id, "zeus_ai");
        assert_eq!(source.chat_id, Some("tweet_123".to_string()));
    }

    // ── OAuth 2.0 PKCE tests ──

    #[test]
    fn test_pkce_pair_shape() {
        let (verifier, challenge) = generate_pkce_pair();
        // RFC 7636: verifier must be 43..=128 chars, unreserved charset.
        assert!(
            verifier.len() >= 43 && verifier.len() <= 128,
            "verifier len out of range: {}",
            verifier.len()
        );
        // base64url(SHA256(_)) with NO_PAD → 43 chars.
        assert_eq!(challenge.len(), 43);
        // URL-safe-no-pad: no '+', '/', or '=' allowed.
        for s in [&verifier, &challenge] {
            assert!(!s.contains('+') && !s.contains('/') && !s.contains('='));
        }
    }

    #[test]
    fn test_pkce_pair_uniqueness() {
        let (v1, _) = generate_pkce_pair();
        let (v2, _) = generate_pkce_pair();
        assert_ne!(v1, v2, "PKCE verifiers must be random");
    }

    #[test]
    fn test_pkce_challenge_matches_verifier() {
        use sha2::{Digest, Sha256};
        let (verifier, challenge) = generate_pkce_pair();
        let expected = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        assert_eq!(challenge, expected);
    }

    #[tokio::test]
    async fn test_adapter_accepts_oauth2_only() {
        // OAuth 2.0 user-context token alone should be sufficient to construct
        // the adapter — no bearer or api_key needed.
        let config = XConfig {
            oauth2_access_token: "user_ctx_token_from_pkce".to_string(),
            ..Default::default()
        };
        let adapter = XAdapter::new(config).await.unwrap();
        assert_eq!(
            adapter.current_oauth2_token().await,
            "user_ctx_token_from_pkce"
        );
    }

    #[tokio::test]
    async fn test_set_oauth2_token_updates_live() {
        let config = XConfig {
            bearer_token: "app_only_bearer".to_string(),
            ..Default::default()
        };
        let adapter = XAdapter::new(config).await.unwrap();
        assert_eq!(adapter.current_oauth2_token().await, "");

        adapter.set_oauth2_token("fresh_from_pkce").await;
        assert_eq!(adapter.current_oauth2_token().await, "fresh_from_pkce");

        // write_auth_header should now return the OAuth 2.0 bearer, not the
        // App-Only bearer.
        let hdr = adapter.write_auth_header("POST", "https://api.x.com/2/tweets");
        assert_eq!(hdr, "Bearer fresh_from_pkce");

        // Clearing it falls back to the App-Only bearer.
        adapter.set_oauth2_token("").await;
        let hdr = adapter.write_auth_header("POST", "https://api.x.com/2/tweets");
        assert_eq!(hdr, "Bearer app_only_bearer");
    }

    #[tokio::test]
    async fn test_auth_priority_oauth2_beats_oauth1a() {
        // With *both* a full OAuth 1.0a quartet AND an OAuth 2.0 user token,
        // OAuth 2.0 must win on write_auth_header.
        let config = XConfig {
            oauth2_access_token: "user_ctx_token".to_string(),
            api_key: "ck".to_string(),
            api_secret: "cs".to_string(),
            access_token: "at".to_string(),
            access_token_secret: "ats".to_string(),
            bearer_token: "app_bearer".to_string(),
            ..Default::default()
        };
        let adapter = XAdapter::new(config).await.unwrap();
        let write_hdr = adapter.write_auth_header("POST", "https://api.x.com/2/tweets");
        assert_eq!(
            write_hdr, "Bearer user_ctx_token",
            "OAuth 2.0 must take priority over OAuth 1.0a for writes"
        );
        let read_hdr = adapter.read_auth_header().unwrap();
        assert_eq!(
            read_hdr, "Bearer user_ctx_token",
            "OAuth 2.0 must take priority over App-Only bearer for reads"
        );
    }

    #[test]
    fn test_xconfig_serde_oauth2_fields() {
        // Existing configs (without OAuth 2.0 fields) must still deserialize —
        // serde defaults give empty strings / 0.
        let legacy = r#"{"bearer_token":"b","api_key":"k","api_secret":"s","access_token":"at","access_token_secret":"ats","client_id":"","client_secret":"","user_id":null,"poll_interval_secs":null,"auto_reply":false}"#;
        let parsed: XConfig = serde_json::from_str(legacy).unwrap();
        assert!(parsed.oauth2_access_token.is_empty());
        assert!(parsed.oauth2_refresh_token.is_empty());
        assert_eq!(parsed.oauth2_expires_at, 0);

        // Round-trip with OAuth 2.0 fields populated.
        let cfg = XConfig {
            oauth2_access_token: "a".into(),
            oauth2_refresh_token: "r".into(),
            oauth2_expires_at: 1_700_000_000,
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: XConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.oauth2_access_token, "a");
        assert_eq!(back.oauth2_refresh_token, "r");
        assert_eq!(back.oauth2_expires_at, 1_700_000_000);
    }

    fn test_x_config_with_oauth2() -> XConfig {
        XConfig {
            oauth2_access_token: "test-oauth2-token".to_string(),
            ..Default::default()
        }
    }

    fn test_x_config_oauth1() -> XConfig {
        XConfig {
            api_key: "test-consumer-key".to_string(),
            api_secret: "test-consumer-secret".to_string(),
            access_token: "test-access-token".to_string(),
            access_token_secret: "test-access-secret".to_string(),
            ..Default::default()
        }
    }

    /// Recompute the OAuth 1.0a HMAC-SHA1 signature the way `oauth1_header`
    /// builds it — from the `oauth_*` params in a captured Authorization
    /// header plus the given body params — and assert it matches the
    /// signature the adapter actually sent. Proves the form body params were
    /// part of the signature base string (RFC 5849 §3.4.1.3).
    fn assert_oauth1_signature_covers_params(
        auth_header: &str,
        method: &str,
        url: &str,
        body_params: &[(&str, &str)],
        consumer_secret: &str,
        token_secret: &str,
    ) {
        let mut params: Vec<(String, String)> = Vec::new();
        let mut captured_signature = String::new();
        for piece in auth_header.trim_start_matches("OAuth ").split(", ") {
            let (k, v) = piece.split_once('=').expect("header piece must be k=v");
            let v = urlencoding::decode(v.trim_matches('"'))
                .expect("header value must urldecode")
                .into_owned();
            if k == "oauth_signature" {
                captured_signature = v;
            } else {
                params.push((k.to_string(), v));
            }
        }
        assert!(
            !captured_signature.is_empty(),
            "no oauth_signature in header: {auth_header}"
        );
        for (k, v) in body_params {
            params.push((k.to_string(), v.to_string()));
        }
        params.sort_by(|a, b| a.0.cmp(&b.0));
        let param_string = params
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        let base_string = format!(
            "{}&{}&{}",
            urlencoding::encode(method),
            urlencoding::encode(url),
            urlencoding::encode(&param_string),
        );
        let signing_key = format!(
            "{}&{}",
            urlencoding::encode(consumer_secret),
            urlencoding::encode(token_secret),
        );
        let mut mac = HmacSha1::new_from_slice(signing_key.as_bytes()).expect("hmac key");
        mac.update(base_string.as_bytes());
        let expected =
            base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        assert_eq!(
            expected, captured_signature,
            "OAuth1 signature must cover the form body params"
        );
    }

    #[tokio::test]
    async fn test_media_upload_oneshot_image_multipart_no_query() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/media/upload"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "id": "777" }
            })))
            .mount(&server)
            .await;

        let adapter = XAdapter::new_with_base_url(test_x_config_oauth1(), server.uri())
            .await
            .unwrap();
        let media_id = adapter
            .upload_media(b"png-bytes", "image/png", None)
            .await
            .unwrap();
        assert_eq!(media_id, "777");

        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests.len(),
            1,
            "one-shot image upload must be a single request"
        );
        let req = &requests[0];
        // Spec contract: POST /2/media/upload accepts NO query parameters
        // ("query parameter [x] is not one of []").
        assert!(
            req.url.query().unwrap_or("").is_empty(),
            "no query params allowed on one-shot upload, got: {:?}",
            req.url.query()
        );
        let auth = req
            .headers
            .get("authorization")
            .expect("Authorization header present")
            .to_str()
            .unwrap();
        assert!(auth.starts_with("OAuth "), "OAuth1-signed, got: {auth}");
        let content_type = req
            .headers
            .get("content-type")
            .expect("content-type present")
            .to_str()
            .unwrap();
        assert!(
            content_type.starts_with("multipart/form-data"),
            "multipart body required, got: {content_type}"
        );
        let body = String::from_utf8_lossy(&req.body);
        assert!(
            body.contains("name=\"media_category\""),
            "media_category part present"
        );
        assert!(body.contains("tweet_image"));
        assert!(body.contains("name=\"media\""), "media part present");
    }

    #[tokio::test]
    async fn test_media_upload_chunked_video_initialize_append_finalize() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/media/upload/initialize"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "id": "888" }
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/media/upload/888/append"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/media/upload/888/finalize"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "id": "888" }
            })))
            .mount(&server)
            .await;

        let adapter = XAdapter::new_with_base_url(test_x_config_oauth1(), server.uri())
            .await
            .unwrap();
        let media_id = adapter
            .upload_media(b"vid-bytes", "video/mp4", None)
            .await
            .unwrap();
        assert_eq!(media_id, "888");

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 3, "initialize + append + finalize");
        let init = &requests[0];
        assert!(init.url.path().ends_with("/media/upload/initialize"));
        let init_body: serde_json::Value = serde_json::from_slice(&init.body).unwrap();
        assert_eq!(init_body["media_type"], "video/mp4");
        assert_eq!(init_body["total_bytes"], 9);
        assert_eq!(init_body["media_category"], "tweet_video");
        let append_body = String::from_utf8_lossy(&requests[1].body);
        assert!(append_body.contains("name=\"segment_index\""));
        assert!(append_body.contains("name=\"media\""));
        for req in &requests {
            let auth = req
                .headers
                .get("authorization")
                .expect("Authorization header present")
                .to_str()
                .unwrap();
            assert!(auth.starts_with("OAuth "), "OAuth1-signed, got: {auth}");
            assert!(
                req.url.query().unwrap_or("").is_empty(),
                "no query params on any chunked media call"
            );
        }
    }

    #[tokio::test]
    async fn test_oauth1_form_param_signing_helper_covers_extra_params() {
        // `write_auth_header_with_params` must fold extra params into the
        // OAuth 1.0a signature base string (RFC 5849 §3.4.1.3). Media upload
        // no longer sends form/query params, but the helper stays for any
        // future form-urlencoded X endpoint — keep its contract pinned.
        let adapter = XAdapter::new_with_base_url(
            test_x_config_oauth1(),
            "https://api.x.com/2".to_string(),
        )
        .await
        .unwrap();
        let url = "https://api.x.com/2/example";
        let params = [("alpha", "1"), ("beta", "two words")];
        let auth = adapter.write_auth_header_with_params("POST", url, &params);
        assert!(auth.starts_with("OAuth "), "OAuth1-signed, got: {auth}");
        assert_oauth1_signature_covers_params(
            &auth,
            "POST",
            url,
            &params,
            "test-consumer-secret",
            "test-access-secret",
        );
    }

    #[tokio::test]
    async fn test_delete_tweet_retries_rate_limit() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, Request, ResponseTemplate};

        let server = MockServer::start().await;
        let attempts = Arc::new(AtomicUsize::new(0));
        let responder_attempts = attempts.clone();
        Mock::given(method("DELETE"))
            .and(path("/tweets/123"))
            .respond_with(move |_request: &Request| {
                if responder_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    ResponseTemplate::new(429)
                        .insert_header("retry-after", "0")
                        .set_body_string("rate limited")
                } else {
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "data": { "deleted": true }
                    }))
                }
            })
            .mount(&server)
            .await;

        let adapter = XAdapter::new_with_base_url(test_x_config_with_oauth2(), server.uri())
            .await
            .unwrap();
        let result = adapter.delete_tweet_result("123").await;

        assert_eq!(result.status, XDeleteStatus::Deleted);
        assert_eq!(result.attempts, 2);
        assert_eq!(result.http_status, Some(200));
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_batch_delete_returns_partial_results_without_aborting() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/tweets/good"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "deleted": true }
            })))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/tweets/bad"))
            .respond_with(ResponseTemplate::new(403).set_body_string("forbidden"))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/tweets/after"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "deleted": true }
            })))
            .mount(&server)
            .await;

        let adapter = XAdapter::new_with_base_url(test_x_config_with_oauth2(), server.uri())
            .await
            .unwrap();
        let ids = vec![
            "good".to_string(),
            "bad".to_string(),
            "after".to_string(),
            "   ".to_string(),
        ];
        let batch = adapter.batch_delete_tweets(&ids).await;

        assert_eq!(batch.deleted, 2);
        assert_eq!(batch.failed, 1);
        assert_eq!(batch.skipped, 1);
        assert_eq!(batch.results.len(), 4);
        assert_eq!(batch.results[0].status, XDeleteStatus::Deleted);
        assert_eq!(batch.results[1].status, XDeleteStatus::Failed);
        assert_eq!(batch.results[1].http_status, Some(403));
        assert_eq!(batch.results[2].status, XDeleteStatus::Deleted);
        assert_eq!(batch.results[3].status, XDeleteStatus::Skipped);
    }

    #[tokio::test]
    async fn test_search_recent_builds_polling_query_and_parses_page() {
        use wiremock::matchers::{method, path, query_param, query_param_contains};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/tweets/search/recent"))
            .and(query_param("query", "zeus"))
            .and(query_param("since_id", "100"))
            .and(query_param("max_results", "10"))
            .and(query_param_contains("tweet.fields", "public_metrics"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{
                    "id": "101",
                    "text": "hello zeus",
                    "author_id": "u1",
                    "created_at": "2026-07-12T00:00:00.000Z",
                    "conversation_id": "101",
                    "public_metrics": {"like_count": 3, "retweet_count": 2, "reply_count": 1, "quote_count": 0, "impression_count": 99, "bookmark_count": 4},
                    "attachments": {"media_keys": ["m1"]}
                }],
                "includes": {
                    "users": [{"id": "u1", "username": "zeus", "name": "Zeus"}],
                    "media": [{"media_key": "m1", "type": "photo", "url": "https://cdn.example/img.jpg", "alt_text": "diagram"}]
                },
                "meta": {"result_count": 1, "newest_id": "101", "oldest_id": "101", "next_token": "next"}
            })))
            .mount(&server)
            .await;

        let adapter = XAdapter::new_with_base_url(test_x_config_with_oauth2(), server.uri())
            .await
            .unwrap();
        let page = adapter
            .search_recent(
                "zeus",
                &XReadOptions {
                    since_id: Some("100".into()),
                    max_results: Some(2),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(page.result_count, 1);
        assert_eq!(page.next_token.as_deref(), Some("next"));
        assert_eq!(page.tweets[0].id, "101");
        assert_eq!(page.tweets[0].author_username.as_deref(), Some("zeus"));
        assert_eq!(page.tweets[0].metrics.as_ref().unwrap().like_count, 3);
        assert_eq!(page.tweets[0].media[0].media_type, MediaType::Image);
    }

    #[tokio::test]
    async fn test_get_mentions_uses_user_id_path_and_since_cursor() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/42/mentions"))
            .and(query_param("since_id", "200"))
            .and(query_param("max_results", "5"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"id": "201", "text": "@zeus ping", "author_id": "u2"}],
                "includes": {"users": [{"id": "u2", "username": "friend", "name": "Friend"}]},
                "meta": {"result_count": 1, "newest_id": "201", "oldest_id": "201"}
            })))
            .mount(&server)
            .await;

        let adapter = XAdapter::new_with_base_url(test_x_config_with_oauth2(), server.uri())
            .await
            .unwrap();
        let page = adapter
            .get_mentions(
                "42",
                &XReadOptions {
                    since_id: Some("200".into()),
                    max_results: Some(1),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(page.tweets.len(), 1);
        assert_eq!(page.tweets[0].author_name.as_deref(), Some("Friend"));
    }

    #[tokio::test]
    async fn test_get_tweet_fetches_by_id_and_parses_reply_reference() {
        use wiremock::matchers::{method, path, query_param_contains};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/tweets/300"))
            .and(query_param_contains("tweet.fields", "referenced_tweets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {
                    "id": "300",
                    "text": "reply body",
                    "author_id": "u3",
                    "referenced_tweets": [{"type": "replied_to", "id": "299"}]
                },
                "includes": {"users": [{"id": "u3", "username": "replybot", "name": "Reply Bot"}]}
            })))
            .mount(&server)
            .await;

        let adapter = XAdapter::new_with_base_url(test_x_config_with_oauth2(), server.uri())
            .await
            .unwrap();
        let tweet = adapter.get_tweet("300").await.unwrap();

        assert_eq!(tweet.id, "300");
        assert_eq!(tweet.in_reply_to.as_deref(), Some("299"));
        assert_eq!(tweet.author_username.as_deref(), Some("replybot"));
    }

    #[tokio::test]
    async fn test_get_user_timeline_uses_pagination_token() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/42/tweets"))
            .and(query_param("pagination_token", "cursor"))
            .and(query_param("max_results", "25"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"id": "401", "text": "timeline", "author_id": "42"}],
                "includes": {"users": [{"id": "42", "username": "zeus", "name": "Zeus"}]},
                "meta": {"result_count": 1, "newest_id": "401", "oldest_id": "401"}
            })))
            .mount(&server)
            .await;

        let adapter = XAdapter::new_with_base_url(test_x_config_with_oauth2(), server.uri())
            .await
            .unwrap();
        let page = adapter
            .get_user_timeline(
                "42",
                &XReadOptions {
                    pagination_token: Some("cursor".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(page.tweets[0].text, "timeline");
        assert_eq!(page.newest_id.as_deref(), Some("401"));
    }

    #[test]
    fn test_build_authorize_url() {
        let url = build_authorize_url(
            "my_client_id",
            "https://example.com/callback",
            X_OAUTH2_DEFAULT_SCOPES,
            "xyz_state",
            "CHAL_abc",
        );
        assert!(url.starts_with(X_OAUTH2_AUTHORIZE_URL));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=my_client_id"));
        assert!(url.contains("code_challenge=CHAL_abc"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=xyz_state"));
        // redirect_uri and scopes must be percent-encoded.
        assert!(url.contains("redirect_uri=https%3A%2F%2Fexample.com%2Fcallback"));
        assert!(url.contains("scope=tweet.read%20tweet.write%20users.read%20offline.access"));
    }

    #[tokio::test]
    async fn test_read_auth_header_rejects_oauth1_only_for_reads() {
        let config = XConfig {
            api_key: "ck".into(),
            api_secret: "cs".into(),
            access_token: "at".into(),
            access_token_secret: "ats".into(),
            ..Default::default()
        };
        let adapter = XAdapter::new(config).await.unwrap();
        let err = adapter.read_auth_header().unwrap_err();
        assert!(
            err.to_string()
                .contains("X reads require bearer_token or OAuth 2.0 access token"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn test_like_tweet_posts_user_action() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/users/42/likes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "liked": true }
            })))
            .mount(&server)
            .await;

        let adapter = XAdapter::new_with_base_url(test_x_config_with_oauth2(), server.uri())
            .await
            .unwrap();
        let result = adapter.like_tweet("42", "99").await.unwrap();

        assert_eq!(result.action, "like");
        assert_eq!(result.user_id, "42");
        assert_eq!(result.target_id, "99");
        assert!(result.success);
    }

    #[tokio::test]
    async fn test_unlike_tweet_deletes_user_action() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/users/42/likes/99"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "liked": false }
            })))
            .mount(&server)
            .await;

        let adapter = XAdapter::new_with_base_url(test_x_config_with_oauth2(), server.uri())
            .await
            .unwrap();
        let result = adapter.unlike_tweet("42", "99").await.unwrap();

        assert_eq!(result.action, "unlike");
        assert_eq!(result.user_id, "42");
        assert_eq!(result.target_id, "99");
        assert!(result.success);
    }
    #[tokio::test]
    async fn test_block_user_posts_user_action() {
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/users/42/blocking"))
            .and(body_json(serde_json::json!({ "target_user_id": "99" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "blocking": true }
            })))
            .mount(&server)
            .await;

        let adapter = XAdapter::new_with_base_url(test_x_config_with_oauth2(), server.uri())
            .await
            .unwrap();
        let result = adapter.block_user("42", "99").await.unwrap();

        assert_eq!(result.action, "block");
        assert_eq!(result.user_id, "42");
        assert_eq!(result.target_id, "99");
        assert!(result.success);
    }

    #[tokio::test]
    async fn test_unmute_user_deletes_user_action() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/users/42/muting/99"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "muting": false }
            })))
            .mount(&server)
            .await;

        let adapter = XAdapter::new_with_base_url(test_x_config_with_oauth2(), server.uri())
            .await
            .unwrap();
        let result = adapter.unmute_user("42", "99").await.unwrap();

        assert_eq!(result.action, "unmute");
        assert_eq!(result.user_id, "42");
        assert_eq!(result.target_id, "99");
        assert!(result.success);
    }

    #[tokio::test]
    async fn test_hide_reply_puts_hidden_flag() {
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/tweets/777/hidden"))
            .and(body_json(serde_json::json!({ "hidden": true })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "hidden": true }
            })))
            .mount(&server)
            .await;

        let adapter = XAdapter::new_with_base_url(test_x_config_with_oauth2(), server.uri())
            .await
            .unwrap();
        let result = adapter.hide_reply("777").await.unwrap();

        assert_eq!(result.action, "hide_reply");
        assert_eq!(result.target_id, "777");
        assert!(result.success);
    }

    #[tokio::test]
    async fn test_get_owned_lists_parses_page() {
        use wiremock::matchers::{method, path, query_param, query_param_contains};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/42/owned_lists"))
            .and(query_param_contains("list.fields", "member_count"))
            .and(query_param("max_results", "50"))
            .and(query_param("pagination_token", "cursor"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{
                    "id": "123",
                    "name": "ops",
                    "description": "infra watchlist",
                    "private": true,
                    "member_count": 7,
                    "follower_count": 3,
                    "owner_id": "42",
                    "created_at": "2026-07-12T00:00:00Z"
                }],
                "meta": { "result_count": 1, "next_token": "next" }
            })))
            .mount(&server)
            .await;

        let adapter = XAdapter::new_with_base_url(test_x_config_with_oauth2(), server.uri())
            .await
            .unwrap();
        let page = adapter
            .get_owned_lists(
                "42",
                &XListOptions {
                    pagination_token: Some("cursor".into()),
                    max_results: Some(50),
                },
            )
            .await
            .unwrap();

        assert_eq!(page.result_count, 1);
        assert_eq!(page.next_token.as_deref(), Some("next"));
        assert_eq!(page.lists[0].id, "123");
        assert_eq!(page.lists[0].name, "ops");
        assert!(page.lists[0].private);
        assert_eq!(page.lists[0].member_count, 7);
    }

    #[tokio::test]
    async fn test_create_list_posts_list_body() {
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/lists"))
            .and(body_json(serde_json::json!({
                "name": "ops",
                "description": "infra watchlist",
                "private": true
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "data": {
                    "id": "123",
                    "name": "ops",
                    "description": "infra watchlist",
                    "private": true,
                    "member_count": 0,
                    "follower_count": 0
                }
            })))
            .mount(&server)
            .await;

        let adapter = XAdapter::new_with_base_url(test_x_config_with_oauth2(), server.uri())
            .await
            .unwrap();
        let list = adapter
            .create_list(" ops ", Some("infra watchlist"), Some(true))
            .await
            .unwrap();

        assert_eq!(list.id, "123");
        assert_eq!(list.name, "ops");
        assert!(list.private);
    }

    #[tokio::test]
    async fn test_add_list_member_posts_member_body() {
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/lists/123/members"))
            .and(body_json(serde_json::json!({ "user_id": "42" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "is_member": true }
            })))
            .mount(&server)
            .await;

        let adapter = XAdapter::new_with_base_url(test_x_config_with_oauth2(), server.uri())
            .await
            .unwrap();
        let result = adapter.add_list_member("123", "42").await.unwrap();

        assert_eq!(result.action, "add_list_member");
        assert_eq!(result.list_id, "123");
        assert_eq!(result.user_id.as_deref(), Some("42"));
        assert!(result.success);
    }

    #[tokio::test]
    async fn test_report_tweet_returns_capability_error() {
        let adapter = XAdapter::new_with_base_url(test_x_config_with_oauth2(), "http://127.0.0.1")
            .await
            .unwrap();
        let err = adapter.report_tweet("777", Some("spam")).await.unwrap_err();
        assert!(
            err.to_string()
                .contains("not available through the public X API v2"),
            "got: {err}"
        );
    }
    #[tokio::test]
    async fn test_get_dm_events_builds_polling_query_and_parses_media() {
        use wiremock::matchers::{method, path, query_param, query_param_contains};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/dm_events"))
            .and(query_param("pagination_token", "tok1"))
            .and(query_param("max_results", "2"))
            .and(query_param_contains("dm_event.fields", "sender_id"))
            .and(query_param_contains("expansions", "attachments.media_keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{
                    "id": "e1",
                    "text": "hello dm",
                    "event_type": "MessageCreate",
                    "created_at": "2026-07-12T00:00:00.000Z",
                    "dm_conversation_id": "c1",
                    "sender_id": "42",
                    "participant_ids": ["42", "99"],
                    "attachments": {"media_keys": ["m1"]}
                }],
                "includes": {"media": [{"media_key": "m1", "type": "photo", "url": "https://cdn.example/img.jpg"}]},
                "meta": {"result_count": 1, "next_token": "tok2"}
            })))
            .mount(&server)
            .await;

        let adapter = XAdapter::new_with_base_url(test_x_config_with_oauth2(), server.uri())
            .await
            .unwrap();
        let page = adapter
            .get_dm_events(&XDmOptions {
                pagination_token: Some("tok1".into()),
                max_results: Some(2),
            })
            .await
            .unwrap();

        assert_eq!(page.result_count, 1);
        assert_eq!(page.next_token.as_deref(), Some("tok2"));
        assert_eq!(page.events[0].sender_id.as_deref(), Some("42"));
        assert_eq!(page.events[0].participant_ids, vec!["42", "99"]);
        assert_eq!(page.events[0].attachments[0].media_key, "m1");
    }

    #[tokio::test]
    async fn test_get_dm_conversation_events_requires_conversation_id() {
        let adapter = XAdapter::new_with_base_url(test_x_config_with_oauth2(), "http://127.0.0.1")
            .await
            .unwrap();
        let err = adapter
            .get_dm_conversation_events(" ", &XDmOptions::default())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("conversation_id"), "got: {err}");
    }

    #[tokio::test]
    async fn test_send_dm_posts_conversation_message_body() {
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/dm_conversations/c1/messages"))
            .and(body_json(serde_json::json!({
                "text": "ops ping",
                "attachments": [{"media_id": "mid1"}]
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "data": {
                    "dm_event_id": "e2",
                    "dm_conversation_id": "c1",
                    "text": "ops ping"
                }
            })))
            .mount(&server)
            .await;

        let adapter = XAdapter::new_with_base_url(test_x_config_with_oauth2(), server.uri())
            .await
            .unwrap();
        let sent = adapter
            .send_dm("c1", " ops ping ", Some("mid1"))
            .await
            .unwrap();

        assert_eq!(sent.dm_event_id.as_deref(), Some("e2"));
        assert_eq!(sent.dm_conversation_id.as_deref(), Some("c1"));
        assert_eq!(sent.text.as_deref(), Some("ops ping"));
    }

    #[tokio::test]
    async fn test_send_dm_to_user_posts_with_user_path() {
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/dm_conversations/with/99/messages"))
            .and(body_json(serde_json::json!({"text": "hello"})))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "data": {"id": "e3", "dm_conversation_id": "c2", "text": "hello"}
            })))
            .mount(&server)
            .await;

        let adapter = XAdapter::new_with_base_url(test_x_config_with_oauth2(), server.uri())
            .await
            .unwrap();
        let sent = adapter.send_dm_to_user("99", "hello", None).await.unwrap();

        assert_eq!(sent.dm_event_id.as_deref(), Some("e3"));
        assert_eq!(sent.participant_id.as_deref(), Some("99"));
    }
    #[tokio::test]
    async fn test_get_tweet_metrics_uses_adapter_base_url() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/tweets/9001"))
            .and(query_param("tweet.fields", "public_metrics"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {
                    "id": "9001",
                    "public_metrics": {
                        "like_count": 7,
                        "retweet_count": 3,
                        "reply_count": 2,
                        "quote_count": 1,
                        "impression_count": 1234,
                        "bookmark_count": 5
                    }
                }
            })))
            .mount(&server)
            .await;

        let adapter = XAdapter::new_with_base_url(test_x_config_with_oauth2(), server.uri())
            .await
            .unwrap();
        let metrics = adapter.get_tweet_metrics("9001").await.unwrap();
        assert_eq!(metrics.like_count, 7);
        assert_eq!(metrics.impression_count, 1234);
        assert_eq!(metrics.bookmark_count, 5);
    }

    #[tokio::test]
    async fn test_get_account_metrics_uses_user_public_metrics() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/42"))
            .and(query_param("user.fields", "public_metrics"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {
                    "id": "42",
                    "username": "zeus",
                    "name": "Zeus",
                    "public_metrics": {
                        "followers_count": 100,
                        "following_count": 25,
                        "tweet_count": 300,
                        "listed_count": 4
                    }
                }
            })))
            .mount(&server)
            .await;

        let adapter = XAdapter::new_with_base_url(test_x_config_with_oauth2(), server.uri())
            .await
            .unwrap();
        let metrics = adapter.get_account_metrics("42").await.unwrap();
        assert_eq!(metrics.user_id, "42");
        assert_eq!(metrics.username.as_deref(), Some("zeus"));
        assert_eq!(metrics.followers_count, 100);
        assert_eq!(metrics.listed_count, 4);
    }
}
