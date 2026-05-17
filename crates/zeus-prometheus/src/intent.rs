//! Intent Analysis Engine
//!
//! Classifies user messages to determine the optimal processing strategy.
//! Uses heuristic pattern matching with optional LLM-backed classification
//! for ambiguous cases. The classifier outputs an [`IntentAnalysis`] that
//! downstream components (e.g. the autonomy engine) use to decide whether
//! to respond directly, execute a tool, plan a complex task, or ask the
//! user for clarification.

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};
use zeus_core::{Message, Result, ToolSchema};
use zeus_llm::LlmClient;

// ============================================================================
// Types
// ============================================================================

/// The classified intent of a user message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    /// A straightforward question or informational request.
    SimpleQuery,
    /// The user wants a specific tool executed (file op, shell, web, etc.).
    ToolUse,
    /// A multi-step task requiring planning and orchestration.
    ComplexTask,
    /// The message is ambiguous or too short to act on confidently.
    Clarification,
    /// General conversation, greetings, thanks, small talk.
    Conversation,
    /// A system/meta command (config, status, slash commands).
    SystemCommand,
}

impl std::fmt::Display for Intent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Intent::SimpleQuery => write!(f, "simple_query"),
            Intent::ToolUse => write!(f, "tool_use"),
            Intent::ComplexTask => write!(f, "complex_task"),
            Intent::Clarification => write!(f, "clarification"),
            Intent::Conversation => write!(f, "conversation"),
            Intent::SystemCommand => write!(f, "system_command"),
        }
    }
}

/// Estimated complexity of a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskComplexity {
    /// Single action, no planning needed.
    Trivial,
    /// One or two tool calls, straightforward.
    Simple,
    /// Multiple steps with some dependencies.
    Moderate,
    /// Multi-step, multi-tool, requires careful planning.
    Complex,
}

impl std::fmt::Display for TaskComplexity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskComplexity::Trivial => write!(f, "trivial"),
            TaskComplexity::Simple => write!(f, "simple"),
            TaskComplexity::Moderate => write!(f, "moderate"),
            TaskComplexity::Complex => write!(f, "complex"),
        }
    }
}

/// The result of classifying a user message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentAnalysis {
    /// The primary classified intent.
    pub intent: Intent,
    /// Estimated task complexity.
    pub complexity: TaskComplexity,
    /// Confidence score in the classification (0.0 to 1.0).
    pub confidence: f32,
    /// Tool names that the message likely refers to.
    pub suggested_tools: Vec<String>,
    /// Whether this action should be confirmed with the user before proceeding.
    pub requires_confirmation: bool,
    /// Brief reasoning for the classification (useful for debugging/logging).
    pub reasoning: String,
}

// ============================================================================
// IntentClassifier
// ============================================================================

/// Heuristic + optional LLM-backed intent classifier.
pub struct IntentClassifier {
    /// Minimum word count before we consider a message substantive.
    min_substantive_words: usize,
}

impl IntentClassifier {
    /// Create a new intent classifier with default settings.
    pub fn new() -> Self {
        Self {
            min_substantive_words: 5,
        }
    }

