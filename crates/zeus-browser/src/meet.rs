//! Google Meet automation via Chrome DevTools Protocol.
//!
//! Provides a `google_meet` agent tool that joins and controls Google Meet calls
//! through a running Chrome instance (headless or visible). The approach ports the
//! proven probe logic from the OpenClaw `google-meet` reference extension: rather
//! than relying on Meet's obfuscated CSS class names (which change frequently), it
//! locates controls by their stable `aria-label` / `data-tooltip` / inner text via
//! an injected JavaScript probe evaluated through [`CdpClient::evaluate`].
//!
//! Supported actions:
//! - `join`     — navigate to a Meet URL, fill the guest name, handle the mic/cam
//!   pre-join prompts, and click Join / Ask to join.
//! - `status`   — report in-call state, mic/cam mute state, and lobby/kick/deny reasons.
//! - `mute` / `unmute` — toggle the microphone.
//! - `cam_off` / `cam_on` — toggle the camera.
//! - `captions` — enable captions, scrape the live caption DOM, and return a transcript.
//! - `leave`    — click the leave-call control.

use async_trait::async_trait;
use serde_json::Value;
use zeus_core::{Error, Result, ToolSchema};

use crate::tools::{BrowserTool, SharedBrowser};

/// `google_meet` agent tool backed by a shared Chrome CDP client.
pub struct GoogleMeetTool {
    pub browser: SharedBrowser,
}

impl GoogleMeetTool {
    pub fn new(browser: SharedBrowser) -> Self {
        Self { browser }
    }
}

/// The set of supported `google_meet` actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeetAction {
    Join,
    Status,
    Mute,
    Unmute,
    CamOff,
    CamOn,
    Captions,
    Leave,
}

impl MeetAction {
    /// Parse an action string (case-insensitive). Returns `None` for unknown actions.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "join" => Some(Self::Join),
            "status" => Some(Self::Status),
            "mute" => Some(Self::Mute),
            "unmute" => Some(Self::Unmute),
            "cam_off" | "camoff" | "camera_off" => Some(Self::CamOff),
            "cam_on" | "camon" | "camera_on" => Some(Self::CamOn),
            "captions" | "transcript" => Some(Self::Captions),
            "leave" | "hangup" | "hang_up" => Some(Self::Leave),
            _ => None,
        }
    }

    /// All valid action names, for error messages and schema docs.
    pub const ALL: &'static [&'static str] = &[
        "join", "status", "mute", "unmute", "cam_off", "cam_on", "captions", "leave",
    ];
}

/// Serialize a Rust string into a safe JavaScript string literal.
///
/// Uses `serde_json` to produce a properly quoted+escaped JS string, so any
/// guest name / URL is injection-safe when embedded into the probe script.
fn js_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

/// Shared JavaScript prelude: helpers for finding controls by accessible label.
///
/// Meet renders its UI with obfuscated class names, but keeps stable
/// `aria-label` / `data-tooltip` attributes for accessibility. We match against
/// those (plus inner text) so the probe survives DOM reshuffles.
const JS_PRELUDE: &str = r#"
const text = (node) => (node?.innerText || node?.textContent || "").trim();
const buttons = [...document.querySelectorAll('button')];
const buttonLabel = (b) =>
  [b.getAttribute("aria-label"), b.getAttribute("data-tooltip"), text(b)]
    .filter(Boolean).join(" ");
const findButton = (re) =>
  buttons.find((b) => re.test(buttonLabel(b)) && !b.disabled);
