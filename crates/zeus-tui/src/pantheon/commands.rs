//! Pantheon IRC command parsing and dispatch.
//!
//! Handles `/me`, `/join`, `/topic`, `/who`, `/clear`, `/help`,
//! `/approve`, `/reject`, and `/missions` and returns a typed `Command`
//! that the app state layer acts on.  Commands that require async API
//! calls return a `PantheonApiEffect` that the TUI event loop must spawn.

use super::app::{IrcMessage, MessageKind, PantheonApp};
use chrono::Utc;

/// An async API call that the TUI event loop must spawn after dispatch returns.
#[derive(Debug, Clone)]
pub enum PantheonApiEffect {
    ApprovePlan { plan_id: String },
    RejectPlan { plan_id: String, reason: String },
    ListMissions,
}

/// A parsed IRC-style command from the input bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `/me <action text>` — send an action message.
    Me(String),
    /// `/join <#channel>` — join or switch to a channel.
    Join(String),
    /// `/topic <text>` — set the current channel topic.
    Topic(String),
    /// `/who` — list users in the current channel.
    Who,
    /// `/clear` — clear the current channel's message history.
    Clear,
    /// `/help` — display command help.
    Help,
    /// `/approve <plan_id>` — approve a pending plan card.
    Approve(String),
    /// `/reject <plan_id> [reason]` — reject a plan card with optional reason.
    Reject { plan_id: String, reason: String },
    /// `/missions` — list active missions.
    Missions,
    /// Unrecognised command — carry the raw input for error display.
    Unknown(String),
}

/// Parse a raw input string into a `Command`.
///
/// Returns `None` if the input doesn't start with `/` (i.e. it's a normal
/// message, not a command).
pub fn parse(input: &str) -> Option<Command> {
    let input = input.trim();
    if !input.starts_with('/') {
        return None;
    }

    // Split into command word and remainder.
    let (cmd, rest) = match input[1..].split_once(char::is_whitespace) {
        Some((c, r)) => (c.to_lowercase(), r.trim().to_string()),
        None         => (input[1..].to_lowercase(), String::new()),
    };

    let command = match cmd.as_str() {
        "me"    => Command::Me(rest),
        "join"  => {
            // Normalise: ensure channel name starts with `#`.
            let ch = if rest.starts_with('#') {
                rest
            } else {
                format!("#{rest}")
            };
            Command::Join(ch)
        }
        "topic" => Command::Topic(rest),
        "who"   => Command::Who,
        "clear" => Command::Clear,
        "help"  => Command::Help,
        "approve" => {
            if rest.is_empty() {
                Command::Unknown("/approve requires a plan_id".to_string())
            } else {
                Command::Approve(rest)
            }
        }
        "reject" => {
            let (plan_id, reason) = match rest.split_once(char::is_whitespace) {
                Some((id, r)) => (id.to_string(), r.trim().to_string()),
                None => (rest, String::new()),
            };
            if plan_id.is_empty() {
                Command::Unknown("/reject requires a plan_id".to_string())
            } else {
                Command::Reject { plan_id, reason }
            }
        }
        "missions" => Command::Missions,
        other   => Command::Unknown(format!("/{other}")),
    };

    Some(command)
}