    /// Classify a user message using heuristic pattern matching.
    ///
    /// This is fast and requires no LLM call. It inspects keyword patterns,
    /// message length, and available tools to produce an [`IntentAnalysis`].
    pub fn classify(&self, message: &str, available_tools: &[ToolSchema]) -> IntentAnalysis {
        let trimmed = message.trim();
        let lower = trimmed.to_lowercase();
        let words: Vec<&str> = trimmed.split_whitespace().collect();
        let word_count = words.len();

        // --- System commands ---
        if self.is_system_command(&lower) {
            return IntentAnalysis {
                intent: Intent::SystemCommand,
                complexity: TaskComplexity::Trivial,
                confidence: 0.95,
                suggested_tools: vec![],
                requires_confirmation: false,
                reasoning: "Message matches system/meta command pattern".to_string(),
            };
        }

        // --- Clarification (very short / ambiguous) ---
        if word_count < self.min_substantive_words && self.is_ambiguous(&lower) {
            return IntentAnalysis {
                intent: Intent::Clarification,
                complexity: TaskComplexity::Trivial,
                confidence: 0.7,
                suggested_tools: vec![],
                requires_confirmation: false,
                reasoning: format!(
                    "Message is short ({} words) and appears ambiguous",
                    word_count
                ),
            };
        }

        // --- Conversation (greetings, thanks, general chat) ---
        if self.is_conversation(&lower) {
            return IntentAnalysis {
                intent: Intent::Conversation,
                complexity: TaskComplexity::Trivial,
                confidence: 0.85,
                suggested_tools: vec![],
                requires_confirmation: false,
                reasoning: "Message matches conversational pattern".to_string(),
            };
        }

        // --- Complex task ---
        if self.is_complex_task(&lower, word_count) {
            let tools = self.extract_tool_hints(&lower, available_tools);
            let complexity = self.estimate_complexity(message);
            // Reduce confidence when the match is word_count-only (no multi-clause structure).
            // Single-clause long messages are often phrased conversationally ("can you update
            // me on X") rather than being genuinely complex multi-step tasks. Lower confidence
            // triggers the < 0.65 cap fallback in gateway, preventing 20-iteration over-cooking.
            let clause_count = count_clauses(&lower);
            let confidence = if clause_count >= 2 { 0.75 } else { 0.6 };
            return IntentAnalysis {
                intent: Intent::ComplexTask,
                complexity,
                confidence,
                suggested_tools: tools,
                requires_confirmation: true,
                reasoning: if clause_count >= 2 {
                    "Message contains complex task indicators with multiple clauses".to_string()
                } else {
                    "Message contains task verb with substantive length (single clause)".to_string()
                },
            };
        }

        // --- Tool use ---
        let tool_hints = self.extract_tool_hints(&lower, available_tools);
        if !tool_hints.is_empty() || self.is_tool_use(&lower) {
            let complexity = self.estimate_complexity(message);
            let confidence = if !tool_hints.is_empty() { 0.85 } else { 0.7 };
            return IntentAnalysis {
                intent: Intent::ToolUse,
                complexity,
                confidence,
                suggested_tools: tool_hints,
                requires_confirmation: false,
                reasoning: "Message references tool operations or file/shell actions".to_string(),
            };
        }

        // --- Simple query ---
        if self.is_simple_query(&lower) {
            return IntentAnalysis {
                intent: Intent::SimpleQuery,
                complexity: TaskComplexity::Trivial,
                confidence: 0.8,
                suggested_tools: vec![],
                requires_confirmation: false,
                reasoning: "Message is a question or informational request".to_string(),
            };
        }

        // --- Fallback: if short, ask for clarification; otherwise treat as query ---
        if word_count < self.min_substantive_words {
            IntentAnalysis {
                intent: Intent::Clarification,
                complexity: TaskComplexity::Trivial,
                confidence: 0.5,
                suggested_tools: vec![],
                requires_confirmation: false,
                reasoning: format!(
                    "Short message ({} words) with no clear intent pattern",
                    word_count
                ),
            }
        } else {
            IntentAnalysis {
                intent: Intent::SimpleQuery,
                complexity: TaskComplexity::Simple,
                confidence: 0.5,
                suggested_tools: vec![],
                requires_confirmation: false,
                reasoning: "No strong pattern match; defaulting to simple query".to_string(),
            }
        }
    }

    /// Classify a message using the LLM for higher-accuracy analysis.
    ///
    /// Sends the message along with available tool descriptions to the LLM
    /// and asks for a structured JSON classification. Falls back to heuristic
    /// classification if the LLM response cannot be parsed.
    pub async fn classify_with_llm(
        &self,
        message: &str,
        tools: &[ToolSchema],
        llm: &LlmClient,
    ) -> Result<IntentAnalysis> {
        let tool_list = tools
            .iter()
            .map(|t| format!("- {}: {}", t.name, t.description))
            .collect::<Vec<_>>()
            .join("\n");

        let system = format!(
            "You are an intent classifier for an AI assistant. Classify the user message.\n\n\
             Available tools:\n{}\n\n\
             Respond ONLY with valid JSON matching this schema:\n\
             {{\n\
               \"intent\": \"simple_query\" | \"tool_use\" | \"complex_task\" | \"clarification\" | \"conversation\" | \"system_command\",\n\
               \"complexity\": \"trivial\" | \"simple\" | \"moderate\" | \"complex\",\n\
               \"confidence\": 0.0-1.0,\n\
               \"suggested_tools\": [\"tool_name\", ...],\n\
               \"requires_confirmation\": true|false,\n\
               \"reasoning\": \"brief explanation\"\n\
             }}",
            tool_list
        );

        let messages = vec![Message::user(format!("Classify this message: {}", message))];

        let response = llm.complete(&messages, &[], Some(&system)).await?;

        // Try to parse the LLM response as JSON
        let json_str = extract_json(&response.content);
        match serde_json::from_str::<IntentAnalysis>(&json_str) {
            Ok(analysis) => {
                debug!(
                    "LLM classified '{}' as {} (confidence: {:.2})",
                    truncate(message, 50),
                    analysis.intent,
                    analysis.confidence,
                );
                Ok(analysis)
            }
            Err(e) => {
                warn!(
                    "Failed to parse LLM classification response ({}), falling back to heuristic",
                    e
                );
                Ok(self.classify(message, tools))
            }
        }
    }

