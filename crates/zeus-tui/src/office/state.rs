//! Office state machine — agents, zones, movement, overlays.

use ratatui::style::Color;
use super::sprites::{self, SpriteColors};
use super::palette as P;

/// Digimon-style behavior FSM for ambient agent movement.
#[derive(Clone, Debug, PartialEq)]
pub enum AgentBehavior {
    Idle,
    Wandering,
    WorkingAtDesk,
    OnBreak,
}

impl AgentBehavior {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Wandering => "wandering",
            Self::WorkingAtDesk => "at desk",
            Self::OnBreak => "on break",
        }
    }
}

/// Agent state in the office.
#[derive(Clone, Debug, PartialEq)]
pub enum AgentState {
    Idle,
    Writing,
    Executing,
    Researching,
    Syncing,
    Error,
}

impl AgentState {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "idle" | "offline" => Self::Idle,
            "writing" | "planning" => Self::Writing,
            "executing" | "running" | "active" | "busy" | "working" => Self::Executing,
            "researching" | "thinking" | "analyzing" => Self::Researching,
            "syncing" | "sending" | "receiving" => Self::Syncing,
            "error" | "failed" => Self::Error,
            _ => Self::Idle,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "IDLE",
            Self::Writing => "WRITING",
            Self::Executing => "EXEC",
            Self::Researching => "RESEARCH",
            Self::Syncing => "SYNC",
            Self::Error => "ERROR",
        }
    }

    pub fn color(&self) -> Color {
        match self {
            Self::Idle => P::DIM,
            Self::Writing => P::ACCENT,
            Self::Executing => P::GREEN,
            Self::Researching => P::CYAN,
            Self::Syncing => P::BLUE,
            Self::Error => P::RED,
        }
    }

    pub fn zone(&self) -> Zone {
        match self {
            Self::Idle => Zone::BreakRoom,
            Self::Writing | Self::Executing => Zone::Engineering,
            Self::Researching | Self::Error => Zone::Research,
            Self::Syncing => Zone::Comms,
        }
    }
}

/// Office zones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Zone {
    Engineering,
    Comms,
    Research,
    BreakRoom,
    Kitchen,
}

impl Zone {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Engineering => "Engineering",
            Self::Comms => "Comms",
            Self::Research => "Research",
            Self::BreakRoom => "Break Room",
            Self::Kitchen => "Kitchen",
        }
    }

    pub fn color(&self) -> Color {
        match self {
            Self::Engineering => P::ACCENT,
            Self::Comms => P::BLUE,
            Self::Research => P::CYAN,
            Self::BreakRoom => P::YELLOW,
            Self::Kitchen => P::GREEN,
        }
    }

    /// Center position in pixel coordinates for this zone (reference 80x40).
    pub fn center(&self) -> (i32, i32) {
        self.center_scaled(80, 40)
    }

    /// Center position scaled to actual pixel dimensions.
    pub fn center_scaled(&self, width: usize, height: usize) -> (i32, i32) {
        let (rx, ry) = match self {
            Self::Engineering => (16, 14),
            Self::Comms => (56, 14),
            Self::Research => (14, 30),
            Self::BreakRoom => (62, 28),
            Self::Kitchen => (10, 34),
        };
        (super::background::scale_x(rx, width), super::background::scale_y(ry, height))
    }

    pub const ALL: [Zone; 5] = [Self::Engineering, Self::Comms, Self::Research, Self::BreakRoom, Self::Kitchen];
}

