//! Intent Parser - Extracts structured intent from natural language

use super::*;
use regex::Regex;
use std::collections::HashMap;

/// Parses raw input into structured intent
pub struct IntentParser {
    /// Action verb patterns
    action_patterns: HashMap<&'static str, Vec<Regex>>,
    /// Entity extraction patterns
    entity_patterns: Vec<(ExtractedEntityType, Regex)>,
    /// Temporal patterns
    temporal_patterns: Vec<(RelativeTime, Regex)>,
    /// Urgency indicators
    urgency_patterns: Vec<(f32, Regex)>,
}

impl IntentParser {
    /// Create a new parser with compiled patterns
    pub fn new() -> Self {
        Self {
            action_patterns: Self::build_action_patterns(),
            entity_patterns: Self::build_entity_patterns(),
            temporal_patterns: Self::build_temporal_patterns(),
            urgency_patterns: Self::build_urgency_patterns(),
        }
    }

    /// Parse input into an Intent
    pub fn parse(&self, input: &str) -> zeus_core::Result<Intent> {
        let normalized = input.trim().to_lowercase();

        // Extract entities first
        let entities = self.extract_entities(input);

        // Determine intent type
        let (intent_type, confidence) = self.classify_intent(&normalized, &entities);

        // Extract temporal references
        let temporal = self.extract_temporal(&normalized);

        // Determine urgency
        let urgency = self.extract_urgency(&normalized);

        let mut intent = Intent::new(input, intent_type, confidence);
        intent.entities = entities;
        intent.temporal = temporal;
        intent.urgency = urgency;

        Ok(intent)
    }

    /// Classify the intent type from input
    fn classify_intent(
        &self,
        input: &str,
        entities: &[ExtractedEntity],
    ) -> (IntentType, Confidence) {
        // Check for question patterns first
        if self.is_question(input) {
            return self.parse_question(input);
        }

        // Check action patterns
        for (action, patterns) in &self.action_patterns {
            for pattern in patterns {
                if pattern.is_match(input) {
                    return self.build_intent_for_action(action, input, entities);
                }
            }
        }

        // Fallback: try to infer from structure
        self.infer_intent(input, entities)
    }

    /// Check if input is a question
    fn is_question(&self, input: &str) -> bool {
        input.ends_with('?')
            || input.starts_with("what ")
            || input.starts_with("who ")
            || input.starts_with("when ")
            || input.starts_with("where ")
            || input.starts_with("why ")
            || input.starts_with("how ")
            || input.starts_with("can ")
            || input.starts_with("could ")
            || input.starts_with("would ")
            || input.starts_with("is ")
            || input.starts_with("are ")
            || input.starts_with("do ")
            || input.starts_with("does ")
    }

    /// Parse a question into intent
    fn parse_question(&self, input: &str) -> (IntentType, Confidence) {
        let question_type = if input.starts_with("how ") {
            if input.contains("how much") || input.contains("how many") {
                QuestionType::Factual
            } else {
                QuestionType::Procedural
            }
        } else if input.starts_with("why ") {
            QuestionType::Explanatory
        } else if input.starts_with("what")
            && (input.contains("should") || input.contains("recommend"))
        {
            QuestionType::Advisory
        } else if input.contains("better")
            || input.contains("vs")
            || input.contains("versus")
            || input.contains("compare")
        {
            QuestionType::Comparative
        } else if input.contains("if ") || input.contains("would ") {
            QuestionType::Hypothetical
        } else {
            QuestionType::Factual
        };

        let topic = self.extract_topic(input);

        (
            IntentType::Question {
                topic,
                question_type,
            },
            Confidence::high(),
        )
    }

    /// Extract the main topic from a question
    fn extract_topic(&self, input: &str) -> String {
        // Remove question words and punctuation
        let cleaned = input
            .trim_end_matches('?')
            .replace("what is ", "")
            .replace("what are ", "")
            .replace("who is ", "")
            .replace("where is ", "")
            .replace("when is ", "")
            .replace("how do i ", "")
            .replace("how can i ", "")
            .replace("can you ", "")
            .replace("could you ", "")
            .trim()
            .to_string();

        if cleaned.is_empty() {
            input.to_string()
        } else {
            cleaned
        }
    }