const pageText = () => document.body?.innerText || "";
const inCall = buttons.some((b) => /leave call/i.test(buttonLabel(b)));
const callControls = document.querySelector('[role="region"][aria-label="Call controls"]');
const controlButtons = callControls ? [...callControls.querySelectorAll('button')] : buttons;
const findControl = (re) => controlButtons.find((b) => re.test(buttonLabel(b)));
const micBtn = findControl(/microphone/i);
const camBtn = findControl(/camera/i);
const isOn = (b, onRe) => b ? onRe.test(buttonLabel(b)) : undefined;
const micMuted = isOn(micBtn, /turn on microphone/i);
const camMuted = isOn(camBtn, /turn on camera/i);
const lobbyWaiting = !inCall && /asking to be let in|you.?ll join when someone lets you in|waiting to be let in|ask to join/i.test(pageText());
const denied = /you can.t join this call|can.t join this meeting|no one responded|your request to join was denied|you.ve been removed/i.test(pageText());
const knocked = /someone will let you in|ask to join/i.test(pageText());
"#;

/// Build the status result object snippet (shared across actions).
const JS_STATUS_OBJ: &str = r#"
({
  inCall,
  micMuted,
  camMuted,
  lobbyWaiting,
  denied,
  knocked,
  url: location.href
})
"#;

/// Build the JS probe for a given action. Returns the full expression to evaluate.
///
/// The returned expression evaluates to a JSON-serializable object (returned via
/// CDP `returnByValue`), which we then surface to the agent as a JSON string.
pub fn build_probe(action: MeetAction, guest_name: Option<&str>) -> String {
    let name = js_str(guest_name.unwrap_or("Zeus"));
    let body = match action {
        MeetAction::Join => format!(
            r#"
// Fill the guest name if a name field is present (pre-join lobby).
const nameInput = [...document.querySelectorAll('input')].find((el) =>
  /your name/i.test(el.getAttribute('aria-label') || el.placeholder || ''));
if (nameInput) {{
  nameInput.focus();
  nameInput.value = {name};
  nameInput.dispatchEvent(new Event('input', {{ bubbles: true }}));
}}
// Turn OFF mic + cam before joining for a quiet, observe-safe entry.
const preMic = findButton(/turn off microphone/i);
if (preMic) preMic.click();
const preCam = findButton(/turn off camera/i);
if (preCam) preCam.click();
// Click Join / Ask to join.
const joinBtn = findButton(/join now|ask to join|switch here/i);
if (joinBtn) joinBtn.click();
JSON.stringify({{ clickedJoin: Boolean(joinBtn), named: Boolean(nameInput), ...{status} }})
"#,
            name = name,
            status = JS_STATUS_OBJ
        ),
        MeetAction::Status => format!("JSON.stringify({})", JS_STATUS_OBJ),
        MeetAction::Mute => control_toggle("microphone", true),
        MeetAction::Unmute => control_toggle("microphone", false),
        MeetAction::CamOff => control_toggle("camera", true),
        MeetAction::CamOn => control_toggle("camera", false),
        MeetAction::Captions => captions_probe(),
        MeetAction::Leave => format!(
            r#"
const leaveBtn = findButton(/leave call|hang up/i);
if (leaveBtn) leaveBtn.click();
JSON.stringify({{ clickedLeave: Boolean(leaveBtn), ...{status} }})
"#,
            status = JS_STATUS_OBJ
        ),
    };

    // Wrap in an IIFE returning a JSON string for returnByValue.
    format!("(() => {{{}\n{}\n}})()", JS_PRELUDE, body)
}

/// Build a mic/cam toggle probe. `target_muted=true` means "end in the muted state".
fn control_toggle(kind: &str, target_muted: bool) -> String {
    // The aria-label reflects the *available action*: "Turn off microphone" means
    // the mic is currently ON; "Turn on microphone" means it is currently OFF.
    // To reach the target state we click the button whose label matches the
    // transition toward that target (if it's already there, there's nothing to do).
    let (click_re, state_var) = match (kind, target_muted) {
        ("microphone", true) => ("/turn off microphone/i", "micMuted"),
        ("microphone", false) => ("/turn on microphone/i", "micMuted"),
        ("camera", true) => ("/turn off camera/i", "camMuted"),
        _ => ("/turn on camera/i", "camMuted"),
    };
    format!(
        r#"
const btn = findButton({re});
const before = {state};
if (btn) btn.click();
JSON.stringify({{
  control: {kind},
  targetMuted: {want},
  clicked: Boolean(btn),
  beforeMuted: before,
  ...{status}
}})
"#,
        re = click_re,
        state = state_var,
        kind = js_str(kind),
        want = target_muted,
        status = JS_STATUS_OBJ
    )
}

