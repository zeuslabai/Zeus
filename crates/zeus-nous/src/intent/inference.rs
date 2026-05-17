//! Intent Inference - Infers implicit meaning and context

use super::*;
use crate::UserContext;

/// Infers implicit context and meaning from intent
pub struct IntentInference;

impl IntentInference {
    /// Create a new inference engine
    pub fn new() -> Self {
        Self
    }

    /// Infer implicit context for an intent
    pub fn infer(&self, intent: &mut Intent, ctx: &UserContext) -> zeus_core::Result<()> {
        // Infer from user preferences
        self.infer_from_preferences(intent, ctx);

        // Infer from patterns
        self.infer_from_patterns(intent, ctx);

        // Infer from common sense
        self.infer_common_sense(intent);

        // Infer missing parameters
        self.infer_missing_params(intent, ctx);

        Ok(())
    }

    /// Find related past intents
    pub fn find_related(&self, intent: &mut Intent, ctx: &UserContext) -> zeus_core::Result<()> {
        // Check recent topics for relevance
        for topic in &ctx.recent_topics {
            let topic_lower = topic.to_lowercase();
            let input_lower = intent.raw_input.to_lowercase();

            // Check for word overlap
            let topic_words: std::collections::HashSet<_> =
                topic_lower.split_whitespace().collect();
            let input_words: std::collections::HashSet<_> =
                input_lower.split_whitespace().collect();

            let overlap: Vec<_> = topic_words.intersection(&input_words).collect();
            if overlap.len() >= 2 {
                intent.implicit_context.push(ImplicitContext {
                    inference: format!("Related to recent topic: {}", topic),
                    source: InferenceSource::RecentContext,
                    confidence: Confidence::medium(),
                });
            }
        }

        Ok(())
    }

    /// Infer from user preferences
    fn infer_from_preferences(&self, intent: &mut Intent, ctx: &UserContext) {
        match &mut intent.intent_type {
            IntentType::Schedule { when, .. } => {
                // If no time specified, check preference
                if when.is_none()
                    && let Some(pref) = ctx.has_preference("meeting_time")
                {
                    intent.implicit_context.push(ImplicitContext {
                        inference: format!("User prefers meetings in the {}", pref.value),
                        source: InferenceSource::UserPreference,
                        confidence: Confidence(pref.confidence),
                    });
                }

                // Check for preferred meeting duration
                if let Some(pref) = ctx.has_preference("meeting_duration") {
                    intent.implicit_context.push(ImplicitContext {
                        inference: format!("Default meeting duration: {}", pref.value),
                        source: InferenceSource::UserPreference,
                        confidence: Confidence(pref.confidence),
                    });
                }
            }
            IntentType::Communicate { channel, .. } => {
                // If no channel specified, check preference
                if channel.is_none()
                    && let Some(pref) = ctx.has_preference("preferred_channel")
                {
                    *channel = Some(pref.value.clone());
                    intent.implicit_context.push(ImplicitContext {
                        inference: format!("Using preferred channel: {}", pref.value),
                        source: InferenceSource::UserPreference,
                        confidence: Confidence(pref.confidence),
                    });
                }
            }
            _ => {}
        }
    }

    /// Infer from learned patterns
    fn infer_from_patterns(&self, intent: &mut Intent, ctx: &UserContext) {
        for pattern in &ctx.patterns {
            let matches = match &pattern.trigger {
                crate::PatternTrigger::Keyword(kw) => {
                    intent.raw_input.to_lowercase().contains(&kw.to_lowercase())
                }
                crate::PatternTrigger::Temporal(when) => {
                    // Check if current time matches
                    // Simplified: just check if "morning"/"afternoon" etc. is mentioned
                    intent
                        .raw_input
                        .to_lowercase()
                        .contains(&when.to_lowercase())
                }
                crate::PatternTrigger::Context(ctx_trigger) => intent
                    .raw_input
                    .to_lowercase()
                    .contains(&ctx_trigger.to_lowercase()),
                crate::PatternTrigger::Event(_) => false, // Would need event tracking
            };

            if matches && pattern.confidence > 0.6 {
                intent.implicit_context.push(ImplicitContext {
                    inference: format!(
                        "Pattern detected ({}x observed): {}",
                        pattern.observations, pattern.typical_action
                    ),
                    source: InferenceSource::LearnedPattern,
                    confidence: Confidence(pattern.confidence),
                });
            }
        }
    }

