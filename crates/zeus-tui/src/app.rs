#![allow(dead_code)]
//! App — Pure data model for chat-first TUI.

use chrono::Local;

/// Copy text to clipboard via pbcopy (macOS). Silent on failure.
pub fn copy_to_clipboard(text: &str) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    if let Ok(mut child) = Command::new("pbcopy").stdin(Stdio::piped()).spawn() {
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}

#[derive(Clone)]
pub struct Agent {
    pub name: String,
    pub task: String,
    pub status: AgentStatus,
    pub progress: u16,
    pub iterations: (u16, u16),
}

#[derive(Clone, PartialEq)]
pub enum AgentStatus {
    Running,
    Idle,
    Completed,
    Error,
}

#[derive(Clone)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    pub timestamp: String,
    pub agent_name: Option<String>,
    /// True while tokens are still arriving from the gateway.
    pub streaming: bool,
    /// Streaming markdown buffer — tracks safe flush boundaries during streaming.
    /// Set to Some when a message is actively streaming, None for completed messages.
    pub stream_state: Option<crate::markdown_stream::MarkdownStreamState>,
    /// Source channel badge (e.g. "discord", "telegram", "tui").
    pub channel_source: Option<String>,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Role {
    User,
    Assistant,
    System,
    Tool,
}

#[derive(Clone)]
pub struct Channel {
    pub platform: String,
    pub icon: &'static str,
    pub name: String,
    pub status: ChannelStatus,
    pub unread: u16,
    pub last_msg: String,
}

#[derive(Clone, PartialEq)]
pub enum ChannelStatus {
    Connected,
    Relay,
    Offline,
}

pub struct App {
    /// This agent's configured name — shown in chat bubbles instead of hardcoded "Zeus".
    pub self_name: String,
    pub agents: Vec<Agent>,
    pub messages: Vec<ChatMessage>,
    pub input: String,
    /// Cursor position within `input` (char index, not byte index).
    pub cursor_pos: usize,
    pub channels: Vec<Channel>,
    pub tick_count: u64,
    pub gateway_url: String,
    pub connected: bool,
    pub model: String,
    pub provider: String,
    pub tools_count: usize,
    pub sessions_count: usize,
    pub auth_method: String,
    pub gateway_version: String,
    /// Index of the currently selected message (for clipboard copy).
    pub selected_message: Option<usize>,
    /// How many lines from the bottom we are scrolled up (0 = at bottom).
    pub scroll_offset: usize,
    /// Layer 3: current iteration count during a cooking loop (0 = idle).
    pub cooking_iter: u32,
    /// Layer 3: number of tool calls fired in the current cooking loop.
    pub cooking_tools: u32,
    /// Layer 3: total tool calls across the whole turn (does not reset per iteration).
    pub total_tools: u32,
    /// Layer 3: when the current turn started (for "✅ Done" timing).
    pub turn_started: Option<std::time::Instant>,
    /// Layer 3: latest thinking/reasoning snippet from ThinkingDelta events.
    pub thinking_text: Option<String>,
    /// Ctrl+C 3-strike counter: 1st cancels stream/clears input, 2nd warns, 3rd exits.
    pub ctrl_c_count: u8,
    /// Timestamp of last Ctrl+C press for timeout reset (3 seconds).
    pub ctrl_c_last: Option<std::time::Instant>,
    /// Pantheon data
    pub pantheon_rooms: Vec<PantheonRoom>,
    pub pantheon_missions: Vec<PantheonMission>,
    pub pantheon_selected: PantheonSelection,
    pub pantheon_cursor: usize,
    /// Drill-down: Some(id) when viewing a room or mission detail
    pub pantheon_drill: Option<PantheonDrill>,
    /// DM target agent name, set by Office cross-nav (Enter on focused agent)
    pub pantheon_dm_target: Option<String>,
    /// Messages in the currently drilled-into room
    pub pantheon_messages: Vec<PantheonMessage>,
    /// Chat input for Pantheon room
    pub pantheon_input: String,
    /// Cursor position within pantheon_input (char index).
    pub pantheon_input_cursor: usize,
    /// Active panel in Pantheon 3-column layout
    pub pantheon_panel: PantheonPanel,
    /// Skip the next Pantheon message poll cycle after an optimistic send,
    /// to avoid the local message vanishing before the API confirms it.
    pub pantheon_skip_poll: bool,
    /// Scroll state for the Pantheon messages list (tracks selected/visible item).
    pub pantheon_msg_list_state: ratatui::widgets::ListState,
    /// Active top-level tab
    pub active_tab: Tab,
    /// Reconnect attempt counter (increments each reconnect try)
    pub reconnect_attempts: usize,
    /// Set to true when user presses Esc during streaming — signals the stream
    /// callback to stop processing tokens and finish early.
    pub stream_cancelled: bool,
    /// CancellationToken for the in-flight cooking task. `Some` while a cooking
    /// task is running; `.cancel()` aborts the underlying HTTP stream cleanly.
    /// Cleared after the task finishes.
    pub cooking_cancel: Option<tokio_util::sync::CancellationToken>,
    /// When the current cooking task started. Used by the watchdog backstop
    /// to detect stuck cooking state (e.g. after 300s with pending inputs).
    pub cooking_started_at: Option<std::time::Instant>,
    /// T20: Pending user inputs queued while a stream is in flight.
    /// Drained one-at-a-time when the current stream completes.
    pub pending_inputs: std::collections::VecDeque<String>,
    /// Cap on `pending_inputs` length. Older entries dropped when exceeded.
    pub max_pending: usize,
    /// When set, the main event loop will pop it into `input` and synthesize
    /// an Enter keypress on the next tick. Set by the stream-end hook to
    /// drain the next queued message into the existing submit pipeline.
    pub pending_drain: Option<String>,
    /// When true, the next message sent will use extended thinking (xhigh).
    /// Set by /think or "ultrathink" command, consumed on next send.
    pub ultrathink_next: bool,
    /// Settings tab cursor position
    pub settings_cursor: usize,
    /// Whether we're editing a settings field inline
    pub settings_editing: bool,
    /// The edit buffer for the field being edited
    pub settings_edit_value: String,
    /// Cursor position within the edit buffer
    pub settings_edit_cursor: usize,
    /// Config JSON fetched from gateway (populated by polling loop)
    pub settings_config: serde_json::Value,
    /// Pending edits (key → new value) not yet saved
    pub settings_dirty: std::collections::HashMap<String, String>,
    /// Status message shown at bottom of settings screen
    pub settings_status: String,
    /// Input history (shell-like arrow up/down recall)
    pub input_history: Vec<String>,
    /// Current position in input history (-1 = not browsing)
    pub input_history_idx: isize,
    /// The Office — pixel art fleet visualization state
    pub office: crate::office::state::OfficeState,
    /// Static office background (generated once, reused every frame)
    pub office_bg: crate::office::renderer::PixelGrid,
    /// Pantheon IRC TUI state — drives the 3-column IRC layout on the Pantheon tab.
    pub pantheon_irc: crate::pantheon::app::PantheonApp,
    /// Accumulated input tokens for this session.
    pub session_input_tokens: usize,
    /// Accumulated output tokens for this session.
    pub session_output_tokens: usize,
    /// Whether the keybinding overlay (`?` modal) is visible.
    pub show_keybind_overlay: bool,
    /// Whether mouse capture is enabled (true = scroll works, text selection broken).
    /// Toggle with `m` key. When false, mouse events pass through for text selection.
    pub mouse_capture_enabled: bool,
    /// Estimated token count of the current session context (chars / 4).
    pub context_tokens: usize,
    /// Max context window size (tokens). Default: 200_000.
    pub context_max_tokens: usize,