    /// Estimate the complexity of a message based on structural cues.
    ///
    /// Considers word count, number of clauses (delimited by commas, "and",
    /// "then", semicolons), and mentions of multiple distinct operations.
    pub fn estimate_complexity(&self, message: &str) -> TaskComplexity {
        let lower = message.to_lowercase();
        let word_count = message.split_whitespace().count();

        // Count clause separators
        let clause_count = count_clauses(&lower);

        // Count distinct operation keywords
        let op_keywords = [
            "create",
            "build",
            "implement",
            "write",
            "read",
            "edit",
            "delete",
            "run",
            "execute",
            "deploy",
            "test",
            "refactor",
            "migrate",
            "install",
            "configure",
            "set up",
            "update",
            "fetch",
            "download",
            "upload",
            "send",
            "move",
            "copy",
            "rename",
        ];
        let op_count = op_keywords.iter().filter(|kw| lower.contains(**kw)).count();

        // Heuristic scoring
        if word_count <= 5 && clause_count <= 1 && op_count <= 1 {
            TaskComplexity::Trivial
        } else if word_count <= 15 && clause_count <= 2 && op_count <= 2 {
            TaskComplexity::Simple
        } else if word_count <= 40 && clause_count <= 4 && op_count <= 4 {
            TaskComplexity::Moderate
        } else {
            TaskComplexity::Complex
        }
    }

    /// Find tool names or related keywords in the message that match available tools.
    pub fn extract_tool_hints(&self, message: &str, tools: &[ToolSchema]) -> Vec<String> {
        let lower = message.to_lowercase();
        let mut hints = Vec::new();

        for tool in tools {
            let tool_lower = tool.name.to_lowercase();

            // Direct tool name match
            if lower.contains(&tool_lower) {
                hints.push(tool.name.clone());
                continue;
            }

            // Keyword-based matching for common tools
            let matched = match tool_lower.as_str() {
                "read_file" => {
                    lower.contains("read") && lower.contains("file")
                        || lower.contains("show me")
                        || lower.contains("cat ")
                        || lower.contains("display")
                            && (lower.contains("file") || lower.contains("contents"))
                }
                "write_file" => {
                    lower.contains("write") && lower.contains("file")
                        || lower.contains("create") && lower.contains("file")
                        || lower.contains("save to")
                }
                "edit_file" => {
                    lower.contains("edit") && lower.contains("file")
                        || lower.contains("modify")
                        || lower.contains("change") && lower.contains("file")
                        || lower.contains("replace") && lower.contains("in")
                }
                "list_dir" => {
                    lower.contains("list")
                        && (lower.contains("dir")
                            || lower.contains("folder")
                            || lower.contains("files"))
                        || lower.contains("ls ")
                        || lower.contains("ls\n")
                        || lower == "ls"
                }
                "shell" => {
                    lower.contains("run ")
                        || lower.contains("execute ")
                        || lower.contains("shell ")
                        || lower.contains("command ")
                        || lower.contains("terminal ")
                        || lower.starts_with("$")
                }
                "web_fetch" => {
                    lower.contains("fetch")
                        || lower.contains("download")
                        || lower.contains("http")
                        || lower.contains("url")
                        || lower.contains("webpage")
                        || lower.contains("website")
                }
                "spawn" => {
                    lower.contains("background")
                        || lower.contains("spawn")
                        || lower.contains("subagent")
                        || lower.contains("parallel")
                }
                "message" => {
                    lower.contains("send message")
                        || lower.contains("message ")
                            && (lower.contains("telegram")
                                || lower.contains("discord")
                                || lower.contains("slack")
                                || lower.contains("email"))
                }
                _ => false,
            };

            if matched {
                hints.push(tool.name.clone());
            }
        }

        hints.dedup();
        hints
    }

