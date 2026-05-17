//! Streaming markdown buffer for the TUI chat panel.
//!
//! The problem: `render_markdown_with_width()` is called every frame with the full
//! message content. During streaming, partial constructs like an unclosed code fence
//! (` ```rust\nfn mai`) render as broken text.
//!
//! The solution: `MarkdownStreamState` buffers incoming deltas and only exposes
//! content up to the last "safe boundary" — a closed code fence, a blank line, or
//! end of a list item. Partial blocks stay buffered until they complete.
//!
//! # Usage
//!
//! ```rust,ignore
//! let mut state = MarkdownStreamState::default();
//!
//! // During streaming — call push() with each delta
//! if let Some(flushed) = state.push("## Hello\n\n") {
//!     // flushed is safe to pass to render_markdown_with_width()
//! }
//!
//! // Partial content with an open code fence — returns None
//! assert!(state.push("```rust\nfn mai").is_none());
//!
//! // Fence closes — now it flushes
//! let flushed = state.push("n() {}\n```\n").unwrap();
//!
//! // When the stream ends, drain whatever's left
//! if let Some(tail) = state.finish() {
//!     // render tail as raw text with cursor
//! }
//! ```

/// Buffers streaming markdown deltas and flushes only completed blocks.
#[derive(Debug, Default, Clone)]
pub struct MarkdownStreamState {
    /// Accumulated content not yet flushed to the renderer.
    pending: String,
    /// All content that has been flushed (safe for full re-render).
    flushed: String,
}

impl MarkdownStreamState {
    /// Create a new empty stream state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a streaming delta. Returns the newly-safe content (up to the last
    /// safe boundary) if any complete blocks are ready, or `None` if the buffer
    /// only contains partial content.
    ///
    /// The returned string should be passed to `render_markdown_with_width()`.
    /// It contains *only* the newly flushed chunk — append it to previously
    /// rendered content rather than re-rendering the whole message.
    #[must_use]
    pub fn push(&mut self, delta: &str) -> Option<String> {
        self.pending.push_str(delta);
        self.try_flush()
    }

    /// Returns all content flushed so far (safe, completed blocks).
    /// Use this for re-rendering the stable portion of the message.
    #[must_use]
    pub fn flushed(&self) -> &str {
        &self.flushed
    }

    /// Returns the unflushed tail (partial/incomplete block in progress).
    /// Render this as raw text with the streaming cursor appended.
    #[must_use]
    pub fn pending(&self) -> &str {
        &self.pending
    }

    /// Called when the stream ends. Drains any remaining buffered content.
    /// Returns the remaining pending content (if any) for final rendering.
    /// After this call, `pending()` will be empty.
    #[must_use]
    pub fn finish(&mut self) -> Option<String> {
        if self.pending.trim().is_empty() {
            self.pending.clear();
            None
        } else {
            let tail = std::mem::take(&mut self.pending);
            self.flushed.push_str(&tail);
            Some(tail)
        }
    }

    /// Reset the state entirely (e.g. for a new message).
    pub fn reset(&mut self) {
        self.pending.clear();
        self.flushed.clear();
    }

    /// Internal: attempt to flush up to the last safe boundary.
    fn try_flush(&mut self) -> Option<String> {
        let split = find_stream_safe_boundary(&self.pending)?;
        let ready = self.pending[..split].to_string();
        self.pending.drain(..split);
        self.flushed.push_str(&ready);
        Some(ready)
    }
}

/// Flush pending content after this many characters even without a hard boundary.
/// Prevents frozen display during long prose paragraphs with no blank lines.
const FLUSH_THRESHOLD_CHARS: usize = 120;

