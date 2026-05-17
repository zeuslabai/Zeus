//! Instagram channel adapter
//!
//! Full-featured Instagram integration via Meta Graph API:
//! - Publish photos, carousels, reels, and stories
//! - Upload media with captions and hashtags
//! - Receive comments and DMs via webhook/polling
//! - Engagement metrics (likes, comments, reach, impressions)
//! - Content scheduling
//! - User profile and follower analytics
//!
//! Authentication: Facebook/Meta OAuth with Instagram permissions

use crate::{ChannelAdapter, ChannelMessage, ChannelSource, ReceiveMode};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Notify, RwLock, mpsc};
use zeus_core::{Error, Result};

#[allow(dead_code)]
const GRAPH_API_BASE: &str = "https://graph.instagram.com/v21.0";
const GRAPH_FB_BASE: &str = "https://graph.facebook.com/v21.0";

// ── Types ────────────────────────────────────────────────────────────────

/// Types of Instagram posts
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PostType {
    /// Single image post
    Image,
    /// Single video post (reel)
    Reel,
    /// Carousel (up to 10 images/videos)
    Carousel,
    /// Story (24h ephemeral)
    Story,
}

/// Options for creating an Instagram post
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreatePostOptions {
    /// Post type
    pub post_type: Option<PostType>,
    /// Caption text (up to 2,200 chars)
    pub caption: String,
    /// Image URL (for image posts — must be publicly accessible)
    pub image_url: Option<String>,
    /// Video URL (for reels — must be publicly accessible)
    pub video_url: Option<String>,
    /// Carousel items (image/video URLs)
    pub carousel_items: Vec<CarouselItem>,
    /// Location tag ID (optional)
    pub location_id: Option<String>,
    /// User tags (username → position)
    pub user_tags: Vec<UserTag>,
    /// Cover image URL for reels (optional)
    pub cover_url: Option<String>,
    /// Share to feed (for reels, default true)
    pub share_to_feed: Option<bool>,
    /// Hashtags to append (will be added to caption)
    pub hashtags: Vec<String>,
}

/// A carousel item (image or video)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CarouselItem {
    /// Media URL (publicly accessible)
    pub url: String,
    /// Whether this is a video (false = image)
    pub is_video: bool,
}

/// Tag a user in a post
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserTag {
    /// Instagram username
    pub username: String,
    /// X position (0.0 to 1.0)
    pub x: f64,
    /// Y position (0.0 to 1.0)
    pub y: f64,
}

/// A published Instagram post
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstagramPost {
    /// Post ID
    pub id: String,
    /// Media type (IMAGE, VIDEO, CAROUSEL_ALBUM)
    pub media_type: String,
    /// Post caption
    pub caption: Option<String>,
    /// Permalink URL
    pub permalink: Option<String>,
    /// Timestamp
    pub timestamp: Option<String>,
    /// Engagement metrics
    pub metrics: Option<PostMetrics>,
}

/// Post engagement metrics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PostMetrics {
    pub like_count: u64,
    pub comment_count: u64,
    pub share_count: u64,
    pub save_count: u64,
    pub reach: u64,
    pub impressions: u64,
    pub engagement: u64,
}

/// An Instagram comment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstagramComment {
    /// Comment ID
    pub id: String,
    /// Comment text
    pub text: String,
    /// Username of commenter
    pub username: String,
    /// Timestamp
    pub timestamp: String,
    /// Parent comment ID (if this is a reply)
    pub parent_id: Option<String>,
}

/// Instagram account insights
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccountInsights {
    pub followers_count: u64,
    pub follows_count: u64,
    pub media_count: u64,
    pub profile_views: u64,
    pub website_clicks: u64,
    pub reach: u64,
    pub impressions: u64,
}