/// Apply a parsed command to `PantheonApp`, returning zero or more messages
/// that should be injected into the active channel's history, plus an optional
/// async API effect the caller must spawn.
pub fn dispatch(app: &mut PantheonApp, cmd: Command) -> (Vec<IrcMessage>, Option<PantheonApiEffect>) {
    let now = Utc::now();
    let nick = app.nick.clone();

    match cmd {
        Command::Me(text) => {
            let msg = IrcMessage {
                nick: nick.clone(),
                content: text,
                kind: MessageKind::Action,
                timestamp: now,
            };
            if let Some(ch) = app.active_channel_mut() {
                ch.messages.push(msg.clone());
            }
            (vec![], None)
        }

        Command::Join(channel) => {
            // If channel already exists, just switch to it.
            if let Some(idx) = app.channels.iter().position(|c| c.name == channel) {
                app.active_channel = idx;
            } else {
                // Create new channel and switch.
                app.channels.push(super::app::IrcChannel::new(&channel));
                app.active_channel = app.channels.len() - 1;
            }
            let system = IrcMessage {
                nick: String::new(),
                content: format!("Now talking in {channel}"),
                kind: MessageKind::System,
                timestamp: now,
            };
            if let Some(ch) = app.active_channel_mut() {
                ch.messages.push(system);
            }
            (vec![], None)
        }

        Command::Topic(text) => {
            if let Some(ch) = app.active_channel_mut() {
                let topic_msg = IrcMessage {
                    nick: nick.clone(),
                    content: text.clone(),
                    kind: MessageKind::Topic,
                    timestamp: now,
                };
                ch.topic = Some(text);
                ch.messages.push(topic_msg);
            }
            (vec![], None)
        }

        Command::Who => {
            let lines: Vec<String> = app
                .active_channel()
                .map(|ch| {
                    ch.users
                        .iter()
                        .map(|u| {
                            let prefix = u.mode.unwrap_or(' ');
                            let status = if u.is_online { "●" } else { "○" };
                            format!("{status} {prefix}{}", u.nick)
                        })
                        .collect()
                })
                .unwrap_or_default();

            let content = if lines.is_empty() {
                "No users in channel.".to_string()
            } else {
                lines.join("  ")
            };

            let msg = IrcMessage {
                nick: String::new(),
                content,
                kind: MessageKind::System,
                timestamp: now,
            };
            if let Some(ch) = app.active_channel_mut() {
                ch.messages.push(msg);
            }
            (vec![], None)
        }

        Command::Clear => {
            if let Some(ch) = app.active_channel_mut() {
                ch.messages.clear();
            }
            (vec![], None)
        }

        Command::Help => {
            let help = concat!(
                "/me <text>              — send an action message\n",
                "/join <#chan>           — join or switch to a channel\n",
                "/topic <text>          — set the channel topic\n",
                "/who                   — list users in this channel\n",
                "/clear                 — clear message history\n",
                "/approve <plan_id>     — approve a pending plan card\n",
                "/reject <plan_id> [reason]  — reject a plan card\n",
                "/missions              — list active missions\n",
                "/help                  — show this help"
            );
            let msg = IrcMessage {
                nick: String::new(),
                content: help.to_string(),
                kind: MessageKind::System,
                timestamp: now,
            };
            if let Some(ch) = app.active_channel_mut() {
                ch.messages.push(msg);
            }
            (vec![], None)
        }

        Command::Approve(plan_id) => {
            let msg = IrcMessage {
                nick: String::new(),
                content: format!("Approving plan {plan_id}…"),
                kind: MessageKind::System,
                timestamp: now,
            };
            if let Some(ch) = app.active_channel_mut() {
                ch.messages.push(msg);
            }
            (vec![], Some(PantheonApiEffect::ApprovePlan { plan_id }))
        }

        Command::Reject { plan_id, reason } => {
            let msg = IrcMessage {
                nick: String::new(),
                content: format!("Rejecting plan {plan_id}…"),
                kind: MessageKind::System,
                timestamp: now,
            };
            if let Some(ch) = app.active_channel_mut() {
                ch.messages.push(msg);
            }
            (vec![], Some(PantheonApiEffect::RejectPlan { plan_id, reason }))
        }

        Command::Missions => {
            let msg = IrcMessage {
                nick: String::new(),
                content: "Fetching active missions…".to_string(),
                kind: MessageKind::System,
                timestamp: now,
            };
            if let Some(ch) = app.active_channel_mut() {
                ch.messages.push(msg);
            }
            (vec![], Some(PantheonApiEffect::ListMissions))
        }

        Command::Unknown(raw) => {
            let msg = IrcMessage {
                nick: String::new(),
                content: format!("Unknown command: {raw}  (try /help)"),
                kind: MessageKind::System,
                timestamp: now,
            };
            if let Some(ch) = app.active_channel_mut() {
                ch.messages.push(msg);
            }
            (vec![], None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_me() {
        assert_eq!(parse("/me waves"), Some(Command::Me("waves".into())));
    }

    #[test]
    fn parses_join_with_hash() {
        assert_eq!(parse("/join #dev"), Some(Command::Join("#dev".into())));
    }

    #[test]
    fn parses_join_without_hash() {
        assert_eq!(parse("/join dev"), Some(Command::Join("#dev".into())));
    }

    #[test]
    fn parses_who() {
        assert_eq!(parse("/who"), Some(Command::Who));
    }

    #[test]
    fn parses_clear() {
        assert_eq!(parse("/clear"), Some(Command::Clear));
    }

    #[test]
    fn parses_help() {
        assert_eq!(parse("/help"), Some(Command::Help));
    }

    #[test]
    fn no_slash_returns_none() {
        assert_eq!(parse("hello"), None);
    }

    #[test]
    fn unknown_command() {
        assert!(matches!(parse("/foo"), Some(Command::Unknown(_))));
    }

    #[test]
    fn parses_approve() {
        assert_eq!(
            parse("/approve plan-abc-123"),
            Some(Command::Approve("plan-abc-123".into()))
        );
    }

    #[test]
    fn approve_without_plan_id_is_unknown() {
        assert!(matches!(parse("/approve"), Some(Command::Unknown(_))));
    }

    #[test]
    fn parses_reject_with_reason() {
        assert_eq!(
            parse("/reject plan-abc-123 not ready yet"),
            Some(Command::Reject {
                plan_id: "plan-abc-123".into(),
                reason: "not ready yet".into(),
            })
        );
    }

    #[test]
    fn parses_reject_without_reason() {
        assert_eq!(
            parse("/reject plan-abc-123"),
            Some(Command::Reject {
                plan_id: "plan-abc-123".into(),
                reason: String::new(),
            })
        );
    }

    #[test]
    fn reject_without_plan_id_is_unknown() {
        assert!(matches!(parse("/reject"), Some(Command::Unknown(_))));
    }

    #[test]
    fn parses_missions() {
        assert_eq!(parse("/missions"), Some(Command::Missions));
    }
}