    /// Build intent for a recognized action
    fn build_intent_for_action(
        &self,
        action: &str,
        input: &str,
        entities: &[ExtractedEntity],
    ) -> (IntentType, Confidence) {
        let confidence = Confidence::high();

        let intent_type = match action {
            "create" | "make" | "new" | "add" | "write" => {
                let target = self
                    .extract_object(input, &["create", "make", "new", "add", "write", "a", "an"]);
                IntentType::Create { target }
            }
            "read" | "show" | "get" | "fetch" | "open" | "view" | "display" => {
                let target = self.extract_object(
                    input,
                    &[
                        "read", "show", "get", "fetch", "open", "view", "display", "me", "my",
                        "the",
                    ],
                );
                IntentType::Read { target }
            }
            "update" | "edit" | "modify" | "change" => {
                let target = self
                    .extract_object(input, &["update", "edit", "modify", "change", "the", "my"]);
                IntentType::Update {
                    target,
                    changes: vec![],
                }
            }
            "delete" | "remove" | "cancel" => {
                let target =
                    self.extract_object(input, &["delete", "remove", "cancel", "the", "my"]);
                IntentType::Delete { target }
            }
            "search" | "find" | "look" | "locate" => {
                let query =
                    self.extract_object(input, &["search", "find", "look", "locate", "for", "up"]);
                IntentType::Search { query, scope: None }
            }
            "schedule" | "book" | "set up" | "arrange" | "plan" => {
                let what = self.extract_object(
                    input,
                    &[
                        "schedule", "book", "set", "up", "arrange", "plan", "a", "an",
                    ],
                );
                let when = self.extract_when(input);
                let who = self.extract_people(entities);
                IntentType::Schedule { what, when, who }
            }
            "send" | "email" | "message" | "text" | "call" | "contact" => {
                let to = self.extract_people(entities).unwrap_or_default();
                let about = self.extract_object(
                    input,
                    &[
                        "send",
                        "email",
                        "message",
                        "text",
                        "about",
                        "regarding",
                        "to",
                    ],
                );
                let channel = self.detect_channel(input);
                IntentType::Communicate { to, about, channel }
            }
            "analyze" | "review" | "examine" | "check" | "assess" => {
                let subject = self.extract_object(
                    input,
                    &[
                        "analyze", "review", "examine", "check", "assess", "the", "my",
                    ],
                );
                IntentType::Analyze {
                    subject,
                    aspect: None,
                }
            }
            "run" | "execute" | "do" | "perform" => {
                let action =
                    self.extract_object(input, &["run", "execute", "do", "perform", "the"]);
                IntentType::Execute {
                    action,
                    parameters: vec![],
                }
            }
            "remember" | "note" | "save" | "store" => {
                let what = self.extract_object(
                    input,
                    &["remember", "note", "save", "store", "that", "this"],
                );
                IntentType::Remember {
                    what,
                    context: None,
                }
            }
            "remind" | "alert" | "notify" => {
                let what =
                    self.extract_object(input, &["remind", "alert", "notify", "me", "to", "about"]);
                let when = self
                    .extract_when(input)
                    .unwrap_or_else(|| "later".to_string());
                IntentType::Remind { what, when }
            }
            _ => {
                return self.infer_intent(input, entities);
            }
        };

        (intent_type, confidence)
    }

    /// Extract the object/target from input
    fn extract_object(&self, input: &str, skip_words: &[&str]) -> String {
        let words: Vec<&str> = input.split_whitespace().collect();
        let filtered: Vec<&str> = words
            .iter()
            .filter(|w| !skip_words.contains(&w.to_lowercase().as_str()))
            .copied()
            .collect();

        if filtered.is_empty() {
            input.to_string()
        } else {
            filtered.join(" ")
        }
    }