/// An agent visible in the office.
#[derive(Clone, Debug)]
pub struct OfficeAgent {
    pub id: String,
    pub name: String,
    pub state: AgentState,
    pub zone: Zone,
    pub x: f32,
    pub y: f32,
    pub target_x: f32,
    pub target_y: f32,
    pub frame: u32,
    pub task: String,
    pub model: String,
    pub sprite_colors: SpriteColors,
    /// S93: "local" for gateway agents, "channel" for Discord-discovered agents
    pub agent_type: String,
    /// S94 T3: Last Discord message text for chat bubble rendering
    pub last_message: String,
    /// S94 T3: Ticks since last_message was set — bubble fades after 30 ticks
    pub message_age: u32,
    /// S94: Digimon-style behavior FSM
    pub behavior: AgentBehavior,
    /// Ticks until next behavior transition
    pub behavior_tick: u32,
    /// Home zone based on persona — agents drift back here when idle
    pub home_zone: Zone,
}

impl OfficeAgent {
    pub fn new(id: &str, name: &str) -> Self {
        let colors = sprites::palette_for(id);
        let home_zone = home_zone_for(id);
        let zone = home_zone;
        let (cx, cy) = zone.center();
        Self {
            id: id.to_string(),
            name: name.to_string(),
            state: AgentState::Idle,
            zone,
            x: cx as f32,
            y: cy as f32,
            target_x: cx as f32,
            target_y: cy as f32,
            frame: 0,
            task: String::new(),
            model: String::new(),
            sprite_colors: colors,
            agent_type: "local".to_string(),
            last_message: String::new(),
            message_age: 30, // start at max age so no bubble shown until first message
            behavior: AgentBehavior::Idle,
            behavior_tick: 0,
            home_zone,
        }
    }

    /// Update target position when state/zone changes.
    pub fn set_state(&mut self, state: AgentState, task: &str, jitter_seed: u32) {
        self.set_state_scaled(state, task, jitter_seed, 80, 40);
    }

    /// Update target position scaled to actual pixel dimensions.
    pub fn set_state_scaled(&mut self, state: AgentState, task: &str, jitter_seed: u32, width: usize, height: usize) {
        self.state = state;
        self.task = task.to_string();
        self.zone = self.state.zone();
        let (cx, cy) = self.zone.center_scaled(width, height);
        // Deterministic jitter so agents don't stack
        let jx = ((jitter_seed * 7 + 3) % 11) as i32 - 5;
        let jy = ((jitter_seed * 13 + 5) % 5) as i32 - 2;
        self.target_x = (cx + jx).max(1).min(width as i32 - 9) as f32;
        self.target_y = (cy + jy).max(5).min(height as i32 - 1) as f32;
    }
}

/// Assign a home zone based on agent persona/id.
/// Coordinators → Comms, engineers/builders → Engineering,
/// researchers/security → Research, others → BreakRoom.
pub fn home_zone_for(id: &str) -> Zone {
    let id = id.to_lowercase();
    if id.contains("100") || id.contains("coord") || id.contains("comms") || id.contains("relay") {
        Zone::Comms
    } else if id.contains("106") || id.contains("107") || id.contains("112") || id.contains("build") || id.contains("eng") {
        Zone::Engineering
    } else if id.contains("research") || id.contains("security") || id.contains("audit") || id.contains("fbsd") {
        Zone::Research
    } else {
        Zone::BreakRoom
    }
}

/// Pick a random-ish target within a zone using a simple LCG seeded by tick + agent id.
pub fn zone_random_pos(zone: Zone, seed: u64) -> (f32, f32) {
    zone_random_pos_scaled(zone, seed, 80, 40)
}

/// Pick a random-ish target within a zone, scaled to actual pixel dimensions.
pub fn zone_random_pos_scaled(zone: Zone, seed: u64, width: usize, height: usize) -> (f32, f32) {
    // LCG: next = (a*seed + c) % m
    let r1 = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    let r2 = r1.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    let (cx, cy) = zone.center_scaled(width, height);
    // Spread ±8 in x, ±4 in y around zone center (scaled)
    let spread_x = (width / 10).max(4) as i32;
    let spread_y = (height / 10).max(2) as i32;
    let dx = ((r1 % (spread_x as u64 * 2 + 1)) as i32) - spread_x;
    let dy = ((r2 % (spread_y as u64 * 2 + 1)) as i32) - spread_y;
    let x = (cx + dx).max(2).min(width as i32 - 10) as f32;
    let y = (cy + dy).max(5).min(height as i32 - 2) as f32;
    (x, y)
}

