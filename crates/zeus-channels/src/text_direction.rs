//! Text Direction Detection for Zeus Channels
//!
//! Detects RTL (right-to-left) scripts in message text using Unicode code point ranges.
//! Supports: Arabic, Hebrew, Syriac, Thaana, NKo, Samaritan, Mandaic.

use serde::{Deserialize, Serialize};

/// Text direction of a message
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextDirection {
    /// Left-to-right (Latin, CJK, Cyrillic, etc.)
    #[default]
    Ltr,
    /// Right-to-left (Arabic, Hebrew, Syriac, Thaana, etc.)
    Rtl,
}

impl std::fmt::Display for TextDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ltr => write!(f, "ltr"),
            Self::Rtl => write!(f, "rtl"),
        }
    }
}

/// Check if a single character belongs to an RTL script.
///
/// Covers the following Unicode blocks:
/// - Arabic (0x0600–0x06FF)
/// - Arabic Supplement (0x0750–0x077F)
/// - Arabic Extended-A (0x08A0–0x08FF)
/// - Arabic Presentation Forms-A (0xFB50–0xFDFF)
/// - Arabic Presentation Forms-B (0xFE70–0xFEFF)
/// - Hebrew (0x0590–0x05FF)
/// - Syriac (0x0700–0x074F)
/// - Syriac Supplement (0x0860–0x086F)
/// - Thaana (0x0780–0x07BF)
/// - NKo (0x07C0–0x07FF)
/// - Samaritan (0x0800–0x083F)
/// - Mandaic (0x0840–0x085F)
fn is_rtl_char(c: char) -> bool {
    let cp = c as u32;
    matches!(cp,
        0x0590..=0x05FF   // Hebrew
        | 0x0600..=0x06FF // Arabic
        | 0x0700..=0x074F // Syriac
        | 0x0750..=0x077F // Arabic Supplement
        | 0x0780..=0x07BF // Thaana
        | 0x07C0..=0x07FF // NKo
        | 0x0800..=0x083F // Samaritan
        | 0x0840..=0x085F // Mandaic
        | 0x0860..=0x086F // Syriac Supplement
        | 0x08A0..=0x08FF // Arabic Extended-A
        | 0xFB50..=0xFDFF // Arabic Presentation Forms-A
        | 0xFE70..=0xFEFF // Arabic Presentation Forms-B
    )
}

/// Check if a character is a strong LTR character (Latin, CJK, Cyrillic, Greek, etc.)
///
/// We only count "strong" directional characters — skip digits, punctuation, whitespace.
fn is_ltr_char(c: char) -> bool {
    let cp = c as u32;
    matches!(cp,
        0x0041..=0x005A   // Latin uppercase
        | 0x0061..=0x007A // Latin lowercase
        | 0x00C0..=0x024F // Latin Extended
        | 0x0370..=0x03FF // Greek
        | 0x0400..=0x04FF // Cyrillic
        | 0x0500..=0x052F // Cyrillic Supplement
        | 0x1100..=0x11FF // Hangul Jamo
        | 0x2E80..=0x9FFF // CJK
        | 0xAC00..=0xD7AF // Hangul Syllables
    )
}

/// Detect the dominant text direction of a string.
///
/// Counts RTL vs LTR strong directional characters. If the first strong
/// directional character is RTL, or if RTL characters outnumber LTR characters,
/// the text is considered RTL.
///
/// Returns `TextDirection::Ltr` for empty strings or strings with no strong
/// directional characters (digits, punctuation, emoji only).
pub fn detect_direction(text: &str) -> TextDirection {
    let mut rtl_count: u32 = 0;
    let mut ltr_count: u32 = 0;
    let mut first_strong: Option<TextDirection> = None;

    for c in text.chars() {
        if is_rtl_char(c) {
            rtl_count += 1;
            if first_strong.is_none() {
                first_strong = Some(TextDirection::Rtl);
            }
        } else if is_ltr_char(c) {
            ltr_count += 1;
            if first_strong.is_none() {
                first_strong = Some(TextDirection::Ltr);
            }
        }
    }

    // If no strong characters found, default to LTR
    if rtl_count == 0 && ltr_count == 0 {
        return TextDirection::Ltr;
    }

    // First strong character rule: if the first strong char is RTL, it's RTL
    if first_strong == Some(TextDirection::Rtl) {
        return TextDirection::Rtl;
    }

    // Majority rule: if RTL chars dominate, it's RTL
    if rtl_count > ltr_count {
        return TextDirection::Rtl;
    }

    TextDirection::Ltr
}