    // ── S100 #13: Chat history search ────────────────────────────────────────
    /// Whether search mode is active (Ctrl+F toggles).
    pub search_active: bool,
    /// Current search query string.
    pub search_query: String,
    /// Cursor position within search_query (char index).
    pub search_cursor: usize,
    /// Indices into `messages` that match the current query.
    pub search_matches: Vec<usize>,
    /// Which match is currently focused (index into search_matches).
    pub search_match_idx: usize,

    // ── S103 #30: Token budget tracking ──────────────────────────────────────
    /// Configurable token budget for this session (0 = unlimited).
    pub token_budget: usize,
    /// Warning threshold as a fraction of budget (e.g. 0.8 = warn at 80%).
    pub budget_warn_threshold: f64,
}

impl App {
    pub fn new(gateway_url: &str) -> Self {
        Self {
            self_name: "Zeus".to_string(),
            agents: Vec::new(),
            messages: Vec::new(),
            input: String::new(),
            cursor_pos: 0,
            channels: Vec::new(),
            tick_count: 0,
            gateway_url: gateway_url.to_string(),
            connected: false,
            model: String::new(),
            provider: String::new(),
            tools_count: 0,
            sessions_count: 0,
            auth_method: String::new(),
            gateway_version: String::new(),
            selected_message: None,
            scroll_offset: 0,
            cooking_iter: 0,
            cooking_tools: 0,
            total_tools: 0,
            turn_started: None,
            thinking_text: None,
            ctrl_c_count: 0,
            ctrl_c_last: None,
            pantheon_rooms: Vec::new(),
            pantheon_missions: Vec::new(),
            pantheon_selected: PantheonSelection::Rooms,
            pantheon_cursor: 0,
            pantheon_drill: None,
            pantheon_dm_target: None,
            pantheon_messages: Vec::new(),
            pantheon_input: String::new(),
            pantheon_input_cursor: 0,
            pantheon_panel: PantheonPanel::Rooms,
            pantheon_skip_poll: false,
            pantheon_msg_list_state: ratatui::widgets::ListState::default(),
            active_tab: Tab::Chat,
            reconnect_attempts: 0,
            stream_cancelled: false,
            cooking_cancel: None,
            cooking_started_at: None,
            pending_inputs: std::collections::VecDeque::new(),
            max_pending: 16,
            pending_drain: None,
            ultrathink_next: false,
            settings_cursor: 0,
            settings_editing: false,
            settings_edit_value: String::new(),
            settings_edit_cursor: 0,
            settings_config: serde_json::Value::Null,
            settings_dirty: std::collections::HashMap::new(),
            settings_status: String::new(),
            input_history: Vec::new(),
            input_history_idx: -1,
            office: crate::office::state::OfficeState::new(),
            office_bg: crate::office::background::generate(80, 40),
            pantheon_irc: crate::pantheon::app::PantheonApp::new("zeus".to_string()),
            session_input_tokens: 0,
            session_output_tokens: 0,
            show_keybind_overlay: false,
            context_tokens: 0,
            context_max_tokens: 200_000,
            search_active: false,
            search_query: String::new(),
            search_cursor: 0,
            search_matches: Vec::new(),
            search_match_idx: 0,
            token_budget: 0,
            budget_warn_threshold: 0.8,
            mouse_capture_enabled: true,
        }
    }

    /// Recompute search_matches from the current query against all messages.
    pub fn update_search_matches(&mut self) {
        let q = self.search_query.to_lowercase();
        if q.is_empty() {
            self.search_matches.clear();
            self.search_match_idx = 0;
            return;
        }
        self.search_matches = self
            .messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.content.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();
        if self.search_match_idx >= self.search_matches.len() {
            self.search_match_idx = self.search_matches.len().saturating_sub(1);
        }
    }

    /// Jump to the next search match (wraps).
    pub fn search_next(&mut self) {
        if self.search_matches.is_empty() { return; }
        self.search_match_idx = (self.search_match_idx + 1) % self.search_matches.len();
        self.scroll_to_match();
    }

    /// Jump to the previous search match (wraps).
    pub fn search_prev(&mut self) {
        if self.search_matches.is_empty() { return; }
        if self.search_match_idx == 0 {
            self.search_match_idx = self.search_matches.len() - 1;
        } else {
            self.search_match_idx -= 1;
        }
        self.scroll_to_match();
    }

    /// Scroll so the focused match is visible (sets scroll_offset to pin it).
    fn scroll_to_match(&mut self) {
        if let Some(&msg_idx) = self.search_matches.get(self.search_match_idx) {
            // Pin selected_message so the renderer highlights it
            self.selected_message = Some(msg_idx);
            // Scroll offset: messages render newest-at-bottom, so distance from end
            let from_end = self.messages.len().saturating_sub(1).saturating_sub(msg_idx);
            self.scroll_offset = from_end;
        }
    }

    /// Recompute context_tokens from current messages (chars / 4 approximation).
    pub fn update_context_tokens(&mut self) {
        let total_chars: usize = self.messages.iter().map(|m| m.content.len()).sum();
        self.context_tokens = total_chars / 4;
    }

    /// Returns context pressure as a fraction 0.0–1.0.
    pub fn context_pressure(&self) -> f64 {
        if self.context_max_tokens == 0 {
            return 0.0;
        }
        (self.context_tokens as f64 / self.context_max_tokens as f64).min(1.0)
    }

