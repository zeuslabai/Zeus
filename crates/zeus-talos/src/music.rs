//! Apple Music automation tools

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
#[cfg(target_os = "macos")]
use serde_json::json;
#[cfg(target_os = "macos")]
use zeus_core::Error;
use zeus_core::{Result, ToolSchema};

#[cfg(target_os = "macos")]
use crate::run_applescript;

/// Play or resume Apple Music playback
pub struct MusicPlayTool;

#[async_trait]
impl TalosTool for MusicPlayTool {
    fn name(&self) -> &'static str {
        "music_play"
    }
    fn description(&self) -> &'static str {
        "Play or resume Apple Music playback"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            run_applescript("tell application \"Music\" to play")?;
            Ok("Music playback started".to_string())
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("Music tools only available on macOS".to_string())
        }
    }
}

/// Pause Apple Music playback
pub struct MusicPauseTool;

#[async_trait]
impl TalosTool for MusicPauseTool {
    fn name(&self) -> &'static str {
        "music_pause"
    }
    fn description(&self) -> &'static str {
        "Pause Apple Music playback"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            run_applescript("tell application \"Music\" to pause")?;
            Ok("Music playback paused".to_string())
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("Music tools only available on macOS".to_string())
        }
    }
}

/// Skip to the next track in Apple Music
pub struct MusicNextTool;

#[async_trait]
impl TalosTool for MusicNextTool {
    fn name(&self) -> &'static str {
        "music_next"
    }
    fn description(&self) -> &'static str {
        "Skip to the next track in Apple Music"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            run_applescript("tell application \"Music\" to next track")?;
            Ok("Skipped to next track".to_string())
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("Music tools only available on macOS".to_string())
        }
    }
}

/// Go to the previous track in Apple Music
pub struct MusicPreviousTool;

#[async_trait]
impl TalosTool for MusicPreviousTool {
    fn name(&self) -> &'static str {
        "music_previous"
    }
    fn description(&self) -> &'static str {
        "Go to the previous track in Apple Music"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            run_applescript("tell application \"Music\" to previous track")?;
            Ok("Went to previous track".to_string())
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("Music tools only available on macOS".to_string())
        }
    }
}

/// Get information about the currently playing track
pub struct MusicNowPlayingTool;

#[async_trait]
impl TalosTool for MusicNowPlayingTool {
    fn name(&self) -> &'static str {
        "music_now_playing"
    }
    fn description(&self) -> &'static str {
        "Get information about the currently playing track in Apple Music"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let script = r#"
                tell application "Music"
                    if player state is not stopped then
                        set trackName to name of current track
                        set trackArtist to artist of current track
                        set trackAlbum to album of current track
                        set trackDuration to duration of current track
                        set trackPosition to player position
                        return trackName & "|||" & trackArtist & "|||" & trackAlbum & "|||" & (trackDuration as string) & "|||" & (trackPosition as string)
                    else
                        return "STOPPED"
                    end if
                end tell
            "#;

            let result = run_applescript(script)?;

            if result == "STOPPED" {
                let info = json!({
                    "status": "stopped",
                    "message": "No track is currently playing"
                });
                return Ok(serde_json::to_string_pretty(&info)?);
            }

            let parts: Vec<&str> = result.split("|||").collect();
            if parts.len() >= 5 {
                let duration: f64 = parts[3].trim().parse().unwrap_or(0.0);
                let position: f64 = parts[4].trim().parse().unwrap_or(0.0);
                let info = json!({
                    "status": "playing",
                    "name": parts[0].trim(),
                    "artist": parts[1].trim(),
                    "album": parts[2].trim(),
                    "duration_seconds": duration,
                    "position_seconds": position,
                });
                Ok(serde_json::to_string_pretty(&info)?)
            } else {
                Ok(result)
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("Music tools only available on macOS".to_string())
        }
    }
}

/// Search the Apple Music library
pub struct MusicSearchTool;

