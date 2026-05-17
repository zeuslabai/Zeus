//! Message chunking for channel delivery
//!
//! Splits long messages into chunks respecting platform limits and message boundaries.

use std::time::Duration;

/// Message chunker with platform-specific limits
pub struct MessageChunker {
    max_length: usize,
    delay_ms: u64,
}

impl MessageChunker {
    /// Get chunker with limits for a specific channel type
    pub fn for_channel(channel_type: &str) -> Self {
        let (max_length, delay_ms) = match channel_type {
            "telegram" => (4096, 100),
            "discord" => (2000, 500),
            "slack" => (40000, 1000),
            "email" => (50000, 0),
            "imessage" => (20000, 500),
            _ => (4096, 100), // Default to Telegram limits
        };

        Self {
            max_length,
            delay_ms,
        }
    }

    /// Create a custom chunker with specific limits
    pub fn new(max_length: usize, delay_ms: u64) -> Self {
        Self {
            max_length,
            delay_ms,
        }
    }

    /// Split message into chunks respecting boundaries
    pub fn chunk(&self, content: &str) -> Vec<String> {
        // If content fits within limit, return as-is
        if content.len() <= self.max_length {
            return vec![content.to_string()];
        }

        // Step 1: Split on paragraph boundaries (\n\n)
        let paragraphs: Vec<&str> = content.split("\n\n").collect();
        let mut segments = Vec::new();

        for para in paragraphs {
            if para.len() <= self.max_length {
                segments.push(para.to_string());
            } else {
                // Step 2: Paragraph too long, split on line boundaries (\n)
                let lines: Vec<&str> = para.split('\n').collect();
                let mut line_segments = Vec::new();

                for line in lines {
                    if line.len() <= self.max_length {
                        line_segments.push(line.to_string());
                    } else {
                        // Step 3: Line too long, split on sentence boundaries (. )
                        let sentences: Vec<&str> = line.split(". ").collect();
                        let mut sentence_segments = Vec::new();

                        for (idx, sentence) in sentences.iter().enumerate() {
                            let mut sent = sentence.to_string();
                            // Re-add the period and space except for last sentence
                            if idx < sentences.len() - 1 && !sent.ends_with('.') {
                                sent.push_str(". ");
                            }

                            if sent.len() <= self.max_length {
                                sentence_segments.push(sent);
                            } else {
                                // Step 4: Hard split at max_length, respecting UTF-8 boundaries
                                let hard_split = self.hard_split(&sent);
                                sentence_segments.extend(hard_split);
                            }
                        }

                        // Greedily combine sentences
                        line_segments.extend(self.combine_segments(sentence_segments, ". "));
                    }
                }

                // Greedily combine lines
                segments.extend(self.combine_segments(line_segments, "\n"));
            }
        }

        // Greedily combine paragraphs
        self.combine_segments(segments, "\n\n")
    }

    /// Get delay between chunk sends
    pub fn delay(&self) -> Duration {
        Duration::from_millis(self.delay_ms)
    }

    /// Hard split a string at max_length boundaries, respecting UTF-8 char boundaries
    fn hard_split(&self, text: &str) -> Vec<String> {
        let mut chunks = Vec::new();
        let mut start = 0;

        while start < text.len() {
            let mut end = start + self.max_length;

            // Don't exceed text length
            if end >= text.len() {
                chunks.push(text[start..].to_string());
                break;
            }

            // Adjust to not split UTF-8 character
            while !text.is_char_boundary(end) && end > start {
                end -= 1;
            }

            chunks.push(text[start..end].to_string());
            start = end;
        }

        chunks
    }

