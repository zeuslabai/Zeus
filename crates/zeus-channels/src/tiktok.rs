//! TikTok Content Posting API channel adapter.
//!
//! Thin zeus-channels wrapper around the same TikTok Content Posting API flow
//! exposed by the Talos tools:
//! 1. `POST /v2/post/publish/video/init/` to create a publish session.
//! 2. `PUT` the video bytes to the returned `upload_url`.
//! 3. `GET /v2/post/publish/status/fetch/?publish_id=...` to poll status.

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use zeus_core::{Error, Result};

use crate::{ChannelAdapter, ChannelMessage, ChannelSource, ReceiveMode};

const TIKTOK_API_BASE: &str = "https://open.tiktokapis.com";

/// TikTok Content Posting API adapter.
#[derive(Debug)]
pub struct TikTokAdapter {
    config: TikTokConfig,
    client: Client,
    api_base: String,
    connected: AtomicBool,
}

impl TikTokAdapter {
    /// Create a TikTok adapter using the default Content Posting API base URL.
    pub async fn new(config: TikTokConfig) -> Result<Self> {
        Self::new_with_base_url(config, TIKTOK_API_BASE).await
    }

    async fn new_with_base_url(config: TikTokConfig, api_base: impl Into<String>) -> Result<Self> {
        if config.access_token.is_empty() {
            return Err(Error::config(
                "TikTok adapter requires access_token with video.publish scope",
            ));
        }

        Ok(Self {
            config,
            client: Client::new(),
            api_base: api_base.into().trim_end_matches('/').to_string(),
            connected: AtomicBool::new(false),
        })
    }

    /// Initialize a TikTok video upload session.
    pub async fn init_video_upload(
        &self,
        options: TikTokUploadOptions,
    ) -> Result<TikTokUploadInit> {
        let body = build_init_body(&options);
        let response = self
            .post_json("/v2/post/publish/video/init/", &body)
            .await?;

        Ok(TikTokUploadInit {
            publish_id: response["data"]["publish_id"]
                .as_str()
                .ok_or_else(|| Error::channel("Missing publish_id in TikTok response"))?
                .to_string(),
            upload_url: response["data"]["upload_url"]
                .as_str()
                .ok_or_else(|| Error::channel("Missing upload_url in TikTok response"))?
                .to_string(),
        })
    }

    /// Upload local video bytes to an initialized TikTok upload URL.
    pub async fn upload_video_file(
        &self,
        upload_url: &str,
        file_path: impl AsRef<Path>,
    ) -> Result<()> {
        let bytes = tokio::fs::read(file_path.as_ref())
            .await
            .map_err(|e| Error::channel(format!("Failed to read TikTok video file: {e}")))?;
        self.upload_video_bytes(upload_url, bytes).await
    }

    /// Upload video bytes to an initialized TikTok upload URL.
    pub async fn upload_video_bytes(
        &self,
        upload_url: &str,
        bytes: impl Into<Vec<u8>>,
    ) -> Result<()> {
        let response = self
            .client
            .put(upload_url)
            .header("Content-Type", "video/mp4")
            .body(bytes.into())
            .send()
            .await
            .map_err(|e| Error::Network(format!("TikTok upload PUT failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::channel(format!(
                "TikTok upload PUT failed with status {status}: {body}"
            )));
        }

        Ok(())
    }

    /// Run the full file-upload flow and return the TikTok publish ID.
    pub async fn publish_video_file(
        &self,
        file_path: impl AsRef<Path>,
        options: TikTokUploadOptions,
    ) -> Result<TikTokUploadInit> {
        let init = self.init_video_upload(options).await?;
        self.upload_video_file(&init.upload_url, file_path).await?;
        Ok(init)
    }

    /// Fetch processing/publication status for a publish ID.
    pub async fn fetch_publish_status(&self, publish_id: &str) -> Result<TikTokPublishStatus> {
        let endpoint = format!("/v2/post/publish/status/fetch/?publish_id={publish_id}");
        let response = self.get_json(&endpoint).await?;

        Ok(TikTokPublishStatus {
            publish_id: publish_id.to_string(),
            status: response["data"]["status"]
                .as_str()
                .unwrap_or("UNKNOWN")
                .to_string(),
            fail_reason: response["data"]["fail_reason"].as_str().map(str::to_string),
            public_url: response["data"]["publicaly_available_post_id"]
                .as_array()
                .and_then(|ids| ids.first())
                .and_then(|id| id.as_str())
                .map(|id| format!("https://www.tiktok.com/@/video/{id}")),
        })
    }

    async fn post_json(&self, endpoint: &str, body: &Value) -> Result<Value> {
        let response = self
            .client
            .post(format!("{}{}", self.api_base, endpoint))
            .bearer_auth(&self.config.access_token)
            .header("Content-Type", "application/json; charset=UTF-8")
            .json(body)
            .send()
            .await
            .map_err(|e| Error::Network(format!("TikTok API call failed: {e}")))?;
        json_response(response).await
    }

    async fn get_json(&self, endpoint: &str) -> Result<Value> {
        let response = self
            .client
            .get(format!("{}{}", self.api_base, endpoint))
            .bearer_auth(&self.config.access_token)
            .send()
            .await
            .map_err(|e| Error::Network(format!("TikTok API call failed: {e}")))?;
        json_response(response).await
    }
}

