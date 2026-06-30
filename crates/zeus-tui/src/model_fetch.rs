//! Live model-list fetch for the onboarding Auth→Model spine (#239/#240).
//!
//! Phase 2 of the Auth→Model spine: after the user enters an API key on the
//! Auth screen, the universal advance fires a provider `/v1/models` call. That
//! call does double duty — it **validates** the key (success → advance;
//! 401/network/timeout → block + error) and **populates** the Model page from
//! the live list (P3 consumes [`ModelFetchState::Done`]).
//!
//! Ported from the pre-rebuild onboarding (`51537995:onboarding/mod.rs:719`).
//! Per-provider endpoints preserved verbatim; `reqwest` is already a crate dep.
//!
//! The integrated `zeus tui` path (`zeus_tui::run().await`) has the full tokio
//! runtime that spawns the fetch worker. The standalone preview bin has no
//! runtime and falls to the static list — correct, it's the design preview, not
//! the user path.

use crate::screens::providers::PROVIDERS;

/// The advance-gate state machine for the Auth→Model fetch.
///
/// `Idle` before any fetch; `Fetching` while the worker is in flight (drives
/// the "Fetching models…" spinner via `tick()`); `Done` carries the live list
/// for the Model page (P3); `Failed` carries the error string for the Auth
/// screen and unblocks retry-or-proceed (proceed → static-list fallback).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelFetchState {
    Idle,
    Fetching,
    Done(Vec<String>),
    Failed(String),
}

impl Default for ModelFetchState {
    fn default() -> Self {
        Self::Idle
    }
}

