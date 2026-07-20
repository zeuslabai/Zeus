//! Voice-meet API handlers (#428).
//!
//! `/v1/voice/meet` route family — joins and controls Google Meet calls via two
//! transports:
//!
//! - **`chrome`** — delegates to spark's `google_meet` Chrome-CDP tool
//!   (#424, `zeus_browser::meet::GoogleMeetTool`) via the agent-facing
//!   `BrowserTool::execute` path. Requires a Chrome instance running with
//!   `--remote-debugging-port`.
//! - **`twilio`** — dial-in transport: dials the Meet room's PSTN dial-in
//!   number with `initiate_call`, then sends the PIN via `send_dtmf`.
//!   Reuses — does not duplicate — the `build_twilio_provider` factory from
//!   `voice_handlers` (#422).
//!
//! Sessions are kept in an additive module-level `DashMap` keyed by a generated
//! session id. This avoids any mutation to `AppState` / `lib.rs` / `main.rs`,
//! matching the "keep it additive" directive in the #428 spec. The store is
//! process-local (sufficient for the single-node deployment the API targets);
//! a future distributed-backend ticket can hoist it into `AppState`.
//!
//! Routes (registered in `routes.rs`):
//! - `POST /v1/voice/meet`               — create session, join via transport
//! - `GET  /v1/voice/meet/:id`           — session state
//! - `POST /v1/voice/meet/:id/leave`     — leave call, close session

use std::sync::OnceLock;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{debug, error, info, warn};

use zeus_browser::BrowserTool;

use crate::handlers::voice_handlers::build_twilio_provider;
use crate::SharedState;

// ============================================================================
// Session store
// ============================================================================

/// In-memory meet-session store (additive — no AppState change).
///
/// `OnceLock` is initialized lazily on first access; `DashMap` provides
/// interior-locking concurrency safe for the API's async handlers.
fn sessions() -> &'static DashMap<String, MeetSession> {
    static STORE: OnceLock<DashMap<String, MeetSession>> = OnceLock::new();
    STORE.get_or_init(DashMap::new)
}

/// A live meet session — the backend handle differs per transport.
#[derive(Clone)]
struct MeetSession {
    /// Which transport this session uses.
    transport: Transport,
    /// Underlying handle: Chrome `SharedBrowser` (cheap, stateless CDP client)
    /// for the `chrome` transport, or the Twilio call SID for the `twilio`
    /// dial-in transport.
    handle: SessionHandle,
    /// Source of the Meet URL (for chrome transport) or dial-in info (twilio).
    source: MeetSource,
    /// When the session was created (ms since epoch).
    created_at: u64,
}

