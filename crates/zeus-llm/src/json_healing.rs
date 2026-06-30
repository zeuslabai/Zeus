//! JSON Response Healing Middleware
//!
//! LLMs (especially via OpenRouter with smaller/open-weight models) frequently
//! return malformed JSON in tool call arguments or response content: trailing
//! commas, unescaped newlines, markdown fences, stray prose, truncated braces,
//! single quotes, etc. This module attempts a series of progressively more
//! aggressive repairs to recover a valid JSON value without losing intent.
//!
//! Usage:
//! ```ignore
//! use crate::json_healing::heal_json;
//! let val = heal_json(raw_str).unwrap_or(serde_json::json!({}));
//! ```
//!
//! Repairs applied (in order):
//! 1. Strip markdown code fences (```json ... ```).
//! 2. Extract the outermost `{...}` or `[...]` block from surrounding prose.
//! 3. Remove trailing commas before `}` / `]`.
//! 4. Replace unescaped control characters inside strings (newlines/tabs).
//! 5. Convert Python-ish literals (`True` / `False` / `None` / single-quoted).
//! 6. Balance unclosed brackets/braces by appending the missing closers.

use serde_json::Value;

/// Attempt to parse `raw` as JSON, applying healing transformations on failure.
/// Returns `None` if even the most aggressive repair fails.
pub fn heal_json(raw: &str) -> Option<Value> {
    // Fast path
    if let Ok(v) = serde_json::from_str::<Value>(raw) {
        return Some(v);
    }

    let candidates = [
        strip_markdown_fences(raw),
        extract_json_block(raw),
        strip_and_extract(raw),
        repair_common(raw),
        repair_common(&strip_and_extract(raw)),
        balance_brackets(&repair_common(&strip_and_extract(raw))),
    ];

    for cand in candidates.iter() {
        if cand.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(cand) {
            return Some(v);
        }
    }
    None
}

/// Public helper: heal to a Value, or return `{}` as a last resort.
/// Reports `true` in the bool slot if healing was required (i.e. raw parse failed).
pub fn heal_json_or_empty(raw: &str) -> (Value, bool) {
    if let Ok(v) = serde_json::from_str::<Value>(raw) {
        return (v, false);
    }
    match heal_json(raw) {
        Some(v) => (v, true),
        None => (serde_json::json!({}), true),
    }
}

fn strip_markdown_fences(s: &str) -> String {
    let t = s.trim();
    // ```json ... ``` or ``` ... ```
    if let Some(rest) = t.strip_prefix("```") {
        let rest = rest.trim_start_matches(|c: char| c.is_alphanumeric());
        let rest = rest.trim_start_matches(|c: char| c == '\n' || c == '\r');
        if let Some(end) = rest.rfind("```") {
            return rest[..end].trim().to_string();
        }
        return rest.trim().to_string();
    }
    t.to_string()
}

/// Find the outermost balanced `{...}` or `[...]` substring.
fn extract_json_block(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut start = None;
    let mut open_ch = b'\0';
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'{' || b == b'[' {
            start = Some(i);
            open_ch = b;
            break;
        }
    }
    let Some(start) = start else {
        return String::new();
    };
    let close_ch = if open_ch == b'{' { b'}' } else { b']' };

    let mut depth = 0i32;
    let mut in_str = false;
    let mut escape = false;
    let mut end = None;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if escape {
            escape = false;
            continue;
        }
        if in_str {
            if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            c if c == open_ch => depth += 1,
            c if c == close_ch => {
                depth -= 1;
                if depth == 0 {
                    end = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    match end {
        Some(e) => s[start..=e].to_string(),
        None => s[start..].to_string(), // unclosed — let balance_brackets handle it
    }
}

fn strip_and_extract(s: &str) -> String {
    extract_json_block(&strip_markdown_fences(s))
}

/// Apply common syntactic repairs that don't require JSON-awareness of structure.
fn repair_common(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut in_str = false;
    let mut escape = false;
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];

        if escape {
            out.push(b as char);
            escape = false;
            i += 1;
            continue;
        }

        if in_str {
            match b {
                b'\\' => {
                    out.push('\\');
                    escape = true;
                }
                b'"' => {
                    out.push('"');
                    in_str = false;
                }
                b'\n' => out.push_str("\\n"),
                b'\r' => out.push_str("\\r"),
                b'\t' => out.push_str("\\t"),
                _ => out.push(b as char),
            }
            i += 1;
            continue;
        }

        // Outside a string
        match b {
            b'"' => {
                out.push('"');
                in_str = true;
            }
            b'\'' => {
                // Convert single-quoted string → double-quoted.
                out.push('"');
                in_str = true;
                // We'll flip the closing single quote below by intercepting b'\''
                // via a small trick: treat single-quote as double-quote in this branch.
                // Easiest: scan ahead to matching ' and rewrite.
                let mut j = i + 1;
                while j < bytes.len() {
                    let c = bytes[j];
                    if c == b'\\' {
                        out.push('\\');
                        if j + 1 < bytes.len() {
                            out.push(bytes[j + 1] as char);
                            j += 2;
                            continue;
                        }
                    } else if c == b'\'' {
                        out.push('"');
                        in_str = false;
                        j += 1;
                        break;
                    } else if c == b'\n' {
                        out.push_str("\\n");
                    } else if c == b'\r' {
                        out.push_str("\\r");
                    } else if c == b'\t' {
                        out.push_str("\\t");
                    } else if c == b'"' {
                        out.push_str("\\\"");
                    } else {
                        out.push(c as char);
                    }
                    j += 1;
                }
                i = j;
                continue;
            }
            _ => {
                // Python-ish literals outside strings
                if starts_with_word(bytes, i, b"True") {
                    out.push_str("true");
                    i += 4;
                    continue;
                }
                if starts_with_word(bytes, i, b"False") {
                    out.push_str("false");
                    i += 5;
                    continue;
                }
                if starts_with_word(bytes, i, b"None") {
                    out.push_str("null");
                    i += 4;
                    continue;
                }
                out.push(b as char);
            }
        }
        i += 1;
    }

    // Strip trailing commas: ,] or ,}  (possibly with whitespace)
    strip_trailing_commas(&out)
}