    /// Extract temporal expression
    fn extract_when(&self, input: &str) -> Option<String> {
        // Look for temporal phrases
        let temporal_phrases = [
            "tomorrow",
            "today",
            "tonight",
            "this morning",
            "this afternoon",
            "this evening",
            "next week",
            "next month",
            "monday",
            "tuesday",
            "wednesday",
            "thursday",
            "friday",
            "saturday",
            "sunday",
            "at ",
            "on ",
            "by ",
            "before ",
            "after ",
        ];

        for phrase in temporal_phrases {
            if let Some(pos) = input.find(phrase) {
                // Extract the temporal phrase and some context
                let end = (pos + 30).min(input.len());
                // Ensure end is on a char boundary
                let mut safe_end = end;
                while safe_end > pos && !input.is_char_boundary(safe_end) {
                    safe_end -= 1;
                }
                let extracted = &input[pos..safe_end];
                // Clean up
                let cleaned = extracted
                    .split([',', '.', '!', '?'])
                    .next()
                    .unwrap_or(extracted)
                    .trim();
                return Some(cleaned.to_string());
            }
        }

        None
    }

    /// Extract people from entities
    fn extract_people(&self, entities: &[ExtractedEntity]) -> Option<Vec<String>> {
        let people: Vec<String> = entities
            .iter()
            .filter(|e| e.entity_type == ExtractedEntityType::Person)
            .map(|e| e.resolved.clone().unwrap_or_else(|| e.text.clone()))
            .collect();

        if people.is_empty() {
            None
        } else {
            Some(people)
        }
    }

    /// Detect communication channel from input
    fn detect_channel(&self, input: &str) -> Option<String> {
        // Platform-specific names must come BEFORE generic words like "message"/"text"
        // so "send a message to telegram" maps to telegram, not imessage.
        let channels = [
            ("telegram", "telegram"),
            ("discord", "discord"),
            ("slack", "slack"),
            ("whatsapp", "whatsapp"),
            ("signal", "signal"),
            ("matrix", "matrix"),
            ("email", "email"),
            ("message", "imessage"),
            ("text", "sms"),
            ("call", "phone"),
        ];

        for (keyword, channel) in channels {
            if input.contains(keyword) {
                return Some(channel.to_string());
            }
        }

        None
    }

    /// Infer intent when no clear pattern matches
    fn infer_intent(&self, input: &str, entities: &[ExtractedEntity]) -> (IntentType, Confidence) {
        // Try to build possibilities based on entities
        let mut possibilities = Vec::new();

        // If we have people, might be communication
        if entities
            .iter()
            .any(|e| e.entity_type == ExtractedEntityType::Person)
        {
            possibilities.push(IntentType::Communicate {
                to: self.extract_people(entities).unwrap_or_default(),
                about: input.to_string(),
                channel: None,
            });
        }

        // If we have datetime, might be scheduling
        if entities
            .iter()
            .any(|e| e.entity_type == ExtractedEntityType::DateTime)
        {
            possibilities.push(IntentType::Schedule {
                what: input.to_string(),
                when: None,
                who: None,
            });
        }

        // Default to search
        possibilities.push(IntentType::Search {
            query: input.to_string(),
            scope: None,
        });

        if possibilities.len() == 1 {
            (
                possibilities
                    .pop()
                    .expect("possibilities has exactly 1 element"),
                Confidence::low(),
            )
        } else {
            (
                IntentType::Unclear {
                    raw: input.to_string(),
                    possibilities,
                },
                Confidence::uncertain(),
            )
        }
    }

    /// Extract entities from input
    fn extract_entities(&self, input: &str) -> Vec<ExtractedEntity> {
        let mut entities = Vec::new();

        for (entity_type, pattern) in &self.entity_patterns {
            for cap in pattern.captures_iter(input) {
                if let Some(m) = cap.get(0) {
                    entities.push(ExtractedEntity {
                        text: m.as_str().to_string(),
                        resolved: None,
                        entity_type: entity_type.clone(),
                        start: m.start(),
                        end: m.end(),
                        confidence: Confidence::medium(),
                    });
                }
            }
        }

        // Sort by position
        entities.sort_by_key(|e| e.start);
        entities
    }

