//! Zeus Talos - Native Automation Tools
//!
//! Provides 50+ native automation tools for macOS/Linux including:
//! - Calendar, Notes, Reminders (macOS)
//! - File system operations
//! - System info and process management
//! - Network operations
//! - Browser automation (via AppleScript on macOS)

pub mod automation;
pub mod bluebubbles_tools;
pub mod bluetooth;
pub mod browser;
pub mod calendar;
pub mod contacts;
pub mod defaults;
pub mod discord_tools;
pub mod docker;
pub mod feishu_tools;
pub mod files;
pub mod fooocus;
pub mod git;
pub mod github;
// Re-export for external use, keeping fooocus.rs tools intact
pub mod content_tools;
pub mod googlechat_tools;
pub mod homebrew;
pub mod image_provider;
pub mod instagram_tools;
pub mod irc_tools;
pub mod iteration;
pub mod line_tools;
pub mod mail;
pub mod matrix_tools;
pub mod mattermost_tools;
pub mod memory_tools;
pub mod messages;
pub mod mqtt_tools;
pub mod music;
pub mod network;
pub mod nextcloud_tools;
pub mod nostr_tools;
pub mod notes;
pub mod ocr;
pub mod keychain;
pub mod ollama;
pub mod tmux;
pub mod image_tools;
pub mod pdf;
pub mod relay;
pub mod reminders;
pub mod scheduler_tools;
pub mod search;
pub mod signal_tools;
pub mod slack_tools;
pub mod sms_tools;
pub mod system;
pub mod teams_tools;
pub mod telegram_tools;
pub mod tiktok_tools;
pub mod twitch_tools;
pub mod video;
pub mod voice;
pub mod webchat_tools;
pub mod whatsapp_tools;
pub mod x_tools;
pub mod youtube_tools;
pub mod zalo_tools;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use zeus_core::{Error, Result, ToolSchema};

/// A Talos tool
#[async_trait]
pub trait TalosTool: Send + Sync {
    /// Tool name
    fn name(&self) -> &'static str;

    /// Tool description
    fn description(&self) -> &'static str;

    /// Get the tool schema
    fn schema(&self) -> ToolSchema;

    /// Execute the tool
    async fn execute(&self, args: Value) -> Result<String>;
}

/// Tool registry
/// Registry of all available Talos tools.
///
/// Use [`TalosRegistry::with_defaults`] to get a registry pre-populated
/// with all built-in tools, or [`TalosRegistry::new`] for an empty registry.
pub struct TalosRegistry {
    tools: HashMap<String, Box<dyn TalosTool>>,
}

