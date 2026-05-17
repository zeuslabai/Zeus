//! Example demonstrating context window management and session compaction
//!
//! This example shows how to use the ContextManager to automatically compact
//! conversation history when it approaches token limits.

use zeus_core::{Message, SessionCompactionConfig};
use zeus_session::ContextManager;

fn main() {
    // Create configuration
    let config = SessionCompactionConfig {
        max_context_tokens: 1000,
        compaction_threshold: 0.8, // Compact at 80% of max (800 tokens)
        summary_model: None,
        compaction_timeout_secs: None,
        ollama_compaction_threshold: None,
        flush_timeout_secs: None,
    };

    // Initialize context manager
    let manager = ContextManager::new(&config);

    // Simulate a conversation with many messages
    let mut messages = vec![];

    // Add some system messages (these will be preserved)
    messages.push(Message::system("You are a helpful AI assistant."));

    // Add conversation messages
    for i in 1..=20 {
        messages.push(Message::user(format!("User message {}", i)));
        messages.push(Message::assistant(format!(
            "This is a response to message {}. Here's some additional context and information \
             that makes the message longer so we can test token estimation.",
            i
        )));
    }

    // Check token estimation
    let estimated_tokens = ContextManager::estimate_tokens(&messages);
    println!("Total messages: {}", messages.len());
    println!("Estimated tokens: {}", estimated_tokens);

    // Check if compaction is needed
    if manager.needs_compaction(&messages) {
        println!(
            "\n⚠️  Compaction needed! Context is over {}% of limit",
            config.compaction_threshold * 100.0
        );
        println!(
            "Trigger point: {} tokens",
            (config.max_context_tokens as f32 * config.compaction_threshold) as usize
        );
    } else {
        println!("\n✓ No compaction needed yet");
    }

    // Demonstrate compaction logic
    println!("\n--- Compaction Strategy ---");
    println!("When compaction is triggered:");
    println!("1. System messages are preserved");
    println!("2. Oldest 60% of non-system messages are summarized");
    println!("3. Recent 40% of messages are kept intact");
    println!("4. Summary replaces the old messages");

    // Simulate compaction calculation
    let non_system_count = messages
        .iter()
        .filter(|m| !matches!(m.role, zeus_core::Role::System))
        .count();
    let to_compact = (non_system_count as f32 * 0.6).ceil() as usize;
    let to_keep = non_system_count - to_compact;

    println!("\nFor {} non-system messages:", non_system_count);
    println!("  - Would compact (summarize): {} messages", to_compact);
    println!("  - Would keep: {} recent messages", to_keep);

    // Show a simpler example with exact threshold
    println!("\n--- Threshold Example ---");
    let threshold_tokens =
        (config.max_context_tokens as f32 * config.compaction_threshold) as usize;
    let chars_needed = threshold_tokens * 4;

    println!(
        "With max_tokens={}, threshold={}:",
        config.max_context_tokens, config.compaction_threshold
    );
    println!("  - Compaction triggers at: {} tokens", threshold_tokens);
    println!("  - Approximately {} characters", chars_needed);

    // Test with a message at exactly the threshold
    let test_message = vec![Message::user("x".repeat(chars_needed))];
    let test_tokens = ContextManager::estimate_tokens(&test_message);
    let would_compact = manager.needs_compaction(&test_message);

    println!("\nTest message with {} chars:", chars_needed);
    println!("  - Estimated tokens: {}", test_tokens);
    println!("  - Would compact: {}", would_compact);

    // Test with a message just over the threshold
    let test_message_over = vec![Message::user("x".repeat(chars_needed + 4))];
    let test_tokens_over = ContextManager::estimate_tokens(&test_message_over);
    let would_compact_over = manager.needs_compaction(&test_message_over);

    println!(
        "\nTest message with {} chars (just over):",
        chars_needed + 4
    );
    println!("  - Estimated tokens: {}", test_tokens_over);
    println!("  - Would compact: {}", would_compact_over);

    println!("\n✓ Example complete!");
}