    /// Extract temporal reference
    fn extract_temporal(&self, input: &str) -> Option<TemporalRef> {
        for (relative, pattern) in &self.temporal_patterns {
            if pattern.is_match(input)
                && let Some(cap) = pattern.captures(input)
            {
                let raw = cap
                    .get(0)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default();
                return Some(TemporalRef {
                    raw,
                    datetime: None, // Would need chrono-english or similar
                    relative: Some(relative.clone()),
                    is_deadline: input.contains("by ") || input.contains("before "),
                });
            }
        }
        None
    }

    /// Extract urgency from input
    fn extract_urgency(&self, input: &str) -> f32 {
        let mut result_urgency = 0.5; // default
        let mut found_any = false;

        for (urgency, pattern) in &self.urgency_patterns {
            if pattern.is_match(input) {
                // Take the first matching pattern (highest priority patterns are first)
                // or the most extreme value if multiple match
                if !found_any {
                    result_urgency = *urgency;
                    found_any = true;
                } else if (*urgency - 0.5).abs() > (result_urgency - 0.5).abs() {
                    // Take the more extreme value
                    result_urgency = *urgency;
                }
            }
        }

        result_urgency
    }

    /// Build action patterns
    fn build_action_patterns() -> HashMap<&'static str, Vec<Regex>> {
        let mut patterns = HashMap::new();

        let actions = [
            (
                "create",
                vec![r"^create\b", r"^make\b", r"^new\b", r"^add\b", r"^write\b"],
            ),
            (
                "read",
                vec![
                    r"^show\b",
                    r"^get\b",
                    r"^fetch\b",
                    r"^open\b",
                    r"^view\b",
                    r"^display\b",
                    r"^read\b",
                ],
            ),
            (
                "update",
                vec![r"^update\b", r"^edit\b", r"^modify\b", r"^change\b"],
            ),
            ("delete", vec![r"^delete\b", r"^remove\b", r"^cancel\b"]),
            (
                "search",
                vec![r"^search\b", r"^find\b", r"^look\b", r"^locate\b"],
            ),
            (
                "schedule",
                vec![
                    r"^schedule\b",
                    r"^book\b",
                    r"^set up\b",
                    r"^arrange\b",
                    r"^plan\b",
                ],
            ),
            (
                "send",
                vec![
                    r"^send\b",
                    r"^email\b",
                    r"^message\b",
                    r"^text\b",
                    r"^contact\b",
                ],
            ),
            (
                "analyze",
                vec![
                    r"^analyze\b",
                    r"^review\b",
                    r"^examine\b",
                    r"^check\b",
                    r"^assess\b",
                ],
            ),
            (
                "run",
                vec![r"^run\b", r"^execute\b", r"^do\b", r"^perform\b"],
            ),
            (
                "remember",
                vec![r"^remember\b", r"^note\b", r"^save\b", r"^store\b"],
            ),
            ("remind", vec![r"^remind\b", r"^alert\b", r"^notify\b"]),
        ];

        for (action, regexes) in actions {
            let compiled: Vec<Regex> = regexes.iter().filter_map(|r| Regex::new(r).ok()).collect();
            patterns.insert(action, compiled);
        }