    /// Greedily combine segments up to max_length with the given separator
    fn combine_segments(&self, segments: Vec<String>, separator: &str) -> Vec<String> {
        if segments.is_empty() {
            return Vec::new();
        }

        let mut result = Vec::new();
        let mut current = String::new();

        for segment in segments {
            let segment_trimmed = segment.trim();
            if segment_trimmed.is_empty() {
                continue;
            }

            if current.is_empty() {
                current = segment;
            } else {
                // Check if adding this segment would exceed limit
                let combined_len = current.len() + separator.len() + segment.len();
                if combined_len <= self.max_length {
                    current.push_str(separator);
                    current.push_str(&segment);
                } else {
                    // Current chunk is full, start a new one
                    result.push(current);
                    current = segment;
                }
            }
        }

        // Don't forget the last chunk
        if !current.is_empty() {
            result.push(current);
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_message_unchanged() {
        let chunker = MessageChunker::for_channel("telegram");
        let content = "Hello, world!";
        let chunks = chunker.chunk(content);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "Hello, world!");
    }

    #[test]
    fn test_paragraph_boundary_splitting() {
        let chunker = MessageChunker::new(50, 0);
        let para1 = "A".repeat(45); // 45 chars
        let para2 = "B".repeat(45); // 45 chars
        let content = format!("{}\n\n{}", para1, para2);

        let chunks = chunker.chunk(&content);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], para1);
        assert_eq!(chunks[1], para2);
    }

    #[test]
    fn test_line_boundary_splitting() {
        let chunker = MessageChunker::new(30, 0);
        let line1 = "A".repeat(25);
        let line2 = "B".repeat(25);
        let content = format!("{}\n{}", line1, line2);

        let chunks = chunker.chunk(&content);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], line1);
        assert_eq!(chunks[1], line2);
    }

    #[test]
    fn test_sentence_boundary_splitting() {
        let chunker = MessageChunker::new(50, 0);
        let sent1 = "A".repeat(30);
        let sent2 = "B".repeat(30);
        let content = format!("{}. {}", sent1, sent2);

        let chunks = chunker.chunk(&content);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].starts_with(&sent1));
        assert!(chunks[1].starts_with(&sent2));
    }

    #[test]
    fn test_hard_split_no_breaks() {
        let chunker = MessageChunker::new(20, 0);
        let content = "A".repeat(50);

        let chunks = chunker.chunk(&content);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), 20);
        assert_eq!(chunks[1].len(), 20);
        assert_eq!(chunks[2].len(), 10);
    }

    #[test]
    fn test_correct_limits_per_channel() {
        let telegram = MessageChunker::for_channel("telegram");
        assert_eq!(telegram.max_length, 4096);

        let discord = MessageChunker::for_channel("discord");
        assert_eq!(discord.max_length, 2000);

        let slack = MessageChunker::for_channel("slack");
        assert_eq!(slack.max_length, 40000);

        let email = MessageChunker::for_channel("email");
        assert_eq!(email.max_length, 50000);

        let imessage = MessageChunker::for_channel("imessage");
        assert_eq!(imessage.max_length, 20000);

        let unknown = MessageChunker::for_channel("unknown");
        assert_eq!(unknown.max_length, 4096); // Default
    }

    #[test]
    fn test_multiple_short_paragraphs_combine() {
        let chunker = MessageChunker::new(100, 0);
        let para1 = "Short para 1";
        let para2 = "Short para 2";
        let para3 = "Short para 3";
        let content = format!("{}\n\n{}\n\n{}", para1, para2, para3);

        let chunks = chunker.chunk(&content);
        // All three should combine into one chunk
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains(para1));
        assert!(chunks[0].contains(para2));
        assert!(chunks[0].contains(para3));
    }

    #[test]
    fn test_utf8_boundary_respect() {
        let chunker = MessageChunker::new(10, 0);
        // UTF-8 multibyte characters
        let content = "🔥🔥🔥🔥🔥🔥🔥🔥🔥🔥"; // Each emoji is 4 bytes

        let chunks = chunker.chunk(&content);
        // Should not split in the middle of an emoji
        for chunk in &chunks {
            assert!(chunk.is_ascii() || chunk.chars().all(|c| !c.is_ascii()));
        }
    }

    #[test]
    fn test_delay_values() {
        let telegram = MessageChunker::for_channel("telegram");
        assert_eq!(telegram.delay(), Duration::from_millis(100));

        let discord = MessageChunker::for_channel("discord");
        assert_eq!(discord.delay(), Duration::from_millis(500));

        let email = MessageChunker::for_channel("email");
        assert_eq!(email.delay(), Duration::from_millis(0));
    }

    #[test]
    fn test_empty_segments_filtered() {
        let chunker = MessageChunker::new(100, 0);
        let content = "Text\n\n\n\nMore text"; // Multiple empty lines

        let chunks = chunker.chunk(&content);
        // Should combine into one, filtering empty segments
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn test_greedy_combination() {
        let chunker = MessageChunker::new(30, 0);
        let para1 = "First"; // 5 chars
        let para2 = "Second"; // 6 chars
        let para3 = "Third"; // 5 chars
        let content = format!("{}\n\n{}\n\n{}", para1, para2, para3);

        let chunks = chunker.chunk(&content);
        // All should fit: 5 + 2 + 6 + 2 + 5 = 20 chars (within 30)
        assert_eq!(chunks.len(), 1);
    }
}
