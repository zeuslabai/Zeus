//! Agent Director — UI Puppet Orchestrator for Agent Studio
//!
//! Converts a user's goal into a sequence of UI puppet actions by using the LLM
//! to plan and execute navigation, clicks, typing, and other UI interactions.
//! The frontend renders these actions in real-time — "Zeus is driving."

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast, mpsc};
use tracing::{debug, warn};
use zeus_core::Result;

// ── Puppet Protocol Types ───────────────────────────────────────────────────

/// A UI action the agent wants the frontend to execute
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum UiAction {
    /// Navigate to a page/route in the app
    Navigate {
        route: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// Click an element by CSS selector or label
    Click {
        target: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// Type text into an input element
    Type {
        target: String,
        value: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        clear_first: Option<bool>,
    },
    /// Scroll the viewport or an element
    Scroll {
        #[serde(skip_serializing_if = "Option::is_none")]
        target: Option<String>,
        direction: ScrollDirection,
        amount: u32,
    },
    /// Select an option from a dropdown/list
    Select { target: String, value: String },
    /// Wait for an element to appear or a fixed delay
    Wait {
        #[serde(skip_serializing_if = "Option::is_none")]
        target: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        delay_ms: Option<u64>,
    },
    /// Highlight an element to show the user what the agent is looking at
    Highlight {
        target: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        color: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
    },
    /// Clear all highlights
    ClearHighlight,
    /// Assert that an element exists / has text (verification step)
    Assert {
        target: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        expected_text: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

/// Result sent back from the frontend after executing a UiAction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiActionResult {
    pub sequence: u32,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_state: Option<PageState>,
    pub timestamp: DateTime<Utc>,
}

/// Snapshot of the current page state reported by the frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageState {
    pub route: String,
    pub title: String,
    #[serde(default)]
    pub visible_elements: Vec<String>,
    #[serde(default)]
    pub form_values: HashMap<String, String>,
}

// ── Puppet Command (what goes over the WebSocket) ───────────────────────────

/// Message sent from backend to frontend over the puppet WebSocket
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PuppetCommand {
    /// Execute a UI action
    Action {
        sequence: u32,
        action: UiAction,
        description: String,
    },
    /// Agent is thinking / planning next step
    Thinking { message: String },
    /// Session status changed
    StatusChange { status: String, reason: String },
    /// Session complete
    Complete {
        summary: String,
        actions_executed: u32,
        actions_failed: u32,
    },
    /// Error
    Error { message: String },
}

/// Message sent from frontend to backend over the puppet WebSocket
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PuppetResponse {
    /// Result of executing a UiAction
    ActionResult(UiActionResult),
    /// User clicked "pause"
    Pause,
    /// User clicked "resume"
    Resume,
    /// User took manual control / intervened
    Intervene { message: String },
    /// Heartbeat / keep-alive
    Ping,
}

// ── Page Map (what the agent knows about the app) ───────────────────────────

/// Map of routes → interactive elements, so the LLM knows what it can do
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PageMap {
    pub routes: HashMap<String, PageInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageInfo {
    pub title: String,
    pub description: String,
    pub elements: Vec<ElementInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementInfo {
    pub selector: String,
    pub kind: String, // "button", "input", "link", "select", "textarea"
    pub label: String,
}

impl PageMap {
    /// Build the Zeus app page map with known routes and interactive elements
    pub fn zeus_default() -> Self {
        let mut routes = HashMap::new();

        routes.insert(
            "/dashboard".into(),
            PageInfo {
                title: "Dashboard".into(),
                description: "Main dashboard with system overview, agent status, and quick actions"
                    .into(),
                elements: vec![
                    ElementInfo {
                        selector: "button.new-session".into(),
                        kind: "button".into(),
                        label: "New Session".into(),
                    },
                    ElementInfo {
                        selector: "input.search".into(),
                        kind: "input".into(),
                        label: "Search".into(),
                    },
                ],
            },
        );

        routes.insert(
            "/chat".into(),
            PageInfo {
                title: "Chat".into(),
                description: "Main chat interface for conversations with Zeus".into(),
                elements: vec![
                    ElementInfo {
                        selector: "textarea.chat-input".into(),
                        kind: "textarea".into(),
                        label: "Message input".into(),
                    },
                    ElementInfo {
                        selector: "button.send".into(),
                        kind: "button".into(),
                        label: "Send message".into(),
                    },
                    ElementInfo {
                        selector: "button.new-session".into(),
                        kind: "button".into(),
                        label: "New session".into(),
                    },
                ],
            },
        );

        routes.insert(
            "/agents".into(),
            PageInfo {
                title: "Agents".into(),
                description: "Agent management — create, configure, and monitor agents".into(),
                elements: vec![
                    ElementInfo {
                        selector: "button.create-agent".into(),
                        kind: "button".into(),
                        label: "Create Agent".into(),
                    },
                    ElementInfo {
                        selector: "input.agent-name".into(),
                        kind: "input".into(),
                        label: "Agent name".into(),
                    },
                    ElementInfo {
                        selector: "select.agent-model".into(),
                        kind: "select".into(),
                        label: "Model selector".into(),
                    },
                ],
            },
        );

        routes.insert(
            "/tools".into(),
            PageInfo {
                title: "Tools".into(),
                description: "Browse and execute available tools".into(),
                elements: vec![ElementInfo {
                    selector: "input.tool-search".into(),
                    kind: "input".into(),
                    label: "Search tools".into(),
                }],
            },
        );

        routes.insert(
            "/settings".into(),
            PageInfo {
                title: "Settings".into(),
                description: "Configuration settings for model, workspace, channels".into(),
                elements: vec![
                    ElementInfo {
                        selector: "input.model-string".into(),
                        kind: "input".into(),
                        label: "Model string".into(),
                    },
                    ElementInfo {
                        selector: "button.save-config".into(),
                        kind: "button".into(),
                        label: "Save configuration".into(),
                    },
                ],
            },
        );

        routes.insert(
            "/deploy".into(),
            PageInfo {
                title: "Deploy".into(),
                description: "One-click deploy targets, history, and pipeline status".into(),
                elements: vec![
                    ElementInfo {
                        selector: "button.new-target".into(),
                        kind: "button".into(),
                        label: "New Deploy Target".into(),
                    },
                    ElementInfo {
                        selector: "button.deploy-now".into(),
                        kind: "button".into(),
                        label: "Deploy Now".into(),
                    },
                ],
            },
        );

        routes.insert(
            "/channels".into(),
            PageInfo {
                title: "Channels".into(),
                description: "Messaging channel configuration — Telegram, Discord, Slack, etc."
                    .into(),
                elements: vec![ElementInfo {
                    selector: "button.add-channel".into(),
                    kind: "button".into(),
                    label: "Add Channel".into(),
                }],
            },
        );

        routes.insert(
            "/skills".into(),
            PageInfo {
                title: "Skills".into(),
                description: "Installed skills and marketplace".into(),
                elements: vec![
                    ElementInfo {
                        selector: "button.install-skill".into(),
                        kind: "button".into(),
                        label: "Install Skill".into(),
                    },
                    ElementInfo {
                        selector: "input.skill-search".into(),
                        kind: "input".into(),
                        label: "Search skills".into(),
                    },
                ],
            },
        );

        routes.insert(
            "/memory".into(),
            PageInfo {
                title: "Memory".into(),
                description: "Workspace memory files and knowledge base".into(),
                elements: vec![
                    ElementInfo {
                        selector: "button.add-memory".into(),
                        kind: "button".into(),
                        label: "Add Memory".into(),
                    },
                    ElementInfo {
                        selector: "input.memory-search".into(),
                        kind: "input".into(),
                        label: "Search memory".into(),
                    },
                ],
            },
        );

        routes.insert(
            "/pantheon".into(),
            PageInfo {
                title: "Pantheon War Room".into(),
                description: "Multi-agent collaboration, missions, and team coordination".into(),
                elements: vec![
                    ElementInfo {
                        selector: "button.new-mission".into(),
                        kind: "button".into(),
                        label: "New Mission".into(),
                    },
                    ElementInfo {
                        selector: "textarea.room-input".into(),
                        kind: "textarea".into(),
                        label: "Room message input".into(),
                    },
                    ElementInfo {
                        selector: "button.send-room".into(),
                        kind: "button".into(),
                        label: "Send to room".into(),
                    },
                ],
            },
        );

        Self { routes }
    }

    /// Format the page map as context for the LLM system prompt
    pub fn to_prompt_context(&self) -> String {
        let mut out = String::from("## Available Pages & Elements\n\n");
        let mut sorted: Vec<_> = self.routes.iter().collect();
        sorted.sort_by_key(|(k, _)| (*k).clone());
        for (route, info) in sorted {
            out.push_str(&format!("### {} (`{}`)\n", info.title, route));
            out.push_str(&format!("{}\n", info.description));
            if !info.elements.is_empty() {
                out.push_str("Elements:\n");
                for el in &info.elements {
                    out.push_str(&format!(
                        "- `{}` ({}) — {}\n",
                        el.selector, el.kind, el.label
                    ));
                }
            }
            out.push('\n');
        }
        out
    }
}

// ── Director Session ────────────────────────────────────────────────────────

/// Live state for an active director session
#[derive(Debug)]
pub struct DirectorSession {
    pub session_id: String,
    pub goal: String,
    pub status: DirectorStatus,
    pub sequence_counter: u32,
    pub actions_completed: u32,
    pub actions_failed: u32,
    pub created_at: DateTime<Utc>,
    pub page_map: PageMap,
    /// Channel to send puppet commands to the WebSocket handler
    pub command_tx: broadcast::Sender<PuppetCommand>,
    /// Channel to receive results from the frontend
    pub result_tx: mpsc::Sender<PuppetResponse>,
    pub result_rx: Option<mpsc::Receiver<PuppetResponse>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectorStatus {
    Planning,
    Driving,
    Paused,
    WaitingForResult,
    Complete,
    Failed,
}

// ── Agent Director ──────────────────────────────────────────────────────────

/// The AgentDirector orchestrates UI puppet sessions.
///
/// It manages active sessions and provides the interface between
/// the LLM planning loop and the WebSocket puppet channel.
pub struct AgentDirector {
    sessions: Arc<RwLock<HashMap<String, Arc<RwLock<DirectorSession>>>>>,
}

impl AgentDirector {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Start a new director session. Returns (session_id, command_rx, result_tx)
    /// for the WebSocket handler to use.
    pub async fn start_session(
        &self,
        session_id: String,
        goal: String,
    ) -> (
        broadcast::Receiver<PuppetCommand>,
        mpsc::Sender<PuppetResponse>,
    ) {
        let (command_tx, command_rx) = broadcast::channel::<PuppetCommand>(256);
        let (result_tx, result_rx) = mpsc::channel::<PuppetResponse>(64);

        let session = DirectorSession {
            session_id: session_id.clone(),
            goal,
            status: DirectorStatus::Planning,
            sequence_counter: 0,
            actions_completed: 0,
            actions_failed: 0,
            created_at: Utc::now(),
            page_map: PageMap::zeus_default(),
            command_tx: command_tx.clone(),
            result_tx: result_tx.clone(),
            result_rx: Some(result_rx),
        };

        self.sessions
            .write()
            .await
            .insert(session_id, Arc::new(RwLock::new(session)));

        (command_rx, result_tx)
    }

    /// Take the result receiver for a session (can only be taken once).
    /// The driving loop uses this to receive frontend results.
    pub async fn take_result_rx(&self, session_id: &str) -> Option<mpsc::Receiver<PuppetResponse>> {
        let sessions = self.sessions.read().await;
        if let Some(session_arc) = sessions.get(session_id) {
            let mut session = session_arc.write().await;
            session.result_rx.take()
        } else {
            None
        }
    }

    /// Send a puppet command to the frontend for a given session
    pub async fn send_command(&self, session_id: &str, command: PuppetCommand) -> Result<()> {
        let sessions = self.sessions.read().await;
        if let Some(session_arc) = sessions.get(session_id) {
            let session = session_arc.read().await;
            session
                .command_tx
                .send(command)
                .map_err(|e| zeus_core::Error::Agent(format!("Send command failed: {}", e)))?;
            Ok(())
        } else {
            Err(zeus_core::Error::Agent(format!(
                "Session {} not found",
                session_id
            )))
        }
    }

    /// Dispatch a single UI action and wait for the result
    pub async fn dispatch_action(
        &self,
        session_id: &str,
        action: UiAction,
        description: &str,
        result_rx: &mut mpsc::Receiver<PuppetResponse>,
    ) -> Result<UiActionResult> {
        let sequence = {
            let sessions = self.sessions.read().await;
            let session_arc = sessions.get(session_id).ok_or_else(|| {
                zeus_core::Error::Agent(format!("Session {} not found", session_id))
            })?;
            let mut session = session_arc.write().await;
            session.sequence_counter += 1;
            session.status = DirectorStatus::WaitingForResult;
            session.sequence_counter
        };

        // Send the action to the frontend
        self.send_command(
            session_id,
            PuppetCommand::Action {
                sequence,
                action,
                description: description.to_string(),
            },
        )
        .await?;

        // Wait for the result (with timeout)
        let timeout = tokio::time::Duration::from_secs(30);
        match tokio::time::timeout(timeout, Self::wait_for_result(result_rx)).await {
            Ok(Some(result)) => {
                // Update counters
                let sessions = self.sessions.read().await;
                if let Some(session_arc) = sessions.get(session_id) {
                    let mut session = session_arc.write().await;
                    if result.success {
                        session.actions_completed += 1;
                    } else {
                        session.actions_failed += 1;
                    }
                    session.status = DirectorStatus::Driving;
                }
                Ok(result)
            }
            Ok(None) => Err(zeus_core::Error::Agent("Frontend disconnected".to_string())),
            Err(_) => Err(zeus_core::Error::Agent(
                "Action timed out after 30s".to_string(),
            )),
        }
    }

    /// Wait for the next ActionResult, handling pause/resume/intervene
    async fn wait_for_result(
        result_rx: &mut mpsc::Receiver<PuppetResponse>,
    ) -> Option<UiActionResult> {
        loop {
            match result_rx.recv().await {
                Some(PuppetResponse::ActionResult(result)) => return Some(result),
                Some(PuppetResponse::Ping) => continue, // ignore keepalive
                Some(PuppetResponse::Pause) => {
                    debug!("Director paused by user, waiting for resume...");
                    // Wait for resume
                    loop {
                        match result_rx.recv().await {
                            Some(PuppetResponse::Resume) => {
                                debug!("Director resumed");
                                break;
                            }
                            Some(PuppetResponse::Intervene { message }) => {
                                warn!("User intervened during pause: {}", message);
                                return None;
                            }
                            None => return None,
                            _ => continue,
                        }
                    }
                }
                Some(PuppetResponse::Intervene { message }) => {
                    warn!("User intervened: {}", message);
                    return None;
                }
                Some(PuppetResponse::Resume) => continue, // already driving
                None => return None,
            }
        }
    }

    /// Mark session as complete
    pub async fn complete_session(&self, session_id: &str, summary: &str) {
        let sessions = self.sessions.read().await;
        if let Some(session_arc) = sessions.get(session_id) {
            let mut session = session_arc.write().await;
            session.status = DirectorStatus::Complete;
            let _ = session.command_tx.send(PuppetCommand::Complete {
                summary: summary.to_string(),
                actions_executed: session.actions_completed,
                actions_failed: session.actions_failed,
            });
        }
    }

    /// Mark session as failed
    pub async fn fail_session(&self, session_id: &str, error: &str) {
        let sessions = self.sessions.read().await;
        if let Some(session_arc) = sessions.get(session_id) {
            let mut session = session_arc.write().await;
            session.status = DirectorStatus::Failed;
            let _ = session.command_tx.send(PuppetCommand::Error {
                message: error.to_string(),
            });
        }
    }

    /// Clean up a finished session
    pub async fn remove_session(&self, session_id: &str) {
        self.sessions.write().await.remove(session_id);
    }

    /// Get session status
    pub async fn get_status(&self, session_id: &str) -> Option<DirectorStatus> {
        let sessions = self.sessions.read().await;
        if let Some(session_arc) = sessions.get(session_id) {
            Some(session_arc.read().await.status.clone())
        } else {
            None
        }
    }

    /// Get the page map prompt context for a session
    pub async fn get_page_map_context(&self, session_id: &str) -> Option<String> {
        let sessions = self.sessions.read().await;
        if let Some(session_arc) = sessions.get(session_id) {
            Some(session_arc.read().await.page_map.to_prompt_context())
        } else {
            None
        }
    }

    /// List active session IDs
    pub async fn active_sessions(&self) -> Vec<String> {
        self.sessions.read().await.keys().cloned().collect()
    }

    /// Pause a session
    pub async fn pause_session(&self, session_id: &str) -> bool {
        let sessions = self.sessions.read().await;
        if let Some(session_arc) = sessions.get(session_id) {
            let mut session = session_arc.write().await;
            if session.status == DirectorStatus::Driving
                || session.status == DirectorStatus::WaitingForResult
            {
                session.status = DirectorStatus::Paused;
                let _ = session.command_tx.send(PuppetCommand::StatusChange {
                    status: "paused".to_string(),
                    reason: "User paused the session".to_string(),
                });
                return true;
            }
        }
        false
    }

    /// Resume a paused session
    pub async fn resume_session(&self, session_id: &str) -> bool {
        let sessions = self.sessions.read().await;
        if let Some(session_arc) = sessions.get(session_id) {
            let mut session = session_arc.write().await;
            if session.status == DirectorStatus::Paused {
                session.status = DirectorStatus::Driving;
                let _ = session.command_tx.send(PuppetCommand::StatusChange {
                    status: "driving".to_string(),
                    reason: "Session resumed".to_string(),
                });
                return true;
            }
        }
        false
    }
}

impl Default for AgentDirector {
    fn default() -> Self {
        Self::new()
    }
}

/// Build the system prompt for the director LLM that plans UI actions.
///
/// The LLM receives the page map and responds with JSON arrays of UiAction.
pub fn build_director_prompt(page_map: &PageMap) -> String {
    format!(
        concat!(
            "You are Zeus AgentDirector — a UI automation pilot.\n",
            "The user gives you a goal. You drive the Zeus web app to accomplish it.\n\n",
            "You control the UI by emitting JSON action objects. Each action is executed\n",
            "by the frontend in sequence. After each action, you receive the result.\n\n",
            "{}\n",
            "## Action Format\n\n",
            "Respond ONLY with a JSON array of actions to execute next. Each action:\n",
            "```json\n",
            "[\n",
            "  {{\"action\": \"navigate\", \"route\": \"/agents\", \"label\": \"Go to agents page\"}},\n",
            "  {{\"action\": \"click\", \"target\": \"button.create-agent\", \"label\": \"Open create dialog\"}},\n",
            "  {{\"action\": \"type\", \"target\": \"input.agent-name\", \"value\": \"Research Bot\"}},\n",
            "  {{\"action\": \"highlight\", \"target\": \"button.save\", \"color\": \"#00ff88\"}},\n",
            "  {{\"action\": \"click\", \"target\": \"button.save\", \"label\": \"Save agent\"}},\n",
            "  {{\"action\": \"wait\", \"delay_ms\": 500}}\n",
            "]\n",
            "```\n\n",
            "## Action Types\n\n",
            "- `navigate` — Go to a route: {{\"action\": \"navigate\", \"route\": \"/path\"}}\n",
            "- `click` — Click element: {{\"action\": \"click\", \"target\": \"selector\"}}\n",
            "- `type` — Type text: {{\"action\": \"type\", \"target\": \"selector\", \"value\": \"text\"}}\n",
            "- `scroll` — Scroll: {{\"action\": \"scroll\", \"direction\": \"down\", \"amount\": 300}}\n",
            "- `select` — Select option: {{\"action\": \"select\", \"target\": \"selector\", \"value\": \"opt\"}}\n",
            "- `wait` — Wait: {{\"action\": \"wait\", \"delay_ms\": 1000}} or {{\"action\": \"wait\", \"target\": \"selector\"}}\n",
            "- `highlight` — Highlight: {{\"action\": \"highlight\", \"target\": \"selector\", \"color\": \"#hex\"}}\n",
            "- `clear_highlight` — Remove highlights: {{\"action\": \"clear_highlight\"}}\n",
            "- `assert` — Verify: {{\"action\": \"assert\", \"target\": \"selector\", \"expected_text\": \"...\"}}\n\n",
            "## Rules\n\n",
            "1. ALWAYS start by navigating to the correct page\n",
            "2. Use highlight before clicking important elements (so the user sees what you're doing)\n",
            "3. Add wait actions after form submissions (500-1000ms)\n",
            "4. If an action fails, try an alternative approach\n",
            "5. When the goal is complete, respond with an empty array `[]`\n",
            "6. Keep action batches small (3-5 actions) so the user can follow along\n",
            "7. Add descriptive labels for user understanding\n",
        ),
        page_map.to_prompt_context()
    )
}

/// Parse the LLM response into a list of UiActions
pub fn parse_action_plan(response: &str) -> Vec<UiAction> {
    // Try to extract JSON array from the response
    let trimmed = response.trim();

    // Direct JSON array
    if trimmed.starts_with('[')
        && let Ok(actions) = serde_json::from_str::<Vec<UiAction>>(trimmed)
    {
        return actions;
    }

    // Extract from markdown code block
    if let Some(start) = trimmed.find("```json") {
        let after = &trimmed[start + 7..];
        if let Some(end) = after.find("```") {
            let json_str = after[..end].trim();
            if let Ok(actions) = serde_json::from_str::<Vec<UiAction>>(json_str) {
                return actions;
            }
        }
    }

    // Extract from bare code block
    if let Some(start) = trimmed.find("```\n") {
        let after = &trimmed[start + 4..];
        if let Some(end) = after.find("```") {
            let json_str = after[..end].trim();
            if let Ok(actions) = serde_json::from_str::<Vec<UiAction>>(json_str) {
                return actions;
            }
        }
    }

    // Try to find first [ and last ]
    if let (Some(start), Some(end)) = (trimmed.find('['), trimmed.rfind(']'))
        && start < end
    {
        let json_str = &trimmed[start..=end];
        if let Ok(actions) = serde_json::from_str::<Vec<UiAction>>(json_str) {
            return actions;
        }
    }

    warn!("Failed to parse action plan from LLM response");
    vec![]
}

// ── Driving Loop — Phase 8 Agent Studio v2 ──────────────────────────────────

/// Configuration for the autonomous driving loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrivingConfig {
    /// Maximum number of LLM → action → result iterations.
    pub max_iterations: u32,
    /// Maximum total actions across all iterations.
    pub max_total_actions: u32,
    /// Delay between action batches (ms) for visual pacing.
    pub batch_delay_ms: u64,
    /// Maximum consecutive failures before aborting.
    pub max_consecutive_failures: u32,
    /// Maximum re-plan attempts after failure.
    pub max_replans: u32,
}

impl Default for DrivingConfig {
    fn default() -> Self {
        Self {
            max_iterations: 30,
            max_total_actions: 100,
            batch_delay_ms: 300,
            max_consecutive_failures: 5,
            max_replans: 3,
        }
    }
}

/// Result of a completed driving loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrivingResult {
    pub session_id: String,
    pub success: bool,
    pub iterations: u32,
    pub actions_executed: u32,
    pub actions_failed: u32,
    pub summary: String,
    pub duration_ms: u64,
}

/// The autonomous driving loop.
///
/// Takes a user goal, calls the LLM to generate action batches,
/// dispatches each action to the frontend via the puppet WebSocket,
/// collects results, and re-plans on failure. Repeats until the
/// goal is complete (LLM returns empty actions) or limits are hit.
pub struct DrivingLoop {
    pub config: DrivingConfig,
}

impl DrivingLoop {
    pub fn new(config: DrivingConfig) -> Self {
        Self { config }
    }

    /// Run the autonomous driving loop for a session.
    ///
    /// Requires:
    /// - `director`: the AgentDirector managing the session
    /// - `llm`: LLM client for planning
    /// - `session_id`: which session to drive
    /// - `goal`: the user's stated goal
    /// - `result_rx`: the mpsc receiver for frontend results
    ///
    /// Returns a `DrivingResult` summarizing the loop outcome.
    pub async fn run(
        &self,
        director: &AgentDirector,
        llm: &zeus_llm::LlmClient,
        session_id: &str,
        goal: &str,
        result_rx: &mut mpsc::Receiver<PuppetResponse>,
    ) -> DrivingResult {
        let started = Utc::now();
        let mut total_actions: u32 = 0;
        let mut total_failed: u32 = 0;
        let mut iterations: u32 = 0;
        let mut consecutive_failures: u32 = 0;
        let mut replan_count: u32 = 0;
        let mut history: Vec<String> = Vec::new();

        // Get page map context for the LLM
        let page_context = director
            .get_page_map_context(session_id)
            .await
            .unwrap_or_default();
        let system_prompt = build_director_prompt(
            &director
                .sessions
                .read()
                .await
                .get(session_id)
                .map(|s| futures::executor::block_on(s.read()).page_map.clone())
                .unwrap_or_else(PageMap::zeus_default),
        );

        // Notify frontend: we're planning
        let _ = director
            .send_command(
                session_id,
                PuppetCommand::Thinking {
                    message: format!("Planning how to: {}", goal),
                },
            )
            .await;

        loop {
            iterations += 1;

            // Safety limits
            if iterations > self.config.max_iterations {
                let msg = format!(
                    "Reached max iterations ({}). Stopping.",
                    self.config.max_iterations
                );
                tracing::warn!("{}", msg);
                director.fail_session(session_id, &msg).await;
                return self.make_result(
                    session_id,
                    false,
                    iterations,
                    total_actions,
                    total_failed,
                    msg,
                    started,
                );
            }

            if total_actions >= self.config.max_total_actions {
                let msg = format!(
                    "Reached max total actions ({}). Stopping.",
                    self.config.max_total_actions
                );
                director.fail_session(session_id, &msg).await;
                return self.make_result(
                    session_id,
                    false,
                    iterations,
                    total_actions,
                    total_failed,
                    msg,
                    started,
                );
            }

            if consecutive_failures >= self.config.max_consecutive_failures {
                let msg = format!(
                    "Too many consecutive failures ({}). Stopping.",
                    consecutive_failures
                );
                director.fail_session(session_id, &msg).await;
                return self.make_result(
                    session_id,
                    false,
                    iterations,
                    total_actions,
                    total_failed,
                    msg,
                    started,
                );
            }

            // Build messages for LLM
            let messages = vec![zeus_core::Message::user(format!(
                "Goal: {}\n\n{}{}",
                goal,
                if history.is_empty() {
                    "This is the first step. Plan the initial actions to accomplish the goal.\n"
                        .to_string()
                } else {
                    format!(
                        "Previous results:\n{}\n\nPlan the next actions based on these results.\n",
                        history.join("\n")
                    )
                },
                page_context,
            ))];

            // Call LLM to plan actions
            let _ = director
                .send_command(
                    session_id,
                    PuppetCommand::Thinking {
                        message: format!("Planning step {}...", iterations),
                    },
                )
                .await;

            let llm_response = match llm.complete(&messages, &[], Some(&system_prompt)).await {
                Ok(resp) => resp,
                Err(e) => {
                    let msg = format!("LLM call failed: {}", e);
                    tracing::error!("{}", msg);
                    replan_count += 1;
                    if replan_count > self.config.max_replans {
                        director.fail_session(session_id, &msg).await;
                        return self.make_result(
                            session_id,
                            false,
                            iterations,
                            total_actions,
                            total_failed,
                            msg,
                            started,
                        );
                    }
                    history.push(format!("- LLM error: {} (retrying)", e));
                    continue;
                }
            };

            // Parse the LLM response into UI actions
            let actions = parse_action_plan(&llm_response.content);

            // Empty actions = goal complete
            if actions.is_empty() {
                let summary = format!(
                    "Goal complete after {} iterations, {} actions ({} failed)",
                    iterations, total_actions, total_failed
                );
                director.complete_session(session_id, &summary).await;
                return self.make_result(
                    session_id,
                    true,
                    iterations,
                    total_actions,
                    total_failed,
                    summary,
                    started,
                );
            }

            // Update status to driving
            let _ = director
                .send_command(
                    session_id,
                    PuppetCommand::StatusChange {
                        status: "driving".to_string(),
                        reason: format!("Executing {} actions", actions.len()),
                    },
                )
                .await;

            // Dispatch each action and collect results
            let mut batch_results = Vec::new();
            // Get the page map for route validation
            let page_map = director
                .sessions
                .read()
                .await
                .get(session_id)
                .map(|s| futures::executor::block_on(s.read()).page_map.clone())
                .unwrap_or_else(PageMap::zeus_default);
            for action in actions {
                // Sanitize action before dispatch (Zeus107 items A-C)
                if let Err(e) = sanitize_action(&action, &page_map) {
                    warn!("Action rejected by sanitizer: {}", e);
                    total_actions += 1;
                    total_failed += 1;
                    consecutive_failures += 1;
                    batch_results.push(format!("- ✗ REJECTED: {}", e));
                    continue;
                }

                let description = action_description(&action);

                // Pacing delay
                if self.config.batch_delay_ms > 0 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(
                        self.config.batch_delay_ms,
                    ))
                    .await;
                }

                match director
                    .dispatch_action(session_id, action.clone(), &description, result_rx)
                    .await
                {
                    Ok(result) => {
                        total_actions += 1;
                        if result.success {
                            consecutive_failures = 0;
                            batch_results.push(format!("- ✓ {}: success", description));
                        } else {
                            total_failed += 1;
                            consecutive_failures += 1;
                            let err = result.error.as_deref().unwrap_or("unknown");
                            batch_results.push(format!("- ✗ {}: FAILED ({})", description, err));
                        }
                    }
                    Err(e) => {
                        total_actions += 1;
                        total_failed += 1;
                        consecutive_failures += 1;
                        batch_results.push(format!("- ✗ {}: ERROR ({})", description, e));
                        // Check if user intervened
                        if e.to_string().contains("disconnected")
                            || e.to_string().contains("intervened")
                        {
                            let msg = "User intervened or disconnected";
                            director.fail_session(session_id, msg).await;
                            return self.make_result(
                                session_id,
                                false,
                                iterations,
                                total_actions,
                                total_failed,
                                msg.to_string(),
                                started,
                            );
                        }
                    }
                }
            }

            // Add batch results to history for next LLM call
            history.push(format!(
                "Step {} results:\n{}",
                iterations,
                batch_results.join("\n")
            ));

            // If all actions in this batch failed, count as a replan
            if batch_results
                .iter()
                .all(|r| r.contains("FAILED") || r.contains("ERROR"))
            {
                replan_count += 1;
                if replan_count > self.config.max_replans {
                    let msg = format!(
                        "Max re-plans ({}) exceeded. All actions in last batch failed.",
                        self.config.max_replans
                    );
                    director.fail_session(session_id, &msg).await;
                    return self.make_result(
                        session_id,
                        false,
                        iterations,
                        total_actions,
                        total_failed,
                        msg,
                        started,
                    );
                }
                let _ = director
                    .send_command(
                        session_id,
                        PuppetCommand::Thinking {
                            message: format!(
                                "All actions failed. Re-planning (attempt {}/{})...",
                                replan_count, self.config.max_replans
                            ),
                        },
                    )
                    .await;
            } else {
                // Reset replan counter on partial success
                replan_count = 0;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn make_result(
        &self,
        session_id: &str,
        success: bool,
        iterations: u32,
        actions_executed: u32,
        actions_failed: u32,
        summary: String,
        started: DateTime<Utc>,
    ) -> DrivingResult {
        let duration = (Utc::now() - started).num_milliseconds().max(0) as u64;
        DrivingResult {
            session_id: session_id.to_string(),
            success,
            iterations,
            actions_executed,
            actions_failed,
            summary,
            duration_ms: duration,
        }
    }
}

impl Default for DrivingLoop {
    fn default() -> Self {
        Self::new(DrivingConfig::default())
    }
}

/// Generate a human-readable description for an action
fn action_description(action: &UiAction) -> String {
    match action {
        UiAction::Navigate { route, label } => label
            .clone()
            .unwrap_or_else(|| format!("Navigate to {}", route)),
        UiAction::Click { target, label } => {
            label.clone().unwrap_or_else(|| format!("Click {}", target))
        }
        UiAction::Type { target, value, .. } => {
            format!("Type '{}' into {}", value, target)
        }
        UiAction::Scroll {
            direction, amount, ..
        } => {
            format!("Scroll {:?} by {}", direction, amount)
        }
        UiAction::Select { target, value } => {
            format!("Select '{}' in {}", value, target)
        }
        UiAction::Wait { target, delay_ms } => {
            if let Some(t) = target {
                format!("Wait for {}", t)
            } else {
                format!("Wait {}ms", delay_ms.unwrap_or(500))
            }
        }
        UiAction::Highlight { target, .. } => {
            format!("Highlight {}", target)
        }
        UiAction::ClearHighlight => "Clear highlights".to_string(),
        UiAction::Assert {
            target,
            expected_text,
        } => {
            if let Some(text) = expected_text {
                format!("Assert {} contains '{}'", target, text)
            } else {
                format!("Assert {} exists", target)
            }
        }
    }
}

// ── Action Sanitization (Zeus107 security items A-C) ────────────────────────

/// Maximum length for a type value field (10KB — item C)
const MAX_TYPE_VALUE_LEN: usize = 10_240;

/// Validate and sanitize a UI action before dispatch.
///
/// - **A**: CSS selector validation — alphanumeric + `.#-_:[]^$*~|="' `
/// - **B**: Route validation — must start with `/` and contain only safe chars
/// - **C**: Value length cap — `Type` values capped at 10KB
fn sanitize_action(action: &UiAction, page_map: &PageMap) -> std::result::Result<(), String> {
    match action {
        UiAction::Navigate { route, .. } => {
            // Item B: validate route against known PageMap routes
            if !page_map.routes.contains_key(route.as_str()) {
                // Allow routes that start with a known prefix (e.g., /agents/123)
                let known = page_map
                    .routes
                    .keys()
                    .any(|k| route.starts_with(k.as_str()));
                if !known && !route.starts_with('/') {
                    return Err(format!("Unknown route: {}", route));
                }
            }
            // Route must be a valid path — no protocol or domain
            if route.contains("://") || route.contains("..") {
                return Err(format!("Invalid route: {}", route));
            }
        }
        UiAction::Click { target, .. }
        | UiAction::Highlight { target, .. }
        | UiAction::Assert { target, .. } => {
            // Item A: validate CSS selector
            if !is_safe_selector(target) {
                return Err(format!("Invalid CSS selector: {}", target));
            }
        }
        UiAction::Type { target, value, .. } => {
            if !is_safe_selector(target) {
                return Err(format!("Invalid CSS selector: {}", target));
            }
            // Item C: cap value length
            if value.len() > MAX_TYPE_VALUE_LEN {
                return Err(format!(
                    "Type value too long ({} bytes, max {})",
                    value.len(),
                    MAX_TYPE_VALUE_LEN
                ));
            }
        }
        UiAction::Select { target, value } => {
            if !is_safe_selector(target) {
                return Err(format!("Invalid CSS selector: {}", target));
            }
            if value.len() > MAX_TYPE_VALUE_LEN {
                return Err(format!("Select value too long ({} bytes)", value.len()));
            }
        }
        UiAction::Scroll { target, .. } => {
            if let Some(t) = target
                && !is_safe_selector(t)
            {
                return Err(format!("Invalid CSS selector: {}", t));
            }
        }
        UiAction::Wait { target, .. } => {
            if let Some(t) = target
                && !is_safe_selector(t)
            {
                return Err(format!("Invalid CSS selector: {}", t));
            }
        }
        UiAction::ClearHighlight => {}
    }
    Ok(())
}

/// Check if a CSS selector string is safe (no script injection).
/// Allows: alphanumeric, `.`, `#`, `-`, `_`, `:`, `[`, `]`, `^`, `$`,
/// `*`, `~`, `|`, `=`, `"`, `'`, ` `, `(`, `)`, `>`, `+`, `,`
fn is_safe_selector(s: &str) -> bool {
    if s.is_empty() || s.len() > 500 {
        return false;
    }
    s.chars().all(|c| {
        c.is_alphanumeric()
            || matches!(
                c,
                '.' | '#'
                    | '-'
                    | '_'
                    | ':'
                    | '['
                    | ']'
                    | '^'
                    | '$'
                    | '*'
                    | '~'
                    | '|'
                    | '='
                    | '"'
                    | '\''
                    | ' '
                    | '('
                    | ')'
                    | '>'
                    | '+'
                    | ','
            )
    })
}

// ── Enhanced PageMap — Phase 5-7 routes ─────────────────────────────────────

impl PageMap {
    /// Extend the default page map with Phase 5-7 routes
    pub fn zeus_full() -> Self {
        let mut map = Self::zeus_default();

        map.routes.insert(
            "/studio".into(),
            PageInfo {
                title: "Agent Studio".into(),
                description:
                    "AI-driven workspace — describe a goal, Zeus drives the UI to accomplish it"
                        .into(),
                elements: vec![
                    ElementInfo {
                        selector: "textarea.studio-input".into(),
                        kind: "textarea".into(),
                        label: "Goal input".into(),
                    },
                    ElementInfo {
                        selector: "button.start-session".into(),
                        kind: "button".into(),
                        label: "Start session".into(),
                    },
                    ElementInfo {
                        selector: "button.pause".into(),
                        kind: "button".into(),
                        label: "Pause driving".into(),
                    },
                    ElementInfo {
                        selector: "button.resume".into(),
                        kind: "button".into(),
                        label: "Resume driving".into(),
                    },
                ],
            },
        );

        map.routes.insert(
            "/economy".into(),
            PageInfo {
                title: "Agent Economy".into(),
                description: "Token wallets, staking, transactions, and marketplace".into(),
                elements: vec![
                    ElementInfo {
                        selector: "button.stake".into(),
                        kind: "button".into(),
                        label: "Stake credits".into(),
                    },
                    ElementInfo {
                        selector: "button.transfer".into(),
                        kind: "button".into(),
                        label: "Transfer credits".into(),
                    },
                    ElementInfo {
                        selector: "input.amount".into(),
                        kind: "input".into(),
                        label: "Amount input".into(),
                    },
                ],
            },
        );

        map.routes.insert(
            "/nous".into(),
            PageInfo {
                title: "Nous Cognitive Engine".into(),
                description:
                    "Self-reflection, learning stats, capabilities, intent analysis, reasoning"
                        .into(),
                elements: vec![
                    ElementInfo {
                        selector: "textarea.understand-input".into(),
                        kind: "textarea".into(),
                        label: "Intent analysis input".into(),
                    },
                    ElementInfo {
                        selector: "textarea.reason-input".into(),
                        kind: "textarea".into(),
                        label: "Reasoning problem input".into(),
                    },
                    ElementInfo {
                        selector: "button.analyze".into(),
                        kind: "button".into(),
                        label: "Analyze intent".into(),
                    },
                ],
            },
        );

        map.routes.insert(
            "/spawner".into(),
            PageInfo {
                title: "Predictive Spawning".into(),
                description: "Spawn health, active agents, history, and task analysis".into(),
                elements: vec![
                    ElementInfo {
                        selector: "textarea.task-input".into(),
                        kind: "textarea".into(),
                        label: "Task description for spawn analysis".into(),
                    },
                    ElementInfo {
                        selector: "button.analyze-spawn".into(),
                        kind: "button".into(),
                        label: "Analyze for spawning".into(),
                    },
                ],
            },
        );

        map.routes.insert(
            "/agora".into(),
            PageInfo {
                title: "Agora Marketplace".into(),
                description: "Skill marketplace — browse, buy, sell agent skills".into(),
                elements: vec![
                    ElementInfo {
                        selector: "button.publish".into(),
                        kind: "button".into(),
                        label: "Publish skill".into(),
                    },
                    ElementInfo {
                        selector: "input.skill-search".into(),
                        kind: "input".into(),
                        label: "Search marketplace".into(),
                    },
                ],
            },
        );

        map.routes.insert(
            "/pantheon/missions".into(),
            PageInfo {
                title: "Pantheon Missions".into(),
                description:
                    "Multi-agent missions — create, monitor, and manage collaborative tasks".into(),
                elements: vec![
                    ElementInfo {
                        selector: "button.new-mission".into(),
                        kind: "button".into(),
                        label: "Launch mission".into(),
                    },
                    ElementInfo {
                        selector: "textarea.mission-goal".into(),
                        kind: "textarea".into(),
                        label: "Mission goal".into(),
                    },
                    ElementInfo {
                        selector: "select.team-size".into(),
                        kind: "select".into(),
                        label: "Team size".into(),
                    },
                ],
            },
        );

        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ui_action_serialize() {
        let action = UiAction::Navigate {
            route: "/agents".into(),
            label: Some("Go to agents".into()),
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("navigate"));
        assert!(json.contains("/agents"));
    }

    #[test]
    fn test_ui_action_deserialize() {
        let json = r#"{"action": "click", "target": "button.save", "label": "Save"}"#;
        let action: UiAction = serde_json::from_str(json).unwrap();
        assert!(matches!(action, UiAction::Click { .. }));
    }

    #[test]
    fn test_parse_action_plan_direct() {
        let response = r#"[
            {"action": "navigate", "route": "/agents"},
            {"action": "click", "target": "button.create-agent"}
        ]"#;
        let actions = parse_action_plan(response);
        assert_eq!(actions.len(), 2);
        assert!(matches!(actions[0], UiAction::Navigate { .. }));
        assert!(matches!(actions[1], UiAction::Click { .. }));
    }

    #[test]
    fn test_parse_action_plan_code_block() {
        let response = "Here are the actions:\n```json\n[\n{\"action\": \"type\", \"target\": \"input.name\", \"value\": \"test\"}\n]\n```";
        let actions = parse_action_plan(response);
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], UiAction::Type { .. }));
    }

    #[test]
    fn test_parse_action_plan_empty() {
        let actions = parse_action_plan("[]");
        assert!(actions.is_empty());
    }

    #[test]
    fn test_parse_action_plan_invalid() {
        let actions = parse_action_plan("I don't know how to do that");
        assert!(actions.is_empty());
    }

    #[test]
    fn test_puppet_command_serialize() {
        let cmd = PuppetCommand::Action {
            sequence: 1,
            action: UiAction::Navigate {
                route: "/chat".into(),
                label: None,
            },
            description: "Navigate to chat".into(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("action"));
        assert!(json.contains("navigate"));
    }

    #[test]
    fn test_puppet_response_deserialize() {
        let json = r#"{"type": "pause"}"#;
        let resp: PuppetResponse = serde_json::from_str(json).unwrap();
        assert!(matches!(resp, PuppetResponse::Pause));
    }

    #[test]
    fn test_puppet_response_action_result() {
        let json = r#"{"type": "action_result", "sequence": 1, "success": true, "timestamp": "2026-02-25T10:00:00Z"}"#;
        let resp: PuppetResponse = serde_json::from_str(json).unwrap();
        assert!(matches!(resp, PuppetResponse::ActionResult(_)));
    }

    #[test]
    fn test_page_map_default() {
        let map = PageMap::zeus_default();
        assert!(map.routes.contains_key("/dashboard"));
        assert!(map.routes.contains_key("/chat"));
        assert!(map.routes.contains_key("/agents"));
        assert!(map.routes.contains_key("/deploy"));
        assert!(map.routes.contains_key("/pantheon"));
    }

    #[test]
    fn test_page_map_prompt_context() {
        let map = PageMap::zeus_default();
        let ctx = map.to_prompt_context();
        assert!(ctx.contains("## Available Pages"));
        assert!(ctx.contains("/dashboard"));
        assert!(ctx.contains("button.create-agent"));
    }

    #[test]
    fn test_build_director_prompt() {
        let map = PageMap::zeus_default();
        let prompt = build_director_prompt(&map);
        assert!(prompt.contains("AgentDirector"));
        assert!(prompt.contains("Action Format"));
        assert!(prompt.contains("/agents"));
    }

    #[test]
    fn test_scroll_direction_serialize() {
        let action = UiAction::Scroll {
            target: None,
            direction: ScrollDirection::Down,
            amount: 300,
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("down"));
    }

    #[test]
    fn test_clear_highlight_roundtrip() {
        let action = UiAction::ClearHighlight;
        let json = serde_json::to_string(&action).unwrap();
        let parsed: UiAction = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, UiAction::ClearHighlight));
    }

    #[test]
    fn test_complete_command() {
        let cmd = PuppetCommand::Complete {
            summary: "Done!".into(),
            actions_executed: 5,
            actions_failed: 1,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("complete"));
        assert!(json.contains("actions_executed"));
    }

    #[tokio::test]
    async fn test_director_start_session() {
        let director = AgentDirector::new();
        let (mut cmd_rx, _result_tx) = director
            .start_session("test-1".into(), "Create an agent".into())
            .await;

        assert_eq!(
            director.get_status("test-1").await,
            Some(DirectorStatus::Planning)
        );

        // Send a command
        director
            .send_command(
                "test-1",
                PuppetCommand::Thinking {
                    message: "Planning...".into(),
                },
            )
            .await
            .unwrap();

        let cmd = cmd_rx.recv().await.unwrap();
        assert!(matches!(cmd, PuppetCommand::Thinking { .. }));
    }

    #[tokio::test]
    async fn test_director_complete_session() {
        let director = AgentDirector::new();
        let (_cmd_rx, _result_tx) = director
            .start_session("test-2".into(), "Deploy app".into())
            .await;

        director.complete_session("test-2", "App deployed").await;

        assert_eq!(
            director.get_status("test-2").await,
            Some(DirectorStatus::Complete)
        );
    }

    #[tokio::test]
    async fn test_director_pause_resume() {
        let director = AgentDirector::new();
        let (_cmd_rx, _result_tx) = director.start_session("test-3".into(), "Task".into()).await;

        // Must be Driving to pause
        {
            let sessions = director.sessions.read().await;
            sessions.get("test-3").unwrap().write().await.status = DirectorStatus::Driving;
        }

        assert!(director.pause_session("test-3").await);
        assert_eq!(
            director.get_status("test-3").await,
            Some(DirectorStatus::Paused)
        );

        assert!(director.resume_session("test-3").await);
        assert_eq!(
            director.get_status("test-3").await,
            Some(DirectorStatus::Driving)
        );
    }

    #[tokio::test]
    async fn test_director_remove_session() {
        let director = AgentDirector::new();
        let (_cmd_rx, _result_tx) = director.start_session("test-4".into(), "Task".into()).await;

        assert!(director.get_status("test-4").await.is_some());
        director.remove_session("test-4").await;
        assert!(director.get_status("test-4").await.is_none());
    }

    #[tokio::test]
    async fn test_director_active_sessions() {
        let director = AgentDirector::new();
        let (_rx1, _tx1) = director.start_session("s1".into(), "Task 1".into()).await;
        let (_rx2, _tx2) = director.start_session("s2".into(), "Task 2".into()).await;

        let active = director.active_sessions().await;
        assert_eq!(active.len(), 2);
        assert!(active.contains(&"s1".to_string()));
        assert!(active.contains(&"s2".to_string()));
    }

    // ── Sanitization tests (Zeus107 security items A-C) ─────────────────

    #[test]
    fn test_sanitize_valid_navigate() {
        let map = PageMap::zeus_default();
        let action = UiAction::Navigate {
            route: "/agents".into(),
            label: None,
        };
        assert!(sanitize_action(&action, &map).is_ok());
    }

    #[test]
    fn test_sanitize_navigate_protocol_injection() {
        let map = PageMap::zeus_default();
        let action = UiAction::Navigate {
            route: "https://evil.com".into(),
            label: None,
        };
        assert!(sanitize_action(&action, &map).is_err());
    }

    #[test]
    fn test_sanitize_navigate_path_traversal() {
        let map = PageMap::zeus_default();
        let action = UiAction::Navigate {
            route: "/../../etc/passwd".into(),
            label: None,
        };
        assert!(sanitize_action(&action, &map).is_err());
    }

    #[test]
    fn test_sanitize_valid_click() {
        let map = PageMap::zeus_default();
        let action = UiAction::Click {
            target: "button.create-agent".into(),
            label: None,
        };
        assert!(sanitize_action(&action, &map).is_ok());
    }

    #[test]
    fn test_sanitize_click_script_injection() {
        let map = PageMap::zeus_default();
        let action = UiAction::Click {
            target: "button<script>alert(1)</script>".into(),
            label: None,
        };
        assert!(sanitize_action(&action, &map).is_err());
    }

    #[test]
    fn test_sanitize_type_value_too_long() {
        let map = PageMap::zeus_default();
        let action = UiAction::Type {
            target: "input.name".into(),
            value: "x".repeat(MAX_TYPE_VALUE_LEN + 1),
            clear_first: None,
        };
        assert!(sanitize_action(&action, &map).is_err());
    }

    #[test]
    fn test_sanitize_type_value_within_limit() {
        let map = PageMap::zeus_default();
        let action = UiAction::Type {
            target: "input.name".into(),
            value: "Hello World".into(),
            clear_first: None,
        };
        assert!(sanitize_action(&action, &map).is_ok());
    }

    #[test]
    fn test_sanitize_empty_selector_rejected() {
        let map = PageMap::zeus_default();
        let action = UiAction::Click {
            target: "".into(),
            label: None,
        };
        assert!(sanitize_action(&action, &map).is_err());
    }

    #[test]
    fn test_is_safe_selector_valid() {
        assert!(is_safe_selector("button.create-agent"));
        assert!(is_safe_selector("#main-nav > ul > li:first-child"));
        assert!(is_safe_selector("input[name='email']"));
        assert!(is_safe_selector("div.class-name_test"));
    }

    #[test]
    fn test_is_safe_selector_invalid() {
        assert!(!is_safe_selector(""));
        assert!(!is_safe_selector("div<script>"));
        assert!(!is_safe_selector(&"a".repeat(501)));
        assert!(!is_safe_selector("div{color:red}"));
    }

    #[test]
    fn test_page_map_zeus_full() {
        let map = PageMap::zeus_full();
        assert!(map.routes.contains_key("/studio"));
        assert!(map.routes.contains_key("/economy"));
        assert!(map.routes.contains_key("/nous"));
        assert!(map.routes.contains_key("/spawner"));
        assert!(map.routes.contains_key("/agora"));
        assert!(map.routes.contains_key("/pantheon/missions"));
        // Also includes defaults
        assert!(map.routes.contains_key("/dashboard"));
        assert!(map.routes.contains_key("/chat"));
    }

    #[test]
    fn test_driving_config_defaults() {
        let config = DrivingConfig::default();
        assert_eq!(config.max_iterations, 30);
        assert_eq!(config.max_total_actions, 100);
        assert_eq!(config.batch_delay_ms, 300);
        assert_eq!(config.max_consecutive_failures, 5);
        assert_eq!(config.max_replans, 3);
    }

    #[test]
    fn test_driving_result_serialize() {
        let result = DrivingResult {
            session_id: "test-1".into(),
            success: true,
            iterations: 3,
            actions_executed: 10,
            actions_failed: 1,
            summary: "Goal achieved".into(),
            duration_ms: 5000,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("test-1"));
        assert!(json.contains("Goal achieved"));
    }

    #[tokio::test]
    async fn test_take_result_rx() {
        let director = AgentDirector::new();
        let (_cmd_rx, _result_tx) = director
            .start_session("take-test".into(), "Goal".into())
            .await;

        // First take succeeds
        let rx = director.take_result_rx("take-test").await;
        assert!(rx.is_some());

        // Second take fails (already taken)
        let rx2 = director.take_result_rx("take-test").await;
        assert!(rx2.is_none());
    }

    #[test]
    fn test_action_description() {
        assert_eq!(
            action_description(&UiAction::Navigate {
                route: "/agents".into(),
                label: Some("Go to agents".into()),
            }),
            "Go to agents"
        );
        assert_eq!(
            action_description(&UiAction::Click {
                target: "button.save".into(),
                label: None,
            }),
            "Click button.save"
        );
        assert_eq!(
            action_description(&UiAction::ClearHighlight),
            "Clear highlights"
        );
    }
}