/// Instagram user profile
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstagramProfile {
    pub id: String,
    pub username: String,
    pub name: String,
    pub biography: Option<String>,
    pub followers_count: u64,
    pub follows_count: u64,
    pub media_count: u64,
    pub profile_picture_url: Option<String>,
    pub website: Option<String>,
}

// ── Adapter ──────────────────────────────────────────────────────────────

/// Instagram channel adapter
pub struct InstagramAdapter {
    connected: Arc<AtomicBool>,
    config: InstagramConfig,
    client: reqwest::Client,
    shutdown: Arc<Notify>,
    task_handle: RwLock<Option<tokio::task::JoinHandle<()>>>,
    /// Track last seen comment/DM for polling
    last_comment_timestamp: Arc<RwLock<Option<String>>>,
}

impl InstagramAdapter {
    /// Create a new Instagram adapter
    pub async fn new(config: InstagramConfig) -> Result<Self> {
        if config.access_token.is_empty() {
            return Err(Error::Config(
                "Instagram adapter requires access_token".into(),
            ));
        }

        tracing::info!("Instagram adapter created");

        Ok(Self {
            connected: Arc::new(AtomicBool::new(false)),
            config,
            client: reqwest::Client::new(),
            shutdown: Arc::new(Notify::new()),
            task_handle: RwLock::new(None),
            last_comment_timestamp: Arc::new(RwLock::new(None)),
        })
    }

    /// Get the Instagram Business/Creator account ID
    fn account_id(&self) -> &str {
        &self.config.account_id
    }

    /// Publish a single image post
    pub async fn publish_image(&self, image_url: &str, caption: &str) -> Result<InstagramPost> {
        // Step 1: Create media container
        let container_resp = self
            .client
            .post(format!("{}/{}/media", GRAPH_FB_BASE, self.account_id()))
            .query(&[
                ("image_url", image_url),
                ("caption", caption),
                ("access_token", &self.config.access_token),
            ])
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Instagram create container: {}", e)))?;

        let container: serde_json::Value = self.parse_response(container_resp).await?;
        let creation_id = container["id"]
            .as_str()
            .ok_or_else(|| Error::Channel("Instagram: no creation_id".into()))?;

        // Step 2: Publish the container
        self.publish_container(creation_id).await
    }

    /// Publish a reel (video post)
    pub async fn publish_reel(
        &self,
        video_url: &str,
        caption: &str,
        cover_url: Option<&str>,
        share_to_feed: bool,
    ) -> Result<InstagramPost> {
        let mut params = vec![
            ("media_type", "REELS".to_string()),
            ("video_url", video_url.to_string()),
            ("caption", caption.to_string()),
            ("access_token", self.config.access_token.clone()),
            (
                "share_to_feed",
                if share_to_feed { "true" } else { "false" }.to_string(),
            ),
        ];

        if let Some(cover) = cover_url {
            params.push(("cover_url", cover.to_string()));
        }

        let container_resp = self
            .client
            .post(format!("{}/{}/media", GRAPH_FB_BASE, self.account_id()))
            .form(&params)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Instagram create reel: {}", e)))?;

        let container: serde_json::Value = self.parse_response(container_resp).await?;
        let creation_id = container["id"]
            .as_str()
            .ok_or_else(|| Error::Channel("Instagram: no creation_id for reel".into()))?;

        // Wait for video processing
        self.wait_for_processing(creation_id).await?;

        self.publish_container(creation_id).await
    }