        patterns
    }

    /// Build entity extraction patterns
    fn build_entity_patterns() -> Vec<(ExtractedEntityType, Regex)> {
        vec![
            // Email addresses
            (
                ExtractedEntityType::Email,
                Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}").expect("valid regex"),
            ),
            // URLs
            (
                ExtractedEntityType::Url,
                Regex::new(r"https?://[^\s]+").expect("valid regex"),
            ),
            // File paths
            (
                ExtractedEntityType::Filename,
                Regex::new(r"[/~][a-zA-Z0-9._/-]+\.[a-zA-Z0-9]+").expect("valid regex"),
            ),
            // Durations
            (
                ExtractedEntityType::Duration,
                Regex::new(r"\b\d+\s*(minutes?|mins?|hours?|hrs?|days?|weeks?)\b")
                    .expect("valid regex"),
            ),
        ]
    }

    /// Build temporal patterns
    fn build_temporal_patterns() -> Vec<(RelativeTime, Regex)> {
        vec![
            (
                RelativeTime::Now,
                Regex::new(r"\b(now|right now|immediately)\b").expect("valid regex"),
            ),
            (
                RelativeTime::Today,
                Regex::new(r"\btoday\b").expect("valid regex"),
            ),
            (
                RelativeTime::Tomorrow,
                Regex::new(r"\btomorrow\b").expect("valid regex"),
            ),
            (
                RelativeTime::ThisWeek,
                Regex::new(r"\bthis week\b").expect("valid regex"),
            ),
            (
                RelativeTime::NextWeek,
                Regex::new(r"\bnext week\b").expect("valid regex"),
            ),
            (
                RelativeTime::ThisMonth,
                Regex::new(r"\bthis month\b").expect("valid regex"),
            ),
            (
                RelativeTime::NextMonth,
                Regex::new(r"\bnext month\b").expect("valid regex"),
            ),
            (
                RelativeTime::Soon,
                Regex::new(r"\b(soon|shortly|in a bit)\b").expect("valid regex"),
            ),
            (
                RelativeTime::Eventually,
                Regex::new(r"\b(eventually|someday|later)\b").expect("valid regex"),
            ),
        ]
    }

    /// Build urgency patterns
    fn build_urgency_patterns() -> Vec<(f32, Regex)> {
        vec![
            (
                1.0,
                Regex::new(r"\b(urgent|asap|emergency|critical|immediately)\b")
                    .expect("valid regex"),
            ),
            (
                0.9,
                Regex::new(r"\b(very important|high priority|right now)\b").expect("valid regex"),
            ),
            (
                0.8,
                Regex::new(r"\b(important|priority|soon)\b").expect("valid regex"),
            ),
            (
                0.6,
                Regex::new(r"\b(when you can|when possible)\b").expect("valid regex"),
            ),
            (
                0.3,
                Regex::new(r"\b(eventually|no rush|low priority|whenever)\b").expect("valid regex"),
            ),
        ]
    }
}