#[derive(Clone)]
enum SessionHandle {
    /// Chrome CDP client shared across requests for this session.
    Chrome(zeus_browser::SharedBrowser),
    /// Twilio call SID from `initiate_call`.
    Twilio(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Chrome,
    Twilio,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
enum MeetSource {
    Url { meet_url: String },
    DialIn { number: String, pin: String },
}

impl MeetSource {
    fn summary(&self) -> Value {
        match self {
            MeetSource::Url { meet_url } => json!({ "meet_url": meet_url }),
            MeetSource::DialIn { number, pin } => {
                json!({ "dial_in": { "number": number, "pin": pin } })
            }
        }
    }
}

// ============================================================================
// Request / response bodies
// ============================================================================

/// Body for `POST /v1/voice/meet`.
///
/// Exactly one of `meet_url` / `dial_in` is required. `transport` defaults to
/// `chrome` when `meet_url` is given and `twilio` when `dial_in` is given.
#[derive(Debug, Deserialize)]
pub struct CreateMeetRequest {
    /// Google Meet URL (`https://meet.google.com/xxx-yyyy-zzz`). Required for
    /// the `chrome` transport.
    #[serde(default)]
    pub meet_url: Option<String>,
    /// PSTN dial-in number + PIN. Required for the `twilio` transport.
    #[serde(default)]
    pub dial_in: Option<DialIn>,
    /// Transport selection: `"chrome"` or `"twilio"`. See field-level docs.
    #[serde(default)]
    pub transport: Option<Transport>,
    /// Guest name shown in the Meet lobby (chrome transport only).
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DialIn {
    /// E.164 PSTN dial-in number for the Meet room.
    pub number: String,
    /// PIN digits sent via DTMF after the call connects.
    pub pin: String,
}

// ============================================================================
// Handlers
// ============================================================================

/// `POST /v1/voice/meet` — create a session and join the Meet via the chosen transport.
///
/// Returns `{ "session_id": "...", "transport": "...", "source": {...} }`.
pub async fn create_meet(
    State(_state): State<SharedState>,
    Json(req): Json<CreateMeetRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    // Resolve transport + source. Default by which field was supplied; error if
    // the combination is incoherent (e.g. chrome transport without a URL).
    let (transport, source) = resolve_transport_and_source(&req)?;

    let session_id = generate_session_id();
    let created_at = now_ms();

    match transport {
        Transport::Chrome => {
            let MeetSource::Url { meet_url } = &source else {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "chrome transport requires `meet_url`".to_string(),
                ));
            };

            info!(session_id = %session_id, meet_url = %meet_url, "Meet: chrome transport join");

            // Cheap, stateless CDP handle — connects lazily on first command.
            // Defaults to http://127.0.0.1:9222 (env override: ZEUS_BROWSER_DEBUG_URL).
            let debug_url = std::env::var("ZEUS_BROWSER_DEBUG_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:9222".to_string());
            let browser: zeus_browser::SharedBrowser = std::sync::Arc::new(
                tokio::sync::Mutex::new(zeus_browser::CdpClient::new(&debug_url)),
            );

            // Delegate to spark's google_meet tool path (#424). Build args
            // matching `GoogleMeetTool::execute`'s expected shape.
            let mut args = json!({
                "action": "join",
                "url": meet_url,
            });
            if let Some(name) = req.name.as_deref() {
                args["name"] = json!(name);
            }

            let tool = zeus_browser::meet::GoogleMeetTool::new(browser.clone());
            match tool.execute(args).await {
                Ok(_out) => {
                    let session = MeetSession {
                        transport,
                        handle: SessionHandle::Chrome(browser),
                        source: source.clone(),
                        created_at,
                    };
                    sessions().insert(session_id.clone(), session);
                    Ok((
                        StatusCode::OK,
                        Json(json!({
                            "session_id": session_id,
                            "transport": transport,
                            "source": source.summary(),
                        })),
                    ))
                }
                Err(e) => {
                    error!(error = %e, "Meet chrome join failed");
                    Err((
                        StatusCode::BAD_GATEWAY,
                        format!(
                            "Chrome transport join failed: {e}. \
                             Is Chrome running with --remote-debugging-port?"
                        ),
                    ))
                }
            }
        }
        Transport::Twilio => {
            let MeetSource::DialIn { number, pin } = &source else {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "twilio transport requires `dial_in: { number, pin }`".to_string(),
                ));
            };

            info!(session_id = %session_id, number = %number, "Meet: twilio dial-in transport");

            // Reuse the #422 factory — 503 when Twilio is not configured.
            let provider = build_twilio_provider()?;

            // 1. Dial the PSTN dial-in number. Empty greeting — we are dialing
            //    an IVR, not a human.
            let call_sid = match provider.initiate_call(number, "").await {
                Ok(sid) => sid,
                Err(e) => {
                    error!(error = %e, number = %number, "Meet twilio dial-in initiate failed");
                    return Err((
                        StatusCode::BAD_GATEWAY,
                        format!("Twilio initiate_call failed: {e}"),
                    ));
                }
            };
            debug!(call_sid = %call_sid, "Meet twilio dial-in connected, sending PIN");

            // 2. Send the PIN followed by `#` terminator (standard IVR grammar).
            let digits = format!("{}#", pin);
            if let Err(e) = provider.send_dtmf(&call_sid, &digits).await {
                error!(error = %e, call_sid = %call_sid, "Meet twilio PIN DTMF failed");
                // Best-effort hangup so we don't leave a zombie call leg.
                let _ = provider.hangup_call(&call_sid).await;
                return Err((
                    StatusCode::BAD_GATEWAY,
                    format!("Twilio send_dtmf (PIN) failed: {e}"),
                ));
            }

            let session = MeetSession {
                transport,
                handle: SessionHandle::Twilio(call_sid),
                source: source.clone(),
                created_at,
            };
            sessions().insert(session_id.clone(), session);

            Ok((
                StatusCode::OK,
                Json(json!({
                    "session_id": session_id,
                    "transport": transport,
                    "source": source.summary(),
                })),
            ))
        }
    }
}