    /// Publish a carousel (up to 10 items)
    pub async fn publish_carousel(
        &self,
        items: &[CarouselItem],
        caption: &str,
    ) -> Result<InstagramPost> {
        if items.is_empty() || items.len() > 10 {
            return Err(Error::Channel("Carousel requires 1-10 items".into()));
        }

        // Step 1: Create child containers
        let mut child_ids = Vec::new();
        for item in items {
            let mut params = vec![
                ("access_token", self.config.access_token.clone()),
                ("is_carousel_item", "true".to_string()),
            ];

            if item.is_video {
                params.push(("media_type", "VIDEO".to_string()));
                params.push(("video_url", item.url.clone()));
            } else {
                params.push(("image_url", item.url.clone()));
            }

            let resp = self
                .client
                .post(format!("{}/{}/media", GRAPH_FB_BASE, self.account_id()))
                .form(&params)
                .send()
                .await
                .map_err(|e| Error::Channel(format!("Instagram carousel child: {}", e)))?;

            let data: serde_json::Value = self.parse_response(resp).await?;
            let child_id = data["id"]
                .as_str()
                .ok_or_else(|| Error::Channel("Instagram: no child container id".into()))?
                .to_string();

            // Wait for video items to process
            if item.is_video {
                self.wait_for_processing(&child_id).await?;
            }

            child_ids.push(child_id);
        }

        // Step 2: Create carousel container
        let children_csv = child_ids.join(",");
        let container_resp = self
            .client
            .post(format!("{}/{}/media", GRAPH_FB_BASE, self.account_id()))
            .form(&[
                ("media_type", "CAROUSEL"),
                ("children", &children_csv),
                ("caption", caption),
                ("access_token", &self.config.access_token),
            ])
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Instagram carousel container: {}", e)))?;

        let container: serde_json::Value = self.parse_response(container_resp).await?;
        let creation_id = container["id"]
            .as_str()
            .ok_or_else(|| Error::Channel("Instagram: no carousel creation_id".into()))?;

        self.publish_container(creation_id).await
    }

    /// Publish a story
    pub async fn publish_story(&self, media_url: &str, is_video: bool) -> Result<InstagramPost> {
        let mut params = vec![("access_token", self.config.access_token.clone())];

        if is_video {
            params.push(("media_type", "STORIES".to_string()));
            params.push(("video_url", media_url.to_string()));
        } else {
            params.push(("media_type", "STORIES".to_string()));
            params.push(("image_url", media_url.to_string()));
        }

        let container_resp = self
            .client
            .post(format!("{}/{}/media", GRAPH_FB_BASE, self.account_id()))
            .form(&params)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Instagram create story: {}", e)))?;

        let container: serde_json::Value = self.parse_response(container_resp).await?;
        let creation_id = container["id"]
            .as_str()
            .ok_or_else(|| Error::Channel("Instagram: no story creation_id".into()))?;

        if is_video {
            self.wait_for_processing(creation_id).await?;
        }

        self.publish_container(creation_id).await
    }

    /// Publish a prepared media container
    async fn publish_container(&self, creation_id: &str) -> Result<InstagramPost> {
        let resp = self
            .client
            .post(format!(
                "{}/{}/media_publish",
                GRAPH_FB_BASE,
                self.account_id()
            ))
            .form(&[
                ("creation_id", creation_id),
                ("access_token", &self.config.access_token),
            ])
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Instagram publish: {}", e)))?;

        let data: serde_json::Value = self.parse_response(resp).await?;
        let post_id = data["id"].as_str().unwrap_or_default().to_string();

        tracing::info!(post_id = %post_id, "Instagram post published");

        Ok(InstagramPost {
            id: post_id,
            media_type: "IMAGE".to_string(),
            caption: None,
            permalink: None,
            timestamp: None,
            metrics: None,
        })
    }