impl Default for IntentParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_create() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("create a new document")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Create { .. }));
    }

    #[test]
    fn test_parse_schedule() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("schedule a meeting tomorrow")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Schedule { .. }));
        assert!(intent.temporal.is_some());
    }

    #[test]
    fn test_parse_question() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("what is the weather today?")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Question { .. }));
    }

    #[test]
    fn test_parse_search() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("find all documents about rust")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Search { .. }));
    }

    #[test]
    fn test_extract_email() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("send email to test@example.com")
            .expect("should parse successfully");
        assert!(
            intent
                .entities
                .iter()
                .any(|e| e.entity_type == ExtractedEntityType::Email)
        );
    }

    #[test]
    fn test_urgency_detection() {
        let parser = IntentParser::new();

        let urgent = parser
            .parse("urgent: fix the bug now")
            .expect("should parse successfully");
        assert!(urgent.urgency > 0.9);

        let normal = parser
            .parse("fix the bug")
            .expect("should parse successfully");
        assert!(normal.urgency < 0.6);

        let low = parser
            .parse("fix the bug eventually, no rush")
            .expect("should parse successfully");
        assert!(low.urgency < 0.5);
    }

    #[test]
    fn test_temporal_extraction() {
        let parser = IntentParser::new();

        let tomorrow = parser
            .parse("remind me tomorrow")
            .expect("should parse successfully");
        assert!(matches!(
            tomorrow.temporal.as_ref().and_then(|t| t.relative.as_ref()),
            Some(RelativeTime::Tomorrow)
        ));
    }

    // ── Action type coverage ──────────────────────────────────────

    #[test]
    fn test_parse_update() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("update the configuration file")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Update { .. }));
    }

    #[test]
    fn test_parse_delete() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("delete the old backups")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Delete { .. }));
    }

    #[test]
    fn test_parse_read() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("read the latest logs")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Read { .. }));
    }

    #[test]
    fn test_parse_show() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("show me the status")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Read { .. }));
    }

    #[test]
    fn test_parse_get() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("get the current temperature")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Read { .. }));
    }

    #[test]
    fn test_parse_analyze() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("analyze the test results")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Analyze { .. }));
    }

    #[test]
    fn test_parse_review() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("review the pull request")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Analyze { .. }));
    }

    #[test]
    fn test_parse_run() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("run the test suite")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Execute { .. }));
    }

    #[test]
    fn test_parse_execute() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("execute the deployment script")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Execute { .. }));
    }

    #[test]
    fn test_parse_remember() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("remember that the server IP is 10.0.0.1")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Remember { .. }));
    }

    #[test]
    fn test_parse_note() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("note this for later reference")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Remember { .. }));
    }

    #[test]
    fn test_parse_remind() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("remind me to check the build")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Remind { .. }));
    }

    // ── Question types ────────────────────────────────────────────

    #[test]
    fn test_question_factual() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("how many files are in the project?")
            .expect("should parse successfully");
        assert!(matches!(
            intent.intent_type,
            IntentType::Question {
                question_type: QuestionType::Factual,
                ..
            }
        ));
    }

    #[test]
    fn test_question_procedural() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("how do I deploy to production?")
            .expect("should parse successfully");
        assert!(matches!(
            intent.intent_type,
            IntentType::Question {
                question_type: QuestionType::Procedural,
                ..
            }
        ));
    }

    #[test]
    fn test_question_explanatory() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("why does the build fail on CI?")
            .expect("should parse successfully");
        assert!(matches!(
            intent.intent_type,
            IntentType::Question {
                question_type: QuestionType::Explanatory,
                ..
            }
        ));
    }

    #[test]
    fn test_question_advisory() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("what should I recommend for the database?")
            .expect("should parse successfully");
        assert!(matches!(
            intent.intent_type,
            IntentType::Question {
                question_type: QuestionType::Advisory,
                ..
            }
        ));
    }

    #[test]
    fn test_question_comparative() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("which is better, Postgres vs MySQL?")
            .expect("should parse successfully");
        assert!(matches!(
            intent.intent_type,
            IntentType::Question {
                question_type: QuestionType::Comparative,
                ..
            }
        ));
    }

    #[test]
    fn test_question_hypothetical() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("what would happen if we doubled the cache size?")
            .expect("should parse successfully");
        assert!(matches!(
            intent.intent_type,
            IntentType::Question {
                question_type: QuestionType::Hypothetical,
                ..
            }
        ));
    }

    // ── Entity extraction ─────────────────────────────────────────

    #[test]
    fn test_extract_url() {
        let parser = IntentParser::new();
        let intent = parser.parse("fetch https://example.com/api/v1").unwrap();
        assert!(
            intent
                .entities
                .iter()
                .any(|e| e.entity_type == ExtractedEntityType::Url)
        );
        assert!(
            intent
                .entities
                .iter()
                .any(|e| e.text == "https://example.com/api/v1")
        );
    }

    #[test]
    fn test_extract_filepath() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("read /etc/config.toml")
            .expect("should parse successfully");
        assert!(
            intent
                .entities
                .iter()
                .any(|e| e.entity_type == ExtractedEntityType::Filename)
        );
        assert!(
            intent
                .entities
                .iter()
                .any(|e| e.text.contains("/etc/config.toml"))
        );
    }

    #[test]
    fn test_extract_duration() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("wait for 30 minutes")
            .expect("should parse successfully");
        assert!(
            intent
                .entities
                .iter()
                .any(|e| e.entity_type == ExtractedEntityType::Duration)
        );
    }

    #[test]
    fn test_extract_duration_hours() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("the task took 2 hours")
            .expect("should parse successfully");
        assert!(
            intent
                .entities
                .iter()
                .any(|e| e.entity_type == ExtractedEntityType::Duration)
        );
    }

    // ── Temporal extraction ───────────────────────────────────────

    #[test]
    fn test_temporal_today() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("finish the report today")
            .expect("should parse successfully");
        assert!(matches!(
            intent.temporal.as_ref().and_then(|t| t.relative.as_ref()),
            Some(RelativeTime::Today)
        ));
    }

    #[test]
    fn test_temporal_next_week() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("schedule it for next week")
            .expect("should parse successfully");
        assert!(matches!(
            intent.temporal.as_ref().and_then(|t| t.relative.as_ref()),
            Some(RelativeTime::NextWeek)
        ));
    }

    #[test]
    fn test_temporal_now() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("do it right now")
            .expect("should parse successfully");
        assert!(matches!(
            intent.temporal.as_ref().and_then(|t| t.relative.as_ref()),
            Some(RelativeTime::Now)
        ));
    }

    #[test]
    fn test_temporal_this_week() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("get it done this week")
            .expect("should parse successfully");
        assert!(matches!(
            intent.temporal.as_ref().and_then(|t| t.relative.as_ref()),
            Some(RelativeTime::ThisWeek)
        ));
    }

    #[test]
    fn test_temporal_is_deadline() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("finish by tomorrow")
            .expect("should parse successfully");
        let temporal = intent.temporal.as_ref().expect("as_ref should succeed");
        assert!(temporal.is_deadline);
        assert!(matches!(temporal.relative, Some(RelativeTime::Tomorrow)));
    }

    // ── Urgency edge cases ────────────────────────────────────────

    #[test]
    fn test_urgency_asap() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("fix this asap")
            .expect("should parse successfully");
        assert!(intent.urgency >= 0.9);
    }

    #[test]
    fn test_urgency_whenever() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("do this whenever you can")
            .expect("should parse successfully");
        // "whenever" matches the low-urgency pattern (0.3)
        assert!(intent.urgency < 0.5);
    }

    #[test]
    fn test_urgency_critical() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("critical production outage")
            .expect("should parse successfully");
        assert!(intent.urgency >= 0.9);
    }

    #[test]
    fn test_urgency_default() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("create a new file")
            .expect("should parse successfully");
        // No urgency keywords -> default 0.5
        assert!((intent.urgency - 0.5).abs() < f32::EPSILON);
    }

    // ── Empty / edge inputs ───────────────────────────────────────

    #[test]
    fn test_empty_input() {
        let parser = IntentParser::new();
        let intent = parser.parse("").expect("should parse successfully");
        // Empty input should still parse without panic
        assert!(matches!(
            intent.intent_type,
            IntentType::Unclear { .. } | IntentType::Search { .. }
        ));
    }

    #[test]
    fn test_single_word_input() {
        let parser = IntentParser::new();
        let intent = parser.parse("hello").expect("should parse successfully");
        // Single word with no recognized pattern falls through to infer
        assert!(matches!(
            intent.intent_type,
            IntentType::Unclear { .. } | IntentType::Search { .. }
        ));
    }

    #[test]
    fn test_very_long_input() {
        let parser = IntentParser::new();
        let long = "create ".to_string() + &"a very detailed ".repeat(100) + "document";
        let intent = parser.parse(&long).expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Create { .. }));
    }

    // ── Communicate intent ────────────────────────────────────────

    #[test]
    fn test_communicate_email() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("email john about the meeting")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Communicate { .. }));
        if let IntentType::Communicate { channel, .. } = &intent.intent_type {
            assert_eq!(channel.as_deref(), Some("email"));
        }
    }

    #[test]
    fn test_communicate_text() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("text alice about the project")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Communicate { .. }));
        if let IntentType::Communicate { channel, .. } = &intent.intent_type {
            assert_eq!(channel.as_deref(), Some("sms"));
        }
    }

    #[test]
    fn test_communicate_send_telegram() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("send a telegram to the team")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Communicate { .. }));
        if let IntentType::Communicate { channel, .. } = &intent.intent_type {
            assert_eq!(channel.as_deref(), Some("telegram"));
        }
    }

    // ── Search intent ─────────────────────────────────────────────

    #[test]
    fn test_search_find_files() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("find all rust files")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Search { .. }));
        if let IntentType::Search { query, .. } = &intent.intent_type {
            assert!(query.contains("rust") || query.contains("files"));
        }
    }

    #[test]
    fn test_search_locate() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("locate the config file")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Search { .. }));
    }

    #[test]
    fn test_search_look_for() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("look for errors in logs")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Search { .. }));
    }

    // ── Schedule intent ───────────────────────────────────────────

    #[test]
    fn test_schedule_meeting_next_week() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("schedule meeting next week")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Schedule { .. }));
        if let IntentType::Schedule { when, .. } = &intent.intent_type {
            assert!(when.is_some());
            assert!(
                when.as_ref()
                    .expect("as_ref should succeed")
                    .contains("next week")
            );
        }
    }

    #[test]
    fn test_schedule_book_appointment_friday() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("book appointment on friday")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Schedule { .. }));
        if let IntentType::Schedule { when, .. } = &intent.intent_type {
            // "on friday" should be extracted by extract_when via "on " prefix
            assert!(when.is_some());
        }
    }

    // ── extract_topic tests (indirect via question parsing) ──────

    #[test]
    fn test_extract_topic_what_is() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("what is rust?")
            .expect("should parse successfully");
        if let IntentType::Question { topic, .. } = &intent.intent_type {
            assert_eq!(topic, "rust");
        } else {
            panic!("Expected Question intent");
        }
    }

    #[test]
    fn test_extract_topic_how_do_i() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("how do i install cargo?")
            .expect("should parse successfully");
        if let IntentType::Question { topic, .. } = &intent.intent_type {
            assert_eq!(topic, "install cargo");
        } else {
            panic!("Expected Question intent");
        }
    }

    #[test]
    fn test_extract_topic_who_is() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("who is the maintainer?")
            .expect("should parse successfully");
        if let IntentType::Question { topic, .. } = &intent.intent_type {
            assert!(topic.contains("maintainer"));
        } else {
            panic!("Expected Question intent");
        }
    }

    // ── detect_channel tests (indirect via communicate intent) ───

    #[test]
    fn test_detect_channel_slack() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("send a slack message about the release")
            .expect("should parse successfully");
        if let IntentType::Communicate { channel, .. } = &intent.intent_type {
            assert_eq!(channel.as_deref(), Some("slack"));
        } else {
            panic!("Expected Communicate intent");
        }
    }

    #[test]
    fn test_detect_channel_discord() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("send a discord notification about the build")
            .expect("should parse successfully");
        if let IntentType::Communicate { channel, .. } = &intent.intent_type {
            assert_eq!(channel.as_deref(), Some("discord"));
        } else {
            panic!("Expected Communicate intent");
        }
    }

    // ── extract_when tests (indirect via schedule intent) ────────

    #[test]
    fn test_extract_when_today() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("schedule deployment today")
            .expect("should parse successfully");
        if let IntentType::Schedule { when, .. } = &intent.intent_type {
            assert!(when.is_some());
            assert!(
                when.as_ref()
                    .expect("as_ref should succeed")
                    .contains("today")
            );
        } else {
            panic!("Expected Schedule intent");
        }
    }

    #[test]
    fn test_extract_when_at_3pm() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("schedule standup at 3pm")
            .expect("should parse successfully");
        if let IntentType::Schedule { when, .. } = &intent.intent_type {
            assert!(when.is_some());
            assert!(
                when.as_ref()
                    .expect("as_ref should succeed")
                    .contains("at 3pm")
            );
        } else {
            panic!("Expected Schedule intent");
        }
    }

    #[test]
    fn test_extract_when_friday() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("plan review on friday afternoon")
            .expect("should parse successfully");
        if let IntentType::Schedule { when, .. } = &intent.intent_type {
            // "on friday afternoon" via the "on " prefix
            assert!(when.is_some());
        } else {
            panic!("Expected Schedule intent");
        }
    }

    // ── Confidence checks ─────────────────────────────────────────

    #[test]
    fn test_high_confidence_on_clear_action() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("create a new project")
            .expect("should parse successfully");
        assert!(intent.confidence.is_confident());
    }

    #[test]
    fn test_low_confidence_on_unclear_input() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("stuff and things")
            .expect("should parse successfully");
        // Ambiguous input should have low confidence
        assert!(!intent.confidence.is_confident());
    }
}