#[async_trait]
impl ChannelAdapter for TikTokAdapter {
    fn channel_type(&self) -> &'static str {
        "tiktok"
    }

    fn receive_mode(&self) -> ReceiveMode {
        ReceiveMode::None
    }

    async fn start(&self, _tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        self.connected.store(true, Ordering::SeqCst);
        tracing::info!("TikTok adapter started");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);
        tracing::info!("TikTok adapter stopped");
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, _content: &str) -> Result<()> {
        if to.channel_type() != "tiktok" {
            return Err(Error::channel("Invalid channel source for TikTok"));
        }

        Err(Error::channel(
            "TikTok sends require publish_video_file or init_video_upload; plain text send is unsupported",
        ))
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
}

/// TikTok adapter configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TikTokConfig {
    /// OAuth2 user access token with `video.publish` scope.
    #[serde(default)]
    pub access_token: String,
}

/// TikTok privacy level for Content Posting uploads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TikTokPrivacyLevel {
    PublicToEveryone,
    MutualFollowFriends,
    FollowerOfCreator,
    #[default]
    SelfOnly,
}

impl TikTokPrivacyLevel {
    fn as_api_str(self) -> &'static str {
        match self {
            Self::PublicToEveryone => "PUBLIC_TO_EVERYONE",
            Self::MutualFollowFriends => "MUTUAL_FOLLOW_FRIENDS",
            Self::FollowerOfCreator => "FOLLOWER_OF_CREATOR",
            Self::SelfOnly => "SELF_ONLY",
        }
    }
}

/// TikTok Content Posting upload options.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TikTokUploadOptions {
    /// Video caption/title, max 2200 chars per TikTok API constraints.
    pub title: String,
    /// Upload privacy level.
    #[serde(default)]
    pub privacy: TikTokPrivacyLevel,
    /// Disable comments for the video.
    #[serde(default)]
    pub disable_comment: bool,
    /// Disable duet for the video.
    #[serde(default)]
    pub disable_duet: bool,
    /// Disable stitch for the video.
    #[serde(default)]
    pub disable_stitch: bool,
}

impl TikTokUploadOptions {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            privacy: TikTokPrivacyLevel::SelfOnly,
            disable_comment: false,
            disable_duet: false,
            disable_stitch: false,
        }
    }
}

/// TikTok initialized upload session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TikTokUploadInit {
    pub publish_id: String,
    pub upload_url: String,
}

/// TikTok publish status response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TikTokPublishStatus {
    pub publish_id: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fail_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_url: Option<String>,
}

fn build_init_body(options: &TikTokUploadOptions) -> Value {
    json!({
        "post_info": {
            "title": options.title,
            "privacy_level": options.privacy.as_api_str(),
            "disable_comment": options.disable_comment,
            "disable_duet": options.disable_duet,
            "disable_stitch": options.disable_stitch,
        },
        "source_info": {
            "source": "FILE_UPLOAD",
        },
    })
}

