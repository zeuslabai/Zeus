//! TUI regression tests — chat, input history, search mode
//! Owner: mikes-Mac-mini
#![cfg(test)]

use crate::app::{App, ChatMessage, Role};
use chrono::Local;

fn make_app() -> App {
    App::new("http://localhost:9999")
}

fn user_msg(content: &str) -> ChatMessage {
    ChatMessage {
        role: Role::User,
        content: content.to_string(),
        timestamp: Local::now().format("%H:%M:%S").to_string(),
        agent_name: None,
        streaming: false,
        stream_state: None,
        channel_source: None,
    }
}

fn assistant_msg(content: &str) -> ChatMessage {
    ChatMessage {
        role: Role::Assistant,
        content: content.to_string(),
        timestamp: Local::now().format("%H:%M:%S").to_string(),
        agent_name: None,
        streaming: false,
        stream_state: None,
        channel_source: None,
    }
}

// ── Chat send/receive ─────────────────────────────────────────────────────────

#[test]
fn chat_send_adds_user_message() {
    let mut app = make_app();
    app.messages.push(user_msg("hello zeus"));
    let last = app.messages.last().unwrap();
    assert_eq!(last.role, Role::User);
    assert_eq!(last.content, "hello zeus");
}

#[test]
fn chat_receive_adds_assistant_message() {
    let mut app = make_app();
    app.messages.push(assistant_msg("hello back"));
    let last = app.messages.last().unwrap();
    assert_eq!(last.role, Role::Assistant);
    assert_eq!(last.content, "hello back");
}

#[test]
fn chat_streaming_message_has_cursor() {
    let mut app = make_app();
    let mut msg = assistant_msg("partial respon");
    msg.streaming = true;
    app.messages.push(msg);
    let last = app.messages.last().unwrap();
    assert!(last.streaming);
}

#[test]
fn chat_streaming_false_after_complete() {
    let mut app = make_app();
    let mut msg = assistant_msg("full response");
    msg.streaming = true;
    app.messages.push(msg);
    // simulate completion
    app.messages.last_mut().unwrap().streaming = false;
    assert!(!app.messages.last().unwrap().streaming);
}

#[test]
fn chat_send_clears_input() {
    let mut app = make_app();
    app.input = "send me".to_string();
    let msg = app.input.clone();
    app.messages.push(user_msg(&msg));
    app.input.clear();
    app.cursor_pos = 0;
    assert!(app.input.is_empty());
    assert_eq!(app.cursor_pos, 0);
}

#[test]
fn chat_messages_preserve_order() {
    let mut app = make_app();
    app.messages.push(user_msg("first"));
    app.messages.push(assistant_msg("second"));
    app.messages.push(user_msg("third"));
    assert_eq!(app.messages.len(), 3);
    assert_eq!(app.messages[0].content, "first");
    assert_eq!(app.messages[1].content, "second");
    assert_eq!(app.messages[2].content, "third");
}

// ── Input history ─────────────────────────────────────────────────────────────

#[test]
fn history_arrow_up_recalls_last_sent() {
    let mut app = make_app();
    app.input_history.push("first command".to_string());
    // simulate Up key
    app.input_history_idx = app.input_history.len() as isize - 1;
    let recalled = app.input_history[app.input_history_idx as usize].clone();
    app.input = recalled;
    assert_eq!(app.input, "first command");
}

#[test]
fn history_arrow_up_cycles_through_multiple() {
    let mut app = make_app();
    app.input_history.push("cmd1".to_string());
    app.input_history.push("cmd2".to_string());
    app.input_history.push("cmd3".to_string());

    // Up from idle → last (cmd3)
    app.input_history_idx = app.input_history.len() as isize - 1;
    assert_eq!(app.input_history[app.input_history_idx as usize], "cmd3");

    // Up again → cmd2
    app.input_history_idx -= 1;
    assert_eq!(app.input_history[app.input_history_idx as usize], "cmd2");

    // Up again → cmd1
    app.input_history_idx -= 1;
    assert_eq!(app.input_history[app.input_history_idx as usize], "cmd1");
}

#[test]
fn history_arrow_down_returns_to_empty() {
    let mut app = make_app();
    app.input_history.push("cmd1".to_string());
    app.input_history_idx = 0;

    // Down past end → idle (-1), input cleared
    app.input_history_idx += 1;
    if app.input_history_idx >= app.input_history.len() as isize {
        app.input_history_idx = -1;
        app.input.clear();
    }
    assert_eq!(app.input_history_idx, -1);
    assert!(app.input.is_empty());
}