/// Build the captions probe: enable captions, install a MutationObserver-backed
/// transcript buffer on `window.__zeusMeetCaptions`, and return collected lines.
fn captions_probe() -> String {
    r#"
const capBtn = findButton(/turn on captions|show captions|captions/i);
let captionsEnabledAttempted = false;
if (capBtn && /turn on captions|show captions/i.test(buttonLabel(capBtn))) {
  capBtn.click();
  captionsEnabledAttempted = true;
}
const captionSelector = '[role="region"][aria-label*="aption" i], [aria-live="polite"][role="region"], div[aria-live="polite"]';
const w = window;
if (!w.__zeusMeetCaptions) {
  w.__zeusMeetCaptions = { lines: [], seen: {} };
}
const st = w.__zeusMeetCaptions;
const recordCaption = (speaker, t) => {
  const clean = String(t || "").replace(/\s+/g, " ").trim();
  const sp = String(speaker || "").replace(/\s+/g, " ").trim();
  if (!clean || clean.length < 2) return;
  if (/^(turn on captions|turn off captions|captions)$/i.test(clean)) return;
  const key = (sp + "\n" + clean).toLowerCase();
  if (st.seen[key]) return;
  st.seen[key] = true;
  st.lines.push({ speaker: sp, text: clean, at: Date.now() });
  if (st.lines.length > 1000) st.lines.shift();
};
const scrape = () => {
  document.querySelectorAll(captionSelector).forEach((region) => {
    const raw = text(region);
    if (!raw) return;
    const pieces = raw.split(/\n+/).map((s) => s.trim()).filter(Boolean);
    if (pieces.length >= 2) recordCaption(pieces[0], pieces.slice(1).join(" "));
    else recordCaption("", pieces[0] || raw);
  });
};
if (inCall && !st.observerInstalled) {
  st.observerInstalled = true;
  new MutationObserver(scrape).observe(document.body, { childList: true, subtree: true, characterData: true });
}
if (inCall) scrape();
const lines = st.lines || [];
const recent = lines.slice(-50);
JSON.stringify({
  captionsEnabledAttempted,
  captioning: document.querySelector(captionSelector) !== null || lines.length > 0,
  transcriptLines: lines.length,
  recentTranscript: recent,
  transcript: recent.map((l) => (l.speaker ? l.speaker + ": " : "") + l.text).join("\n"),
  ...STATUS
})
"#
    .replace("STATUS", JS_STATUS_OBJ)
}