#[async_trait]
impl TalosTool for MusicSearchTool {
    fn name(&self) -> &'static str {
        "music_search"
    }
    fn description(&self) -> &'static str {
        "Search the Apple Music library for tracks"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "query",
            "string",
            "Search query to find tracks",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing query".to_string()))?;

            let script = format!(
                r#"
                set resultList to ""
                tell application "Music"
                    set foundTracks to (search library playlist 1 for "{}")
                    set trackCount to 0
                    repeat with t in foundTracks
                        if trackCount >= 20 then exit repeat
                        set resultList to resultList & (name of t) & " - " & (artist of t) & " [" & (album of t) & "]" & linefeed
                        set trackCount to trackCount + 1
                    end repeat
                end tell
                return resultList
            "#,
                crate::sanitize_applescript(query)
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Music tools only available on macOS".to_string())
        }
    }
}

/// Get or set the Apple Music volume
pub struct MusicVolumeTool;

#[async_trait]
impl TalosTool for MusicVolumeTool {
    fn name(&self) -> &'static str {
        "music_volume"
    }
    fn description(&self) -> &'static str {
        "Get or set the Apple Music volume (0-100)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "level",
            "integer",
            "Volume level 0-100 (omit to get current volume)",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            if let Some(level) = args.get("level").and_then(|v| v.as_u64()) {
                if level > 100 {
                    return Err(Error::Tool("Volume must be 0-100".to_string()));
                }
                let script = format!(
                    "tell application \"Music\" to set sound volume to {}",
                    level
                );
                run_applescript(&script)?;
                Ok(format!("Music volume set to {}", level))
            } else {
                let result = run_applescript("tell application \"Music\" to return sound volume")?;
                Ok(format!("Music volume: {}", result))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Music tools only available on macOS".to_string())
        }
    }
}

/// List Apple Music playlists
pub struct MusicPlaylistsTool;

#[async_trait]
impl TalosTool for MusicPlaylistsTool {
    fn name(&self) -> &'static str {
        "music_playlists"
    }
    fn description(&self) -> &'static str {
        "List all playlists in Apple Music"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let script = r#"
                set playlistList to ""
                tell application "Music"
                    repeat with p in playlists
                        set playlistList to playlistList & (name of p) & linefeed
                    end repeat
                end tell
                return playlistList
            "#;

            run_applescript(script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("Music tools only available on macOS".to_string())
        }
    }
}

/// Play a specific playlist in Apple Music
pub struct MusicPlayPlaylistTool;

#[async_trait]
impl TalosTool for MusicPlayPlaylistTool {
    fn name(&self) -> &'static str {
        "music_play_playlist"
    }
    fn description(&self) -> &'static str {
        "Play a specific playlist in Apple Music"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "name",
            "string",
            "Name of the playlist to play",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let name = args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing name".to_string()))?;

            let script = format!(
                "tell application \"Music\" to play playlist \"{}\"",
                crate::sanitize_applescript(name)
            );
            run_applescript(&script)?;
            Ok(format!("Playing playlist: {}", name))
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Music tools only available on macOS".to_string())
        }
    }
}

/// Toggle or set shuffle mode in Apple Music
pub struct MusicShuffleTool;

#[async_trait]
impl TalosTool for MusicShuffleTool {
    fn name(&self) -> &'static str {
        "music_shuffle"
    }
    fn description(&self) -> &'static str {
        "Get or set shuffle mode in Apple Music"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "enabled",
            "boolean",
            "Set shuffle on (true) or off (false). Omit to get current state.",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            if let Some(enabled) = args.get("enabled").and_then(|v| v.as_bool()) {
                let value = if enabled { "true" } else { "false" };
                let script = format!(
                    "tell application \"Music\" to set shuffle enabled to {}",
                    value
                );
                run_applescript(&script)?;
                Ok(format!(
                    "Shuffle {}",
                    if enabled { "enabled" } else { "disabled" }
                ))
            } else {
                let result =
                    run_applescript("tell application \"Music\" to return shuffle enabled")?;
                let state = if result.trim() == "true" {
                    "enabled"
                } else {
                    "disabled"
                };
                Ok(format!("Shuffle is {}", state))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Music tools only available on macOS".to_string())
        }
    }
}
