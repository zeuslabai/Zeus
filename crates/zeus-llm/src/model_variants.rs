//! Model variant suffix resolution (F5).
//!
//! Lets users write ergonomic model strings like `:fast`, `:cheap`,
//! `:quality` (or a bare tier name) and have them resolved to a concrete
//! `provider/model` pair from the user's `[model_routing]` config.
//!
//! Examples:
//! - `":fast"`       → `model_routing.speed`
//! - `":cheap"`      → `model_routing.speed` (fallback to `general`)
//! - `":quality"`    → `model_routing.reasoning`
//! - `"anthropic/claude-sonnet-4:quality"` — suffix wins, resolves to
//!   whatever `reasoning` points at; if routing is disabled or unset,
//!   the prefix (`anthropic/claude-sonnet-4`) is kept verbatim.
//! - `"openai/gpt-5"` — no suffix, returned unchanged.
//!
//! Unknown suffixes pass through unchanged and are logged once at DEBUG.

use zeus_core::ModelRoutingCoreConfig as RoutingConfig;

/// Known variant suffixes (all case-insensitive, `:`-prefixed or bare).
const FAST: &str = "fast";
const CHEAP: &str = "cheap";
const QUALITY: &str = "quality";

/// Resolve a model string that may contain a variant suffix.
///
/// Returns the input verbatim when:
/// - No `:` suffix is present (and the string isn't a bare tier name).
/// - Routing is disabled.
/// - The suffix is unknown.
/// - The matching routing entry is empty AND no fallback prefix exists.
pub fn resolve_variant(input: &str, routing: &RoutingConfig) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return input.to_string();
    }

    // Split on the LAST ':' so "anthropic/claude-sonnet-4:quality" splits
    // correctly (the provider prefix uses '/', not ':').
    let (prefix, suffix) = match trimmed.rsplit_once(':') {
        Some((p, s)) => (p, s.to_lowercase()),
        None => {
            // Bare tier? (`:fast` with no colon, e.g. user typed just `fast`)
            let lower = trimmed.to_lowercase();
            if is_known_variant(&lower) {
                ("", lower)
            } else {
                return input.to_string();
            }
        }
    };

    if !is_known_variant(&suffix) {
        return input.to_string();
    }

    if !routing.enabled {
        // Routing off — strip the suffix and return the prefix if present,
        // else pass the input back unchanged (can't resolve).
        return if prefix.is_empty() {
            input.to_string()
        } else {
            prefix.to_string()
        };
    }

    let resolved: Option<&String> = match suffix.as_str() {
        FAST => routing.speed.as_ref().or(routing.general.as_ref()),
        CHEAP => routing
            .speed
            .as_ref()
            .or(routing.general.as_ref()),
        QUALITY => routing
            .reasoning
            .as_ref()
            .or(routing.general.as_ref()),
        _ => None,
    };

    match resolved {
        Some(m) if !m.is_empty() => {
            tracing::debug!(
                "model variant ':{}' resolved to '{}'",
                suffix,
                m
            );
            m.clone()
        }
        _ => {
            // No resolution available — prefer prefix, else original.
            if !prefix.is_empty() {
                prefix.to_string()
            } else {
                input.to_string()
            }
        }
    }
}

fn is_known_variant(s: &str) -> bool {
    matches!(s, FAST | CHEAP | QUALITY)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routing() -> RoutingConfig {
        RoutingConfig {
            enabled: true,
            reasoning: Some("anthropic/claude-opus-4-20250514".into()),
            code: Some("anthropic/claude-sonnet-4-20250514".into()),
            research: Some("google/gemini-2.0-flash".into()),
            speed: Some("groq/llama-3.3-70b-versatile".into()),
            creative: Some("anthropic/claude-sonnet-4-20250514".into()),
            review: Some("anthropic/claude-sonnet-4-20250514".into()),
            general: Some("anthropic/claude-sonnet-4-20250514".into()),
        }
    }

    #[test]
    fn passthrough_when_no_suffix() {
        let r = routing();
        assert_eq!(
            resolve_variant("openai/gpt-5", &r),
            "openai/gpt-5"
        );
    }

    #[test]
    fn resolves_fast_prefixed() {
        let r = routing();
        assert_eq!(
            resolve_variant(":fast", &r),
            "groq/llama-3.3-70b-versatile"
        );
    }

    #[test]
    fn resolves_quality_prefixed() {
        let r = routing();
        assert_eq!(
            resolve_variant(":quality", &r),
            "anthropic/claude-opus-4-20250514"
        );
    }

    #[test]
    fn resolves_cheap_falls_back_to_speed() {
        let r = routing();
        assert_eq!(
            resolve_variant(":cheap", &r),
            "groq/llama-3.3-70b-versatile"
        );
    }

    #[test]
    fn suffix_wins_over_explicit_model() {
        let r = routing();
        assert_eq!(
            resolve_variant("anthropic/claude-sonnet-4:quality", &r),
            "anthropic/claude-opus-4-20250514"
        );
    }

    #[test]
    fn bare_tier_name_resolves() {
        let r = routing();
        assert_eq!(
            resolve_variant("fast", &r),
            "groq/llama-3.3-70b-versatile"
        );
        assert_eq!(
            resolve_variant("QUALITY", &r),
            "anthropic/claude-opus-4-20250514"
        );
    }

    #[test]
    fn unknown_suffix_passes_through() {
        let r = routing();
        assert_eq!(
            resolve_variant("anthropic/claude-3:weird", &r),
            "anthropic/claude-3:weird"
        );
    }

    #[test]
    fn routing_disabled_strips_suffix() {
        let mut r = routing();
        r.enabled = false;
        assert_eq!(
            resolve_variant("anthropic/claude-sonnet-4:fast", &r),
            "anthropic/claude-sonnet-4"
        );
    }

    #[test]
    fn routing_disabled_bare_variant_passes_through() {
        let mut r = routing();
        r.enabled = false;
        // No prefix to fall back to, no routing — keep input.
        assert_eq!(resolve_variant(":fast", &r), ":fast");
    }

    #[test]
    fn missing_routing_entry_falls_back_to_general() {
        let mut r = routing();
        r.speed = None;
        assert_eq!(
            resolve_variant(":fast", &r),
            "anthropic/claude-sonnet-4-20250514"
        );
    }

    #[test]
    fn empty_input_passthrough() {
        let r = routing();
        assert_eq!(resolve_variant("", &r), "");
    }

    #[test]
    fn case_insensitive_suffix() {
        let r = routing();
        assert_eq!(
            resolve_variant("openai/gpt-5:FAST", &r),
            "groq/llama-3.3-70b-versatile"
        );
    }
}