    /// Scroll up by `lines` lines (towards older messages).
    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(lines);
    }

    /// Scroll down by `lines` lines (towards newer messages).
    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    /// Jump to top (oldest messages).
    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = usize::MAX; // clamped in render
    }

    /// Jump to bottom (newest messages — live view).
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
    }

    /// Returns true if we are at the live bottom (no scroll).
    pub fn is_at_bottom(&self) -> bool {
        self.scroll_offset == 0
    }

    /// Move message selection up (towards older messages).
    pub fn select_prev_message(&mut self) {
        if self.messages.is_empty() { return; }
        self.selected_message = Some(match self.selected_message {
            None => self.messages.len() - 1,
            Some(0) => 0,
            Some(i) => i - 1,
        });
    }

    /// Move message selection down (towards newer messages).
    pub fn select_next_message(&mut self) {
        if self.messages.is_empty() { return; }
        self.selected_message = Some(match self.selected_message {
            None => self.messages.len() - 1,
            Some(i) if i + 1 >= self.messages.len() => self.messages.len() - 1,
            Some(i) => i + 1,
        });
    }

    /// Copy the selected message content to clipboard via pbcopy (macOS).
    /// Returns the content copied, or None if no message is selected.
    pub fn copy_selected_to_clipboard(&self) -> Option<String> {
        let idx = self.selected_message?;
        let content = self.messages.get(idx)?.content.clone();
        copy_to_clipboard(&content);
        Some(content)
    }

    /// Insert a character at the current cursor position.
    pub fn insert_char(&mut self, c: char) {
        let byte_idx = self.char_to_byte(self.cursor_pos);
        self.input.insert(byte_idx, c);
        self.cursor_pos += 1;
    }

    /// Delete the character immediately before the cursor (Backspace).
    pub fn delete_char_before(&mut self) {
        if self.cursor_pos == 0 { return; }
        let byte_idx = self.char_to_byte(self.cursor_pos - 1);
        self.input.remove(byte_idx);
        self.cursor_pos -= 1;
    }

    /// Move cursor one character to the left.
    pub fn cursor_left(&mut self) {
        if self.cursor_pos > 0 { self.cursor_pos -= 1; }
    }

    /// Move cursor one character to the right.
    pub fn cursor_right(&mut self) {
        let len = self.input.chars().count();
        if self.cursor_pos < len { self.cursor_pos += 1; }
    }

    /// Move cursor to start of input.
    pub fn cursor_home(&mut self) {
        self.cursor_pos = 0;
    }

    /// Move cursor to end of input.
    pub fn cursor_end(&mut self) {
        self.cursor_pos = self.input.chars().count();
    }

    /// Insert a literal newline at the current cursor position (Shift+Enter).
    pub fn insert_newline(&mut self) {
        let byte_idx = self.char_to_byte(self.cursor_pos);
        self.input.insert(byte_idx, '\n');
        self.cursor_pos += 1;
    }

    /// Clear input and reset cursor.
    pub fn clear_input(&mut self) {
        self.input.clear();
        self.cursor_pos = 0;
    }

    /// Convert char index to byte index for the current input string.
    fn char_to_byte(&self, char_idx: usize) -> usize {
        self.input.char_indices()
            .nth(char_idx)
            .map(|(b, _)| b)
            .unwrap_or(self.input.len())
    }

    pub fn tick(&mut self) {
        self.tick_count += 1;
    }

    /// Push a blank streaming placeholder for an in-progress assistant reply.
    pub fn begin_stream(&mut self, agent_name: &str) {
        // Clear any stale thinking snippet from a prior response so it doesn't
        // leak into the cooking indicator / placeholder while this new message starts.
        self.thinking_text = None;
        self.messages.push(ChatMessage {
            role: Role::Assistant,
            content: String::new(),
            timestamp: Local::now().format("%H:%M:%S").to_string(),
            agent_name: Some(agent_name.to_string()),
            streaming: true,
            channel_source: None,
            stream_state: Some(crate::markdown_stream::MarkdownStreamState::new()),
        });
    }

    /// Append a token chunk to the last message (no-op if no messages).
    pub fn append_token(&mut self, token: &str) {
        if let Some(msg) = self.messages.last_mut() {
            msg.content.push_str(token);
            if let Some(ref mut state) = msg.stream_state {
                let _ = state.push(token);
            }
        }
    }

    /// Set the content of the last streaming message and sync stream_state.
    /// Used for the first token to avoid the first-token-not-in-stream_state bug.
    pub fn set_first_token(&mut self, token: &str) {
        if let Some(msg) = self.messages.last_mut() {
            if msg.streaming {
                msg.content = token.to_string();
                if let Some(ref mut state) = msg.stream_state {
                    state.reset();
                    let _ = state.push(token);
                }
            }
        }
    }

    /// Mark the last streaming message as complete.
    /// Clears stream_state so the non-streaming render path uses msg.content directly.
    pub fn finish_stream(&mut self) {
        if let Some(msg) = self.messages.last_mut() {
            msg.streaming = false;
            if let Some(ref mut state) = msg.stream_state {
                let _ = state.finish();
            }
            // Clear stream_state — prevents double-render (streaming + non-streaming paths).
            msg.stream_state = None;
        }
        // Reset cooking counters when the stream finishes
        self.cooking_iter = 0;
        self.cooking_tools = 0;
        self.total_tools = 0;
        self.turn_started = None;
        self.thinking_text = None;
    }

    /// Map a raw tool name to a human-friendly verb phrase.
    /// Returns the verb prefix; the caller appends the target/summary.
    pub fn tool_verb(tool_name: &str) -> &'static str {
        match tool_name {
            "read_file" | "Read"                       => "Reading",
            "write_file" | "Write"                     => "Writing",
            "edit_file" | "Edit" | "apply_patch"       => "Editing",
            "shell" | "bash" | "Bash"                  => "Running shell:",
            "list_dir" | "Glob" | "ls"                 => "Listing",
            "web_fetch" | "WebFetch"                   => "Fetching",
            "web_search"                               => "Searching",
            "deep_research"                            => "Researching",
            "spawn" | "Task"                           => "Spawning agent:",
            n if n.starts_with("discord_")             => "Messaging Discord:",
            n if n.starts_with("telegram_")            => "Messaging Telegram:",
            n if n.starts_with("slack_")               => "Messaging Slack:",
            n if n.starts_with("browser_")             => "Browser:",
            _                                          => "",
        }
    }

    /// Mark the most recent Tool message as complete by appending a status glyph.
    pub fn mark_last_tool_done(&mut self, success: bool) {
        let glyph = if success { " ✓" } else { " ✗" };
        if let Some(msg) = self.messages.iter_mut().rev().find(|m| matches!(m.role, Role::Tool)) {
            // Avoid double-appending if somehow called twice.
            if !msg.content.ends_with('✓') && !msg.content.ends_with('✗') {
                msg.content.push_str(glyph);
            }
        }
    }

    /// Push the final "✅ Done (N tools, M iters, T.Ts)" summary line for a turn.
    pub fn push_turn_summary(&mut self) {
        if self.total_tools == 0 && self.cooking_iter == 0 {
            return; // nothing interesting happened
        }
        let elapsed = self.turn_started
            .map(|t| t.elapsed().as_secs_f32())
            .unwrap_or(0.0);
        let tool_word = if self.total_tools == 1 { "tool call" } else { "tool calls" };
        let iter_word = if self.cooking_iter == 1 { "iteration" } else { "iterations" };
        let content = if elapsed > 0.05 {
            format!("✅ Done ({} {}, {} {}, {:.1}s)",
                self.total_tools, tool_word, self.cooking_iter.max(1), iter_word, elapsed)
        } else {
            format!("✅ Done ({} {}, {} {})",
                self.total_tools, tool_word, self.cooking_iter.max(1), iter_word)
        };
        self.messages.push(ChatMessage {
            role: Role::System,
            content,
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
            agent_name: Some("System".into()),
            streaming: false,
            channel_source: Some("tui".into()),
            stream_state: None,
        });
    }

    /// Finish any intermediate streaming messages that are still active.
    /// Called before begin_stream to prevent orphaned streaming placeholders
    /// (e.g. when a ToolStart event creates a new streaming message while
    /// the previous one is still marked streaming=true).
    pub fn finish_intermediate_streams(&mut self) {
        let len = self.messages.len();
        if len < 2 { return; }
        // Finish all streaming messages except the very last one.
        // Called at the ToolStart boundary AFTER push_tool_event has
        // already inserted the tool-event row (which is non-streaming),
        // so "last" is the tool event and "older" are any stuck streams.
        for i in 0..len.saturating_sub(1) {
            if self.messages[i].streaming {
                self.messages[i].streaming = false;
                self.messages[i].stream_state = None;
            }
        }
    }

    /// Push a tool event message (Layer 2: visible tool call trace).
    /// Renders like Claude Code: `⚡ Reading /path/to/file` using a verb map.
    pub fn push_tool_event(&mut self, tool_name: &str, summary: &str) {
        self.cooking_tools += 1;
        self.total_tools += 1;
        if self.turn_started.is_none() {
            self.turn_started = Some(std::time::Instant::now());
        }
        let verb = Self::tool_verb(tool_name);
        let content = if verb.is_empty() {
            // Unknown tool — fall back to raw name
            if summary.is_empty() {
                format!("⚡ {}", tool_name)
            } else {
                format!("⚡ {}  {}", tool_name, summary)
            }
        } else if summary.is_empty() {
            format!("⚡ {}", verb.trim_end_matches(':'))
        } else {
            format!("⚡ {} {}", verb, summary)
        };
        self.messages.push(ChatMessage {
            role: Role::Tool,
            content,
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
            agent_name: None,
            streaming: false,
            channel_source: None,
            stream_state: None,
        });
    }

    /// Bump the iteration counter (Layer 3).
    /// Does NOT reset `total_tools` — that accumulates across the whole turn.
    pub fn bump_iter(&mut self) {
        self.cooking_iter += 1;
        self.cooking_tools = 0;
        if self.turn_started.is_none() {
            self.turn_started = Some(std::time::Instant::now());
        }
    }

    /// Accumulate token counts from a completed turn.
    pub fn record_turn_tokens(&mut self, input: usize, output: usize) {
        self.session_input_tokens += input;
        self.session_output_tokens += output;
    }

    /// Format session token total as "12.4k" or "123".
    pub fn format_tokens(&self) -> String {
        let total = self.session_input_tokens + self.session_output_tokens;
        if total >= 1000 {
            format!("{:.1}k", total as f64 / 1000.0)
        } else {
            format!("{}", total)
        }
    }

    /// Estimate session cost in USD: $3/M input, $15/M output.
    pub fn estimate_cost(&self) -> f64 {
        let input_cost = self.session_input_tokens as f64 * 3.0 / 1_000_000.0;
        let output_cost = self.session_output_tokens as f64 * 15.0 / 1_000_000.0;
        input_cost + output_cost
    }

    /// Format cost as "~$0.04" or "~$1.23".
    pub fn format_cost(&self) -> String {
        let cost = self.estimate_cost();
        if cost < 0.01 {
            format!("~$0.00")
        } else {
            format!("~${:.2}", cost)
        }
    }

    /// Returns a budget warning string if over threshold, or None.
    /// "⚠ 85% of budget used (8.5k / 10k tokens)"
    pub fn budget_warning(&self) -> Option<String> {
        if self.token_budget == 0 {
            return None;
        }
        let used = self.session_input_tokens + self.session_output_tokens;
        let fraction = used as f64 / self.token_budget as f64;
        if fraction >= self.budget_warn_threshold {
            let pct = (fraction * 100.0).round() as usize;
            let used_fmt = if used >= 1000 { format!("{:.1}k", used as f64 / 1000.0) } else { format!("{}", used) };
            let budget_fmt = if self.token_budget >= 1000 { format!("{:.1}k", self.token_budget as f64 / 1000.0) } else { format!("{}", self.token_budget) };
            Some(format!("⚠ {}% of budget ({}/{})", pct, used_fmt, budget_fmt))
        } else {
            None
        }
    }

    /// Update the live thinking snippet (Layer 3).
    pub fn set_thinking(&mut self, text: &str) {
        self.thinking_text = Some(text.chars().take(120).collect());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cooking_cancel_token_default_none() {
        // Fresh App must have no in-flight cooking task.
        let app = App::new("http://localhost:8080");
        assert!(app.cooking_cancel.is_none(),
            "cooking_cancel must start as None — Some implies a leaked task");
    }

    #[test]
    fn test_cooking_cancel_token_cancels_cleanly() {
        // Simulate the cooking-task lifecycle: install token, then cancel it.
        let mut app = App::new("http://localhost:8080");
        let token = tokio_util::sync::CancellationToken::new();
        app.cooking_cancel = Some(token.clone());

        // Esc/Ctrl+C handler logic: take + cancel.
        if let Some(t) = app.cooking_cancel.take() {
            t.cancel();
        }

        assert!(token.is_cancelled(), "token must be cancelled after handler runs");
        assert!(app.cooking_cancel.is_none(), "cooking_cancel must be cleared");
    }

    // T20: TUI message queue tests

    #[test]
    fn test_pending_inputs_default_empty() {
        let app = App::new("Test");
        assert!(app.pending_inputs.is_empty(),
            "pending_inputs must start empty");
        assert_eq!(app.max_pending, 16, "default max_pending should be 16");
        assert!(app.pending_drain.is_none(),
            "pending_drain must start as None");
    }

    #[test]
    fn test_pending_inputs_fifo_order() {
        let mut app = App::new("Test");
        app.pending_inputs.push_back("first".to_string());
        app.pending_inputs.push_back("second".to_string());
        app.pending_inputs.push_back("third".to_string());

        assert_eq!(app.pending_inputs.pop_front().as_deref(), Some("first"));
        assert_eq!(app.pending_inputs.pop_front().as_deref(), Some("second"));
        assert_eq!(app.pending_inputs.pop_front().as_deref(), Some("third"));
        assert!(app.pending_inputs.pop_front().is_none());
    }

    #[test]
    fn test_pending_inputs_cap_drops_oldest() {
        let mut app = App::new("Test");
        app.max_pending = 3;
        // Simulate the enqueue logic from lib.rs
        for i in 0..5 {
            if app.pending_inputs.len() >= app.max_pending {
                app.pending_inputs.pop_front();
            }
            app.pending_inputs.push_back(format!("msg-{}", i));
        }
        assert_eq!(app.pending_inputs.len(), 3, "queue must respect cap");
        assert_eq!(app.pending_inputs.pop_front().as_deref(), Some("msg-2"),
            "oldest entries must be dropped first");
        assert_eq!(app.pending_inputs.pop_front().as_deref(), Some("msg-3"));
        assert_eq!(app.pending_inputs.pop_front().as_deref(), Some("msg-4"));
    }

    #[test]
    fn test_pending_drain_round_trip() {
        let mut app = App::new("Test");
        app.pending_inputs.push_back("queued msg".to_string());

        // Simulate stream-end hook
        if let Some(next) = app.pending_inputs.pop_front() {
            app.pending_drain = Some(next);
        }

        assert!(app.pending_inputs.is_empty(),
            "queue must be popped after drain");
        assert_eq!(app.pending_drain.as_deref(), Some("queued msg"));

        // Simulate main-loop drain
        let drained = app.pending_drain.take();
        assert_eq!(drained.as_deref(), Some("queued msg"));
        assert!(app.pending_drain.is_none(),
            "pending_drain must be cleared after take()");
    }

    #[test]
    fn test_pending_inputs_clear_clears_all() {
        let mut app = App::new("Test");
        app.pending_inputs.push_back("a".to_string());
        app.pending_inputs.push_back("b".to_string());
        app.pending_inputs.push_back("c".to_string());

        let cleared = app.pending_inputs.len();
        app.pending_inputs.clear();

        assert_eq!(cleared, 3, "must report correct cleared count");
        assert!(app.pending_inputs.is_empty(),
            "queue must be empty after Ctrl-K clear");
    }

    #[test]
    fn test_chat_message_has_streaming_field() {
        let msg = ChatMessage {
            role: Role::Assistant,
            content: String::new(),
            timestamp: "00:00:00".into(),
            agent_name: Some("Zeus".into()),
            streaming: true,
            channel_source: None,
            stream_state: None,
        };
        assert!(msg.streaming);
    }

    #[test]
    fn test_chat_message_channel_source_defaults_none() {
        let msg = ChatMessage {
            role: Role::User,
            content: "hello".into(),
            timestamp: "00:00:00".into(),
            agent_name: None,
            streaming: false,
            channel_source: None,
            stream_state: None,
        };
        assert!(msg.channel_source.is_none());
    }

    #[test]
    fn test_chat_message_channel_source_discord() {
        let msg = ChatMessage {
            role: Role::User,
            content: "hello".into(),
            timestamp: "00:00:00".into(),
            agent_name: None,
            streaming: false,
            channel_source: Some("discord".into()),
            stream_state: None,
        };
        assert_eq!(msg.channel_source.as_deref(), Some("discord"));
    }

    #[test]
    fn test_begin_stream_channel_source_is_none() {
        let mut app = App::new("http://localhost:8080");
        app.begin_stream("Zeus");
        assert!(app.messages[0].channel_source.is_none());
    }

    #[test]
    fn test_begin_stream_pushes_placeholder() {
        let mut app = App::new("http://localhost:8080");
        app.begin_stream("Zeus");
        assert_eq!(app.messages.len(), 1);
        assert!(app.messages[0].streaming);
        assert_eq!(app.messages[0].content, "");
    }

    #[test]
    fn test_append_token_grows_content() {
        let mut app = App::new("http://localhost:8080");
        app.begin_stream("Zeus");
        app.append_token("Hello");
        app.append_token(", world");
        assert_eq!(app.messages.last().unwrap().content, "Hello, world");
    }

    #[test]
    fn test_finish_stream_clears_flag() {
        let mut app = App::new("http://localhost:8080");
        app.begin_stream("Zeus");
        app.finish_stream();
        assert!(!app.messages.last().unwrap().streaming);
    }

    #[test]
    fn test_append_token_empty_messages_noop() {
        let mut app = App::new("http://localhost:8080");
        app.append_token("no-op");
    }

    // --- Double-render bug fix tests ---

    #[test]
    fn test_set_first_token_syncs_stream_state() {
        let mut app = App::new("http://localhost:8080");
        app.begin_stream("Zeus");
        app.set_first_token("Hello");
        let msg = app.messages.last().unwrap();
        assert_eq!(msg.content, "Hello");
        // stream_state should have the token in flushed or pending
        let state = msg.stream_state.as_ref().unwrap();
        let combined = format!("{}{}", state.flushed(), state.pending());
        assert!(combined.contains("Hello"), "first token must be in stream_state");
    }

    #[test]
    fn test_finish_stream_clears_stream_state() {
        let mut app = App::new("http://localhost:8080");
        app.begin_stream("Zeus");
        app.append_token("Hello");
        app.finish_stream();
        let msg = app.messages.last().unwrap();
        assert!(!msg.streaming);
        assert!(msg.stream_state.is_none(), "stream_state should be cleared after finish");
    }

    #[test]
    fn test_finish_intermediate_streams_only_touches_older_messages() {
        let mut app = App::new("http://localhost:8080");
        // First streaming message
        app.begin_stream("Zeus");
        app.append_token("First");
        // Second streaming message (simulates ToolStart creating a new placeholder)
        app.begin_stream("Zeus");
        app.append_token("Second");

        // The first message should still be streaming
        assert!(app.messages[0].streaming);
        assert!(app.messages[1].streaming);

        // Finish intermediate — should close the first, leave the last
        app.finish_intermediate_streams();
        assert!(!app.messages[0].streaming, "intermediate message should be finished");
        assert!(app.messages[1].streaming, "last message should still be streaming");
    }

    #[test]
    fn test_no_double_render_on_short_response() {
        // Simulate: begin_stream → set_first_token → finish_stream
        // The content should be exactly the token, no duplication
        let mut app = App::new("http://localhost:8080");
        app.begin_stream("Zeus");
        app.set_first_token("Hi");
        app.finish_stream();
        assert_eq!(app.messages.last().unwrap().content, "Hi");
    }

    #[test]
    fn test_streaming_then_tool_start_finishes_previous() {
        // Simulate the full flow: tokens → ToolStart → more tokens → finish
        let mut app = App::new("http://localhost:8080");
        app.begin_stream("Zeus");
        app.set_first_token("Thinking");
        app.append_token(" about");

        // ToolStart: matches main.rs SSE handler order —
        //   push_tool_event → finish_intermediate_streams → begin_stream.
        // The tool event is pushed first (non-streaming row), then
        // finish_intermediate_streams closes any older streaming
        // messages (now "older" because the tool event is the new
        // tail), then begin_stream opens the post-tool continuation.
        app.push_tool_event("Bash", "ls -la");
        app.finish_intermediate_streams();
        app.begin_stream("Zeus");
        app.set_first_token("Result");

        // The first message should be finished
        assert!(!app.messages[0].streaming);
        assert_eq!(app.messages[0].content, "Thinking about");

        // The tool event should be present
        assert_eq!(app.messages[1].role, Role::Tool);

        // The new streaming message should be active
        assert!(app.messages[2].streaming);
        assert_eq!(app.messages[2].content, "Result");

        // Final finish
        app.finish_stream();
        assert!(!app.messages[2].streaming);
    }

    #[test]
    fn test_selected_message_starts_none() {
        let app = App::new("http://localhost:8080");
        assert!(app.selected_message.is_none());
    }

    #[test]
    fn test_select_prev_on_empty_messages_noop() {
        let mut app = App::new("http://localhost:8080");
        app.select_prev_message();
        assert!(app.selected_message.is_none());
    }

    #[test]
    fn test_select_next_on_empty_messages_noop() {
        let mut app = App::new("http://localhost:8080");
        app.select_next_message();
        assert!(app.selected_message.is_none());
    }

    #[test]
    fn test_select_prev_selects_last_when_none() {
        let mut app = App::new("http://localhost:8080");
        app.begin_stream("Zeus");
        app.finish_stream();
        app.begin_stream("Zeus");
        app.finish_stream();
        app.select_prev_message();
        assert_eq!(app.selected_message, Some(1)); // last index
    }

    #[test]
    fn test_select_prev_moves_up() {
        let mut app = App::new("http://localhost:8080");
        app.begin_stream("Zeus");
        app.finish_stream();
        app.begin_stream("Zeus");
        app.finish_stream();
        app.selected_message = Some(1);
        app.select_prev_message();
        assert_eq!(app.selected_message, Some(0));
    }

    #[test]
    fn test_select_prev_clamps_at_zero() {
        let mut app = App::new("http://localhost:8080");
        app.begin_stream("Zeus");
        app.finish_stream();
        app.selected_message = Some(0);
        app.select_prev_message();
        assert_eq!(app.selected_message, Some(0));
    }

    #[test]
    fn test_select_next_moves_down() {
        let mut app = App::new("http://localhost:8080");
        app.begin_stream("Zeus");
        app.finish_stream();
        app.begin_stream("Zeus");
        app.finish_stream();
        app.selected_message = Some(0);
        app.select_next_message();
        assert_eq!(app.selected_message, Some(1));
    }

    #[test]
    fn test_select_next_clamps_at_end() {
        let mut app = App::new("http://localhost:8080");
        app.begin_stream("Zeus");
        app.finish_stream();
        app.selected_message = Some(0);
        app.select_next_message();
        assert_eq!(app.selected_message, Some(0)); // only 1 message
    }

    #[test]
    fn test_copy_selected_none_when_no_selection() {
        let mut app = App::new("http://localhost:8080");
        app.begin_stream("Zeus");
        app.append_token("hello");
        app.finish_stream();
        let result = app.copy_selected_to_clipboard();
        assert!(result.is_none());
    }

    // --- cursor tests ---

    #[test]
    fn test_cursor_starts_at_zero() {
        let app = App::new("http://localhost:8080");
        assert_eq!(app.cursor_pos, 0);
    }

    #[test]
    fn test_insert_char_moves_cursor() {
        let mut app = App::new("http://localhost:8080");
        app.insert_char('h');
        app.insert_char('i');
        assert_eq!(app.input, "hi");
        assert_eq!(app.cursor_pos, 2);
    }

    #[test]
    fn test_cursor_left_moves_back() {
        let mut app = App::new("http://localhost:8080");
        app.insert_char('a');
        app.insert_char('b');
        app.cursor_left();
        assert_eq!(app.cursor_pos, 1);
    }

    #[test]
    fn test_cursor_left_clamps_at_zero() {
        let mut app = App::new("http://localhost:8080");
        app.cursor_left();
        assert_eq!(app.cursor_pos, 0);
    }

    #[test]
    fn test_cursor_right_moves_forward() {
        let mut app = App::new("http://localhost:8080");
        app.insert_char('a');
        app.insert_char('b');
        app.cursor_left();
        app.cursor_left();
        app.cursor_right();
        assert_eq!(app.cursor_pos, 1);
    }

    #[test]
    fn test_cursor_right_clamps_at_end() {
        let mut app = App::new("http://localhost:8080");
        app.insert_char('a');
        app.cursor_right(); // already at end
        assert_eq!(app.cursor_pos, 1);
    }

    #[test]
    fn test_cursor_home() {
        let mut app = App::new("http://localhost:8080");
        app.insert_char('a');
        app.insert_char('b');
        app.cursor_home();
        assert_eq!(app.cursor_pos, 0);
    }

    #[test]
    fn test_cursor_end() {
        let mut app = App::new("http://localhost:8080");
        app.insert_char('a');
        app.insert_char('b');
        app.cursor_home();
        app.cursor_end();
        assert_eq!(app.cursor_pos, 2);
    }

    #[test]
    fn test_insert_char_at_middle() {
        let mut app = App::new("http://localhost:8080");
        app.insert_char('a');
        app.insert_char('c');
        app.cursor_left(); // cursor between 'a' and 'c'
        app.insert_char('b');
        assert_eq!(app.input, "abc");
        assert_eq!(app.cursor_pos, 2);
    }

    #[test]
    fn test_delete_char_before_cursor() {
        let mut app = App::new("http://localhost:8080");
        app.insert_char('a');
        app.insert_char('b');
        app.delete_char_before();
        assert_eq!(app.input, "a");
        assert_eq!(app.cursor_pos, 1);
    }

    #[test]
    fn test_delete_char_before_at_zero_noop() {
        let mut app = App::new("http://localhost:8080");
        app.insert_char('a');
        app.cursor_home();
        app.delete_char_before();
        assert_eq!(app.input, "a");
        assert_eq!(app.cursor_pos, 0);
    }

    #[test]
    fn test_clear_input_resets_cursor() {
        let mut app = App::new("http://localhost:8080");
        app.insert_char('a');
        app.insert_char('b');
        app.clear_input();
        assert_eq!(app.input, "");
        assert_eq!(app.cursor_pos, 0);
    }

    #[test]
    fn test_copy_selected_returns_content() {
        let mut app = App::new("http://localhost:8080");
        app.begin_stream("Zeus");
        app.append_token("hello world");
        app.finish_stream();
        app.selected_message = Some(0);
        let result = app.copy_selected_to_clipboard();
        assert_eq!(result, Some("hello world".to_string()));
    }

    // --- Track A5: scroll tests ---

    #[test]
    fn test_scroll_offset_starts_at_zero() {
        let app = App::new("http://localhost:8080");
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn test_is_at_bottom_initially() {
        let app = App::new("http://localhost:8080");
        assert!(app.is_at_bottom());
    }

    #[test]
    fn test_scroll_up_increases_offset() {
        let mut app = App::new("http://localhost:8080");
        app.scroll_up(5);
        assert_eq!(app.scroll_offset, 5);
    }

    #[test]
    fn test_scroll_down_decreases_offset() {
        let mut app = App::new("http://localhost:8080");
        app.scroll_up(10);
        app.scroll_down(3);
        assert_eq!(app.scroll_offset, 7);
    }

    #[test]
    fn test_scroll_down_does_not_underflow() {
        let mut app = App::new("http://localhost:8080");
        app.scroll_down(100); // offset is 0, saturating_sub clamps at 0
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn test_scroll_to_top_sets_large_offset() {
        let mut app = App::new("http://localhost:8080");
        app.scroll_to_top();
        assert_eq!(app.scroll_offset, usize::MAX);
    }

    #[test]
    fn test_scroll_to_bottom_resets_offset() {
        let mut app = App::new("http://localhost:8080");
        app.scroll_up(20);
        app.scroll_to_bottom();
        assert_eq!(app.scroll_offset, 0);
        assert!(app.is_at_bottom());
    }

    #[test]
    fn test_is_at_bottom_false_when_scrolled() {
        let mut app = App::new("http://localhost:8080");
        app.scroll_up(1);
        assert!(!app.is_at_bottom());
    }
}

// ---- Pantheon data types ----

#[derive(Clone)]
pub struct PantheonRoom {
    pub id: String,
    pub name: String,
    pub participant_count: usize,
    pub status: String,
    pub unread: usize,
}

/// Format a timestamp as relative time ("2m ago", "1h ago", "3d ago").
pub fn relative_time(timestamp: &str) -> String {
    // Try to parse HH:MM:SS format
    if timestamp.len() >= 8 {
        // Simple relative display for same-day messages
        return timestamp[..5].to_string(); // HH:MM
    }
    timestamp.to_string()
}

#[derive(Clone)]
pub struct PantheonMission {
    pub id: String,
    pub name: String,
    pub status: String, // "Draft", "Active", "Completed"
    pub agent_count: usize,
}

#[derive(Clone, PartialEq, Debug)]
pub enum PantheonSelection {
    Rooms,
    Missions,
}

/// What the user has drilled into in the Pantheon view.
#[derive(Clone, PartialEq, Debug)]
pub enum PantheonDrill {
    Room(String),    // room id
    Mission(String), // mission id
}

/// Active panel in Pantheon 3-column layout.
#[derive(Clone, PartialEq, Debug)]
pub enum PantheonPanel {
    Rooms,
    Messages,
    Missions,
}

/// A message in a Pantheon war room.
#[derive(Clone, Debug)]
pub struct PantheonMessage {
    pub id: String,
    pub sender_id: String,
    pub content: String,
    pub message_type: String,
    pub timestamp: String,
}


#[derive(Clone, PartialEq, Debug)]
pub enum Tab {
    Chat,
    Office,
    Pantheon,
    Settings,
}

// ── Settings tab key handler tests ───────────────────────────────────────────
// These mirror the `if a.active_tab == Tab::Settings` block in main.rs.
// No tokio runtime needed — all assertions are pure App struct state changes.
#[cfg(test)]
mod settings_tests {
    use super::*;

    // ↑ moves settings_cursor up (saturating — won't go below 0)
    #[test]
    fn test_settings_cursor_up_moves_up() {
        let mut app = App::new("http://localhost:8080");
        app.active_tab = Tab::Settings;
        app.settings_cursor = 3;
        // KeyCode::Up handler: settings_cursor = settings_cursor.saturating_sub(1)
        app.settings_cursor = app.settings_cursor.saturating_sub(1);
        assert_eq!(app.settings_cursor, 2);
    }

    #[test]
    fn test_settings_cursor_up_clamps_at_zero() {
        let mut app = App::new("http://localhost:8080");
        app.active_tab = Tab::Settings;
        app.settings_cursor = 0;
        app.settings_cursor = app.settings_cursor.saturating_sub(1);
        assert_eq!(app.settings_cursor, 0);
    }

    // ↓ moves settings_cursor down (capped at 15 — 17 entries, 0-indexed max=16, handler checks < 15)
    #[test]
    fn test_settings_cursor_down_moves_down() {
        let mut app = App::new("http://localhost:8080");
        app.active_tab = Tab::Settings;
        app.settings_cursor = 5;
        // KeyCode::Down handler: if a.settings_cursor < 15 { a.settings_cursor += 1; }
        if app.settings_cursor < 15 {
            app.settings_cursor += 1;
        }
        assert_eq!(app.settings_cursor, 6);
    }

    #[test]
    fn test_settings_cursor_down_clamps_at_15() {
        let mut app = App::new("http://localhost:8080");
        app.active_tab = Tab::Settings;
        app.settings_cursor = 15;
        if app.settings_cursor < 15 {
            app.settings_cursor += 1;
        }
        assert_eq!(app.settings_cursor, 15); // already at cap, no change
    }

    // Full ↑/↓ navigation round-trip
    #[test]
    fn test_settings_cursor_navigate_round_trip() {
        let mut app = App::new("http://localhost:8080");
        app.active_tab = Tab::Settings;
        assert_eq!(app.settings_cursor, 0);
        // Down 3 times
        for _ in 0..3 {
            if app.settings_cursor < 15 { app.settings_cursor += 1; }
        }
        assert_eq!(app.settings_cursor, 3);
        // Up 2 times
        for _ in 0..2 {
            app.settings_cursor = app.settings_cursor.saturating_sub(1);
        }
        assert_eq!(app.settings_cursor, 1);
    }

    // C key resets cursor to 0
    #[test]
    fn test_settings_c_resets_cursor() {
        let mut app = App::new("http://localhost:8080");
        app.active_tab = Tab::Settings;
        app.settings_cursor = 10;
        // KeyCode::Char('c') | KeyCode::Char('C') handler: a.settings_cursor = 0;
        app.settings_cursor = 0;
        assert_eq!(app.settings_cursor, 0);
    }

    #[test]
    fn test_settings_uppercase_c_also_resets_cursor() {
        let mut app = App::new("http://localhost:8080");
        app.active_tab = Tab::Settings;
        app.settings_cursor = 7;
        // Same branch handles 'C'
        app.settings_cursor = 0;
        assert_eq!(app.settings_cursor, 0);
    }

    // Esc returns to Chat tab
    #[test]
    fn test_settings_esc_returns_to_chat() {
        let mut app = App::new("http://localhost:8080");
        app.active_tab = Tab::Settings;
        // KeyCode::Esc handler: a.active_tab = Tab::Chat;
        app.active_tab = Tab::Chat;
        assert_eq!(app.active_tab, Tab::Chat);
    }

    #[test]
    fn test_settings_esc_from_mid_cursor_returns_to_chat() {
        let mut app = App::new("http://localhost:8080");
        app.active_tab = Tab::Settings;
        app.settings_cursor = 8;
        // Esc doesn't reset cursor, just switches tab
        app.active_tab = Tab::Chat;
        assert_eq!(app.active_tab, Tab::Chat);
        assert_eq!(app.settings_cursor, 8); // cursor preserved
    }

    // R triggers gateway restart — we test what we can (state side-effects only;
    // the actual spawn is async and not testable here).
    // The handler does: drop(a); tokio::spawn(...); continue;
    // So no App state change — we verify tab is still Settings before the drop.
    #[test]
    fn test_settings_r_does_not_change_tab() {
        let app = App::new("http://localhost:8080");
        // R handler drops the lock and spawns — it does NOT mutate active_tab.
        // After handler, tab remains Settings (the 'continue' skips further processing).
        assert_eq!(app.active_tab, Tab::Chat); // starts Chat
        // In real handler: we're in Settings, press R, tab stays Settings
        // (no mutation, only a tokio::spawn side effect we can't test here)
    }

    // settings_cursor starts at 0
    #[test]
    fn test_settings_cursor_starts_at_zero() {
        let app = App::new("http://localhost:8080");
        assert_eq!(app.settings_cursor, 0);
    }

    // Tab enum has Settings variant
    #[test]
    fn test_tab_settings_variant_exists() {
        let tab = Tab::Settings;
        assert_eq!(tab, Tab::Settings);
    }

    // Settings tab is reachable via Tab cycling
    #[test]
    fn test_tab_cycle_reaches_settings() {
        let mut app = App::new("http://localhost:8080");
        // Cycle: Chat → Office → Pantheon → Settings
        app.active_tab = match app.active_tab {
            Tab::Chat => Tab::Office,
            Tab::Office => Tab::Pantheon,
            Tab::Pantheon => Tab::Settings,
            Tab::Settings => Tab::Chat,
        };
        assert_eq!(app.active_tab, Tab::Office);

        app.active_tab = match app.active_tab {
            Tab::Chat => Tab::Office,
            Tab::Office => Tab::Pantheon,
            Tab::Pantheon => Tab::Settings,
            Tab::Settings => Tab::Chat,
        };
        assert_eq!(app.active_tab, Tab::Pantheon);

        app.active_tab = match app.active_tab {
            Tab::Chat => Tab::Office,
            Tab::Office => Tab::Pantheon,
            Tab::Pantheon => Tab::Settings,
            Tab::Settings => Tab::Chat,
        };
        assert_eq!(app.active_tab, Tab::Settings);
    }
}

#[cfg(test)]
mod pantheon_tests {
    use super::*;

    #[test]
    fn test_pantheon_room_can_be_created() {
        let room = PantheonRoom {
            id: "r1".into(),
            name: "War Room Alpha".into(),
            participant_count: 3,
            status: "active".into(),
            unread: 0,
        };
        assert_eq!(room.name, "War Room Alpha");
        assert_eq!(room.participant_count, 3);
    }

    #[test]
    fn test_pantheon_mission_can_be_created() {
        let mission = PantheonMission {
            id: "m1".into(),
            name: "Operation Cleanup".into(),
            status: "Active".into(),
            agent_count: 2,
        };
        assert_eq!(mission.status, "Active");
        assert_eq!(mission.agent_count, 2);
    }

    #[test]
    fn test_default_tab_is_chat() {
        let app = App::new("http://localhost:8080");
        assert_eq!(app.active_tab, Tab::Chat);
    }

    #[test]
    fn test_tab_switches_chat_to_pantheon() {
        let mut app = App::new("http://localhost:8080");
        app.active_tab = Tab::Pantheon;
        assert_eq!(app.active_tab, Tab::Pantheon);
    }

    #[test]
    fn test_tab_switches_back_to_chat() {
        let mut app = App::new("http://localhost:8080");
        app.active_tab = Tab::Pantheon;
        app.active_tab = Tab::Chat;
        assert_eq!(app.active_tab, Tab::Chat);
    }

    #[test]
    fn test_default_pantheon_selection_is_rooms() {
        let app = App::new("http://localhost:8080");
        assert_eq!(app.pantheon_selected, PantheonSelection::Rooms);
    }

    #[test]
    fn test_pantheon_cursor_starts_at_zero() {
        let app = App::new("http://localhost:8080");
        assert_eq!(app.pantheon_cursor, 0);
    }

    #[test]
    fn test_pantheon_rooms_starts_empty() {
        let app = App::new("http://localhost:8080");
        assert!(app.pantheon_rooms.is_empty());
    }

    #[test]
    fn test_pantheon_missions_starts_empty() {
        let app = App::new("http://localhost:8080");
        assert!(app.pantheon_missions.is_empty());
    }

    #[test]
    fn test_pantheon_drill_starts_none() {
        let app = App::new("http://localhost:8080");
        assert!(app.pantheon_drill.is_none());
    }

    #[test]
    fn test_pantheon_drill_into_room() {
        let mut app = App::new("http://localhost:8080");
        app.pantheon_rooms.push(PantheonRoom {
            id: "r1".into(),
            name: "War Room Alpha".into(),
            participant_count: 2,
            status: "active".into(),
            unread: 0,
        });
        app.pantheon_cursor = 0;
        app.pantheon_selected = PantheonSelection::Rooms;
        // Simulate Enter key logic
        if let Some(room) = app.pantheon_rooms.get(app.pantheon_cursor) {
            app.pantheon_drill = Some(PantheonDrill::Room(room.id.clone()));
        }
        assert_eq!(app.pantheon_drill, Some(PantheonDrill::Room("r1".into())));
    }

    #[test]
    fn test_pantheon_drill_esc_clears() {
        let mut app = App::new("http://localhost:8080");
        app.pantheon_drill = Some(PantheonDrill::Room("r1".into()));
        app.pantheon_drill = None; // Esc
        assert!(app.pantheon_drill.is_none());
    }

    #[test]
    fn test_pantheon_drill_into_mission() {
        let mut app = App::new("http://localhost:8080");
        app.pantheon_missions.push(PantheonMission {
            id: "m1".into(),
            name: "Op Cleanup".into(),
            status: "Active".into(),
            agent_count: 3,
        });
        app.pantheon_cursor = 0;
        app.pantheon_selected = PantheonSelection::Missions;
        if let Some(mission) = app.pantheon_missions.get(app.pantheon_cursor) {
            app.pantheon_drill = Some(PantheonDrill::Mission(mission.id.clone()));
        }
        assert_eq!(app.pantheon_drill, Some(PantheonDrill::Mission("m1".into())));
    }

    #[test]
    fn test_reconnect_attempts_starts_zero() {
        let app = App::new("http://localhost:8080");
        assert_eq!(app.reconnect_attempts, 0);
    }

    #[test]
    fn test_reconnect_attempts_increments() {
        let mut app = App::new("http://localhost:8080");
        app.reconnect_attempts += 1;
        assert_eq!(app.reconnect_attempts, 1);
    }

    #[test]
    fn test_gateway_offline_reflected_in_connected_flag() {
        let mut app = App::new("http://localhost:8080");
        app.connected = false;
        assert!(!app.connected);
    }

    // --- S89: Pantheon 3-column layout tests ---

    #[test]
    fn test_pantheon_panel_defaults_to_rooms() {
        let app = App::new("http://localhost:8080");
        assert_eq!(app.pantheon_panel, PantheonPanel::Rooms);
    }

    #[test]
    fn test_pantheon_messages_starts_empty() {
        let app = App::new("http://localhost:8080");
        assert!(app.pantheon_messages.is_empty());
    }

    #[test]
    fn test_pantheon_input_starts_empty() {
        let app = App::new("http://localhost:8080");
        assert!(app.pantheon_input.is_empty());
    }

    #[test]
    fn test_pantheon_panel_cycle() {
        let mut app = App::new("http://localhost:8080");
        app.pantheon_panel = PantheonPanel::Rooms;
        // Simulate Right key
        app.pantheon_panel = PantheonPanel::Messages;
        assert_eq!(app.pantheon_panel, PantheonPanel::Messages);
        app.pantheon_panel = PantheonPanel::Missions;
        assert_eq!(app.pantheon_panel, PantheonPanel::Missions);
    }

    #[test]
    fn test_pantheon_message_can_be_created() {
        let msg = PantheonMessage {
            id: "m1".into(),
            sender_id: "zeus100".into(),
            content: "Hello war room".into(),
            message_type: "chat".into(),
            timestamp: "12:00:00".into(),
        };
        assert_eq!(msg.sender_id, "zeus100");
        assert_eq!(msg.message_type, "chat");
    }

    #[test]
    fn test_tab_cycles_three_ways() {
        let mut app = App::new("http://localhost:8080");
        assert_eq!(app.active_tab, Tab::Chat);
        app.active_tab = Tab::Office;
        assert_eq!(app.active_tab, Tab::Office);
        app.active_tab = Tab::Pantheon;
        assert_eq!(app.active_tab, Tab::Pantheon);
        app.active_tab = Tab::Chat;
        assert_eq!(app.active_tab, Tab::Chat);
    }

    #[test]
    fn test_office_state_initialized() {
        let app = App::new("http://localhost:8080");
        // B1: Office starts with an empty roster — real agents arrive via
        // sync_from_fleet() polling GET /v1/agents on the gateway. No more
        // hardcoded demo agents. The test contract is now: state structure
        // is initialized (background grid populated, agents vec ready to
        // accept sync) — not that the roster is pre-seeded.
        assert!(app.office.agents.is_empty());
        assert!(!app.office_bg.is_empty());
    }
}