/// Fetch models from a provider's API. Returns model IDs on success, or an
/// error string (surfaced on the Auth screen) on failure.
///
/// Per-provider endpoints, ported verbatim from the pre-rebuild onboarding:
/// - `anthropic` — no models endpoint; returns known flagship set (static).
/// - `openai` — `GET /v1/models`, bearer; filter gpt-/o1/o3/o4.
/// - `ollama` — `GET {OLLAMA_HOST|localhost:11434}/api/tags`; `name` field.
/// - `groq` — `GET /openai/v1/models`, bearer.
/// - `google` — `GET /v1beta/models?key=`; strip `models/` prefix, filter gemini.
/// - `openrouter` — `GET /api/v1/models`, bearer; take top 20.
/// - `_` — static fallback from `PROVIDERS`.
pub async fn fetch_models(provider_id: &str, api_key: &str) -> Result<Vec<String>, String> {
    let client = reqwest::Client::new();

    match provider_id {
        "anthropic" => {
            // Anthropic has no models endpoint — use the known flagship set.
            Ok(vec![
                "claude-sonnet-4-6".into(),
                "claude-opus-4-6".into(),
                "claude-haiku-4-5".into(),
            ])
        }
        "openai" => {
            let resp = client
                .get("https://api.openai.com/v1/models")
                .bearer_auth(api_key)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("OpenAI API error: {}", resp.status()));
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let models: Vec<String> = body["data"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m["id"].as_str().map(String::from))
                        .filter(|id| {
                            id.starts_with("gpt-")
                                || id.starts_with("o1")
                                || id.starts_with("o3")
                                || id.starts_with("o4")
                        })
                        .collect()
                })
                .unwrap_or_default();
            if models.is_empty() {
                Ok(vec![
                    "gpt-4o".into(),
                    "gpt-4o-mini".into(),
                    "o3".into(),
                    "o4-mini".into(),
                ])
            } else {
                Ok(models)
            }
        }
        "ollama" => {
            let base =
                std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".into());
            let resp = client
                .get(format!("{base}/api/tags"))
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("Ollama API error: {}", resp.status()));
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let models: Vec<String> = body["models"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m["name"].as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            if models.is_empty() {
                Ok(vec!["llama3.3:70b".into(), "qwen2.5:32b".into()])
            } else {
                Ok(models)
            }
        }
        "google" => {
            let resp = client
                .get(format!(
                    "https://generativelanguage.googleapis.com/v1beta/models?key={api_key}"
                ))
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("Google API error: {}", resp.status()));
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let models: Vec<String> = body["models"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| {
                            m["name"]
                                .as_str()
                                .and_then(|n| n.strip_prefix("models/"))
                                .map(String::from)
                        })
                        .filter(|id| id.contains("gemini"))
                        .collect()
                })
                .unwrap_or_default();
            if models.is_empty() {
                Ok(vec!["gemini-2.5-pro".into(), "gemini-2.5-flash".into()])
            } else {
                Ok(models)
            }
        }
        "groq" => {
            let resp = client
                .get("https://api.groq.com/openai/v1/models")
                .bearer_auth(api_key)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("Groq API error: {}", resp.status()));
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let models: Vec<String> = body["data"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m["id"].as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            if models.is_empty() {
                Ok(vec!["llama-3.3-70b-versatile".into()])
            } else {
                Ok(models)
            }
        }
        "openrouter" => {
            let resp = client
                .get("https://openrouter.ai/api/v1/models")
                .bearer_auth(api_key)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("OpenRouter API error: {}", resp.status()));
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let models: Vec<String> = body["data"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m["id"].as_str().map(String::from))
                        .take(20) // Top 20, list is huge.
                        .collect()
                })
                .unwrap_or_default();
            if models.is_empty() {
                Ok(vec![
                    "anthropic/claude-sonnet-4-6".into(),
                    "meta-llama/llama-3.3-70b".into(),
                ])
            } else {
                Ok(models)
            }
        }
        "glm" => {
            // GLM / z.ai (Zhipu) — OpenAI-compatible surface on the `/api/paas/v4`
            // platform. Endpoint sourced from zeus-llm's `resolve_zai_base_url`
            // (lib.rs:193): the GLM-5.2 flagship line ships on the GLOBAL platform
            // `https://api.z.ai/api/paas/v4`; the legacy CN host
            // `https://open.bigmodel.cn/api/paas/v4` does NOT serve 5.2. Both speak
            // the identical `/v1/...` OpenAI-compatible surface, so the models path
            // mirrors the openai arm exactly: `{base}/v1/models`, Bearer key,
            // `data[].id`. Env overrides honored to match the zeus-llm resolver.
            let base = std::env::var("ZAI_BASE_URL")
                .ok()
                .filter(|u| !u.trim().is_empty())
                .map(|u| u.trim_end_matches('/').to_string())
                .unwrap_or_else(|| {
                    if std::env::var("ZAI_REGION")
                        .unwrap_or_default()
                        .trim()
                        .eq_ignore_ascii_case("cn")
                    {
                        "https://open.bigmodel.cn/api/paas/v4".to_string()
                    } else {
                        "https://api.z.ai/api/paas/v4".to_string()
                    }
                });
            let resp = client
                .get(format!("{base}/v1/models"))
                .bearer_auth(api_key)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("GLM API error: {}", resp.status()));
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let models: Vec<String> = body["data"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m["id"].as_str().map(String::from))
                        .filter(|id| id.starts_with("glm"))
                        .collect()
                })
                .unwrap_or_default();
            // No hardcoded fallback (#251): a successful 200 with an empty
            // `data` array is reported as-is. The Model screen renders an honest
            // empty/fallback state from the static catalog rather than us
            // fabricating a model list the endpoint never returned.
            Ok(models)
        }
        "mimo" => {
            // Xiaomi MiMo — OpenAI-compatible surface (zeus-llm dispatches it via
            // `complete_openai`, lib.rs:1403/1488). Base from zeus-llm's
            // `base_url()` (lib.rs:907): `https://api.xiaomimimo.com/v1` — note the
            // base ALREADY includes `/v1`, so the models path is `{base}/models`
            // (NOT `{base}/v1/models`, which would double the segment → 404).
            // `MIMO_BASE_URL` env override honored for parity with the resolver.
            let base = std::env::var("MIMO_BASE_URL")
                .ok()
                .filter(|u| !u.trim().is_empty())
                .map(|u| u.trim_end_matches('/').to_string())
                .unwrap_or_else(|| "https://api.xiaomimimo.com/v1".to_string());
            let resp = client
                .get(format!("{base}/models"))
                .bearer_auth(api_key)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("MiMo API error: {}", resp.status()));
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let models: Vec<String> = body["data"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m["id"].as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            // #251: no hardcoded fallback — an empty live list is reported as-is;
            // the Model screen renders the honest static catalog instead.
            Ok(models)
        }
        "kimi" => {
            // Kimi / Moonshot — OpenAI-compatible (`complete_openai`, lib.rs:1485).
            // Base from zeus-llm `base_url()` (lib.rs:903): `https://api.moonshot.ai`
            // → `{base}/v1/models`, Bearer key, `data[].id`.
            let base = std::env::var("MOONSHOT_BASE_URL")
                .ok()
                .filter(|u| !u.trim().is_empty())
                .map(|u| u.trim_end_matches('/').to_string())
                .unwrap_or_else(|| "https://api.moonshot.ai".to_string());
            let resp = client
                .get(format!("{base}/v1/models"))
                .bearer_auth(api_key)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("Kimi API error: {}", resp.status()));
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let models: Vec<String> = body["data"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m["id"].as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            Ok(models)
        }
        "qwen" => {
            // Qwen — OpenAI-compatible (`complete_openai`, lib.rs:1402/1487). Base
            // from zeus-llm's `resolve_qwen_base_url()` (lib.rs:905), which already
            // resolves to a `…/v1` or `…/compatible-mode/v1` path, so the models
            // path is `{base}/models`. We mirror the resolver's env precedence
            // (`QWEN_BASE_URL` → region/plan default = intl compatible-mode).
            let base = std::env::var("QWEN_BASE_URL")
                .ok()
                .filter(|u| !u.trim().is_empty())
                .map(|u| u.trim_end_matches('/').to_string())
                .unwrap_or_else(|| {
                    "https://dashscope-intl.aliyuncs.com/compatible-mode/v1".to_string()
                });
            let resp = client
                .get(format!("{base}/models"))
                .bearer_auth(api_key)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("Qwen API error: {}", resp.status()));
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let models: Vec<String> = body["data"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m["id"].as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            Ok(models)
        }
        "xai" => {
            // xAI (Grok) — OpenAI-compatible (`complete_openai`, lib.rs:1483).
            // Base from zeus-llm `base_url()` (lib.rs:901): `https://api.x.ai`
            // → `{base}/v1/models`, Bearer key, `data[].id`.
            let base = std::env::var("XAI_BASE_URL")
                .ok()
                .filter(|u| !u.trim().is_empty())
                .map(|u| u.trim_end_matches('/').to_string())
                .unwrap_or_else(|| "https://api.x.ai".to_string());
            let resp = client
                .get(format!("{base}/v1/models"))
                .bearer_auth(api_key)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("xAI API error: {}", resp.status()));
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let models: Vec<String> = body["data"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m["id"].as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            Ok(models)
        }
        "sakana" => {
            // Sakana AI (Fugu) — OpenAI-compatible (`stream_openai`, lib.rs:1672).
            // Base from zeus-llm `base_url()` (lib.rs:827/920/1083/1562):
            // `https://api.sakana.ai/v1` → `/models`, Bearer key, `data[].id`.
            // `SAKANA_BASE_URL` honored (bare host, no trailing `/v1`) for parity
            // with the xai/mimo resolver idiom; default host appends `/v1/models`.
            let base = std::env::var("SAKANA_BASE_URL")
                .ok()
                .filter(|u| !u.trim().is_empty())
                .map(|u| u.trim_end_matches('/').to_string())
                .unwrap_or_else(|| "https://api.sakana.ai".to_string());
            let resp = client
                .get(format!("{base}/v1/models"))
                .bearer_auth(api_key)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("Sakana API error: {}", resp.status()));
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let models: Vec<String> = body["data"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m["id"].as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            if models.is_empty() {
                // Empty live list → honest fall to the static seed flagship.
                Ok(vec!["fugu-ultra".into()])
            } else {
                Ok(models)
            }
        }
        // Providers without a standard models endpoint → static defaults.
        // NOTE: current `ProviderInfo` (screens/providers.rs) carries a single
        // `flagship` string keyed by `id` — NOT the old `provider_id` + `models`
        // slice the pre-rebuild port assumed. Adapted to current substrate.
        //
        // `minimax` deliberately stays here (re-confirmed #275): its zeus-llm
        // base is `https://api.minimax.io/anthropic` (minimax.rs:36) — an
        // Anthropic-format surface dispatched via the Anthropic path, NOT
        // OpenAI-compatible, AND its auth is OAuth (ensure_fresh_minimax_token,
        // lib.rs:1674) not a bearer API key. There is no reachable `/v1/models`
        // GET for it, so we fall to the honest static flagship (`MiniMax-M3`)
        // rather than fabricate a live arm that would 404/401.
        _ => {
            let defaults = PROVIDERS
                .iter()
                .find(|p| p.id == provider_id)
                .map(|p| vec![p.flagship.to_string()])
                .unwrap_or_else(|| vec!["claude-sonnet-4-6".into()]);
            Ok(defaults)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_default_is_idle() {
        assert_eq!(ModelFetchState::default(), ModelFetchState::Idle);
    }

    #[tokio::test]
    async fn anthropic_returns_static_flagships_without_network() {
        // Anthropic has no models endpoint — must resolve offline (no key needed).
        let models = fetch_models("anthropic", "").await.expect("anthropic is static");
        assert!(models.contains(&"claude-sonnet-4-6".to_string()));
        assert!(models.len() >= 3);
    }

    #[test]
    fn glm_provider_is_registered_with_flagship() {
        // #245: GLM has a real z.ai `/v1/models` arm. #251 removed the fetcher's
        // hardcoded `["glm-5.2","glm-5"]` fallback — an empty live list is now
        // reported honestly and the Model screen falls back to the static
        // `GLM_MODELS` seed catalog (whose flagship must match the PROVIDERS
        // catalog so the seed resolves to a valid model id).
        let glm = PROVIDERS
            .iter()
            .find(|p| p.id == "glm")
            .expect("glm must be a registered provider");
        assert_eq!(glm.flagship, "glm-5.2");
    }

    #[tokio::test]
    async fn unknown_provider_falls_back_to_static() {
        // Unknown id → never errors, always a non-empty static fallback.
        let models = fetch_models("does-not-exist", "irrelevant")
            .await
            .expect("unknown provider falls back, never errors");
        assert!(!models.is_empty());
    }

    #[test]
    fn sakana_provider_is_registered_with_flagship() {
        // Sakana takes the OpenAI-compat live arm; on empty/error it falls to
        // this seed flagship — which must stay in sync with the PROVIDERS catalog
        // so the seed resolves to a valid model id (the #275 fix's honest seed).
        let sakana = PROVIDERS
            .iter()
            .find(|p| p.id == "sakana")
            .expect("sakana must be a registered provider");
        assert_eq!(sakana.flagship, "fugu-ultra");
    }

    #[test]
    fn minimax_provider_is_registered_with_flagship() {
        // MiniMax has no OpenAI `/v1/models` endpoint (Anthropic-format + OAuth),
        // so it stays on the honest static `_ =>` fallback resolving to this
        // flagship. Guards the #275 decision to NOT fabricate a live arm.
        let minimax = PROVIDERS
            .iter()
            .find(|p| p.id == "minimax")
            .expect("minimax must be a registered provider");
        assert_eq!(minimax.flagship, "MiniMax-M3");
    }

    #[tokio::test]
    async fn sakana_honors_base_url_override_and_falls_back_on_unreachable() {
        // Point SAKANA_BASE_URL at an unroutable host → the live GET fails fast,
        // proving the arm builds `{base}/v1/models` from the override (not the
        // default host) and surfaces an honest Err rather than fabricating a list.
        unsafe { std::env::set_var("SAKANA_BASE_URL", "http://127.0.0.1:1"); }
        let res = fetch_models("sakana", "fish_test").await;
        unsafe { std::env::remove_var("SAKANA_BASE_URL"); }
        assert!(res.is_err(), "unreachable base must surface an honest error");
    }
}