#[async_trait]
impl BrowserTool for GoogleMeetTool {
    fn name(&self) -> &'static str {
        "google_meet"
    }

    fn description(&self) -> &'static str {
        "Join and control a Google Meet call via Chrome. Actions: join (open the Meet URL, set your name, handle mic/cam prompts, click Join), status, mute/unmute the mic, cam_on/cam_off the camera, captions (enable captions and return a live transcript), leave. Requires Chrome running with --remote-debugging-port."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "action",
                "string",
                "The Meet action: join | status | mute | unmute | cam_on | cam_off | captions | leave",
                true,
            )
            .with_param(
                "url",
                "string",
                "Google Meet URL (https://meet.google.com/xxx-yyyy-zzz). Required for action=join.",
                false,
            )
            .with_param(
                "name",
                "string",
                "Guest display name for action=join (default: Zeus).",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let action_str = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing required parameter: action".to_string()))?;
        let action = MeetAction::parse(action_str).ok_or_else(|| {
            Error::Tool(format!(
                "Unknown action '{}'. Valid: {}",
                action_str,
                MeetAction::ALL.join(", ")
            ))
        })?;

        let browser = self.browser.lock().await;

        // join requires navigating to the Meet URL first.
        if action == MeetAction::Join {
            let url = args
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    Error::Tool("Missing required parameter: url (for action=join)".to_string())
                })?;
            if !url.contains("meet.google.com") {
                return Err(Error::Tool(format!(
                    "url does not look like a Google Meet link: {}",
                    url
                )));
            }
            browser
                .navigate(url)
                .await
                .map_err(|e| Error::Tool(format!("Failed to navigate to Meet URL: {}", e)))?;
            // Give the Meet SPA a moment to render its pre-join lobby before probing.
            tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
        }

        let guest_name = args.get("name").and_then(|v| v.as_str());
        let probe = build_probe(action, guest_name);
        let result = browser
            .evaluate(&probe)
            .await
            .map_err(|e| Error::Tool(format!("Meet probe failed: {}", e)))?;

        let out = result.as_str().unwrap_or("").to_string();
        Ok(format!("google_meet action={} result: {}", action_str, out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_parse_valid() {
        assert_eq!(MeetAction::parse("join"), Some(MeetAction::Join));
        assert_eq!(MeetAction::parse("STATUS"), Some(MeetAction::Status));
        assert_eq!(MeetAction::parse("mute"), Some(MeetAction::Mute));
        assert_eq!(MeetAction::parse("unmute"), Some(MeetAction::Unmute));
        assert_eq!(MeetAction::parse("cam_off"), Some(MeetAction::CamOff));
        assert_eq!(MeetAction::parse("camera_on"), Some(MeetAction::CamOn));
        assert_eq!(MeetAction::parse("transcript"), Some(MeetAction::Captions));
        assert_eq!(MeetAction::parse("leave"), Some(MeetAction::Leave));
    }

    #[test]
    fn test_action_parse_invalid() {
        assert_eq!(MeetAction::parse("explode"), None);
        assert_eq!(MeetAction::parse(""), None);
    }

    #[test]
    fn test_join_probe_is_injection_safe() {
        let evil = "x\");alert(1);//";
        let probe = build_probe(MeetAction::Join, Some(evil));
        // The raw evil payload must never appear unescaped in the script.
        assert!(!probe.contains(evil));
        // But its JSON-escaped form must be present.
        assert!(probe.contains(&js_str(evil)));
        assert!(probe.contains("join now"));
        assert!(probe.contains("clickedJoin"));
    }

    #[test]
    fn test_status_probe_shape() {
        let probe = build_probe(MeetAction::Status, None);
        assert!(probe.contains("inCall"));
        assert!(probe.contains("micMuted"));
        assert!(probe.contains("camMuted"));
        assert!(probe.contains("lobbyWaiting"));
        assert!(probe.contains("JSON.stringify"));
    }

    #[test]
    fn test_mute_unmute_probes() {
        let mute = build_probe(MeetAction::Mute, None);
        assert!(mute.contains("/turn off microphone/i"));
        let unmute = build_probe(MeetAction::Unmute, None);
        assert!(unmute.contains("/turn on microphone/i"));
    }

    #[test]
    fn test_cam_probes() {
        let off = build_probe(MeetAction::CamOff, None);
        assert!(off.contains("/turn off camera/i"));
        let on = build_probe(MeetAction::CamOn, None);
        assert!(on.contains("/turn on camera/i"));
    }

    #[test]
    fn test_captions_probe() {
        let probe = build_probe(MeetAction::Captions, None);
        assert!(probe.contains("__zeusMeetCaptions"));
        assert!(probe.contains("MutationObserver"));
        assert!(probe.contains("recentTranscript"));
        assert!(probe.contains("turn on captions"));
    }

    #[test]
    fn test_leave_probe() {
        let probe = build_probe(MeetAction::Leave, None);
        assert!(probe.contains("leave call"));
        assert!(probe.contains("clickedLeave"));
    }

    #[test]
    fn test_prelude_uses_aria_labels() {
        // The whole resilience strategy hinges on aria-label matching, not classes.
        assert!(JS_PRELUDE.contains("aria-label"));
        assert!(JS_PRELUDE.contains("data-tooltip"));
        assert!(JS_PRELUDE.contains("Call controls"));
    }
}
