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
//! Replies use the v2 request shape `POST /2/tweets` with body:
//! `{"text": "...", "reply": {"in_reply_to_tweet_id": "..."}}`.
//! For Python/tweepy callers the equivalent is
//! `client.create_tweet(text="...", in_reply_to_tweet_id=TWEET_ID)` — NOT the
//! legacy v1.1 `reply={"in_reply_to_tweet_id": ...}` dict form, which v2
//! rejects. Validated end-to-end via ZeusMarketing integration test
//! (see `memory/2026-04-23.md`).

use crate::{AgentSendIdentity, ChannelAdapter, ChannelMessage, ChannelSource, ReceiveMode};
use async_trait::async_trait;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Notify, RwLock, mpsc};
use zeus_core::{Error, Result};

type HmacSha1 = Hmac<Sha1>;

const X_API_BASE: &str = "https://api.x.com/2";
const X_UPLOAD_BASE: &str = "https://upload.twitter.com/1.1";

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
            shutdown: Arc::new(Notify::new()),
            task_handle: RwLock::new(None),
            last_mention_id: Arc::new(RwLock::new(None)),
            oauth2_access_token: Arc::new(RwLock::new(initial_oauth2)),
        })
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
    fn read_auth_header(&self) -> String {
        let oauth2 = self.try_oauth2_token_snapshot();
        if !oauth2.is_empty() {
            format!("Bearer {}", oauth2)
        } else if !self.config.bearer_token.is_empty() {
            format!("Bearer {}", self.config.bearer_token)
        } else {
            // Fall back to OAuth 1.0a for reads too
            self.oauth1_header("GET", &format!("{}/users/me", X_API_BASE), &[])
        }
    }

    /// Get authorization header for write operations.
    ///
    /// Priority order:
    ///   1. OAuth 2.0 user-context access token (PKCE) — required for posting
    ///      tweets on most API tiers; the canonical write auth in 2024+.
    ///   2. OAuth 1.0a (legacy, still works for write endpoints with full
    ///      consumer + access token pair).
    ///   3. OAuth 2.0 App-Only bearer token (last resort — often read-only,
    ///      especially on free tier).
    fn write_auth_header(&self, method: &str, url: &str) -> String {
        let oauth2 = self.try_oauth2_token_snapshot();
        if !oauth2.is_empty() {
            return format!("Bearer {}", oauth2);
        }

        let has_oauth1 = !self.config.api_key.is_empty()
            && !self.config.api_secret.is_empty()
            && !self.config.access_token.is_empty()
            && !self.config.access_token_secret.is_empty();
        if has_oauth1 {
            self.oauth1_header(method, url, &[])
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

        let mut mac = HmacSha1::new_from_slice(signing_key.as_bytes())
            .expect("HMAC accepts any key length");
        mac.update(base_string.as_bytes());
        let signature = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());

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
        // Step 1: INIT
        let upload_url = format!("{}/media/upload.json", X_UPLOAD_BASE);
        let init_resp = self
            .client
            .post(&upload_url)
            .header("Authorization", self.write_auth_header("POST", &upload_url))
            .form(&[
                ("command", "INIT"),
                ("total_bytes", &data.len().to_string()),
                ("media_type", mime_type),
            ])
            .send()
            .await
            .map_err(|e| Error::Channel(format!("X media upload INIT error: {}", e)))?;

        if !init_resp.status().is_success() {
            let text = init_resp.text().await.unwrap_or_else(|_| "unknown".into());
            return Err(Error::Channel(format!("X media INIT failed: {}", text)));
        }

        let init_data: serde_json::Value = init_resp
            .json()
            .await
            .map_err(|e| Error::Channel(format!("X media INIT parse: {}", e)))?;

        let media_id = init_data["media_id_string"]
            .as_str()
            .ok_or_else(|| Error::Channel("X media: no media_id in INIT response".into()))?
            .to_string();

        // Step 2: APPEND (chunked for large files)
        let chunk_size = 5 * 1024 * 1024; // 5MB chunks
        for (i, chunk) in data.chunks(chunk_size).enumerate() {
            let part = reqwest::multipart::Part::bytes(chunk.to_vec())
                .file_name("media")
                .mime_str(mime_type)
                .map_err(|e| Error::Channel(format!("MIME error: {}", e)))?;

            let form = reqwest::multipart::Form::new()
                .text("command", "APPEND")
                .text("media_id", media_id.clone())
                .text("segment_index", i.to_string())
                .part("media_data", part);

            let append_resp = self
                .client
                .post(&upload_url)
                .header("Authorization", self.write_auth_header("POST", &upload_url))
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

        // Step 3: FINALIZE
        let finalize_resp = self
            .client
            .post(&upload_url)
            .header("Authorization", self.write_auth_header("POST", &upload_url))
            .form(&[("command", "FINALIZE"), ("media_id", &media_id)])
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

        // Step 4: Set alt text if provided
        if let Some(alt) = alt_text {
            let meta_url = format!("{}/media/metadata/create.json", X_UPLOAD_BASE);
            let _ = self
                .client
                .post(&meta_url)
                .header("Authorization", self.write_auth_header("POST", &meta_url))
                .json(&serde_json::json!({
                    "media_id": media_id,
                    "alt_text": { "text": alt }
                }))
                .send()
                .await;
        }

        tracing::info!(media_id = %media_id, "X media uploaded");
        Ok(media_id)
    }

    /// Delete a tweet
    pub async fn delete_tweet(&self, tweet_id: &str) -> Result<()> {
        let delete_url = format!("{}/tweets/{}", X_API_BASE, tweet_id);
        let resp = self
            .client
            .delete(&delete_url)
            .header("Authorization", self.write_auth_header("DELETE", &delete_url))
            .send()
            .await
            .map_err(|e| Error::Channel(format!("X delete error: {}", e)))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_else(|_| "unknown".into());
            return Err(Error::Channel(format!(
                "X delete tweet {} failed: {}",
                tweet_id, text
            )));
        }

        tracing::info!(tweet_id = %tweet_id, "X tweet deleted");
        Ok(())
    }

    /// Get tweet metrics
    pub async fn get_tweet_metrics(&self, tweet_id: &str) -> Result<TweetMetrics> {
        let resp = self
            .client
            .get(format!(
                "{}/tweets/{}?tweet.fields=public_metrics",
                X_API_BASE, tweet_id
            ))
            .header("Authorization", self.read_auth_header())
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

    /// Get authenticated user's profile
    pub async fn get_me(&self) -> Result<XUserProfile> {
        let resp = self
            .client
            .get(format!(
                "{}/users/me?user.fields=description,public_metrics,verified,profile_image_url",
                X_API_BASE
            ))
            .header("Authorization", self.read_auth_header())
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

    async fn send_as(&self, to: &ChannelSource, content: &str, _identity: &AgentSendIdentity) -> Result<()> {
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
    fn test_create_tweet_options_default() {
        let opts = CreateTweetOptions::default();
        assert!(opts.text.is_empty());
        assert!(opts.reply_to.is_none());
        assert!(opts.media_ids.is_empty());
        assert!(opts.poll_options.is_empty());
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
        let read_hdr = adapter.read_auth_header();
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
}
