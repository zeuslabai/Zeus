//! Context Resolution - Resolves entities and references against known context

use super::*;
use crate::UserContext;

/// Resolves ambiguous references against user context
pub struct ContextResolver;

impl ContextResolver {
    /// Create a new context resolver
    pub fn new() -> Self {
        Self
    }

    /// Resolve entities in the intent against known context
    pub fn resolve(&self, intent: &mut Intent, ctx: &UserContext) -> zeus_core::Result<()> {
        // Resolve each entity
        for entity in &mut intent.entities {
            self.resolve_entity(entity, ctx);
        }

        // Resolve implicit references in the intent type
        self.resolve_intent_references(intent, ctx);

        Ok(())
    }

    /// Resolve a single entity
    fn resolve_entity(&self, entity: &mut ExtractedEntity, ctx: &UserContext) {
        match entity.entity_type {
            ExtractedEntityType::Person => {
                // Try to match against known people
                if let Some(known) = ctx.get_entity(&entity.text) {
                    entity.resolved = Some(known.name.clone());
                    entity.confidence = Confidence::high();
                } else {
                    // Check for partial matches
                    for known in &ctx.entities {
                        if matches!(known.entity_type, crate::EntityType::Person)
                            && (self.fuzzy_match(&entity.text, &known.name)
                                || known
                                    .aliases
                                    .iter()
                                    .any(|a| self.fuzzy_match(&entity.text, a)))
                        {
                            entity.resolved = Some(known.name.clone());
                            entity.confidence = Confidence::medium();
                            break;
                        }
                    }
                }
            }
            ExtractedEntityType::Project => {
                if let Some(known) = ctx.get_entity(&entity.text) {
                    entity.resolved = Some(known.name.clone());
                    entity.confidence = Confidence::high();
                }
            }
            _ => {}
        }
    }

    /// Resolve references within the intent type itself
    fn resolve_intent_references(&self, intent: &mut Intent, ctx: &UserContext) {
        match &mut intent.intent_type {
            IntentType::Schedule {
                who: Some(people), ..
            } => {
                for person in people.iter_mut() {
                    if let Some(known) = ctx.get_entity(person) {
                        *person = known.name.clone();
                    }
                }
            }
            IntentType::Schedule { .. } => {}
            IntentType::Communicate { to, .. } => {
                for recipient in to.iter_mut() {
                    if let Some(known) = ctx.get_entity(recipient) {
                        *recipient = known.name.clone();
                    }
                }
            }
            _ => {}
        }

        // Resolve pronouns and references
        self.resolve_pronouns(intent, ctx);
    }

    /// Resolve pronouns like "him", "her", "them", "it", "that"
    fn resolve_pronouns(&self, intent: &mut Intent, ctx: &UserContext) {
        let input_lower = intent.raw_input.to_lowercase();

        // "the usual" - check for patterns
        if (input_lower.contains("the usual") || input_lower.contains("as usual"))
            && let Some(pattern) = self.find_matching_pattern(&input_lower, ctx)
        {
            intent.implicit_context.push(ImplicitContext {
                inference: format!("'the usual' refers to: {}", pattern.typical_action),
                source: InferenceSource::LearnedPattern,
                confidence: Confidence(pattern.confidence),
            });
        }

        // "that project" / "the project" - reference recent topic
        if (input_lower.contains("that project") || input_lower.contains("the project"))
            && let Some(project) = ctx.recent_topics.iter().find(|t| t.contains("project"))
        {
            intent.implicit_context.push(ImplicitContext {
                inference: format!("'the project' refers to: {}", project),
                source: InferenceSource::RecentContext,
                confidence: Confidence::medium(),
            });
        }

        // "him"/"her"/"them" - reference recent person
        if input_lower.contains(" him ")
            || input_lower.contains(" her ")
            || input_lower.contains(" them ")
        {
            // Find most recently mentioned person
            let recent_people: Vec<_> = ctx
                .entities
                .iter()
                .filter(|e| matches!(e.entity_type, crate::EntityType::Person))
                .take(1)
                .collect();

            if let Some(person) = recent_people.first() {
                intent.implicit_context.push(ImplicitContext {
                    inference: format!("Pronoun likely refers to: {}", person.name),
                    source: InferenceSource::RecentContext,
                    confidence: Confidence::low(),
                });
            }
        }
    }

    /// Find a matching pattern for "the usual" type references
    fn find_matching_pattern<'a>(
        &self,
        input: &str,
        ctx: &'a UserContext,
    ) -> Option<&'a crate::Pattern> {
        ctx.patterns.iter().find(|p| match &p.trigger {
            crate::PatternTrigger::Keyword(kw) => input.contains(&kw.to_lowercase()),
            _ => false,
        })
    }

    /// Simple fuzzy matching for names
    fn fuzzy_match(&self, input: &str, target: &str) -> bool {
        let input_lower = input.to_lowercase();
        let target_lower = target.to_lowercase();

        // Exact match
        if input_lower == target_lower {
            return true;
        }

        // Prefix match (e.g., "Mike" matches "Michael")
        if target_lower.starts_with(&input_lower) || input_lower.starts_with(&target_lower) {
            return true;
        }

        // Contains match (e.g., "Smith" matches "John Smith")
        if target_lower.contains(&input_lower) {
            return true;
        }

        // First name match for full names
        let input_parts: Vec<&str> = input_lower.split_whitespace().collect();
        let target_parts: Vec<&str> = target_lower.split_whitespace().collect();

        if !input_parts.is_empty() && !target_parts.is_empty() && input_parts[0] == target_parts[0]
        {
            return true;
        }

        false
    }
}

impl Default for ContextResolver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctx_with_person() -> UserContext {
        let mut ctx = UserContext::default();
        ctx.entities.push(crate::Entity {
            name: "John Smith".to_string(),
            aliases: vec!["John".to_string(), "JS".to_string()],
            entity_type: crate::EntityType::Person,
            attributes: serde_json::json!({}),
            relationships: vec![],
        });
        ctx
    }

    #[test]
    fn test_resolve_person_exact() {
        let resolver = ContextResolver::new();
        let ctx = make_ctx_with_person();

        let mut entity = ExtractedEntity {
            text: "John Smith".to_string(),
            resolved: None,
            entity_type: ExtractedEntityType::Person,
            start: 0,
            end: 10,
            confidence: Confidence::medium(),
        };

        resolver.resolve_entity(&mut entity, &ctx);
        assert_eq!(entity.resolved, Some("John Smith".to_string()));
        assert!(entity.confidence.is_confident());
    }

    #[test]
    fn test_resolve_person_alias() {
        let resolver = ContextResolver::new();
        let ctx = make_ctx_with_person();

        let mut entity = ExtractedEntity {
            text: "John".to_string(),
            resolved: None,
            entity_type: ExtractedEntityType::Person,
            start: 0,
            end: 4,
            confidence: Confidence::medium(),
        };

        resolver.resolve_entity(&mut entity, &ctx);
        assert_eq!(entity.resolved, Some("John Smith".to_string()));
    }

    #[test]
    fn test_fuzzy_match() {
        let resolver = ContextResolver::new();

        assert!(resolver.fuzzy_match("John", "John Smith"));
        assert!(resolver.fuzzy_match("Smith", "John Smith"));
        assert!(resolver.fuzzy_match("Mich", "Michael")); // Prefix match
        assert!(resolver.fuzzy_match("Michael", "Michael Smith")); // First name match
        assert!(!resolver.fuzzy_match("Bob", "John Smith"));
    }
}