    // ========================================================================
    // Private helpers
    // ========================================================================

    /// Check if the message looks like a system/meta command.
    fn is_system_command(&self, lower: &str) -> bool {
        lower.starts_with('/')
            || lower.starts_with("config")
            || lower.starts_with("settings")
            || lower.starts_with("status")
            || lower == "help"
            || lower == "quit"
            || lower == "exit"
            || lower.starts_with("set ")
            || lower.starts_with("switch ")
    }

    /// Check if a short message is ambiguous (single words that could mean anything).
    fn is_ambiguous(&self, lower: &str) -> bool {
        let ambiguous_patterns = [
            "yes", "no", "ok", "sure", "maybe", "hmm", "right", "that", "this", "it", "them",
            "do it", "go ahead", "proceed", "continue", "again", "more", "same", "next",
        ];
        ambiguous_patterns
            .iter()
            .any(|p| lower == *p || lower == format!("{}.", p))
    }

    /// Check if the message is general conversation (pure social, no task content).
    ///
    /// Only matches short, purely social messages. Longer messages or those
    /// containing task verbs fall through to ComplexTask/ToolUse/SimpleQuery
    /// even if they start with a greeting or thanks.
    fn is_conversation(&self, lower: &str) -> bool {
        let word_count = lower.split_whitespace().count();

        // Gate: messages longer than 8 words almost always contain task content
        // after the social prefix. Let them fall through to more specific classifiers.
        if word_count > 8 {
            return false;
        }

        let greetings = [
            "hello", "hi", "hey", "good morning", "good afternoon",
            "good evening", "howdy", "greetings", "yo", "sup",
            "what's up", "whats up",
        ];
        // Tightened: removed "nice", "great", "awesome", "cool", "perfect",
        // "wonderful", "excellent" — these commonly precede task content
        // ("awesome, now build X", "great, fix the bug").
        let thanks = [
            "thanks", "thank you", "thx", "ty", "appreciated", "cheers",
        ];
        let farewell = [
            "bye", "goodbye", "see you", "later", "good night", "gnight", "cya",
        ];

        let matches_social = greetings.iter().any(|g| lower.starts_with(g))
            || thanks.iter().any(|t| lower.starts_with(t))
            || farewell.iter().any(|f| lower.starts_with(f));

        if !matches_social {
            return false;
        }

        // Even short social-prefix messages should not classify as Conversation
        // if they contain task verbs (e.g. "hey build this", "thanks, now fix it").
        let task_verbs = [
            "create", "build", "implement", "write", "fix", "debug", "test",
            "deploy", "run", "execute", "make", "generate", "code", "add",
            "update", "install", "configure", "refactor", "explain", "research",
            "analyze", "audit", "review", "search", "find", "check",
        ];
        if task_verbs.iter().any(|v| lower.contains(v)) {
            return false;
        }

        true
    }

    /// Check if the message describes a complex multi-step task.
    fn is_complex_task(&self, lower: &str, word_count: usize) -> bool {
        // S65: Continuation phrases should route to cooking loop so agents
        // keep working instead of treating them as simple conversation.
        let continuation_phrases = [
            "continue",
            "keep going",
            "carry on",
            "next task",
            "move on",
            "proceed",
            "don't stop",
            "do not stop",
            "finish it",
            "complete it",
            "keep working",
            "work autonomously",
            "do all",
            "do everything",
        ];
        if continuation_phrases.iter().any(|p| lower.contains(p)) {
            return true;
        }

        let task_verbs = [
            "create",
            "build",
            "implement",
            "refactor",
            "migrate",
            "set up",
            "design",
            "architect",
            "develop",
            "deploy",
            // Sprint 18: expanded verb list — these are common coding/creative
            // task verbs that should trigger the cooking loop so the agent
            // actually executes with tools instead of just acknowledging.
            "generate",
            "make",
            "write",
            "code",
            "produce",
            "construct",
            "put together",
            "set up",
            "configure",
            "install",
            "fix",
            "debug",
            "test",
            "update",
            "add",
        ];
        let has_task_verb = task_verbs.iter().any(|v| lower.contains(v));
        let clause_count = count_clauses(lower);

        // A complex task has a strong task verb AND some substance.
        // Lowered from (>= 3 clauses OR >= 20 words) because short creative
        // prompts like "build me a html page" (7 words, 1 clause) were
        // falling through to SimpleQuery → RespondDirectly → no cooking.
        has_task_verb && (clause_count >= 2 || word_count >= 10)
    }