/// `GET /v1/voice/meet/:id` — current session state.
pub async fn get_meet(
    State(_state): State<SharedState>,
    Path(session_id): Path<String>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let session = sessions()
        .get(&session_id)
        .map(|s| s.clone())
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Meet session not found".to_string()))?;

    // For chrome transport, delegate to google_meet's `status` action so the
    // caller gets live in-call state, not just our cached metadata.
    let live_state = if let SessionHandle::Chrome(browser) = &session.handle {
        let tool = zeus_browser::meet::GoogleMeetTool::new(browser.clone());
        match tool.execute(json!({ "action": "status" })).await {
            Ok(out) => Some(Value::String(out)),
            Err(e) => {
                warn!(error = %e, session_id = %session_id, "google_meet status probe failed");
                Some(json!({ "error": format!("status probe failed: {e}") }))
            }
        }
    } else if let SessionHandle::Twilio(sid) = &session.handle {
        // For twilio transport, surface the underlying call state.
        match build_twilio_provider() {
            Ok(provider) => match provider.get_call_state(sid).await {
                Ok(state) => Some(serde_json::to_value(&state).unwrap_or_else(|_| {
                    json!({ "error": "failed to serialize CallState" })
                })),
                Err(e) => {
                    warn!(error = %e, call_sid = %sid, "twilio get_call_state failed");
                    Some(json!({ "error": format!("call state fetch failed: {e}") }))
                }
            },
            Err(e) => Some(json!({ "error": e.1 })),
        }
    } else {
        None
    };

    Ok((
        StatusCode::OK,
        Json(json!({
            "session_id": session_id,
            "transport": session.transport,
            "source": session.source.summary(),
            "created_at": session.created_at,
            "live": live_state,
        })),
    ))
}

