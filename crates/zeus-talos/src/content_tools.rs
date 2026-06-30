//! Content production tools
//!
//! FFmpeg-based video editing: trim, concat, add captions, add audio, resize for platform.
//! Requires: ffmpeg + ffprobe binaries in PATH.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
use tokio::process::Command;
use zeus_core::{Error, Result, ToolSchema};

async fn run_ffmpeg(args: &[&str]) -> Result<String> {
    let output = Command::new("ffmpeg")
        .args(args)
        .output()
        .await
        .map_err(|e| Error::Tool(format!("ffmpeg not found or failed to start: {e}")))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if stderr.contains("Error") || stderr.contains("Invalid") || stderr.contains("No such file") {
            Err(Error::Tool(format!("ffmpeg error: {stderr}")))
        } else {
            Ok(stderr)
        }
    }
}

// ---------------------------------------------------------------------------
// 1. FfmpegTrimTool
// ---------------------------------------------------------------------------

pub struct FfmpegTrimTool;

#[async_trait]
impl TalosTool for FfmpegTrimTool {
    fn name(&self) -> &'static str { "ffmpeg_trim" }
    fn description(&self) -> &'static str {
        "Trim a video to a specific time range using FFmpeg"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("input", "string", "Input video file path", true)
            .with_param("output", "string", "Output video file path", true)
            .with_param("start", "string", "Start time (HH:MM:SS or seconds)", true)
            .with_param("duration", "string", "Duration to keep (HH:MM:SS or seconds)", false)
            .with_param("end", "string", "End time instead of duration (HH:MM:SS or seconds)", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let input = args["input"].as_str().ok_or_else(|| Error::Tool("Missing 'input'".into()))?;
        let output = args["output"].as_str().ok_or_else(|| Error::Tool("Missing 'output'".into()))?;
        let start = args["start"].as_str().ok_or_else(|| Error::Tool("Missing 'start'".into()))?;

        let mut ffmpeg_args = vec!["-y", "-i", input, "-ss", start];
        if let Some(dur) = args["duration"].as_str() {
            ffmpeg_args.extend_from_slice(&["-t", dur]);
        } else if let Some(end) = args["end"].as_str() {
            ffmpeg_args.extend_from_slice(&["-to", end]);
        }
        ffmpeg_args.extend_from_slice(&["-c", "copy", output]);
        run_ffmpeg(&ffmpeg_args).await?;
        Ok(format!("Trimmed video saved to: {output}"))
    }
}

// ---------------------------------------------------------------------------
// 2. FfmpegConcatTool
// ---------------------------------------------------------------------------

pub struct FfmpegConcatTool;

#[async_trait]
impl TalosTool for FfmpegConcatTool {
    fn name(&self) -> &'static str { "ffmpeg_concat" }
    fn description(&self) -> &'static str {
        "Concatenate multiple video files into one using FFmpeg"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("inputs", "array", "Array of input video file paths to concatenate in order", true)
            .with_param("output", "string", "Output video file path", true)
            .with_param("reencode", "boolean", "Re-encode for compatibility (default false = copy streams)", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let inputs = args["inputs"].as_array()
            .ok_or_else(|| Error::Tool("Missing 'inputs' array".into()))?;
        let output = args["output"].as_str()
            .ok_or_else(|| Error::Tool("Missing 'output'".into()))?;
        let reencode = args["reencode"].as_bool().unwrap_or(false);

        let list_path = format!("{output}.concat_list.txt");
        let list_content: String = inputs.iter()
            .filter_map(|v| v.as_str())
            .map(|p| format!("file '{}'\n", p))
            .collect();
        tokio::fs::write(&list_path, &list_content).await
            .map_err(|e| Error::Tool(format!("Failed to write concat list: {e}")))?;

        let codec_args: Vec<&str> = if reencode {
            vec!["-c:v", "libx264", "-c:a", "aac"]
        } else {
            vec!["-c", "copy"]
        };

        let mut ffmpeg_args = vec!["-y", "-f", "concat", "-safe", "0", "-i", &list_path];
        ffmpeg_args.extend_from_slice(&codec_args);
        ffmpeg_args.push(output);

        let result = run_ffmpeg(&ffmpeg_args).await;
        let _ = tokio::fs::remove_file(&list_path).await;
        result?;
        Ok(format!("Concatenated {} clips → {output}", inputs.len()))
    }
}

// ---------------------------------------------------------------------------
// 3. FfmpegAddCaptionsTool
// ---------------------------------------------------------------------------

pub struct FfmpegAddCaptionsTool;