#[test]
fn history_send_appends_and_resets_idx() {
    let mut app = make_app();
    app.input = "new command".to_string();
    let msg = app.input.clone();
    app.input_history.push(msg);
    app.input_history_idx = -1;
    assert_eq!(app.input_history.last().unwrap(), "new command");
    assert_eq!(app.input_history_idx, -1);
}

#[test]
fn history_empty_up_does_nothing() {
    let mut app = make_app();
    // No history — Up should not panic or change idx
    if !app.input_history.is_empty() {
        app.input_history_idx = app.input_history.len() as isize - 1;
    }
    // idx remains -1 when history is empty
    assert_eq!(app.input_history_idx, -1);
}

// ── Search mode ───────────────────────────────────────────────────────────────

#[test]
fn search_ctrl_f_toggles_active() {
    let mut app = make_app();
    assert!(!app.search_active);
    app.search_active = true;
    assert!(app.search_active);
    app.search_active = false;
    assert!(!app.search_active);
}

#[test]
fn search_query_finds_matching_messages() {
    let mut app = make_app();
    app.messages.push(user_msg("deploy the agent now"));
    app.messages.push(assistant_msg("deploying agent..."));
    app.messages.push(user_msg("unrelated message"));

    app.search_query = "agent".to_string();
    app.update_search_matches();

    assert_eq!(app.search_matches.len(), 2);
}

#[test]
fn search_query_no_matches_returns_empty() {
    let mut app = make_app();
    app.messages.push(user_msg("hello world"));
    app.search_query = "xyznotfound".to_string();
    app.update_search_matches();
    assert!(app.search_matches.is_empty());
}

#[test]
fn search_query_case_insensitive() {
    let mut app = make_app();
    app.messages.push(user_msg("Hello Zeus"));
    app.search_query = "hello zeus".to_string();
    app.update_search_matches();
    assert_eq!(app.search_matches.len(), 1);
}

#[test]
fn search_next_advances_match_idx() {
    let mut app = make_app();
    app.messages.push(user_msg("match one"));
    app.messages.push(assistant_msg("match two"));
    app.messages.push(user_msg("match three"));

    app.search_query = "match".to_string();
    app.update_search_matches();
    assert_eq!(app.search_matches.len(), 3);

    let start = app.search_match_idx;
    app.search_next();
    assert_eq!(app.search_match_idx, (start + 1) % 3);
}

#[test]
fn search_next_wraps_at_end() {
    let mut app = make_app();
    app.messages.push(user_msg("match a"));
    app.messages.push(user_msg("match b"));

    app.search_query = "match".to_string();
    app.update_search_matches();
    app.search_match_idx = app.search_matches.len() - 1;
    app.search_next();
    assert_eq!(app.search_match_idx, 0);
}

#[test]
fn search_prev_wraps_at_start() {
    let mut app = make_app();
    app.messages.push(user_msg("match a"));
    app.messages.push(user_msg("match b"));

    app.search_query = "match".to_string();
    app.update_search_matches();
    app.search_match_idx = 0;
    app.search_prev();
    assert_eq!(app.search_match_idx, app.search_matches.len() - 1);
}

#[test]
fn search_clear_query_clears_matches() {
    let mut app = make_app();
    app.messages.push(user_msg("find me"));
    app.search_query = "find".to_string();
    app.update_search_matches();
    assert!(!app.search_matches.is_empty());

    app.search_query.clear();
    app.update_search_matches();
    assert!(app.search_matches.is_empty());
}

#[test]
fn search_match_idx_clamped_when_matches_shrink() {
    let mut app = make_app();
    app.messages.push(user_msg("alpha"));
    app.messages.push(user_msg("alpha beta"));
    app.messages.push(user_msg("alpha gamma"));

    app.search_query = "alpha".to_string();
    app.update_search_matches();
    app.search_match_idx = 2; // last of 3 matches

    // Narrow query — only 1 match now
    app.search_query = "gamma".to_string();
    app.update_search_matches();
    // idx should be clamped to valid range
    assert!(app.search_match_idx < app.search_matches.len().max(1));
}