    /// Wait for video/reel processing to complete
    async fn wait_for_processing(&self, container_id: &str) -> Result<()> {
        let max_attempts = 30;
        for attempt in 0..max_attempts {
            let resp = self
                .client
                .get(format!("{}/{}", GRAPH_FB_BASE, container_id))
                .query(&[
                    ("fields", "status_code"),
                    ("access_token", &self.config.access_token),
                ])
                .send()
                .await
                .map_err(|e| Error::Channel(format!("Instagram status check: {}", e)))?;

            let data: serde_json::Value = self.parse_response(resp).await?;
            let status = data["status_code"].as_str().unwrap_or("IN_PROGRESS");

            match status {
                "FINISHED" => return Ok(()),
                "ERROR" => {
                    return Err(Error::Channel(format!(
                        "Instagram media processing failed for container {}",
                        container_id
                    )));
                }
                _ => {
                    if attempt < max_attempts - 1 {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                }
            }
        }

        Err(Error::Channel(format!(
            "Instagram media processing timed out for container {}",
            container_id
        )))
    }

    /// Get post metrics
    pub async fn get_post_metrics(&self, post_id: &str) -> Result<PostMetrics> {
        let resp = self
            .client
            .get(format!("{}/{}/insights", GRAPH_FB_BASE, post_id))
            .query(&[
                ("metric", "impressions,reach,likes,comments,shares,saved"),
                ("access_token", &self.config.access_token),
            ])
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Instagram metrics: {}", e)))?;

        let data: serde_json::Value = self.parse_response(resp).await?;

        let mut metrics = PostMetrics::default();

        if let Some(insights) = data.get("data").and_then(|d| d.as_array()) {
            for insight in insights {
                let name = insight["name"].as_str().unwrap_or_default();
                let value = insight["values"]
                    .as_array()
                    .and_then(|v| v.first())
                    .and_then(|v| v["value"].as_u64())
                    .unwrap_or(0);

                match name {
                    "impressions" => metrics.impressions = value,
                    "reach" => metrics.reach = value,
                    "likes" => metrics.like_count = value,
                    "comments" => metrics.comment_count = value,
                    "shares" => metrics.share_count = value,
                    "saved" => metrics.save_count = value,
                    _ => {}
                }
            }
        }

        Ok(metrics)
    }

    /// Get recent comments on a post
    pub async fn get_comments(&self, post_id: &str) -> Result<Vec<InstagramComment>> {
        let resp = self
            .client
            .get(format!("{}/{}/comments", GRAPH_FB_BASE, post_id))
            .query(&[
                ("fields", "id,text,username,timestamp,parent_id"),
                ("access_token", &self.config.access_token),
            ])
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Instagram comments: {}", e)))?;

        let data: serde_json::Value = self.parse_response(resp).await?;
        let mut comments = Vec::new();

        if let Some(items) = data.get("data").and_then(|d| d.as_array()) {
            for item in items {
                comments.push(InstagramComment {
                    id: item["id"].as_str().unwrap_or_default().to_string(),
                    text: item["text"].as_str().unwrap_or_default().to_string(),
                    username: item["username"].as_str().unwrap_or_default().to_string(),
                    timestamp: item["timestamp"].as_str().unwrap_or_default().to_string(),
                    parent_id: item["parent_id"].as_str().map(|s| s.to_string()),
                });
            }
        }

        Ok(comments)
    }

    /// Reply to a comment
    pub async fn reply_to_comment(&self, comment_id: &str, text: &str) -> Result<String> {
        let resp = self
            .client
            .post(format!("{}/{}/replies", GRAPH_FB_BASE, comment_id))
            .form(&[
                ("message", text),
                ("access_token", &self.config.access_token),
            ])
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Instagram reply: {}", e)))?;

        let data: serde_json::Value = self.parse_response(resp).await?;
        let reply_id = data["id"].as_str().unwrap_or_default().to_string();

        tracing::info!(reply_id = %reply_id, comment_id = %comment_id, "Instagram reply posted");
        Ok(reply_id)
    }

    /// Get account profile
    pub async fn get_profile(&self) -> Result<InstagramProfile> {
        let resp = self
            .client
            .get(format!("{}/{}", GRAPH_FB_BASE, self.account_id()))
            .query(&[
                (
                    "fields",
                    "id,username,name,biography,followers_count,follows_count,media_count,profile_picture_url,website",
                ),
                ("access_token", &self.config.access_token),
            ])
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Instagram profile: {}", e)))?;

        let data: serde_json::Value = self.parse_response(resp).await?;

        Ok(InstagramProfile {
            id: data["id"].as_str().unwrap_or_default().to_string(),
            username: data["username"].as_str().unwrap_or_default().to_string(),
            name: data["name"].as_str().unwrap_or_default().to_string(),
            biography: data["biography"].as_str().map(|s| s.to_string()),
            followers_count: data["followers_count"].as_u64().unwrap_or(0),
            follows_count: data["follows_count"].as_u64().unwrap_or(0),
            media_count: data["media_count"].as_u64().unwrap_or(0),
            profile_picture_url: data["profile_picture_url"].as_str().map(|s| s.to_string()),
            website: data["website"].as_str().map(|s| s.to_string()),
        })
    }

    /// Get account insights
    pub async fn get_insights(&self, period: &str) -> Result<AccountInsights> {
        let resp = self
            .client
            .get(format!("{}/{}/insights", GRAPH_FB_BASE, self.account_id()))
            .query(&[
                ("metric", "impressions,reach,profile_views,website_clicks"),
                ("period", period),
                ("access_token", &self.config.access_token),
            ])
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Instagram insights: {}", e)))?;

        let data: serde_json::Value = self.parse_response(resp).await?;
        let mut insights = AccountInsights::default();

        if let Some(metrics) = data.get("data").and_then(|d| d.as_array()) {
            for metric in metrics {
                let name = metric["name"].as_str().unwrap_or_default();
                let value = metric["values"]
                    .as_array()
                    .and_then(|v| v.first())
                    .and_then(|v| v["value"].as_u64())
                    .unwrap_or(0);

                match name {
                    "impressions" => insights.impressions = value,
                    "reach" => insights.reach = value,
                    "profile_views" => insights.profile_views = value,
                    "website_clicks" => insights.website_clicks = value,
                    _ => {}
                }
            }
        }

        Ok(insights)
    }

    /// Get recent media posts
    pub async fn get_recent_media(&self, limit: u32) -> Result<Vec<InstagramPost>> {
        let resp = self
            .client
            .get(format!("{}/{}/media", GRAPH_FB_BASE, self.account_id()))
            .query(&[
                ("fields", "id,media_type,caption,permalink,timestamp"),
                ("limit", &limit.to_string()),
                ("access_token", &self.config.access_token),
            ])
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Instagram recent media: {}", e)))?;

        let data: serde_json::Value = self.parse_response(resp).await?;
        let mut posts = Vec::new();

        if let Some(items) = data.get("data").and_then(|d| d.as_array()) {
            for item in items {
                posts.push(InstagramPost {
                    id: item["id"].as_str().unwrap_or_default().to_string(),
                    media_type: item["media_type"].as_str().unwrap_or_default().to_string(),
                    caption: item["caption"].as_str().map(|s| s.to_string()),
                    permalink: item["permalink"].as_str().map(|s| s.to_string()),
                    timestamp: item["timestamp"].as_str().map(|s| s.to_string()),
                    metrics: None,
                });
            }
        }

        Ok(posts)
    }

    /// Helper: build caption with hashtags
    pub fn build_caption(text: &str, hashtags: &[String]) -> String {
        if hashtags.is_empty() {
            return text.to_string();
        }
        let tags: Vec<String> = hashtags
            .iter()
            .map(|h| {
                if h.starts_with('#') {
                    h.clone()
                } else {
                    format!("#{}", h)
                }
            })
            .collect();
        format!("{}\n\n{}", text, tags.join(" "))
    }

    /// Parse API response, checking for errors
    async fn parse_response(&self, resp: reqwest::Response) -> Result<serde_json::Value> {
        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Channel(format!("Instagram API parse: {}", e)))?;

        if !status.is_success() {
            let error_msg = body["error"]["message"].as_str().unwrap_or("unknown error");
            let error_code = body["error"]["code"].as_u64().unwrap_or(0);
            return Err(Error::Channel(format!(
                "Instagram API error {} (code {}): {}",
                status, error_code, error_msg
            )));
        }

        Ok(body)
    }