async fn json_response(response: reqwest::Response) -> Result<Value> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| Error::Network(format!("TikTok response read failed: {e}")))?;

    if !status.is_success() {
        return Err(Error::channel(format!(
            "TikTok API returned status {status}: {body}"
        )));
    }

    serde_json::from_str(&body).map_err(|e| Error::channel(format!("Invalid TikTok response: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_empty() {
        let config = TikTokConfig::default();
        assert!(config.access_token.is_empty());
    }

    #[test]
    fn privacy_serializes_as_content_posting_api_value() {
        assert_eq!(TikTokPrivacyLevel::SelfOnly.as_api_str(), "SELF_ONLY");
        assert_eq!(
            TikTokPrivacyLevel::PublicToEveryone.as_api_str(),
            "PUBLIC_TO_EVERYONE"
        );
    }

    #[test]
    fn init_body_matches_talos_content_posting_flow() {
        let mut options = TikTokUploadOptions::new("launch clip");
        options.privacy = TikTokPrivacyLevel::PublicToEveryone;
        options.disable_comment = true;
        options.disable_duet = true;
        options.disable_stitch = false;

        let body = build_init_body(&options);
        assert_eq!(body["post_info"]["title"], "launch clip");
        assert_eq!(body["post_info"]["privacy_level"], "PUBLIC_TO_EVERYONE");
        assert_eq!(body["post_info"]["disable_comment"], true);
        assert_eq!(body["post_info"]["disable_duet"], true);
        assert_eq!(body["post_info"]["disable_stitch"], false);
        assert_eq!(body["source_info"]["source"], "FILE_UPLOAD");
    }

    #[tokio::test]
    async fn new_requires_access_token() {
        let err = TikTokAdapter::new(TikTokConfig::default())
            .await
            .expect_err("missing access token should fail");
        assert!(err.to_string().contains("access_token"));
    }

    #[tokio::test]
    async fn init_video_upload_posts_expected_payload() {
        use wiremock::matchers::{body_json, header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v2/post/publish/video/init/"))
            .and(header("authorization", "Bearer tt_test"))
            .and(body_json(json!({
                "post_info": {
                    "title": "clip",
                    "privacy_level": "SELF_ONLY",
                    "disable_comment": false,
                    "disable_duet": false,
                    "disable_stitch": false,
                },
                "source_info": {
                    "source": "FILE_UPLOAD",
                },
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "publish_id": "pub_123",
                    "upload_url": format!("{}/upload", server.uri()),
                }
            })))
            .mount(&server)
            .await;

        let adapter = TikTokAdapter::new_with_base_url(
            TikTokConfig {
                access_token: "tt_test".to_string(),
            },
            server.uri(),
        )
        .await
        .unwrap();

        let init = adapter
            .init_video_upload(TikTokUploadOptions::new("clip"))
            .await
            .unwrap();

        assert_eq!(init.publish_id, "pub_123");
        assert!(init.upload_url.ends_with("/upload"));
    }

    #[tokio::test]
    async fn upload_video_bytes_puts_to_upload_url() {
        use wiremock::matchers::{body_bytes, header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/upload"))
            .and(header("content-type", "video/mp4"))
            .and(body_bytes(vec![1, 2, 3, 4]))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let adapter = TikTokAdapter::new_with_base_url(
            TikTokConfig {
                access_token: "tt_test".to_string(),
            },
            server.uri(),
        )
        .await
        .unwrap();

        adapter
            .upload_video_bytes(&format!("{}/upload", server.uri()), vec![1, 2, 3, 4])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn fetch_publish_status_maps_public_url() {
        use wiremock::matchers::{header, method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/post/publish/status/fetch/"))
            .and(query_param("publish_id", "pub_123"))
            .and(header("authorization", "Bearer tt_test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "status": "PUBLISH_COMPLETE",
                    "publicaly_available_post_id": ["7390000000000000000"]
                }
            })))
            .mount(&server)
            .await;

        let adapter = TikTokAdapter::new_with_base_url(
            TikTokConfig {
                access_token: "tt_test".to_string(),
            },
            server.uri(),
        )
        .await
        .unwrap();

        let status = adapter.fetch_publish_status("pub_123").await.unwrap();
        assert_eq!(status.publish_id, "pub_123");
        assert_eq!(status.status, "PUBLISH_COMPLETE");
        assert_eq!(
            status.public_url.as_deref(),
            Some("https://www.tiktok.com/@/video/7390000000000000000")
        );
    }

    #[tokio::test]
    async fn channel_send_rejects_plain_text() {
        let adapter = TikTokAdapter::new(TikTokConfig {
            access_token: "tt_test".to_string(),
        })
        .await
        .unwrap();
        let source = ChannelSource::new("tiktok", "creator");
        let err = adapter.send(&source, "hello").await.unwrap_err();
        assert!(err.to_string().contains("plain text send is unsupported"));
    }
}