/// Convenience: detect direction and return as an HTML dir attribute value ("ltr" or "rtl")
pub fn dir_attr(text: &str) -> &'static str {
    match detect_direction(text) {
        TextDirection::Ltr => "ltr",
        TextDirection::Rtl => "rtl",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // === TextDirection enum ===

    #[test]
    fn test_default_is_ltr() {
        assert_eq!(TextDirection::default(), TextDirection::Ltr);
    }

    #[test]
    fn test_display() {
        assert_eq!(TextDirection::Ltr.to_string(), "ltr");
        assert_eq!(TextDirection::Rtl.to_string(), "rtl");
    }

    #[test]
    fn test_serde() {
        let json = serde_json::to_string(&TextDirection::Rtl).expect("should serialize to JSON");
        assert_eq!(json, "\"rtl\"");
        let back: TextDirection =
            serde_json::from_str("\"ltr\"").expect("should parse successfully");
        assert_eq!(back, TextDirection::Ltr);
    }

    // === is_rtl_char ===

    #[test]
    fn test_hebrew_chars() {
        assert!(is_rtl_char('\u{05D0}')); // Alef
        assert!(is_rtl_char('\u{05E9}')); // Shin
        assert!(is_rtl_char('\u{05EA}')); // Tav
    }

    #[test]
    fn test_arabic_chars() {
        assert!(is_rtl_char('\u{0627}')); // Alif
        assert!(is_rtl_char('\u{0628}')); // Ba
        assert!(is_rtl_char('\u{064A}')); // Ya
    }

    #[test]
    fn test_syriac_char() {
        assert!(is_rtl_char('\u{0710}')); // Syriac Alaph
    }

    #[test]
    fn test_thaana_char() {
        assert!(is_rtl_char('\u{0780}')); // Thaana Haa
    }

    #[test]
    fn test_nko_char() {
        assert!(is_rtl_char('\u{07C0}')); // NKo digit zero
    }

    #[test]
    fn test_samaritan_char() {
        assert!(is_rtl_char('\u{0800}')); // Samaritan Alaf
    }

    #[test]
    fn test_mandaic_char() {
        assert!(is_rtl_char('\u{0840}')); // Mandaic Halqa
    }

    #[test]
    fn test_arabic_presentation_forms() {
        assert!(is_rtl_char('\u{FB50}')); // Arabic Presentation Forms-A
        assert!(is_rtl_char('\u{FE70}')); // Arabic Presentation Forms-B
    }

    #[test]
    fn test_latin_not_rtl() {
        assert!(!is_rtl_char('A'));
        assert!(!is_rtl_char('z'));
        assert!(!is_rtl_char('0'));
    }

    // === detect_direction ===

    #[test]
    fn test_empty_string() {
        assert_eq!(detect_direction(""), TextDirection::Ltr);
    }

    #[test]
    fn test_english_text() {
        assert_eq!(detect_direction("Hello, world!"), TextDirection::Ltr);
    }

    #[test]
    fn test_hebrew_text() {
        assert_eq!(detect_direction("שלום עולם"), TextDirection::Rtl);
    }

    #[test]
    fn test_arabic_text() {
        assert_eq!(detect_direction("مرحبا بالعالم"), TextDirection::Rtl);
    }

    #[test]
    fn test_hebrew_sentence() {
        assert_eq!(detect_direction("אני אוהב לתכנת בראסט"), TextDirection::Rtl);
    }

    #[test]
    fn test_arabic_sentence() {
        assert_eq!(
            detect_direction("أنا أحب البرمجة بلغة رست"),
            TextDirection::Rtl
        );
    }

    #[test]
    fn test_mixed_hebrew_english_rtl_dominant() {
        // Hebrew dominant
        assert_eq!(detect_direction("שלום hello עולם"), TextDirection::Rtl);
    }

    #[test]
    fn test_mixed_english_hebrew_ltr_dominant() {
        // English dominant — but first strong is LTR
        assert_eq!(detect_direction("Hello world שלום"), TextDirection::Ltr);
    }

    #[test]
    fn test_numbers_only() {
        // No strong directional characters
        assert_eq!(detect_direction("12345"), TextDirection::Ltr);
    }

    #[test]
    fn test_punctuation_only() {
        assert_eq!(detect_direction("!@#$%"), TextDirection::Ltr);
    }

    #[test]
    fn test_emoji_only() {
        assert_eq!(detect_direction("🎉🚀✨"), TextDirection::Ltr);
    }

    #[test]
    fn test_hebrew_with_numbers() {
        assert_eq!(detect_direction("שלום 123 עולם"), TextDirection::Rtl);
    }

    #[test]
    fn test_arabic_with_punctuation() {
        assert_eq!(detect_direction("!مرحبا"), TextDirection::Rtl);
    }

    #[test]
    fn test_first_strong_rtl_wins() {
        // First strong char is RTL, even if equal counts
        assert_eq!(detect_direction("אhello"), TextDirection::Rtl);
    }

    #[test]
    fn test_cyrillic_is_ltr() {
        assert_eq!(detect_direction("Привет мир"), TextDirection::Ltr);
    }

    #[test]
    fn test_chinese_is_ltr() {
        assert_eq!(detect_direction("你好世界"), TextDirection::Ltr);
    }

    #[test]
    fn test_syriac_text() {
        // Syriac Alaph + Beth
        assert_eq!(
            detect_direction("\u{0710}\u{0712}\u{0713}"),
            TextDirection::Rtl
        );
    }

    #[test]
    fn test_thaana_text() {
        // Thaana (Maldivian script)
        assert_eq!(
            detect_direction("\u{0780}\u{0781}\u{0782}"),
            TextDirection::Rtl
        );
    }

    // === dir_attr ===

    #[test]
    fn test_dir_attr_ltr() {
        assert_eq!(dir_attr("Hello"), "ltr");
    }

    #[test]
    fn test_dir_attr_rtl() {
        assert_eq!(dir_attr("שלום"), "rtl");
    }

    // === is_ltr_char ===

    #[test]
    fn test_ltr_latin() {
        assert!(is_ltr_char('A'));
        assert!(is_ltr_char('z'));
    }

    #[test]
    fn test_ltr_greek() {
        assert!(is_ltr_char('α'));
        assert!(is_ltr_char('Ω'));
    }

    #[test]
    fn test_ltr_cyrillic() {
        assert!(is_ltr_char('Б'));
        assert!(is_ltr_char('я'));
    }

    #[test]
    fn test_digits_not_strong() {
        assert!(!is_ltr_char('0'));
        assert!(!is_ltr_char('9'));
        assert!(!is_rtl_char('5'));
    }

    #[test]
    fn test_space_not_strong() {
        assert!(!is_ltr_char(' '));
        assert!(!is_rtl_char(' '));
    }
}