    /// Apply common sense inference
    fn infer_common_sense(&self, intent: &mut Intent) {
        let input_lower = intent.raw_input.to_lowercase();

        // Meeting with team implies team members
        if matches!(&intent.intent_type, IntentType::Schedule { what, .. } if what.contains("team"))
        {
            intent.implicit_context.push(ImplicitContext {
                inference: "Team meeting implies including team members".to_string(),
                source: InferenceSource::CommonSense,
                confidence: Confidence::high(),
            });
        }

        // "Quick" implies short duration
        if input_lower.contains("quick") {
            intent.implicit_context.push(ImplicitContext {
                inference: "'Quick' suggests 15-30 minute duration".to_string(),
                source: InferenceSource::CommonSense,
                confidence: Confidence::medium(),
            });
        }

        // "Important" or "critical" increases urgency
        if (input_lower.contains("important") || input_lower.contains("critical"))
            && intent.urgency < 0.8
        {
            intent.urgency = 0.8;
        }

        // "All" or "everyone" implies comprehensive scope
        if input_lower.contains("all ")
            || input_lower.contains("everyone")
            || input_lower.contains("everything")
        {
            intent.implicit_context.push(ImplicitContext {
                inference: "Comprehensive scope requested".to_string(),
                source: InferenceSource::CommonSense,
                confidence: Confidence::high(),
            });
        }

        // "Same as" or "like last time" implies repetition
        if input_lower.contains("same as")
            || input_lower.contains("like last time")
            || input_lower.contains("again")
        {
            intent.implicit_context.push(ImplicitContext {
                inference: "User wants to repeat a previous action".to_string(),
                source: InferenceSource::CommonSense,
                confidence: Confidence::high(),
            });
        }

        // "Not" or "don't" indicates negative intent
        if input_lower.contains("don't ")
            || input_lower.contains("not ")
            || input_lower.contains("never ")
        {
            intent.implicit_context.push(ImplicitContext {
                inference: "Negative/exclusion constraint detected".to_string(),
                source: InferenceSource::CommonSense,
                confidence: Confidence::high(),
            });
        }
    }

    /// Infer missing parameters based on context
    fn infer_missing_params(&self, intent: &mut Intent, ctx: &UserContext) {
        match &mut intent.intent_type {
            IntentType::Schedule { what, when, who } => {
                // If scheduling with a specific person, they should be invited
                for entity in &intent.entities {
                    if entity.entity_type == ExtractedEntityType::Person {
                        let name = entity
                            .resolved
                            .clone()
                            .unwrap_or_else(|| entity.text.clone());
                        if who.is_none() {
                            *who = Some(vec![name]);
                        } else if let Some(people) = who
                            && !people.contains(&name)
                        {
                            people.push(name);
                        }
                    }
                }

                // If "lunch" mentioned, infer time
                if what.to_lowercase().contains("lunch") && when.is_none() {
                    *when = Some("around noon".to_string());
                    intent.implicit_context.push(ImplicitContext {
                        inference: "Lunch typically around noon".to_string(),
                        source: InferenceSource::CommonSense,
                        confidence: Confidence::medium(),
                    });
                }

                // If "coffee" mentioned, infer shorter duration
                if what.to_lowercase().contains("coffee") {
                    intent.implicit_context.push(ImplicitContext {
                        inference: "Coffee meetings are typically 30 minutes".to_string(),
                        source: InferenceSource::CommonSense,
                        confidence: Confidence::medium(),
                    });
                }
            }
            IntentType::Communicate {
                to,
                about: _,
                channel,
            } => {
                // Infer channel from recipient context
                if channel.is_none() && !to.is_empty() {
                    for recipient in to.iter() {
                        if let Some(entity) = ctx.get_entity(recipient) {
                            // Check if we know their preferred channel
                            if let Some(pref_channel) = entity.attributes.get("preferred_channel")
                                && let Some(ch) = pref_channel.as_str()
                            {
                                *channel = Some(ch.to_string());
                                intent.implicit_context.push(ImplicitContext {
                                    inference: format!("{} prefers {}", recipient, ch),
                                    source: InferenceSource::KnownEntity,
                                    confidence: Confidence::medium(),
                                });
                                break;
                            }
                        }
                    }
                }
            }
            IntentType::Search { scope, .. } => {
                // Infer scope from recent context
                if scope.is_none() && !ctx.recent_topics.is_empty() {
                    // Check if any recent topic is a project
                    for topic in &ctx.recent_topics {
                        if let Some(entity) = ctx.get_entity(topic)
                            && matches!(entity.entity_type, crate::EntityType::Project)
                        {
                            *scope = Some(topic.clone());
                            intent.implicit_context.push(ImplicitContext {
                                inference: format!("Searching within context of: {}", topic),
                                source: InferenceSource::RecentContext,
                                confidence: Confidence::low(),
                            });
                            break;
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

impl Default for IntentInference {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_common_sense_quick() {
        let inference = IntentInference::new();
        let mut intent = Intent::new(
            "schedule a quick meeting",
            IntentType::Schedule {
                what: "quick meeting".to_string(),
                when: None,
                who: None,
            },
            Confidence::high(),
        );

        inference.infer_common_sense(&mut intent);

        assert!(
            intent
                .implicit_context
                .iter()
                .any(|c| c.inference.contains("15-30 minute"))
        );
    }

    #[test]
    fn test_common_sense_urgency() {
        let inference = IntentInference::new();
        let mut intent = Intent::new(
            "this is important, do it now",
            IntentType::Execute {
                action: "something".to_string(),
                parameters: vec![],
            },
            Confidence::high(),
        );
        intent.urgency = 0.5;

        inference.infer_common_sense(&mut intent);

        assert!(intent.urgency >= 0.8);
    }

    #[test]
    fn test_lunch_time_inference() {
        let inference = IntentInference::new();
        let ctx = UserContext::default();

        let mut intent = Intent::new(
            "schedule lunch with the team",
            IntentType::Schedule {
                what: "lunch".to_string(),
                when: None,
                who: None,
            },
            Confidence::high(),
        );

        inference.infer_missing_params(&mut intent, &ctx);

        if let IntentType::Schedule { when, .. } = &intent.intent_type {
            assert!(when.is_some());
            assert!(
                when.as_ref()
                    .expect("as_ref should succeed")
                    .contains("noon")
            );
        }
    }
}