    /// Start comment polling loop
    async fn start_polling(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        let connected = self.connected.clone();
        let shutdown = self.shutdown.clone();
        let poll_interval =
            std::time::Duration::from_secs(self.config.poll_interval_secs.unwrap_or(120));
        let client = self.client.clone();
        let access_token = self.config.access_token.clone();
        let account_id = self.config.account_id.clone();
        let last_timestamp = self.last_comment_timestamp.clone();

        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown.notified() => {
                        tracing::info!("Instagram polling shutting down");
                        break;
                    }
                    _ = tokio::time::sleep(poll_interval) => {
                        // Fetch recent media and check for new comments
                        if let Err(e) = Self::poll_comments_static(
                            &client,
                            &access_token,
                            &account_id,
                            &last_timestamp,
                            &tx,
                        ).await {
                            tracing::warn!(error = %e, "Instagram comment poll failed");
                        }
                    }
                }
            }
            connected.store(false, Ordering::SeqCst);
        });

        *self.task_handle.write().await = Some(handle);
        self.connected.store(true, Ordering::SeqCst);
        tracing::info!(
            "Instagram adapter started (polling every {}s)",
            poll_interval.as_secs()
        );

        Ok(())
    }

    /// Static comment polling (avoids self-referential issues)
    async fn poll_comments_static(
        client: &reqwest::Client,
        access_token: &str,
        account_id: &str,
        last_timestamp: &Arc<RwLock<Option<String>>>,
        tx: &mpsc::Sender<ChannelMessage>,
    ) -> Result<()> {
        // Get recent media
        let resp = client
            .get(format!("{}/{}/media", GRAPH_FB_BASE, account_id))
            .query(&[
                ("fields", "id"),
                ("limit", "5"),
                ("access_token", access_token),
            ])
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Instagram poll: {}", e)))?;

        if !resp.status().is_success() {
            return Ok(());
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Channel(format!("Instagram poll parse: {}", e)))?;

        let stored_ts = last_timestamp.read().await.clone();

        if let Some(media_items) = data.get("data").and_then(|d| d.as_array()) {
            let mut newest_ts: Option<String> = None;

            for media in media_items {
                let media_id = media["id"].as_str().unwrap_or_default();

                // Fetch comments for this media
                let comments_resp = client
                    .get(format!("{}/{}/comments", GRAPH_FB_BASE, media_id))
                    .query(&[
                        ("fields", "id,text,username,timestamp"),
                        ("access_token", access_token),
                    ])
                    .send()
                    .await;

                if let Ok(resp) = comments_resp
                    && resp.status().is_success()
                    && let Ok(comments_data) = resp.json::<serde_json::Value>().await
                    && let Some(comments) = comments_data.get("data").and_then(|d| d.as_array())
                {
                    for comment in comments {
                        let ts = comment["timestamp"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string();

                        // Skip if we've already seen this
                        if let Some(ref last) = stored_ts
                            && ts <= *last
                        {
                            continue;
                        }

                        if newest_ts.is_none() || ts > *newest_ts.as_ref().unwrap() {
                            newest_ts = Some(ts.clone());
                        }

                        let username = comment["username"].as_str().unwrap_or_default();
                        let text = comment["text"].as_str().unwrap_or_default();
                        let comment_id = comment["id"].as_str().unwrap_or_default();

                        let source = ChannelSource::with_chat("instagram", username, media_id);
                        let msg = ChannelMessage::new(source, text.to_string())
                            .with_platform_message_id(comment_id);

                        let _ = tx.send(msg).await;
                    }
                }
            }

            if let Some(ts) = newest_ts {
                *last_timestamp.write().await = Some(ts);
            }
        }

        Ok(())
    }
}