/// Full office state.
pub struct OfficeState {
    pub agents: Vec<OfficeAgent>,
    pub tick: u64,
    pub connected: bool,
    pub focused_agent: Option<usize>,
    pub show_memo: bool,
    pub show_help: bool,
    pub memo_text: Vec<String>,
    pub memo_date: String,
    /// Current scene pixel dimensions (updated on resize).
    pub scene_width: usize,
    pub scene_height: usize,
}

impl OfficeState {
    pub fn new() -> Self {
        // B1: Start with empty roster — real agents come from `sync_from_fleet()`
        // which polls GET /v1/agents on the gateway. No more hardcoded demo agents.
        Self {
            agents: Vec::new(),
            tick: 0,
            connected: false,
            focused_agent: None,
            show_memo: false,
            show_help: false,
            memo_text: Vec::new(),
            memo_date: String::new(),
            scene_width: 80,
            scene_height: 40,
        }
    }

    /// Advance one logic tick: move agents toward targets, increment frames + FSM.
    pub fn tick(&mut self) {
        self.tick += 1;
        let tick = self.tick;
        for agent in &mut self.agents {
            agent.frame += 1;
            // S94 T3: age the chat bubble
            if agent.message_age < u32::MAX {
                agent.message_age = agent.message_age.saturating_add(1);
            }

            // ── S94: Behavior FSM tick ──
            if agent.behavior_tick > 0 {
                agent.behavior_tick -= 1;
            } else {
                // Weighted transition table — varies by current behavior
                // Seed combines tick + agent id hash for deterministic-ish variance
                let id_seed: u64 = agent.id.bytes().fold(0u64, |acc, b| acc.wrapping_add(b as u64));
                let seed = tick.wrapping_mul(id_seed.wrapping_add(1)).wrapping_mul(6364136223846793005);
                let roll = (seed >> 33) % 100; // 0..99

                let next = match agent.behavior {
                    AgentBehavior::Idle => {
                        // Idle: 40% stay, 30% wander, 20% work, 10% break
                        if roll < 40 { AgentBehavior::Idle }
                        else if roll < 70 { AgentBehavior::Wandering }
                        else if roll < 90 { AgentBehavior::WorkingAtDesk }
                        else { AgentBehavior::OnBreak }
                    }
                    AgentBehavior::Wandering => {
                        // Wandering: 20% keep wandering, 50% settle at desk, 20% idle, 10% break
                        if roll < 20 { AgentBehavior::Wandering }
                        else if roll < 70 { AgentBehavior::WorkingAtDesk }
                        else if roll < 90 { AgentBehavior::Idle }
                        else { AgentBehavior::OnBreak }
                    }
                    AgentBehavior::WorkingAtDesk => {
                        // Working: 60% keep working, 20% idle, 15% wander, 5% break
                        if roll < 60 { AgentBehavior::WorkingAtDesk }
                        else if roll < 80 { AgentBehavior::Idle }
                        else if roll < 95 { AgentBehavior::Wandering }
                        else { AgentBehavior::OnBreak }
                    }
                    AgentBehavior::OnBreak => {
                        // Break: 30% stay on break, 40% idle, 20% wander, 10% back to work
                        if roll < 30 { AgentBehavior::OnBreak }
                        else if roll < 70 { AgentBehavior::Idle }
                        else if roll < 90 { AgentBehavior::Wandering }
                        else { AgentBehavior::WorkingAtDesk }
                    }
                };

                // Set new target based on next behavior
                let target_zone = match next {
                    AgentBehavior::Idle => agent.home_zone,
                    AgentBehavior::Wandering => {
                        // Pick a random zone, weighted toward home
                        let zone_roll = (seed >> 20) % 4;
                        if zone_roll == 0 { Zone::Engineering }
                        else if zone_roll == 1 { Zone::Comms }
                        else if zone_roll == 2 { Zone::Research }
                        else { Zone::BreakRoom }
                    }
                    AgentBehavior::WorkingAtDesk => agent.home_zone,
                    AgentBehavior::OnBreak => Zone::BreakRoom,
                };

                let pos_seed = seed.wrapping_add(tick);
                let (tx, ty) = zone_random_pos_scaled(target_zone, pos_seed, self.scene_width, self.scene_height);
                agent.target_x = tx;
                agent.target_y = ty;
                agent.zone = target_zone;

                // Duration: how many ticks before next FSM evaluation
                let duration = match next {
                    AgentBehavior::Idle => 20 + (seed % 20) as u32,
                    AgentBehavior::Wandering => 8 + (seed % 12) as u32,
                    AgentBehavior::WorkingAtDesk => 40 + (seed % 40) as u32,
                    AgentBehavior::OnBreak => 30 + (seed % 30) as u32,
                };
                agent.behavior = next;
                agent.behavior_tick = duration;
            }

            // ── Movement toward target ──
            let dx = agent.target_x - agent.x;
            let dy = agent.target_y - agent.y;
            let speed_x = if agent.behavior == AgentBehavior::Wandering { 1.0 } else { 2.0 };
            if dx.abs() > 0.5 {
                agent.x += dx.signum() * dx.abs().min(speed_x);
            } else if agent.behavior == AgentBehavior::Idle {
                // Idle sway: subtle ±1px oscillation around target
                let sway = if tick % 8 < 4 { 1.0 } else { -1.0 };
                agent.x = agent.target_x + sway;
            }
            if dy.abs() > 0.5 {
                agent.y += dy.signum() * dy.abs().min(1.0);
            }
        }
    }