/// Find the byte offset of the last "safe" flush boundary in a markdown string.
///
/// A safe boundary is a point where rendering cannot produce broken output:
/// - After a blank line (paragraph boundary)
/// - After a closing code fence (``` or ~~~)
/// - After a sentence boundary (`. `, `! `, `? ` outside a fence) — reduces
///   perceived streaming latency for long prose without blank lines
/// - After FLUSH_THRESHOLD_CHARS characters without any boundary (fallback)
///
/// Returns `None` if no safe boundary exists (e.g. the buffer contains only an
/// open code fence with no closing fence yet).
fn find_stream_safe_boundary(markdown: &str) -> Option<usize> {
    let mut in_fence = false;
    let mut last_boundary: Option<usize> = None;
    let mut cursor = 0usize;
    let bytes = markdown.as_bytes();

    for line in markdown.split_inclusive('\n') {
        let trimmed = line.trim_start();

        // Code fence toggle
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            if !in_fence {
                // Closing fence — everything up to and including this line is safe
                last_boundary = Some(cursor + line.len());
            }
            cursor += line.len();
            continue;
        }

        // Inside an open fence — nothing is safe, skip sentence scanning
        if in_fence {
            cursor += line.len();
            continue;
        }

        // Blank line outside a fence — paragraph boundary
        if trimmed.is_empty() {
            last_boundary = Some(cursor + line.len());
            cursor += line.len();
            continue;
        }

        // Scan for sentence-end boundaries within this prose line.
        // A sentence ends at `. `, `! `, or `? ` (space after punctuation)
        // or at `.\n`, `!\n`, `?\n` (end of line). This lets us flush during
        // long paragraphs without waiting for a blank line.
        let line_start = cursor;
        let line_end = cursor + line.len();
        let mut i = line_start;
        while i + 1 < line_end {
            let b = bytes[i];
            if matches!(b, b'.' | b'!' | b'?') {
                let next = bytes[i + 1];
                if next == b' ' || next == b'\n' {
                    // +2 to include the punctuation and the space/newline
                    last_boundary = Some(i + 2);
                }
            }
            i += 1;
        }

        cursor += line.len();
    }

    // Fallback: if we've accumulated more than FLUSH_THRESHOLD_CHARS without
    // any boundary and we're not inside a fence, flush at the last whitespace.
    if last_boundary.is_none() && !in_fence && markdown.len() >= FLUSH_THRESHOLD_CHARS {
        // Find the last ASCII whitespace byte to avoid splitting mid-word
        if let Some(pos) = markdown[..markdown.len()]
            .rfind(|c: char| c.is_ascii_whitespace())
        {
            last_boundary = Some(pos + 1);
        }
    }

    last_boundary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_flushes_at_blank_lines() {
        let mut state = MarkdownStreamState::new();
        // No blank line yet — nothing safe to flush
        assert!(state.push("Some text").is_none());
        // Blank line completes the paragraph
        let flushed = state.push("\n\n").expect("blank line is a safe boundary");
        assert!(flushed.contains("Some text"));
    }

    #[test]
    fn open_code_fence_blocks_flush() {
        let mut state = MarkdownStreamState::new();
        // Open fence — unsafe
        assert!(state.push("```rust\nfn mai").is_none());
        // Still open — still unsafe
        assert!(state.push("n() {}\n").is_none());
        // Closing fence — now safe
        let flushed = state.push("```\n").expect("closed fence flushes");
        assert!(flushed.contains("fn main()"));
    }

    #[test]
    fn completed_paragraph_before_open_fence_flushes_paragraph() {
        let mut state = MarkdownStreamState::new();
        // Paragraph completes
        let _ = state.push("Hello world\n\n");
        // Now an open fence starts — the paragraph was already flushed
        assert!(state.push("```python\nprint(").is_none());
        assert_eq!(state.pending(), "```python\nprint(");
    }

    #[test]
    fn finish_drains_pending() {
        let mut state = MarkdownStreamState::new();
        state.push("Incomplete line with no boundary");
        let tail = state.finish().expect("finish drains pending");
        assert!(tail.contains("Incomplete line"));
        assert!(state.pending().is_empty());
    }

    #[test]
    fn finish_returns_none_on_empty_pending() {
        let mut state = MarkdownStreamState::new();
        state.push("Done\n\n"); // flushes via push
        assert!(state.finish().is_none());
    }

    #[test]
    fn reset_clears_all_state() {
        let mut state = MarkdownStreamState::new();
        state.push("some content\n\n");
        state.reset();
        assert!(state.flushed().is_empty());
        assert!(state.pending().is_empty());
    }

    #[test]
    fn multiple_paragraphs_flush_incrementally() {
        let mut state = MarkdownStreamState::new();
        let chunk1 = state.push("# Heading\n\n").expect("heading + blank flushes");
        assert!(chunk1.contains("Heading"));

        let chunk2 = state.push("Body text\n\n").expect("body + blank flushes");
        assert!(chunk2.contains("Body text"));

        assert!(state.pending().is_empty());
    }

    #[test]
    fn tilde_fence_also_works() {
        let mut state = MarkdownStreamState::new();
        assert!(state.push("~~~\nsome code\n").is_none());
        let flushed = state.push("~~~\n").expect("tilde close fence flushes");
        assert!(flushed.contains("some code"));
    }
}