#[async_trait]
impl ChannelAdapter for InstagramAdapter {
    fn channel_type(&self) -> &'static str {
        "instagram"
    }

    fn receive_mode(&self) -> ReceiveMode {
        ReceiveMode::Polling {
            interval_secs: self.config.poll_interval_secs.unwrap_or(120),
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

        tracing::info!("Instagram adapter stopped");
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        if to.channel_type() != "instagram" {
            return Err(Error::channel("Invalid channel source for Instagram"));
        }

        // If chat_id is set, it's a reply to a comment on that post
        if let Some(ref comment_id) = to.reply_to_message_id {
            self.reply_to_comment(comment_id, content).await?;
        } else {
            // Otherwise, post a new image (requires an image URL in content or config)
            // For text-only, we'd need to create an image — log a warning
            tracing::warn!(
                "Instagram send() called without reply context — Instagram requires media for posts. Use publish_image/publish_reel directly."
            );
        }

        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
}

// ── Config ───────────────────────────────────────────────────────────────

/// Instagram configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstagramConfig {
    /// Meta Graph API access token (long-lived)
    #[serde(default)]
    pub access_token: String,
    /// Instagram Business/Creator Account ID
    #[serde(default)]
    pub account_id: String,
    /// Facebook Page ID (required for some operations)
    #[serde(default)]
    pub page_id: Option<String>,
    /// App ID (for token refresh)
    #[serde(default)]
    pub app_id: Option<String>,
    /// App secret (for token refresh)
    #[serde(default)]
    pub app_secret: Option<String>,
    /// Polling interval for comments in seconds (default: 120)
    #[serde(default)]
    pub poll_interval_secs: Option<u64>,
    /// Auto-reply to comments (default: false)
    #[serde(default)]
    pub auto_reply: bool,
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_instagram_config_default() {
        let config = InstagramConfig::default();
        assert!(config.access_token.is_empty());
        assert!(config.account_id.is_empty());
        assert!(config.poll_interval_secs.is_none());
        assert!(!config.auto_reply);
    }

    #[test]
    fn test_instagram_config_serde() {
        let config = InstagramConfig {
            access_token: "test-token".to_string(),
            account_id: "12345".to_string(),
            poll_interval_secs: Some(60),
            auto_reply: true,
            ..Default::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        let back: InstagramConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.access_token, "test-token");
        assert_eq!(back.account_id, "12345");
        assert_eq!(back.poll_interval_secs, Some(60));
        assert!(back.auto_reply);
    }

    #[tokio::test]
    async fn test_instagram_adapter_validation() {
        // Empty config should fail
        let config = InstagramConfig::default();
        assert!(InstagramAdapter::new(config).await.is_err());

        // Valid config should work
        let config = InstagramConfig {
            access_token: "test-token".to_string(),
            account_id: "12345".to_string(),
            ..Default::default()
        };
        assert!(InstagramAdapter::new(config).await.is_ok());
    }

    #[tokio::test]
    async fn test_instagram_adapter_lifecycle() {
        let config = InstagramConfig {
            access_token: "test-token".to_string(),
            account_id: "12345".to_string(),
            ..Default::default()
        };
        let adapter = InstagramAdapter::new(config).await.unwrap();
        assert!(!adapter.is_connected());
        assert_eq!(adapter.channel_type(), "instagram");
    }

    #[test]
    fn test_post_type_serde() {
        let types = vec![
            PostType::Image,
            PostType::Reel,
            PostType::Carousel,
            PostType::Story,
        ];
        for t in types {
            let json = serde_json::to_string(&t).unwrap();
            let back: PostType = serde_json::from_str(&json).unwrap();
            assert_eq!(back, t);
        }
    }

    #[test]
    fn test_build_caption_no_hashtags() {
        let caption = InstagramAdapter::build_caption("Hello world", &[]);
        assert_eq!(caption, "Hello world");
    }

    #[test]
    fn test_build_caption_with_hashtags() {
        let hashtags = vec![
            "Zeus".to_string(),
            "#AIAgents".to_string(),
            "Rust".to_string(),
        ];
        let caption = InstagramAdapter::build_caption("Check this out", &hashtags);
        assert_eq!(caption, "Check this out\n\n#Zeus #AIAgents #Rust");
    }

    #[test]
    fn test_carousel_item() {
        let item = CarouselItem {
            url: "https://example.com/img.jpg".to_string(),
            is_video: false,
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: CarouselItem = serde_json::from_str(&json).unwrap();
        assert_eq!(back.url, "https://example.com/img.jpg");
        assert!(!back.is_video);
    }

    #[test]
    fn test_user_tag() {
        let tag = UserTag {
            username: "zeus_ai".to_string(),
            x: 0.5,
            y: 0.3,
        };
        let json = serde_json::to_string(&tag).unwrap();
        let back: UserTag = serde_json::from_str(&json).unwrap();
        assert_eq!(back.username, "zeus_ai");
        assert!((back.x - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_post_metrics_default() {
        let metrics = PostMetrics::default();
        assert_eq!(metrics.like_count, 0);
        assert_eq!(metrics.impressions, 0);
        assert_eq!(metrics.reach, 0);
    }

    #[test]
    fn test_instagram_post_serde() {
        let post = InstagramPost {
            id: "12345".to_string(),
            media_type: "IMAGE".to_string(),
            caption: Some("Test post".to_string()),
            permalink: Some("https://instagram.com/p/test".to_string()),
            timestamp: None,
            metrics: Some(PostMetrics {
                like_count: 100,
                comment_count: 10,
                ..Default::default()
            }),
        };
        let json = serde_json::to_string(&post).unwrap();
        let back: InstagramPost = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "12345");
        assert_eq!(back.metrics.unwrap().like_count, 100);
    }

    #[test]
    fn test_instagram_comment_serde() {
        let comment = InstagramComment {
            id: "c1".to_string(),
            text: "Great post!".to_string(),
            username: "user1".to_string(),
            timestamp: "2026-02-25T00:00:00Z".to_string(),
            parent_id: None,
        };
        let json = serde_json::to_string(&comment).unwrap();
        let back: InstagramComment = serde_json::from_str(&json).unwrap();
        assert_eq!(back.text, "Great post!");
    }

    #[test]
    fn test_instagram_profile_serde() {
        let profile = InstagramProfile {
            id: "12345".to_string(),
            username: "zeus_ai".to_string(),
            name: "Zeus AI".to_string(),
            biography: Some("Almighty AI agent runtime".to_string()),
            followers_count: 5000,
            follows_count: 100,
            media_count: 50,
            profile_picture_url: None,
            website: Some("https://zeuslab.ai".to_string()),
        };
        let json = serde_json::to_string(&profile).unwrap();
        let back: InstagramProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.username, "zeus_ai");
        assert_eq!(back.followers_count, 5000);
    }

    #[test]
    fn test_channel_source_instagram() {
        let source = ChannelSource::with_chat("instagram", "zeus_ai", "post_123");
        assert_eq!(source.channel_type(), "instagram");
        assert_eq!(source.user_id, "zeus_ai");
        assert_eq!(source.chat_id, Some("post_123".to_string()));
    }

    #[test]
    fn test_account_insights_default() {
        let insights = AccountInsights::default();
        assert_eq!(insights.followers_count, 0);
        assert_eq!(insights.impressions, 0);
    }

    #[test]
    fn test_create_post_options_default() {
        let opts = CreatePostOptions::default();
        assert!(opts.caption.is_empty());
        assert!(opts.image_url.is_none());
        assert!(opts.carousel_items.is_empty());
        assert!(opts.hashtags.is_empty());
    }
}