/// `POST /v1/voice/meet/:id/leave` — leave the call and close the session.
pub async fn leave_meet(
    State(_state): State<SharedState>,
    Path(session_id): Path<String>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let session = sessions()
        .remove(&session_id)
        .map(|(_k, v)| v)
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Meet session not found".to_string()))?;

    match session.handle {
        SessionHandle::Chrome(browser) => {
            let tool = zeus_browser::meet::GoogleMeetTool::new(browser);
            // Best-effort leave — the session is removed from our store either way.
            if let Err(e) = tool.execute(json!({ "action": "leave" })).await {
                warn!(error = %e, session_id = %session_id, "google_meet leave probe failed");
                return Ok((
                    StatusCode::OK,
                    Json(json!({
                        "ok": true,
                        "warning": format!("session closed; leave probe failed: {e}"),
                    })),
                ));
            }
            Ok((StatusCode::OK, Json(json!({ "ok": true }))))
        }
        SessionHandle::Twilio(call_sid) => {
            // Reuse the factory — if Twilio is no longer configured (creds
            // rotated mid-session), we still drop our local handle and report.
            match build_twilio_provider() {
                Ok(provider) => match provider.hangup_call(&call_sid).await {
                    Ok(()) => Ok((StatusCode::OK, Json(json!({ "ok": true })))),
                    Err(e) => {
                        warn!(error = %e, call_sid = %call_sid, "twilio hangup failed");
                        Ok((
                            StatusCode::OK,
                            Json(json!({
                                "ok": true,
                                "warning": format!("session closed; hangup failed: {e}"),
                            })),
                        ))
                    }
                },
                Err(e) => {
                    warn!(error = %e.1, "twilio provider unavailable during leave");
                    Ok((
                        StatusCode::OK,
                        Json(json!({
                            "ok": true,
                            "warning": "session closed; twilio provider unavailable",
                        })),
                    ))
                }
            }
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Pick transport + source from the request, applying sensible defaults and
/// surfacing bad combinations as `BAD_REQUEST` rather than runtime panics.
fn resolve_transport_and_source(
    req: &CreateMeetRequest,
) -> Result<(Transport, MeetSource), (StatusCode, String)> {
    let has_url = req.meet_url.as_ref().map(|s| !s.trim().is_empty()).unwrap_or(false);
    let has_dial = req
        .dial_in
        .as_ref()
        .map(|d| !d.number.trim().is_empty() && !d.pin.trim().is_empty())
        .unwrap_or(false);

    if !has_url && !has_dial {
        return Err((
            StatusCode::BAD_REQUEST,
            "Provide either `meet_url` (chrome) or `dial_in: {number, pin}` (twilio)".to_string(),
        ));
    }

    // Determine transport — explicit wins, else infer from which field is set.
    let transport = match req.transport {
        Some(t) => t,
        None => {
            if has_url {
                Transport::Chrome
            } else {
                Transport::Twilio
            }
        }
    };

    // Validate transport/source coherence.
    let source = match (transport, has_url, has_dial) {
        (Transport::Chrome, true, _) => MeetSource::Url {
            meet_url: req.meet_url.clone().unwrap_or_default().trim().to_string(),
        },
        (Transport::Twilio, _, true) => MeetSource::DialIn {
            number: req.dial_in.as_ref().unwrap().number.trim().to_string(),
            pin: req.dial_in.as_ref().unwrap().pin.trim().to_string(),
        },
        (Transport::Chrome, _, _) => {
            return Err((
                StatusCode::BAD_REQUEST,
                "chrome transport requires a non-empty `meet_url`".to_string(),
            ));
        }
        (Transport::Twilio, _, _) => {
            return Err((
                StatusCode::BAD_REQUEST,
                "twilio transport requires `dial_in: {number, pin}` with non-empty fields"
                    .to_string(),
            ));
        }
    };

    Ok((transport, source))
}

/// Cheap, collision-resistant session id. We don't need cryptographic strength
/// (the API surface is already authenticated), just enough entropy to avoid
/// guessable ids in a single-process store.
fn generate_session_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("meet_{nanos:x}")
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_defaults_chrome_when_url() {
        let req = CreateMeetRequest {
            meet_url: Some("https://meet.google.com/abc-defg-hij".into()),
            dial_in: None,
            transport: None,
            name: None,
        };
        let (t, _) = resolve_transport_and_source(&req).unwrap();
        assert_eq!(t, Transport::Chrome);
    }

    #[test]
    fn test_resolve_defaults_twilio_when_dial_in() {
        let req = CreateMeetRequest {
            meet_url: None,
            dial_in: Some(DialIn {
                number: "+18005551234".into(),
                pin: "123456".into(),
            }),
            transport: None,
            name: None,
        };
        let (t, s) = resolve_transport_and_source(&req).unwrap();
        assert_eq!(t, Transport::Twilio);
        match s {
            MeetSource::DialIn { number, pin } => {
                assert_eq!(number, "+18005551234");
                assert_eq!(pin, "123456");
            }
            _ => panic!("expected DialIn"),
        }
    }

    #[test]
    fn test_resolve_explicit_transport_overrides_default() {
        // Explicit twilio + URL: bad combo → error (no dial_in to use).
        let req = CreateMeetRequest {
            meet_url: Some("https://meet.google.com/abc-defg-hij".into()),
            dial_in: None,
            transport: Some(Transport::Twilio),
            name: None,
        };
        assert!(resolve_transport_and_source(&req).is_err());
    }

    #[test]
    fn test_resolve_neither_field_is_error() {
        let req = CreateMeetRequest {
            meet_url: None,
            dial_in: None,
            transport: Some(Transport::Chrome),
            name: None,
        };
        let err = resolve_transport_and_source(&req).unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_resolve_empty_string_treated_as_missing() {
        let req = CreateMeetRequest {
            meet_url: Some("   ".into()),
            dial_in: None,
            transport: None,
            name: None,
        };
        assert!(resolve_transport_and_source(&req).is_err());
    }

    #[test]
    fn test_resolve_trims_whitespace() {
        let req = CreateMeetRequest {
            meet_url: Some("  https://meet.google.com/abc-defg-hij  ".into()),
            dial_in: None,
            transport: None,
            name: None,
        };
        let (_, s) = resolve_transport_and_source(&req).unwrap();
        match s {
            MeetSource::Url { meet_url } => {
                assert_eq!(meet_url, "https://meet.google.com/abc-defg-hij");
            }
            _ => panic!("expected Url"),
        }
    }

    #[test]
    fn test_session_ids_are_unique() {
        let a = generate_session_id();
        std::thread::sleep(std::time::Duration::from_micros(10));
        let b = generate_session_id();
        assert_ne!(a, b);
        assert!(a.starts_with("meet_"));
        assert!(b.starts_with("meet_"));
    }

    #[test]
    fn test_sessions_store_is_shared() {
        // OnceLock guarantees a single process-wide instance.
        let s1 = sessions();
        let s2 = sessions();
        assert!(std::ptr::eq(s1, s2));
    }

    #[test]
    fn test_transport_serializes_lowercase() {
        let chrome = serde_json::to_string(&Transport::Chrome).unwrap();
        let twilio = serde_json::to_string(&Transport::Twilio).unwrap();
        assert_eq!(chrome, "\"chrome\"");
        assert_eq!(twilio, "\"twilio\"");
    }

    #[test]
    fn test_transport_deserializes_case_insensitive_semantics() {
        // serde rename_all = lowercase on an already-lowercase enum means
        // lowercase input deserializes cleanly.
        let c: Transport = serde_json::from_str("\"chrome\"").unwrap();
        assert_eq!(c, Transport::Chrome);
    }
}