    /// Cycle focus to next agent (Tab key).
    pub fn cycle_focus(&mut self) {
        if self.agents.is_empty() { return; }
        self.focused_agent = Some(match self.focused_agent {
            None => 0,
            Some(i) => (i + 1) % self.agents.len(),
        });
    }

    /// Clear focus (Esc key).
    pub fn clear_focus(&mut self) {
        self.focused_agent = None;
        self.show_memo = false;
        self.show_help = false;
    }

    /// Count agents in a given zone.
    pub fn zone_count(&self, zone: Zone) -> usize {
        self.agents.iter().filter(|a| a.zone == zone).count()
    }

    /// Count agents not idle.
    pub fn active_count(&self) -> usize {
        self.agents.iter().filter(|a| a.state != AgentState::Idle).count()
    }

    /// Count agents in error state.
    pub fn error_count(&self) -> usize {
        self.agents.iter().filter(|a| a.state == AgentState::Error).count()
    }
}

/// Sync office agents from fleet API response data.
/// Existing agents get updated in-place (preserving position/animation).
/// New agents spawn at office entrance. Missing agents are removed.
pub fn sync_from_fleet(state: &mut OfficeState, fleet: &[crate::api::AgentResponse]) {
    use std::collections::HashSet;
    let fleet_ids: HashSet<&str> = fleet.iter().map(|a| a.id.as_str()).collect();

    // Update existing or add new
    for ag in fleet {
        let agent_state = AgentState::from_str(&ag.status);
        let task = ag.current_task.clone().unwrap_or_default();
        let model = ag.metadata.get("model").cloned().unwrap_or_default();

        // S93: derive agent_type — default to "local" for legacy agents without the field
        let agent_type = if ag.agent_type.is_empty() { "local" } else { &ag.agent_type };

        if let Some(existing) = state.agents.iter_mut().find(|a| a.id == ag.id) {
            // Update state if changed — triggers zone movement
            if existing.state != agent_state || existing.task != task {
                let seed = existing.id.bytes().fold(0u32, |acc, b| acc.wrapping_add(b as u32));
                existing.set_state(agent_state, &task, seed);
                // S94 T3: new task = new message, reset bubble age
                if !task.is_empty() {
                    existing.last_message = task.clone();
                    existing.message_age = 0;
                }
            }
            existing.model = model;
            existing.agent_type = agent_type.to_string();
            // S94 T3: copy current_task into last_message when task changes
            if existing.task != task && !task.is_empty() {
                existing.last_message = task.clone();
                existing.message_age = 0;
            }
        } else {
            // New agent — spawn at entrance, then move to zone
            let mut new_agent = OfficeAgent::new(&ag.id, &ag.name);
            new_agent.x = 40.0; // office entrance (center bottom)
            new_agent.y = 38.0;
            let seed = new_agent.id.bytes().fold(0u32, |acc, b| acc.wrapping_add(b as u32));
            new_agent.set_state(agent_state, &task, seed);
            new_agent.model = model;
            new_agent.agent_type = agent_type.to_string();
            // S94 T3: set initial message bubble
            if !task.is_empty() {
                new_agent.last_message = task.clone();
                new_agent.message_age = 0;
            }
            // S93: Channel agents get sprite colors assigned via palette_for (same as local)
            // sprite_colors is already set by OfficeAgent::new — no extra step needed
            state.agents.push(new_agent);
        }
    }

    // Remove agents no longer in fleet
    state.agents.retain(|a| fleet_ids.contains(a.id.as_str()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_to_zone_mapping() {
        assert_eq!(AgentState::Idle.zone(), Zone::BreakRoom);
        assert_eq!(AgentState::Writing.zone(), Zone::Engineering);
        assert_eq!(AgentState::Executing.zone(), Zone::Engineering);
        assert_eq!(AgentState::Researching.zone(), Zone::Research);
        assert_eq!(AgentState::Syncing.zone(), Zone::Comms);
        assert_eq!(AgentState::Error.zone(), Zone::Research);
    }

    #[test]
    fn test_agent_movement() {
        let mut state = OfficeState::new();
        state.agents.clear();
        state.agents.push(OfficeAgent::new("test", "Test Agent"));
        state.agents[0].x = 10.0;
        state.agents[0].y = 10.0;
        state.agents[0].target_x = 20.0;
        state.agents[0].target_y = 15.0;

        state.tick();
        // Should move toward target
        assert!(state.agents[0].x > 10.0);
        assert!(state.agents[0].y > 10.0);
    }

    #[test]
    fn test_agent_stops_at_target() {
        let mut state = OfficeState::new();
        state.agents.clear();
        let mut agent = OfficeAgent::new("test", "Test Agent");
        agent.state = AgentState::Executing; // non-idle to avoid sway
        agent.behavior = AgentBehavior::WorkingAtDesk; // prevent FSM from changing target
        agent.behavior_tick = 999; // prevent FSM transition during test
        agent.x = 10.0;
        agent.y = 10.0;
        agent.target_x = 10.0;
        agent.target_y = 10.0;
        state.agents.push(agent);

        let before_x = state.agents[0].x;
        state.tick();
        assert_eq!(state.agents[0].x, before_x);
    }

    #[test]
    fn test_cycle_focus() {
        let mut state = OfficeState::new();
        state.agents.clear();
        state.agents.push(OfficeAgent::new("a", "Agent A"));
        state.agents.push(OfficeAgent::new("b", "Agent B"));

        assert_eq!(state.focused_agent, None);
        state.cycle_focus();
        assert_eq!(state.focused_agent, Some(0));
        state.cycle_focus();
        assert_eq!(state.focused_agent, Some(1));
        state.cycle_focus();
        assert_eq!(state.focused_agent, Some(0)); // wraps
    }

    #[test]
    fn test_clear_focus() {
        let mut state = OfficeState::new();
        state.agents.clear();
        state.agents.push(OfficeAgent::new("a", "Agent A"));
        state.focused_agent = Some(0);
        state.show_memo = true;
        state.show_help = true;
        state.clear_focus();
        assert_eq!(state.focused_agent, None);
        assert!(!state.show_memo);
        assert!(!state.show_help);
    }

    #[test]
    fn test_zone_count() {
        let mut state = OfficeState::new();
        state.agents.clear();
        let mut a = OfficeAgent::new("a", "A");
        a.zone = Zone::Engineering;
        state.agents.push(a);
        let mut b = OfficeAgent::new("b", "B");
        b.zone = Zone::Engineering;
        state.agents.push(b);
        let mut c = OfficeAgent::new("c", "C");
        c.zone = Zone::BreakRoom;
        state.agents.push(c);

        assert_eq!(state.zone_count(Zone::Engineering), 2);
        assert_eq!(state.zone_count(Zone::BreakRoom), 1);
        assert_eq!(state.zone_count(Zone::Comms), 0);
    }

    #[test]
    fn test_agent_state_from_str() {
        assert_eq!(AgentState::from_str("busy"), AgentState::Executing);
        assert_eq!(AgentState::from_str("IDLE"), AgentState::Idle);
        assert_eq!(AgentState::from_str("unknown"), AgentState::Idle);
        assert_eq!(AgentState::from_str("thinking"), AgentState::Researching);
    }

    #[test]
    fn test_set_state_updates_zone_and_target() {
        let mut agent = OfficeAgent::new("test", "Test");
        agent.set_state(AgentState::Executing, "cargo build", 42);
        assert_eq!(agent.zone, Zone::Engineering);
        assert_eq!(agent.task, "cargo build");
        // Target should be near engineering center (16, 14)
        assert!((agent.target_x - 16.0).abs() < 10.0);
        assert!((agent.target_y - 14.0).abs() < 10.0);
    }

    #[test]
    fn test_sync_from_fleet_adds_new_agents() {
        let mut state = OfficeState::new();
        state.agents.clear();
        let fleet = vec![
            crate::api::AgentResponse {
                id: "zeus100".into(), name: "Zeus100".into(),
                status: "busy".into(), health_score: 0.9, load_pct: 0.5,
                last_heartbeat: String::new(),
                metadata: std::collections::HashMap::new(),
                current_task: Some("building".into()),
                agent_type: "local".into(),
            },
        ];
        sync_from_fleet(&mut state, &fleet);
        assert_eq!(state.agents.len(), 1);
        assert_eq!(state.agents[0].id, "zeus100");
        assert_eq!(state.agents[0].task, "building");
    }

    #[test]
    fn test_sync_from_fleet_updates_existing() {
        let mut state = OfficeState::new();
        state.agents.clear();
        state.agents.push(OfficeAgent::new("zeus100", "Zeus100"));
        let fleet = vec![
            crate::api::AgentResponse {
                id: "zeus100".into(), name: "Zeus100".into(),
                status: "idle".into(), health_score: 1.0, load_pct: 0.0,
                last_heartbeat: String::new(),
                metadata: std::collections::HashMap::new(),
                current_task: Some("coffee break".into()),
                agent_type: "local".into(),
            },
        ];
        sync_from_fleet(&mut state, &fleet);
        assert_eq!(state.agents.len(), 1);
        assert_eq!(state.agents[0].task, "coffee break");
        assert_eq!(state.agents[0].state, AgentState::Idle);
    }

    #[test]
    fn test_sync_from_fleet_removes_stale() {
        let mut state = OfficeState::new();
        state.agents.clear();
        state.agents.push(OfficeAgent::new("old-agent", "Old"));
        let fleet: Vec<crate::api::AgentResponse> = vec![];
        sync_from_fleet(&mut state, &fleet);
        assert!(state.agents.is_empty());
    }

    #[test]
    fn test_office_starts_empty() {
        // B1: OfficeState::new() starts with an empty roster — real agents
        // are added via sync_from_fleet() from GET /v1/agents polling.
        let state = OfficeState::new();
        assert!(state.agents.is_empty());
    }
}
