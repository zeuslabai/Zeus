//! Content production pipeline for the cron scheduler.
//!
//! Wires FFmpeg editing tools (zeus-talos) and social upload tools into a
//! schedulable `TaskType::ContentPipeline` cron job. Pipeline stages:
//!
//!   1. Optional trim  — `ffmpeg_trim` crops source to \[start, end\]
//!   2. Platform resize — `ffmpeg_resize` scales + pads for the target platform
//!   3. Optional captions — `ffmpeg_add_captions` burns in an `.srt` subtitle file
//!   4. Upload  — YouTube Data API v3 · TikTok Content Posting API v2 · Instagram Graph API v21
//!
//! # Prerequisites
//! - `ffmpeg` binary in PATH
//! - Platform token env vars: `YOUTUBE_ACCESS_TOKEN`, `TIKTOK_ACCESS_TOKEN`,
//!   `INSTAGRAM_ACCESS_TOKEN` + `INSTAGRAM_BUSINESS_ACCOUNT_ID`
//!
//! # Instagram note
//! Instagram Graph API v21.0 requires a **publicly accessible URL** (`media_url`),
//! not a local file path. Set the `media_url` field in `ContentPipeline` when
//! targeting the `"instagram"` platform. The processed file will be ready at
//! `{input_path}.pipeline.instagram.mp4` for manual hosting if needed.

use serde_json::json;
use tracing::{info, warn};
use zeus_talos::{
    content_tools::{FfmpegAddCaptionsTool, FfmpegResizeTool, FfmpegTrimTool},
    instagram_tools::InstagramSendReelTool,
    tiktok_tools::TikTokUploadTool,
    youtube_tools::YouTubeUploadTool,
    TalosTool,
};

/// Execute a content production pipeline job.
///
/// Returns `(success, output_message)` — matching the `execute_task` convention.
///
/// # Arguments
/// - `input_path`   — local source video file path (ffmpeg stages + youtube/tiktok upload)
/// - `platform`     — upload target: `"youtube"` · `"tiktok"` · `"instagram"`
/// - `title`        — video title for upload metadata
/// - `description`  — video description for upload metadata
/// - `trim_start`   — optional trim start time (HH:MM:SS or seconds)
/// - `trim_end`     — optional trim end time (HH:MM:SS or seconds)
/// - `captions_srt` — optional path to `.srt` subtitle file to burn in
/// - `media_url`    — public video URL required for Instagram uploads
#[allow(clippy::too_many_arguments)]
pub async fn execute_content_pipeline(
    input_path: &str,
    platform: &str,
    title: &str,
    description: &str,
    trim_start: &Option<String>,
    trim_end: &Option<String>,
    captions_srt: &Option<String>,
    media_url: &Option<String>,
) -> (bool, String) {
    let base = format!("{input_path}.pipeline");
    let mut current_path = input_path.to_string();
    // Paths of intermediate files to clean up after upload.
    let mut intermediates: Vec<String> = Vec::new();

    // ------------------------------------------------------------------
    // Stage 1 — Optional trim
    // ------------------------------------------------------------------
    if let Some(start) = trim_start {
        let trimmed = format!("{base}.trimmed.mp4");
        let mut args = json!({
            "input": current_path,
            "output": trimmed,
            "start": start,
        });
        if let Some(end) = trim_end {
            args["end"] = json!(end);
        }
        match FfmpegTrimTool.execute(args).await {
            Ok(_) => {
                intermediates.push(current_path.clone());
                current_path = trimmed;
                info!("content_pipeline: trim → {current_path}");
            }
            Err(e) => {
                cleanup_intermediates(&intermediates);
                return (false, format!("trim failed: {e}"));
            }
        }
    }

    // ------------------------------------------------------------------
    // Stage 2 — Platform resize (always runs)
    // ------------------------------------------------------------------
    let resized = format!("{base}.{platform}.mp4");
    match FfmpegResizeTool
        .execute(json!({
            "input": current_path,
            "output": resized,
            "platform": platform,
        }))
        .await
    {
        Ok(_) => {
            intermediates.push(current_path.clone());
            current_path = resized;
            info!("content_pipeline: resize({platform}) → {current_path}");
        }
        Err(e) => {
            cleanup_intermediates(&intermediates);
            return (false, format!("resize failed: {e}"));
        }
    }

    // ------------------------------------------------------------------
    // Stage 3 — Optional caption burn-in (non-fatal)
    // ------------------------------------------------------------------
    if let Some(srt) = captions_srt {
        let captioned = format!("{base}.captioned.mp4");
        match FfmpegAddCaptionsTool
            .execute(json!({
                "input": current_path,
                "output": captioned,
                "srt": srt,
            }))
            .await
        {
            Ok(_) => {
                intermediates.push(current_path.clone());
                current_path = captioned;
                info!("content_pipeline: captions → {current_path}");
            }
            Err(e) => {
                // Non-fatal: log and continue without captions rather than aborting
                warn!("content_pipeline: captions failed ({e}) — proceeding without");
            }
        }
    }

    // ------------------------------------------------------------------
    // Stage 4 — Upload to platform
    // ------------------------------------------------------------------
    let upload_result = match platform {
        "youtube" => {
            YouTubeUploadTool
                .execute(json!({
                    "file": current_path,
                    "title": title,
                    "description": description,
                }))
                .await
        }
        "tiktok" => {
            TikTokUploadTool
                .execute(json!({
                    "file": current_path,
                    "title": title,
                }))
                .await
        }
        "instagram" => match media_url {
            Some(url) => {
                InstagramSendReelTool
                    .execute(json!({
                        "video_url": url,
                        "caption": format!("{title}\n\n{description}"),
                        "share_to_feed": true,
                    }))
                    .await
            }
            None => Ok(format!(
                "instagram: video ready at {current_path} — set media_url to a public URL to trigger upload"
            )),
        },
        "x" => {
            // X (Twitter) adapter — credentials + upload tool land via T5 (merakizzz)
            // and T1 (gateway_relays.rs wiring). Accept the job but surface the
            // missing-adapter state clearly so the queue doesn't silently drop.
            Ok(format!(
                "x: content queued at {current_path} — X upload adapter not yet wired (pending T1 gateway_relays.rs + T5 credentials)"
            ))
        }
        other => Err(zeus_core::Error::Tool(format!(
            "unknown platform '{other}' — valid: youtube · tiktok · instagram · x"
        ))),
    };

    // Clean up intermediate files, preserving the original input and final output.
    for f in &intermediates {
        if f != input_path {
            let _ = std::fs::remove_file(f);
        }
    }

    match upload_result {
        Ok(msg) => {
            info!("content_pipeline: complete — {msg}");
            (true, format!("[{platform}] '{title}': {msg}"))
        }
        Err(e) => (false, format!("[{platform}] upload failed: {e}")),
    }
}

fn cleanup_intermediates(files: &[String]) {
    for f in files {
        let _ = std::fs::remove_file(f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_noop_on_missing_files() {
        // cleanup_intermediates must not panic when files are absent.
        cleanup_intermediates(&[
            "/tmp/zeus_pipeline_nonexistent_a.mp4".to_string(),
            "/tmp/zeus_pipeline_nonexistent_b.mp4".to_string(),
        ]);
    }
}