    /// Check if the message requests a tool operation.
    fn is_tool_use(&self, lower: &str) -> bool {
        let tool_patterns = [
            // Explicit file/shell operations
            "read the file",
            "read file",
            "open file",
            "show file",
            "write to",
            "save to",
            "create file",
            "create a file",
            "edit the file",
            "edit file",
            "modify the file",
            "modify file",
            "list dir",
            "list directory",
            "list files",
            "list folder",
            "run command",
            "run the command",
            "execute command",
            "shell command",
            "fetch url",
            "fetch the url",
            "download",
            "send message",
            "send a message",
            "spawn",
            // Sprint 18: implicit tool needs — prompts that require
            // write_file/send_file even if they don't name those tools.
            "attach",
            "build me",
            "make me",
            "generate a",
            "generate the",
            "code a",
            "code the",
            "write a",
            "write the",
            "create a website",
            "create a page",
            "create a site",
            "build a website",
            "build a page",
            "build a site",
        ];
        tool_patterns.iter().any(|p| lower.contains(p))
    }

    /// Check if the message is a simple informational query.
    fn is_simple_query(&self, lower: &str) -> bool {
        lower.starts_with("what ")
            || lower.starts_with("what's ")
            || lower.starts_with("whats ")
            || lower.starts_with("who ")
            || lower.starts_with("where ")
            || lower.starts_with("when ")
            || lower.starts_with("why ")
            || lower.starts_with("how ")
            || lower.starts_with("can you explain")
            || lower.starts_with("explain ")
            || lower.starts_with("tell me")
            || lower.starts_with("describe ")
            || lower.starts_with("define ")
            || lower.starts_with("is ")
            || lower.starts_with("are ")
            || lower.starts_with("do ")
            || lower.starts_with("does ")
            || lower.contains('?')
    }
}