#[async_trait]
impl TalosTool for FfmpegAddCaptionsTool {
    fn name(&self) -> &'static str { "ffmpeg_add_captions" }
    fn description(&self) -> &'static str {
        "Burn subtitles/captions from an SRT file into a video using FFmpeg"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("input", "string", "Input video file path", true)
            .with_param("srt", "string", "Path to SRT subtitle file", true)
            .with_param("output", "string", "Output video file path", true)
            .with_param("font_size", "integer", "Font size (default 24)", false)
            .with_param("font_color", "string", "Font color hex or name (default 'white')", false)
            .with_param("position", "string", "Position: 'bottom' (default) or 'top'", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let input = args["input"].as_str().ok_or_else(|| Error::Tool("Missing 'input'".into()))?;
        let srt = args["srt"].as_str().ok_or_else(|| Error::Tool("Missing 'srt'".into()))?;
        let output = args["output"].as_str().ok_or_else(|| Error::Tool("Missing 'output'".into()))?;
        let font_size = args["font_size"].as_i64().unwrap_or(24);
        let font_color = args["font_color"].as_str().unwrap_or("white");
        let position = args["position"].as_str().unwrap_or("bottom");
        let alignment = if position == "top" { "6" } else { "2" };

        let color_hex = if font_color == "white" { "FFFFFF" } else { font_color };
        let vf = format!(
            "subtitles={}:force_style='FontSize={},PrimaryColour=&H{},Alignment={}'",
            srt, font_size, color_hex, alignment
        );
        run_ffmpeg(&["-y", "-i", input, "-vf", &vf, "-c:a", "copy", output]).await?;
        Ok(format!("Captions burned into video → {output}"))
    }
}

// ---------------------------------------------------------------------------
// 4. FfmpegAddAudioTool
// ---------------------------------------------------------------------------

pub struct FfmpegAddAudioTool;

#[async_trait]
impl TalosTool for FfmpegAddAudioTool {
    fn name(&self) -> &'static str { "ffmpeg_add_audio" }
    fn description(&self) -> &'static str {
        "Overlay or replace audio track on a video using FFmpeg"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("input", "string", "Input video file path", true)
            .with_param("audio", "string", "Audio file path (mp3/wav/aac) to overlay", true)
            .with_param("output", "string", "Output video file path", true)
            .with_param("replace", "boolean", "Replace original audio entirely (default false = mix)", false)
            .with_param("audio_volume", "number", "Volume multiplier for overlay audio (default 1.0)", false)
            .with_param("video_volume", "number", "Volume multiplier for original audio (default 1.0)", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let input = args["input"].as_str().ok_or_else(|| Error::Tool("Missing 'input'".into()))?;
        let audio = args["audio"].as_str().ok_or_else(|| Error::Tool("Missing 'audio'".into()))?;
        let output = args["output"].as_str().ok_or_else(|| Error::Tool("Missing 'output'".into()))?;
        let replace = args["replace"].as_bool().unwrap_or(false);

        if replace {
            run_ffmpeg(&[
                "-y", "-i", input, "-i", audio,
                "-map", "0:v", "-map", "1:a",
                "-c:v", "copy", "-shortest", output,
            ]).await?;
        } else {
            let vol_audio = args["audio_volume"].as_f64().unwrap_or(1.0);
            let vol_video = args["video_volume"].as_f64().unwrap_or(1.0);
            let filter = format!(
                "[0:a]volume={vol_video}[a0];[1:a]volume={vol_audio}[a1];[a0][a1]amix=inputs=2[aout]"
            );
            run_ffmpeg(&[
                "-y", "-i", input, "-i", audio,
                "-filter_complex", &filter,
                "-map", "0:v", "-map", "[aout]",
                "-c:v", "copy", "-shortest", output,
            ]).await?;
        }
        Ok(format!("Audio added to video → {output}"))
    }
}

// ---------------------------------------------------------------------------
// 5. FfmpegResizeTool  (platform formatting)
// ---------------------------------------------------------------------------

pub struct FfmpegResizeTool;

#[async_trait]
impl TalosTool for FfmpegResizeTool {
    fn name(&self) -> &'static str { "ffmpeg_resize" }
    fn description(&self) -> &'static str {
        "Resize/reformat video for social media. Presets: tiktok (1080x1920), instagram_reel (1080x1920), instagram_square (1080x1080), youtube (1920x1080), youtube_shorts (1080x1920)"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("input", "string", "Input video file path", true)
            .with_param("output", "string", "Output video file path", true)
            .with_param("preset", "string", "Platform preset: tiktok, instagram_reel, instagram_square, youtube, youtube_shorts", false)
            .with_param("width", "integer", "Custom width (used if no preset)", false)
            .with_param("height", "integer", "Custom height (used if no preset)", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let input = args["input"].as_str().ok_or_else(|| Error::Tool("Missing 'input'".into()))?;
        let output = args["output"].as_str().ok_or_else(|| Error::Tool("Missing 'output'".into()))?;

        let (w, h) = match args["preset"].as_str() {
            Some("tiktok") | Some("instagram_reel") | Some("youtube_shorts") => (1080i64, 1920i64),
            Some("instagram_square") => (1080, 1080),
            Some("youtube") => (1920, 1080),
            _ => (
                args["width"].as_i64().unwrap_or(1080),
                args["height"].as_i64().unwrap_or(1920),
            ),
        };

        let vf = format!(
            "scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:black"
        );
        run_ffmpeg(&[
            "-y", "-i", input, "-vf", &vf,
            "-c:v", "libx264", "-c:a", "aac", "-movflags", "+faststart",
            output,
        ]).await?;
        Ok(format!("Resized to {w}×{h} → {output}"))
    }
}