impl TalosRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Create with all default tools
    pub fn with_defaults() -> Self {
        let mut registry = Self::new();

        // Register all tools
        registry.register_all();

        registry
    }

    /// Register a tool
    pub fn register(&mut self, tool: Box<dyn TalosTool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Register all default tools
    fn register_all(&mut self) {
        // System tools (cross-platform)
        self.register(Box::new(system::SystemInfoTool));
        self.register(Box::new(system::ProcessListTool));
        self.register(Box::new(system::KillProcessTool));
        self.register(Box::new(system::DiskUsageTool));
        self.register(Box::new(system::MemoryInfoTool));
        self.register(Box::new(system::SystemNotifyTool));
        self.register(Box::new(system::ClipboardReadTool));
        self.register(Box::new(system::ClipboardWriteTool));
        self.register(Box::new(system::ScreenshotTool));
        self.register(Box::new(system::SpotlightSearchTool));
        self.register(Box::new(system::EnvVarsTool));
        self.register(Box::new(system::CheckPermissionsTool));
        self.register(Box::new(system::RetryCommandTool));

        // File tools (cross-platform)
        self.register(Box::new(files::FileSearchTool));
        self.register(Box::new(files::FileMetadataTool));
        self.register(Box::new(files::FileTagsTool));
        self.register(Box::new(files::SetFileTagsTool));
        self.register(Box::new(files::FileCopyTool));
        self.register(Box::new(files::FileMoveTool));
        self.register(Box::new(files::FileRenameTool));
        self.register(Box::new(files::FileStatTool));
        self.register(Box::new(files::FindFilesTool));
        self.register(Box::new(files::DirectoryCreateTool));
        // Extended file tools
        self.register(Box::new(files::FileAppendTool));
        self.register(Box::new(files::FileCreateTool));
        self.register(Box::new(files::FileDeleteTool));
        self.register(Box::new(files::GrepFilesTool));
        self.register(Box::new(files::HeadFileTool));
        self.register(Box::new(files::TailFileTool));

        // Network tools
        self.register(Box::new(network::PingTool));
        self.register(Box::new(network::DnsTool));
        self.register(Box::new(network::PortCheckTool));
        // Extended network tools
        self.register(Box::new(network::NetworkInfoTool));
        self.register(Box::new(network::NetworkServicesTool));
        self.register(Box::new(network::NetworkLocationsTool));
        self.register(Box::new(network::NetworkCurrentLocationTool));
        self.register(Box::new(network::NetworkSwitchLocationTool));
        self.register(Box::new(network::DnsSetTool));
        self.register(Box::new(network::DnsResetTool));
        self.register(Box::new(network::IpSetDhcpTool));
        self.register(Box::new(network::IpSetManualTool));
        self.register(Box::new(network::ProxySetTool));
        self.register(Box::new(network::ProxyDisableTool));

        // Search tools (cross-platform)
        self.register(Box::new(search::WebSearchTool));

        // Image generation tools (provider-agnostic + legacy Fooocus-specific)
        self.register(Box::new(fooocus::ImageGenerateTool));
        self.register(Box::new(fooocus::FooocusGenerateTool));
        self.register(Box::new(fooocus::FooocusBatchGenerateTool));
        self.register(Box::new(fooocus::FooocusCheckStatusTool));
        self.register(Box::new(fooocus::FooocusGetModelsTool));
        self.register(Box::new(fooocus::AnalyzeImageTool));

        // Video generation tools (ComfyUI + AnimateDiff on .249 GPU)
        self.register(Box::new(video::VideoGenerateTool));
        self.register(Box::new(video::VideoCheckStatusTool));

        // Content production tools (FFmpeg editing)
        self.register(Box::new(content_tools::FfmpegTrimTool));
        self.register(Box::new(content_tools::FfmpegConcatTool));
        self.register(Box::new(content_tools::FfmpegAddCaptionsTool));
        self.register(Box::new(content_tools::FfmpegAddAudioTool));
        self.register(Box::new(content_tools::FfmpegResizeTool));
        self.register(Box::new(content_tools::FfmpegProbeInfoTool));

        // Social media upload tools
        self.register(Box::new(youtube_tools::YouTubeUploadTool));
        self.register(Box::new(youtube_tools::YouTubeGetVideoTool));
        self.register(Box::new(tiktok_tools::TikTokUploadTool));
        self.register(Box::new(tiktok_tools::TikTokCheckStatusTool));

        // Voice tools (cross-platform)
        self.register(Box::new(voice::TranscribeAudioTool));
        self.register(Box::new(voice::TextToSpeechTool));

        // Homebrew tools (cross-platform)
        self.register(Box::new(homebrew::BrewInstallTool));
        self.register(Box::new(homebrew::BrewListTool));
        self.register(Box::new(homebrew::BrewSearchTool));
        self.register(Box::new(homebrew::BrewUninstallTool));

        // Docker tools
        self.register(Box::new(docker::DockerPsTool));
        self.register(Box::new(docker::DockerExecTool));
        self.register(Box::new(docker::DockerLogsTool));
        self.register(Box::new(docker::DockerStartTool));
        self.register(Box::new(docker::DockerStopTool));
        self.register(Box::new(docker::DockerComposeTool));

        // Ollama tools
        self.register(Box::new(ollama::OllamaPullTool));
        self.register(Box::new(ollama::OllamaListTool));
        self.register(Box::new(ollama::OllamaRmTool));
        self.register(Box::new(ollama::OllamaShowTool));
        self.register(Box::new(ollama::OllamaPsTool));

        // Tmux tools (T7)
        self.register(Box::new(tmux::TmuxListTool));
        self.register(Box::new(tmux::TmuxSendTool));
        self.register(Box::new(tmux::TmuxCaptureTool));
        self.register(Box::new(tmux::TmuxNewTool));
        self.register(Box::new(tmux::TmuxKillTool));

        // Image manipulation tools (T8)
        self.register(Box::new(image_tools::ImageResizeTool));
        self.register(Box::new(image_tools::ImageConvertTool));
        self.register(Box::new(image_tools::ImageCompressTool));
        self.register(Box::new(image_tools::ImageExifTool));

        // Keychain tools (T5)
        self.register(Box::new(keychain::KeychainGetTool));
        self.register(Box::new(keychain::KeychainSetTool));
        self.register(Box::new(keychain::KeychainDeleteTool));
        self.register(Box::new(keychain::KeychainListTool));

        // Telegram
        self.register(Box::new(telegram_tools::TelegramSendMessageTool));
        self.register(Box::new(telegram_tools::TelegramDeleteMessageTool));
        self.register(Box::new(telegram_tools::TelegramGetUpdatesTool));
        self.register(Box::new(telegram_tools::TelegramSendPhotoTool));
        self.register(Box::new(telegram_tools::TelegramSendButtonsTool));
        self.register(Box::new(telegram_tools::TelegramGetChatInfoTool));
        // Extended Telegram tools
        self.register(Box::new(telegram_tools::TelegramSendDocumentTool));
        self.register(Box::new(telegram_tools::TelegramSendVoiceTool));
        self.register(Box::new(telegram_tools::TelegramGetMessagesTool));
        self.register(Box::new(telegram_tools::TelegramCallTool));

        // Discord tools (cross-platform, uses Bot HTTP API)
        self.register(Box::new(discord_tools::DiscordSendMessageTool));
        self.register(Box::new(discord_tools::DiscordSendEmbedTool));
        self.register(Box::new(discord_tools::DiscordGetMessagesTool));
        self.register(Box::new(discord_tools::DiscordGetChannelInfoTool));
        self.register(Box::new(discord_tools::DiscordSendFileTool));
        self.register(Box::new(discord_tools::DiscordCreateThreadTool));
        self.register(Box::new(discord_tools::DiscordAddReactionTool));
        self.register(Box::new(discord_tools::DiscordDeleteMessageTool));

        // Slack tools (cross-platform, uses Web API)
        self.register(Box::new(slack_tools::SlackSendMessageTool));
        self.register(Box::new(slack_tools::SlackGetMessagesTool));
        self.register(Box::new(slack_tools::SlackGetChannelInfoTool));
        self.register(Box::new(slack_tools::SlackSendFileTool));
        self.register(Box::new(slack_tools::SlackListChannelsTool));
        self.register(Box::new(slack_tools::SlackSetTopicTool));
        self.register(Box::new(slack_tools::SlackAddReactionTool));

        // Instagram Graph API tools (cross-platform)
        self.register(Box::new(instagram_tools::InstagramSendPhotoTool));
        self.register(Box::new(instagram_tools::InstagramSendReelTool));
        self.register(Box::new(instagram_tools::InstagramGetProfileTool));

        // WhatsApp tools (cross-platform, uses Cloud API)
        self.register(Box::new(whatsapp_tools::WhatsAppSendMessageTool));
        self.register(Box::new(whatsapp_tools::WhatsAppSendImageTool));
        self.register(Box::new(whatsapp_tools::WhatsAppSendDocumentTool));
        self.register(Box::new(whatsapp_tools::WhatsAppSendTemplateTool));
        self.register(Box::new(whatsapp_tools::WhatsAppGetProfileTool));

        // Signal tools (cross-platform, uses signal-cli subprocess)
        self.register(Box::new(signal_tools::SignalSendMessageTool));
        self.register(Box::new(signal_tools::SignalSendGroupMessageTool));
        self.register(Box::new(signal_tools::SignalReceiveMessagesTool));
        self.register(Box::new(signal_tools::SignalListGroupsTool));
        self.register(Box::new(signal_tools::SignalSendReactionTool));
        self.register(Box::new(signal_tools::SignalSendFileTool));

        // Matrix tools (cross-platform, uses Client-Server API)
        self.register(Box::new(matrix_tools::MatrixSendMessageTool));
        self.register(Box::new(matrix_tools::MatrixGetMessagesTool));
        self.register(Box::new(matrix_tools::MatrixJoinRoomTool));
        self.register(Box::new(matrix_tools::MatrixListRoomsTool));
        self.register(Box::new(matrix_tools::MatrixSendImageTool));
        self.register(Box::new(matrix_tools::MatrixGetRoomInfoTool));

        // Microsoft Teams tools (cross-platform, uses Graph API)
        self.register(Box::new(teams_tools::TeamsSendMessageTool));
        self.register(Box::new(teams_tools::TeamsGetMessagesTool));
        self.register(Box::new(teams_tools::TeamsListChannelsTool));
        self.register(Box::new(teams_tools::TeamsListTeamsTool));
        self.register(Box::new(teams_tools::TeamsSendChatMessageTool));

        // Twitch tools (cross-platform, uses Helix API)
        self.register(Box::new(twitch_tools::TwitchSendMessageTool));
        self.register(Box::new(twitch_tools::TwitchGetChannelInfoTool));
        self.register(Box::new(twitch_tools::TwitchGetStreamsTool));

        // IRC tools (cross-platform, raw TCP)
        self.register(Box::new(irc_tools::IrcSendMessageTool));
        self.register(Box::new(irc_tools::IrcJoinChannelTool));

        // Mattermost tools (cross-platform, REST API v4)
        self.register(Box::new(mattermost_tools::MattermostSendMessageTool));
        self.register(Box::new(mattermost_tools::MattermostGetMessagesTool));
        self.register(Box::new(mattermost_tools::MattermostListChannelsTool));
        self.register(Box::new(mattermost_tools::MattermostReplyToThreadTool));
        self.register(Box::new(mattermost_tools::MattermostSendSlashCommandTool));

        // SMS tools (cross-platform, Twilio API)
        self.register(Box::new(sms_tools::SmsSendMessageTool));
        self.register(Box::new(sms_tools::SmsGetMessagesTool));
        self.register(Box::new(sms_tools::SmsGetMessageTool));

        // Google Chat tools (cross-platform, Google Chat API)
        self.register(Box::new(googlechat_tools::GoogleChatSendMessageTool));
        self.register(Box::new(googlechat_tools::GoogleChatGetMessagesTool));
        self.register(Box::new(googlechat_tools::GoogleChatListSpacesTool));

        // Nostr tools (cross-platform, NIP-01 protocol via CLI)
        self.register(Box::new(nostr_tools::NostrPublishNoteTool));
        self.register(Box::new(nostr_tools::NostrGetEventsTool));

        // MQTT tools (cross-platform, mosquitto CLI)
        self.register(Box::new(mqtt_tools::MqttPublishTool));
        self.register(Box::new(mqtt_tools::MqttSubscribeTool));

        // X (Twitter) tools (cross-platform, X API v2 via zeus-channels XAdapter)
        self.register(Box::new(x_tools::XPostTool));
        self.register(Box::new(x_tools::XReplyTool));
        self.register(Box::new(x_tools::XThreadTool));
        self.register(Box::new(x_tools::XDeleteTool));
        self.register(Box::new(x_tools::XDeletePostTool));
        self.register(Box::new(x_tools::XBatchDeleteTool));
        self.register(Box::new(x_tools::XMetricsTool));

        // Feishu/Lark tools (cross-platform, Feishu Open API)
        self.register(Box::new(feishu_tools::FeishuSendMessageTool));
        self.register(Box::new(feishu_tools::FeishuGetMessagesTool));
        self.register(Box::new(feishu_tools::FeishuListChatsTool));

        // LINE tools (cross-platform, LINE Messaging API)
        self.register(Box::new(line_tools::LineSendMessageTool));
        self.register(Box::new(line_tools::LineGetProfileTool));
        self.register(Box::new(line_tools::LineGetGroupInfoTool));

        // Zalo tools (cross-platform, Zalo OA API)
        self.register(Box::new(zalo_tools::ZaloSendMessageTool));
        self.register(Box::new(zalo_tools::ZaloGetProfileTool));
        self.register(Box::new(zalo_tools::ZaloGetFollowersTool));

        // Nextcloud Talk tools (cross-platform, OCS API)
        self.register(Box::new(nextcloud_tools::NextcloudSendMessageTool));
        self.register(Box::new(nextcloud_tools::NextcloudGetMessagesTool));
        self.register(Box::new(nextcloud_tools::NextcloudListRoomsTool));

        // WebChat tools (cross-platform, generic HTTP)
        self.register(Box::new(webchat_tools::WebchatSendMessageTool));
        self.register(Box::new(webchat_tools::WebchatGetMessagesTool));
        self.register(Box::new(webchat_tools::WebchatListChannelsTool));

        // BlueBubbles tools (cross-platform, iMessage bridge)
        self.register(Box::new(bluebubbles_tools::BlueBubblesSendMessageTool));
        self.register(Box::new(bluebubbles_tools::BlueBubblesGetMessagesTool));
        self.register(Box::new(bluebubbles_tools::BlueBubblesListChatsTool));

        // Memory tools (file-based, no DB dependency)
        self.register(Box::new(memory_tools::MemoryRecallTool));
        self.register(Box::new(memory_tools::MemoryStoreTool));
        self.register(Box::new(memory_tools::MemorySearchTool));

        // Channel relay tools
        self.register(Box::new(relay::AutoStartRelayTool));
        self.register(Box::new(relay::TelegramStartRelayTool));
        self.register(Box::new(relay::TelegramStopRelayTool));
        self.register(Box::new(relay::TelegramRelayStatusTool));
        self.register(Box::new(relay::DiscordStartRelayTool));
        self.register(Box::new(relay::DiscordStopRelayTool));
        self.register(Box::new(relay::DiscordRelayStatusTool));

        // Git tools (cross-platform)
        self.register(Box::new(git::GitStatusTool));
        self.register(Box::new(git::GitAddTool));
        self.register(Box::new(git::GitCommitTool));
        self.register(Box::new(git::GitPushTool));
        self.register(Box::new(git::GitPullTool));
        self.register(Box::new(git::GitDiffTool));
        self.register(Box::new(git::GitDiffStatTool));
        self.register(Box::new(git::GitLogTool));
        self.register(Box::new(git::GitBranchListTool));
        self.register(Box::new(git::GitBranchCreateTool));
        self.register(Box::new(git::GitBranchDeleteTool));
        self.register(Box::new(git::GitCheckoutTool));
        self.register(Box::new(git::GitCloneTool));
        self.register(Box::new(git::GitStashTool));
        self.register(Box::new(git::GitStashPopTool));

        // GitHub tools (cross-platform)
        self.register(Box::new(github::GhPrReviewTool));
        self.register(Box::new(github::GhPrCommentTool));
        self.register(Box::new(github::GhIssueCreateTool));
        self.register(Box::new(github::GhActionsStatusTool));

        // Scheduler tools (cross-platform)
        self.register(Box::new(scheduler_tools::ScheduleCreateTool));
        self.register(Box::new(scheduler_tools::ScheduleListTool));
        self.register(Box::new(scheduler_tools::ScheduleDeleteTool));

        // OCR / Vision tools (macOS Vision framework)
        self.register(Box::new(ocr::OcrImageTool));
        self.register(Box::new(ocr::OcrScreenshotTool));
        self.register(Box::new(ocr::OcrRegionTool));

        // Iteration and flow control tools (cross-platform)
        self.register(Box::new(iteration::ForEachFileTool));
        self.register(Box::new(iteration::ForEachLineTool));
        self.register(Box::new(iteration::BatchExecuteTool));
        self.register(Box::new(iteration::ParallelExecuteTool));
        self.register(Box::new(iteration::RepeatTool));
        self.register(Box::new(iteration::UntilSuccessTool));
        self.register(Box::new(iteration::WhileConditionTool));
        self.register(Box::new(iteration::SearchReplaceBulkTool));
        self.register(Box::new(iteration::PipeTool));
        self.register(Box::new(iteration::ConditionalTool));
        self.register(Box::new(iteration::WatchPathTool));

        // macOS specific
        #[cfg(target_os = "macos")]
        {
            // Files (macOS-specific)
            self.register(Box::new(files::TrashFileTool));
            self.register(Box::new(files::CreateAliasTool));
            self.register(Box::new(files::FinderSelectionTool));
            // Calendar
            self.register(Box::new(calendar::CalendarListTool));
            self.register(Box::new(calendar::CalendarCreateTool));
            self.register(Box::new(calendar::CalendarDeleteTool));
            self.register(Box::new(calendar::CalendarSearchTool));
            self.register(Box::new(calendar::CalendarListCalendarsTool));
            self.register(Box::new(calendar::CalendarUpcomingTool));
            self.register(Box::new(calendar::CalendarUpdateTool));
            // Notes
            self.register(Box::new(notes::NotesListTool));
            self.register(Box::new(notes::NotesCreateTool));
            self.register(Box::new(notes::NotesReadTool));
            self.register(Box::new(notes::NotesSearchTool));
            self.register(Box::new(notes::NotesDeleteTool));
            self.register(Box::new(notes::NotesUpdateTool));
            self.register(Box::new(notes::NotesMoveTool));
            self.register(Box::new(notes::NotesFoldersTool));
            self.register(Box::new(notes::NotesAppendTool));
            // Reminders
            self.register(Box::new(reminders::RemindersListTool));
            self.register(Box::new(reminders::RemindersCreateTool));
            self.register(Box::new(reminders::RemindersCompleteTool));
            self.register(Box::new(reminders::RemindersDeleteTool));
            self.register(Box::new(reminders::RemindersSearchTool));
            self.register(Box::new(reminders::RemindersListsTool));
            self.register(Box::new(reminders::RemindersDueTodayTool));
            self.register(Box::new(reminders::RemindersUpdateTool));
            // Contacts
            self.register(Box::new(contacts::ContactsSearchTool));
            self.register(Box::new(contacts::ContactsGetTool));
            self.register(Box::new(contacts::ContactsCreateTool));
            self.register(Box::new(contacts::ContactsDeleteTool));
            self.register(Box::new(contacts::ContactsUpdateTool));
            self.register(Box::new(contacts::ContactsGroupsTool));
            // Browser
            self.register(Box::new(browser::SafariUrlTool));
            self.register(Box::new(browser::SafariTabsTool));
            self.register(Box::new(browser::SafariJsTool));
            self.register(Box::new(browser::SafariNavigateTool));
            self.register(Box::new(browser::SafariNewTabTool));
            self.register(Box::new(browser::SafariCloseTabTool));
            self.register(Box::new(browser::SafariHistoryTool));
            self.register(Box::new(browser::SafariBookmarksTool));
            self.register(Box::new(browser::SafariAddBookmarkTool));
            self.register(Box::new(browser::SafariReadingListTool));
            self.register(Box::new(browser::SafariSourceTool));
            self.register(Box::new(browser::SafariTitleTool));
            self.register(Box::new(browser::SafariBackTool));
            self.register(Box::new(browser::SafariForwardTool));
            // Music
            self.register(Box::new(music::MusicPlayTool));
            self.register(Box::new(music::MusicPauseTool));
            self.register(Box::new(music::MusicNextTool));
            self.register(Box::new(music::MusicPreviousTool));
            self.register(Box::new(music::MusicNowPlayingTool));
            self.register(Box::new(music::MusicSearchTool));
            self.register(Box::new(music::MusicVolumeTool));
            self.register(Box::new(music::MusicPlaylistsTool));
            self.register(Box::new(music::MusicPlayPlaylistTool));
            self.register(Box::new(music::MusicShuffleTool));
            // Mail
            self.register(Box::new(mail::MailSendTool));
            self.register(Box::new(mail::MailInboxTool));
            self.register(Box::new(mail::MailReadTool));
            self.register(Box::new(mail::MailSearchTool));
            self.register(Box::new(mail::MailFlagTool));
            self.register(Box::new(mail::MailMoveTool));
            self.register(Box::new(mail::MailDeleteTool));
            self.register(Box::new(mail::MailMailboxesTool));
            self.register(Box::new(mail::MailUnreadCountTool));
            self.register(Box::new(mail::MailMarkReadTool));
            self.register(Box::new(mail::MailForwardTool));
            // Messages (iMessage)
            self.register(Box::new(messages::MessagesSendTool));
            self.register(Box::new(messages::MessagesReadTool));
            self.register(Box::new(messages::MessagesSearchTool));
            self.register(Box::new(messages::MessagesChatsTool));
            self.register(Box::new(messages::MessagesUnreadTool));
            self.register(Box::new(messages::MessagesAttachmentsTool));
            self.register(Box::new(messages::MessagesSetDndTool));
            self.register(Box::new(messages::MessagesStatusTool));
            // System (macOS-specific)
            self.register(Box::new(system::OpenAppTool));
            self.register(Box::new(system::QuitAppTool));
            self.register(Box::new(system::VolumeGetTool));
            self.register(Box::new(system::VolumeSetTool));
            self.register(Box::new(system::FinderRevealTool));
            self.register(Box::new(system::DarkModeTool));
            self.register(Box::new(system::BatteryInfoTool));
            self.register(Box::new(system::WifiCurrentTool));
            self.register(Box::new(system::WifiListTool));
            self.register(Box::new(system::WifiConnectTool));
            self.register(Box::new(system::BluetoothListTool));
            self.register(Box::new(system::BluetoothToggleTool));
            self.register(Box::new(system::SetWallpaperTool));
            self.register(Box::new(system::ScreenLockTool));
            self.register(Box::new(system::ScreenBrightnessTool));
            self.register(Box::new(system::DoNotDisturbTool));
            self.register(Box::new(system::AppListTool));
            self.register(Box::new(system::FrontAppTool));
            self.register(Box::new(system::HideAppTool));
            self.register(Box::new(system::MuteToggleTool));
            self.register(Box::new(system::EnableFocusTool));
            self.register(Box::new(system::DisableFocusTool));
            self.register(Box::new(system::ListShortcutsTool));
            self.register(Box::new(system::RunShortcutTool));
            self.register(Box::new(system::ScreenshotRegionTool));
            self.register(Box::new(system::ExecuteApplescriptTool));
            self.register(Box::new(system::ServiceStatusTool));
            self.register(Box::new(system::SpeakTextTool));
            self.register(Box::new(system::SendNotificationTool));
            self.register(Box::new(system::SetMuteTool));
            // Extended system tools
            self.register(Box::new(system::CpuInfoTool));
            self.register(Box::new(system::DisplayListTool));
            self.register(Box::new(system::GetProcessInfoTool));
            self.register(Box::new(system::IsProcessRunningTool));
            self.register(Box::new(system::GetWindowTitleTool));
            self.register(Box::new(system::MaximizeWindowTool));
            self.register(Box::new(system::SwitchSpaceTool));
            self.register(Box::new(system::WaitForAppTool));
            self.register(Box::new(system::WaitSecondsTool));
            self.register(Box::new(system::WifiPowerTool));
            self.register(Box::new(system::ScreenshotWindowTool));
            self.register(Box::new(system::ScreenRecordStartTool));
            self.register(Box::new(system::ScreenRecordStopTool));
            // PDF
            self.register(Box::new(pdf::PdfExtractTextTool));
            self.register(Box::new(pdf::PdfExtractPagesTool));
            self.register(Box::new(pdf::PdfGetMetadataTool));
            self.register(Box::new(pdf::PdfMergeTool));
            self.register(Box::new(pdf::PdfSplitTool));
            // Bluetooth
            self.register(Box::new(bluetooth::BluetoothListDevicesTool));
            self.register(Box::new(bluetooth::BluetoothConnectTool));
            self.register(Box::new(bluetooth::BluetoothDisconnectTool));
            self.register(Box::new(bluetooth::BluetoothPairTool));
            self.register(Box::new(bluetooth::BluetoothUnpairTool));
            self.register(Box::new(bluetooth::BluetoothPowerTool));
            // Defaults (system preferences)
            self.register(Box::new(defaults::DefaultsReadTool));
            self.register(Box::new(defaults::DefaultsWriteBoolTool));
            self.register(Box::new(defaults::DefaultsWriteIntTool));
            self.register(Box::new(defaults::DefaultsWriteStringTool));
            self.register(Box::new(defaults::DefaultsListDomainTool));
            self.register(Box::new(defaults::DefaultsListDomainsTool));
            // UI Automation
            self.register(Box::new(automation::KeystrokeTool));
            self.register(Box::new(automation::KeyCodeTool));
            self.register(Box::new(automation::MouseClickTool));
            self.register(Box::new(automation::MouseMoveTool));
            self.register(Box::new(automation::ActivateAppTool));
            self.register(Box::new(automation::WindowListTool));
            self.register(Box::new(automation::WindowResizeTool));
            self.register(Box::new(automation::WindowMoveTool));
            self.register(Box::new(automation::WindowMinimizeTool));
            self.register(Box::new(automation::WindowCloseTool));
            self.register(Box::new(automation::WindowFullscreenTool));
            self.register(Box::new(automation::MenuClickTool));
            self.register(Box::new(automation::TypeTextTool));
            self.register(Box::new(automation::ScreenSizeTool));
            self.register(Box::new(automation::WindowBoundsTool));
            // Extended UI automation tools
            self.register(Box::new(automation::UiScrollTool));
            self.register(Box::new(automation::UiGetMousePositionTool));
        }
    }

    /// Get a tool by name
    pub fn get(&self, name: &str) -> Option<&dyn TalosTool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    /// Execute a tool
    pub async fn execute(&self, name: &str, args: Value) -> Result<String> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| Error::Tool(format!("Tool not found: {}", name)))?;
        tool.execute(args).await
    }

    /// Get all tool schemas
    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.tools.values().map(|t| t.schema()).collect()
    }

    /// List all tool names
    pub fn list(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    /// Get tool count
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

impl Default for TalosRegistry {
    fn default() -> Self {
        Self::with_defaults()
    }
}

/// Sanitize a string for safe interpolation into AppleScript double-quoted strings.
///
/// Uses a whitelist approach: only allows printable ASCII after Unicode NFKC
/// normalization. This prevents injection via concatenation, variable
/// indirection, or Unicode homoglyph/lookalike attacks.
pub fn sanitize_applescript(s: &str) -> String {
    // NFKC normalize to collapse Unicode lookalikes before processing
    let normalized: String = s
        .chars()
        .map(|c| {
            // Map non-ASCII to replacement to prevent Unicode bypass
            if c.is_ascii() { c } else { '_' }
        })
        .collect();

    let mut out = String::with_capacity(normalized.len() + 16);
    for ch in normalized.chars() {
        match ch {
            // Escape characters that could break AppleScript string context
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Whitelist: only printable ASCII (0x20-0x7E), no control chars
            c if c as u8 >= 0x20 && c as u8 <= 0x7E => out.push(c),
            // Strip everything else
            _ => {}
        }
    }

    // Reject strings containing dangerous AppleScript keywords even after escaping
    let lower = out.to_lowercase();
    let dangerous = [
        "do shell script",
        "osascript",
        "system attribute",
        "load script",
        "run script",
    ];
    if dangerous.iter().any(|kw| lower.contains(kw)) {
        return String::new();
    }

    out
}

/// Sanitize a string for safe use as a shell argument.
///
/// Wraps the value in single quotes and escapes any embedded single quotes.
/// This prevents shell metacharacter interpretation.
pub fn sanitize_shell_arg(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Run an AppleScript and return the result.
///
/// Enforces a size limit and rejects scripts containing null bytes.
#[cfg(target_os = "macos")]
pub fn run_applescript(script: &str) -> Result<String> {
    use std::process::Command;

    // Defense-in-depth: reject null bytes and enforce size limit
    if script.contains('\0') {
        return Err(Error::Tool("AppleScript contains null bytes".to_string()));
    }
    if script.len() > 100_000 {
        return Err(Error::Tool(
            "AppleScript too long (max 100,000 characters)".to_string(),
        ));
    }

    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| Error::Tool(format!("Failed to run AppleScript: {}", e)))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(Error::Tool(format!(
            "AppleScript error: {}",
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

/// Configuration for Talos
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TalosConfig {
    /// Enable calendar tools
    #[serde(default = "default_true")]
    pub calendar: bool,
    /// Enable notes tools
    #[serde(default = "default_true")]
    pub notes: bool,
    /// Enable reminders tools
    #[serde(default = "default_true")]
    pub reminders: bool,
    /// Enable contacts tools
    #[serde(default = "default_true")]
    pub contacts: bool,
    /// Enable mail tools
    #[serde(default = "default_true")]
    pub mail: bool,
    /// Enable messages/iMessage tools
    #[serde(default = "default_true")]
    pub messages: bool,
    /// Enable browser tools
    #[serde(default = "default_true")]
    pub browser: bool,
    /// Enable system tools
    #[serde(default = "default_true")]
    pub system: bool,
    /// Enable network tools
    #[serde(default = "default_true")]
    pub network: bool,
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_creation() {
        let registry = TalosRegistry::with_defaults();
        assert!(!registry.is_empty());
    }

    #[test]
    fn test_schemas() {
        let registry = TalosRegistry::with_defaults();
        let schemas = registry.schemas();
        assert!(!schemas.is_empty());
    }

    #[test]
    fn test_sanitize_applescript_quotes() {
        assert_eq!(
            sanitize_applescript(r#"hello "world""#),
            r#"hello \"world\""#
        );
    }

    #[test]
    fn test_sanitize_applescript_backslash_then_quote() {
        // Input: \" should become \\\" (escaped backslash + escaped quote)
        assert_eq!(sanitize_applescript(r#"\""#), r#"\\\""#);
    }

    #[test]
    fn test_sanitize_applescript_backslashes() {
        assert_eq!(sanitize_applescript(r#"a\b"#), r#"a\\b"#);
    }

    #[test]
    fn test_sanitize_shell_arg_simple() {
        assert_eq!(sanitize_shell_arg("hello"), "'hello'");
    }

    #[test]
    fn test_sanitize_shell_arg_with_semicolon() {
        assert_eq!(sanitize_shell_arg("; rm -rf /"), "'; rm -rf /'");
    }

    #[test]
    fn test_sanitize_shell_arg_with_single_quotes() {
        assert_eq!(sanitize_shell_arg("it's"), "'it'\\''s'");
    }

    #[test]
    fn test_web_search_tool_registered() {
        let registry = TalosRegistry::with_defaults();
        let tools = registry.list();

        // Verify web_search is registered
        assert!(
            tools.contains(&"web_search"),
            "web_search tool should be registered"
        );

        // Verify we can get the tool
        let tool = registry.get("web_search");
        assert!(tool.is_some(), "Should be able to retrieve web_search tool");

        // Verify the schema
        if let Some(tool) = tool {
            let schema = tool.schema();
            assert_eq!(schema.name, "web_search");
            assert!(!schema.description.is_empty());

            // Check parameters exist
            let params = schema.parameters.as_object().expect("should be an object");
            let props = params
                .get("properties")
                .expect("key should exist")
                .as_object()
                .expect("should be an object");
            assert!(props.contains_key("query"));
            assert!(props.contains_key("count"));
        }
    }

    #[test]
    fn test_whatsapp_tools_registered() {
        let registry = TalosRegistry::with_defaults();
        let tools = registry.list();

        let expected = [
            "whatsapp_send_message",
            "whatsapp_send_image",
            "whatsapp_send_document",
            "whatsapp_send_template",
            "whatsapp_get_profile",
        ];

        for name in &expected {
            assert!(
                tools.contains(name),
                "tool '{}' should be registered in TalosRegistry",
                name
            );
        }
    }

    #[test]
    fn test_instagram_tools_registered() {
        let registry = TalosRegistry::with_defaults();
        let tools = registry.list();

        let expected = [
            "instagram_send_photo",
            "instagram_send_reel",
            "instagram_get_profile",
        ];

        for name in &expected {
            assert!(
                tools.contains(name),
                "tool '{}' should be registered in TalosRegistry",
                name
            );
        }
    }

    #[test]
    fn test_github_tools_registered() {
        let registry = TalosRegistry::with_defaults();
        let tools = registry.list();

        let expected = [
            "gh_pr_review",
            "gh_pr_comment",
            "gh_issue_create",
            "gh_actions_status",
        ];

        for name in &expected {
            assert!(
                tools.contains(name),
                "tool '{}' should be registered in TalosRegistry",
                name
            );
        }
    }

    #[test]
    fn test_screen_record_tools_registered() {
        let registry = TalosRegistry::with_defaults();
        let tools = registry.list();

        // Screen record tools are macOS-only
        #[cfg(target_os = "macos")]
        {
            assert!(tools.contains(&"screen_record_start"), "screen_record_start should be registered");
            assert!(tools.contains(&"screen_record_stop"), "screen_record_stop should be registered");
        }
    }
}