fn starts_with_word(bytes: &[u8], i: usize, word: &[u8]) -> bool {
    if i + word.len() > bytes.len() {
        return false;
    }
    if &bytes[i..i + word.len()] != word {
        return false;
    }
    // boundary check — previous and next char must not be alphanum/_
    let prev_ok = i == 0 || !is_word_char(bytes[i - 1]);
    let next_ok =
        i + word.len() == bytes.len() || !is_word_char(bytes[i + word.len()]);
    prev_ok && next_ok
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn strip_trailing_commas(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut in_str = false;
    let mut escape = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if escape {
            out.push(b as char);
            escape = false;
            i += 1;
            continue;
        }
        if in_str {
            if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_str = false;
            }
            out.push(b as char);
            i += 1;
            continue;
        }
        if b == b'"' {
            in_str = true;
            out.push(b as char);
            i += 1;
            continue;
        }
        if b == b',' {
            // look ahead past whitespace
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b'}' || bytes[j] == b']') {
                // skip comma
                i += 1;
                continue;
            }
        }
        out.push(b as char);
        i += 1;
    }
    out
}

/// Append missing `}` / `]` closers to a truncated JSON string.
fn balance_brackets(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    let mut stack: Vec<u8> = Vec::new();
    let mut in_str = false;
    let mut escape = false;
    for &b in s.as_bytes() {
        if escape {
            escape = false;
            continue;
        }
        if in_str {
            if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => stack.push(b'}'),
            b'[' => stack.push(b']'),
            b'}' | b']' => {
                if stack.last().copied() == Some(b) {
                    stack.pop();
                }
            }
            _ => {}
        }
    }
    let mut out = s.to_string();
    if in_str {
        out.push('"');
    }
    while let Some(c) = stack.pop() {
        out.push(c as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn passes_valid_json_through() {
        let v = heal_json(r#"{"a":1,"b":"x"}"#).unwrap();
        assert_eq!(v, json!({"a":1,"b":"x"}));
    }

    #[test]
    fn strips_markdown_fences() {
        let raw = "```json\n{\"a\":1}\n```";
        assert_eq!(heal_json(raw).unwrap(), json!({"a":1}));
    }

    #[test]
    fn strips_trailing_commas() {
        let raw = r#"{"a":1, "b":2,}"#;
        assert_eq!(heal_json(raw).unwrap(), json!({"a":1,"b":2}));
    }

    #[test]
    fn extracts_from_prose() {
        let raw = "Sure, here you go: {\"name\":\"Mike\"} — hope this helps!";
        assert_eq!(heal_json(raw).unwrap(), json!({"name":"Mike"}));
    }

    #[test]
    fn converts_python_literals() {
        let raw = r#"{"ok": True, "err": False, "x": None}"#;
        assert_eq!(heal_json(raw).unwrap(), json!({"ok":true,"err":false,"x":null}));
    }

    #[test]
    fn converts_single_quotes() {
        let raw = r#"{'name': 'Zeus', 'rank': 1}"#;
        assert_eq!(heal_json(raw).unwrap(), json!({"name":"Zeus","rank":1}));
    }

    #[test]
    fn balances_missing_closers() {
        let raw = r#"{"a":1, "b":{"c":2"#;
        let v = heal_json(raw).unwrap();
        assert_eq!(v, json!({"a":1,"b":{"c":2}}));
    }

    #[test]
    fn heals_unescaped_newlines_in_strings() {
        let raw = "{\"msg\":\"line1\nline2\"}";
        let v = heal_json(raw).unwrap();
        assert_eq!(v["msg"], "line1\nline2");
    }

    #[test]
    fn returns_none_for_total_garbage() {
        assert!(heal_json("totally not json at all").is_none());
    }

    #[test]
    fn heal_or_empty_flags_healing() {
        let (_, healed) = heal_json_or_empty(r#"{"a":1}"#);
        assert!(!healed);
        let (_, healed) = heal_json_or_empty(r#"{"a":1,}"#);
        assert!(healed);
        let (v, healed) = heal_json_or_empty("garbage");
        assert!(healed);
        assert_eq!(v, json!({}));
    }

    #[test]
    fn heals_tool_call_arguments_shape() {
        // Realistic malformed tool-call args from a small model
        let raw = "```json\n{\"path\": \"/tmp/x\", \"content\": \"hello\nworld\",}\n```";
        let v = heal_json(raw).unwrap();
        assert_eq!(v["path"], "/tmp/x");
        assert_eq!(v["content"], "hello\nworld");
    }
}