impl Default for IntentClassifier {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Count the number of clause boundaries in a message.
///
/// Looks for commas, semicolons, "and", "then", "after that", "first", "next",
/// "finally", etc.
fn count_clauses(lower: &str) -> usize {
    let mut count = 1; // At least one clause

    // Punctuation-based
    count += lower.matches(',').count();
    count += lower.matches(';').count();

    // Count " and then " as a single conjunction (not as both " and then " and " then ")
    let and_then_count = lower.matches(" and then ").count();
    count += and_then_count;

    // Count standalone " then " that is NOT part of " and then "
    let then_count = lower.matches(" then ").count();
    count += then_count.saturating_sub(and_then_count);

    // Other sequence markers (these don't overlap with the above)
    let other_sequence = [
        " after that ",
        " next ",
        " finally ",
        " also ",
        " additionally ",
        " followed by ",
    ];
    for sw in &other_sequence {
        count += lower.matches(sw).count();
    }

    // Count standalone " and " that is NOT part of " and then "
    let and_count = lower.matches(" and ").count();
    count += and_count.saturating_sub(and_then_count);

    count
}

/// Extract JSON from a response that may be wrapped in markdown code blocks.
fn extract_json(text: &str) -> String {
    let trimmed = text.trim();

    if let Some(start) = trimmed.find("```json") {
        let after = &trimmed[start + 7..];
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    if let Some(start) = trimmed.find("```") {
        let after = &trimmed[start + 3..];
        let after = if let Some(nl) = after.find('\n') {
            &after[nl + 1..]
        } else {
            after
        };
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    if let Some(start) = trimmed.find('{')
        && let Some(end) = trimmed.rfind('}')
    {
        return trimmed[start..=end].to_string();
    }
    trimmed.to_string()
}

/// Truncate a string for display/logging.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", zeus_core::truncate_str(s, max))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a small set of tool schemas for testing.
    fn test_tools() -> Vec<ToolSchema> {
        vec![
            ToolSchema::new("read_file", "Read a file's contents"),
            ToolSchema::new("write_file", "Write content to a file"),
            ToolSchema::new("edit_file", "Edit a file with search/replace"),
            ToolSchema::new("list_dir", "List directory contents"),
            ToolSchema::new("shell", "Execute a shell command"),
            ToolSchema::new("web_fetch", "Fetch a URL"),
            ToolSchema::new("spawn", "Spawn a background subagent"),
            ToolSchema::new("message", "Send a message via a channel"),
        ]
    }

    #[test]
    fn test_classify_simple_query() {
        let classifier = IntentClassifier::new();
        let tools = test_tools();

        let analysis = classifier.classify("What is Rust?", &tools);
        assert_eq!(analysis.intent, Intent::SimpleQuery);
        assert_eq!(analysis.complexity, TaskComplexity::Trivial);
        assert!(analysis.confidence >= 0.7);
    }

    #[test]
    fn test_classify_tool_use() {
        let classifier = IntentClassifier::new();
        let tools = test_tools();

        let analysis =
            classifier.classify("Read the file main.rs and show me its contents", &tools);
        assert_eq!(analysis.intent, Intent::ToolUse);
        assert!(analysis.suggested_tools.contains(&"read_file".to_string()));
    }

    #[test]
    fn test_classify_complex_task() {
        let classifier = IntentClassifier::new();
        let tools = test_tools();

        let analysis = classifier.classify(
            "Create a new REST API with authentication, database integration, and rate limiting, then deploy it to production",
            &tools,
        );
        assert_eq!(analysis.intent, Intent::ComplexTask);
        assert!(analysis.complexity >= TaskComplexity::Moderate);
        assert!(analysis.requires_confirmation);
    }

    #[test]
    fn test_classify_system_command() {
        let classifier = IntentClassifier::new();
        let tools = test_tools();

        let analysis = classifier.classify("/status", &tools);
        assert_eq!(analysis.intent, Intent::SystemCommand);
        assert!(analysis.confidence >= 0.9);
    }

    #[test]
    fn test_classify_system_command_config() {
        let classifier = IntentClassifier::new();
        let tools = test_tools();

        let analysis = classifier.classify("config show secrets", &tools);
        assert_eq!(analysis.intent, Intent::SystemCommand);
    }

    #[test]
    fn test_classify_conversation() {
        let classifier = IntentClassifier::new();
        let tools = test_tools();

        let analysis = classifier.classify("hello", &tools);
        assert_eq!(analysis.intent, Intent::Conversation);
        assert_eq!(analysis.complexity, TaskComplexity::Trivial);
    }

    #[test]
    fn test_classify_conversation_thanks() {
        let classifier = IntentClassifier::new();
        let tools = test_tools();

        let analysis = classifier.classify("thanks, that was helpful!", &tools);
        assert_eq!(analysis.intent, Intent::Conversation);
    }

    #[test]
    fn test_conversation_does_not_match_task_with_greeting() {
        let classifier = IntentClassifier::new();
        let tools = test_tools();

        // "awesome stuff guys" — removed "awesome" from thanks list
        let analysis = classifier.classify("awesome stuff guys", &tools);
        assert_ne!(analysis.intent, Intent::Conversation, "positive words should not trigger Conversation");

        // "hey build me a website" — greeting + task verb
        let analysis = classifier.classify("hey build me a website", &tools);
        assert_ne!(analysis.intent, Intent::Conversation, "greeting + task verb should not be Conversation");

        // Long message starting with thanks
        let analysis = classifier.classify(
            "thanks for the help, now I need you to refactor the entire authentication module and add JWT support",
            &tools,
        );
        assert_ne!(analysis.intent, Intent::Conversation, "long message with task content should not be Conversation");
    }

    #[test]
    fn test_conversation_matches_pure_social() {
        let classifier = IntentClassifier::new();
        let tools = test_tools();

        assert_eq!(classifier.classify("hello", &tools).intent, Intent::Conversation);
        assert_eq!(classifier.classify("thanks!", &tools).intent, Intent::Conversation);
        assert_eq!(classifier.classify("bye for now", &tools).intent, Intent::Conversation);
        assert_eq!(classifier.classify("good morning team", &tools).intent, Intent::Conversation);
        assert_eq!(classifier.classify("cheers mate", &tools).intent, Intent::Conversation);
    }

    #[test]
    fn test_classify_clarification() {
        let classifier = IntentClassifier::new();
        let tools = test_tools();

        let analysis = classifier.classify("yes", &tools);
        assert_eq!(analysis.intent, Intent::Clarification);
    }

    #[test]
    fn test_classify_clarification_do_it() {
        let classifier = IntentClassifier::new();
        let tools = test_tools();

        let analysis = classifier.classify("do it", &tools);
        assert_eq!(analysis.intent, Intent::Clarification);
    }

    #[test]
    fn test_estimate_complexity_trivial() {
        let classifier = IntentClassifier::new();

        let complexity = classifier.estimate_complexity("list files");
        assert_eq!(complexity, TaskComplexity::Trivial);
    }

    #[test]
    fn test_estimate_complexity_simple() {
        let classifier = IntentClassifier::new();

        let complexity =
            classifier.estimate_complexity("read the config file and check for errors");
        assert!(complexity <= TaskComplexity::Simple);
    }

    #[test]
    fn test_estimate_complexity_complex() {
        let classifier = IntentClassifier::new();

        let complexity = classifier.estimate_complexity(
            "Create a new microservice with a REST API, implement authentication using JWT tokens, \
             set up a PostgreSQL database with migrations, add rate limiting middleware, write \
             comprehensive tests for all endpoints, and deploy the whole thing to AWS using Terraform",
        );
        assert_eq!(complexity, TaskComplexity::Complex);
    }

    #[test]
    fn test_extract_tool_hints_read_file() {
        let classifier = IntentClassifier::new();
        let tools = test_tools();

        let hints = classifier.extract_tool_hints("read the file main.rs", &tools);
        assert!(hints.contains(&"read_file".to_string()));
    }

    #[test]
    fn test_extract_tool_hints_shell() {
        let classifier = IntentClassifier::new();
        let tools = test_tools();

        let hints = classifier.extract_tool_hints("run command cargo test", &tools);
        assert!(hints.contains(&"shell".to_string()));
    }

    #[test]
    fn test_extract_tool_hints_multiple() {
        let classifier = IntentClassifier::new();
        let tools = test_tools();

        let hints = classifier.extract_tool_hints(
            "read file config.toml and then run command cargo build",
            &tools,
        );
        assert!(hints.contains(&"read_file".to_string()));
        assert!(hints.contains(&"shell".to_string()));
    }

    #[test]
    fn test_extract_tool_hints_web_fetch() {
        let classifier = IntentClassifier::new();
        let tools = test_tools();

        let hints = classifier.extract_tool_hints("fetch https://example.com", &tools);
        assert!(hints.contains(&"web_fetch".to_string()));
    }

    #[test]
    fn test_extract_tool_hints_no_match() {
        let classifier = IntentClassifier::new();
        let tools = test_tools();

        let hints = classifier.extract_tool_hints("what is the meaning of life?", &tools);
        assert!(hints.is_empty());
    }

    #[test]
    fn test_count_clauses() {
        assert_eq!(count_clauses("hello"), 1);
        assert_eq!(count_clauses("do this and that"), 2);
        assert_eq!(count_clauses("first, do this, then do that"), 4); // 1 base + 2 commas + 1 "then"
        assert_eq!(
            count_clauses("create a file and then edit it and also deploy it"),
            4
        ); // 1 base + 1 "and then" + 1 "and" + 1 "also"
    }

    #[test]
    fn test_intent_display() {
        assert_eq!(format!("{}", Intent::SimpleQuery), "simple_query");
        assert_eq!(format!("{}", Intent::ComplexTask), "complex_task");
        assert_eq!(format!("{}", Intent::SystemCommand), "system_command");
    }

    #[test]
    fn test_intent_serialization_roundtrip() {
        let analysis = IntentAnalysis {
            intent: Intent::ToolUse,
            complexity: TaskComplexity::Simple,
            confidence: 0.85,
            suggested_tools: vec!["read_file".to_string()],
            requires_confirmation: false,
            reasoning: "test".to_string(),
        };

        let json = serde_json::to_string(&analysis).unwrap();
        let deserialized: IntentAnalysis = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.intent, Intent::ToolUse);
        assert_eq!(deserialized.complexity, TaskComplexity::Simple);
        assert!((deserialized.confidence - 0.85).abs() < f32::EPSILON);
        assert_eq!(deserialized.suggested_tools, vec!["read_file".to_string()]);
    }

    #[test]
    fn test_classify_question_with_mark() {
        let classifier = IntentClassifier::new();
        let tools = test_tools();

        let analysis = classifier.classify("How do I configure the database connection?", &tools);
        assert_eq!(analysis.intent, Intent::SimpleQuery);
    }

    #[test]
    fn test_classify_tool_use_list_dir() {
        let classifier = IntentClassifier::new();
        let tools = test_tools();

        let analysis = classifier.classify("list the files in the current directory", &tools);
        assert_eq!(analysis.intent, Intent::ToolUse);
        assert!(analysis.suggested_tools.contains(&"list_dir".to_string()));
    }

    #[test]
    fn test_classify_tool_use_write() {
        let classifier = IntentClassifier::new();
        let tools = test_tools();

        let analysis =
            classifier.classify("create a file called hello.txt with some content", &tools);
        assert_eq!(analysis.intent, Intent::ToolUse);
        assert!(analysis.suggested_tools.contains(&"write_file".to_string()));
    }

    #[test]
    fn test_default_classifier() {
        let classifier = IntentClassifier::default();
        assert_eq!(classifier.min_substantive_words, 5);
    }

    #[test]
    fn test_classify_tool_use_edit() {
        let classifier = IntentClassifier::new();
        let tools = test_tools();

        let analysis = classifier.classify(
            "edit the file config.toml and replace the old value",
            &tools,
        );
        assert_eq!(analysis.intent, Intent::ToolUse);
        assert!(analysis.suggested_tools.contains(&"edit_file".to_string()));
    }

    #[test]
    fn test_classify_tool_use_web() {
        let classifier = IntentClassifier::new();
        let tools = test_tools();

        let analysis = classifier.classify(
            "fetch the webpage at https://example.com and show me the content",
            &tools,
        );
        assert_eq!(analysis.intent, Intent::ToolUse);
        assert!(analysis.suggested_tools.contains(&"web_fetch".to_string()));
    }

    #[test]
    fn test_classify_unknown_input() {
        let classifier = IntentClassifier::new();
        let tools = test_tools();

        // Empty string
        let analysis_empty = classifier.classify("", &tools);
        // Should be clarification or fallback
        assert!(
            analysis_empty.intent == Intent::Clarification
                || analysis_empty.intent == Intent::SimpleQuery,
            "Empty input should be clarification or fallback, got: {:?}",
            analysis_empty.intent
        );

        // Gibberish
        let analysis_gibberish = classifier.classify("xkcd asdf qwer", &tools);
        // Short gibberish with no pattern should be clarification
        assert!(
            analysis_gibberish.intent == Intent::Clarification
                || analysis_gibberish.intent == Intent::SimpleQuery,
            "Gibberish should be clarification or simple query, got: {:?}",
            analysis_gibberish.intent
        );
    }

    #[test]
    fn test_estimate_complexity_very_long() {
        let classifier = IntentClassifier::new();

        // Very long message with many operations and clauses
        let long_msg = "First, create a new project directory, then initialize git, \
            and install all dependencies, after that configure the database connection, \
            set up the authentication middleware, implement the REST API endpoints, \
            write comprehensive unit tests and integration tests, build the Docker image, \
            deploy it to the staging server, run the smoke tests, and finally promote to production, \
            additionally set up monitoring and alerting, followed by documentation updates";

        let complexity = classifier.estimate_complexity(long_msg);
        assert_eq!(
            complexity,
            TaskComplexity::Complex,
            "Very long multi-step message should be Complex"
        );
    }

    #[test]
    fn test_classify_question_without_mark() {
        let classifier = IntentClassifier::new();
        let tools = test_tools();

        // "how" starts the message, which triggers is_simple_query
        let analysis = classifier.classify("how do I set up a Rust project", &tools);
        assert_eq!(
            analysis.intent,
            Intent::SimpleQuery,
            "Question-word sentence without ? should still be SimpleQuery"
        );

        // "what" starts the message
        let analysis2 =
            classifier.classify("what is the best way to handle errors in Rust", &tools);
        assert_eq!(analysis2.intent, Intent::SimpleQuery);
    }
}