// ---------------------------------------------------------------------------
// 6. FfmpegProbeInfoTool
// ---------------------------------------------------------------------------

pub struct FfmpegProbeInfoTool;

#[async_trait]
impl TalosTool for FfmpegProbeInfoTool {
    fn name(&self) -> &'static str { "ffmpeg_probe" }
    fn description(&self) -> &'static str {
        "Get video metadata (duration, resolution, codec, fps) using ffprobe"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("input", "string", "Video file path to probe", true)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let input = args["input"].as_str().ok_or_else(|| Error::Tool("Missing 'input'".into()))?;

        let output = Command::new("ffprobe")
            .args(["-v", "quiet", "-print_format", "json", "-show_format", "-show_streams", input])
            .output()
            .await
            .map_err(|e| Error::Tool(format!("ffprobe not found: {e}")))?;

        if !output.status.success() {
            return Err(Error::Tool(format!(
                "ffprobe error: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let info: Value = serde_json::from_slice(&output.stdout)
            .map_err(|e| Error::Tool(format!("Invalid ffprobe output: {e}")))?;

        let format = &info["format"];
        let duration = format["duration"].as_str().unwrap_or("?");
        let size = format["size"].as_str().unwrap_or("?");
        let format_name = format["format_long_name"].as_str().unwrap_or("?");
        let mut result = format!("Format: {format_name}\nDuration: {duration}s\nSize: {size} bytes\n");

        if let Some(streams) = info["streams"].as_array() {
            for stream in streams {
                let codec_type = stream["codec_type"].as_str().unwrap_or("?");
                let codec_name = stream["codec_name"].as_str().unwrap_or("?");
                if codec_type == "video" {
                    let w = stream["width"].as_i64().unwrap_or(0);
                    let h = stream["height"].as_i64().unwrap_or(0);
                    let fps = stream["r_frame_rate"].as_str().unwrap_or("?");
                    result.push_str(&format!("Video: {codec_name} {w}×{h} @ {fps}\n"));
                } else if codec_type == "audio" {
                    let rate = stream["sample_rate"].as_str().unwrap_or("?");
                    let channels = stream["channels"].as_i64().unwrap_or(0);
                    result.push_str(&format!("Audio: {codec_name} {rate}Hz {channels}ch\n"));
                }
            }
        }
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trim_schema() {
        let t = FfmpegTrimTool;
        assert_eq!(t.name(), "ffmpeg_trim");
        let s = t.schema();
        let p = s.parameters.as_object().unwrap();
        let req: Vec<&str> = p["required"].as_array().unwrap()
            .iter().filter_map(|v| v.as_str()).collect();
        assert!(req.contains(&"input") && req.contains(&"output") && req.contains(&"start"));
    }

    #[test]
    fn test_concat_schema() {
        let t = FfmpegConcatTool;
        assert_eq!(t.name(), "ffmpeg_concat");
        let s = t.schema();
        let p = s.parameters.as_object().unwrap();
        let req: Vec<&str> = p["required"].as_array().unwrap()
            .iter().filter_map(|v| v.as_str()).collect();
        assert!(req.contains(&"inputs") && req.contains(&"output"));
    }

    #[test]
    fn test_captions_schema() {
        let t = FfmpegAddCaptionsTool;
        assert_eq!(t.name(), "ffmpeg_add_captions");
        let s = t.schema();
        let p = s.parameters.as_object().unwrap();
        let req: Vec<&str> = p["required"].as_array().unwrap()
            .iter().filter_map(|v| v.as_str()).collect();
        assert!(req.contains(&"srt") && req.contains(&"input"));
    }

    #[test]
    fn test_add_audio_schema() {
        let t = FfmpegAddAudioTool;
        assert_eq!(t.name(), "ffmpeg_add_audio");
        let s = t.schema();
        let p = s.parameters.as_object().unwrap();
        let req: Vec<&str> = p["required"].as_array().unwrap()
            .iter().filter_map(|v| v.as_str()).collect();
        assert!(req.contains(&"audio"));
    }

    #[test]
    fn test_resize_presets() {
        let t = FfmpegResizeTool;
        assert_eq!(t.name(), "ffmpeg_resize");
        assert!(t.description().contains("tiktok"));
        assert!(t.description().contains("instagram_reel"));
        assert!(t.description().contains("youtube_shorts"));
    }

    #[test]
    fn test_probe_schema() {
        let t = FfmpegProbeInfoTool;
        assert_eq!(t.name(), "ffmpeg_probe");
        let s = t.schema();
        let p = s.parameters.as_object().unwrap();
        let req: Vec<&str> = p["required"].as_array().unwrap()
            .iter().filter_map(|v| v.as_str()).collect();
        assert!(req.contains(&"input"));
    }
}
