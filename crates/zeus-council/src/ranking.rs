/// Parse model rankings from stage 2 review text.
///
/// We ask each reviewer to emit lines like:
///   RANK: Model A=9, Model B=7, Model C=8
/// This module parses that structured output into a score map.

use std::collections::HashMap;

/// Parse a ranking line from a review response.
/// Returns a map of label → score, or empty map if none found.
pub fn parse_rankings(review_text: &str) -> HashMap<String, f32> {
    let mut scores = HashMap::new();

    for line in review_text.lines() {
        let trimmed = line.trim();
        if !trimmed.to_uppercase().starts_with("RANK:") {
            continue;
        }
        // Everything after "RANK:"
        let rest = &trimmed["RANK:".len()..].trim().to_string().clone();
        for entry in rest.split(',') {
            let entry = entry.trim();
            if let Some(eq_pos) = entry.find('=') {
                let label = entry[..eq_pos].trim().to_string();
                let score_str = entry[eq_pos + 1..].trim();
                if let Ok(score) = score_str.parse::<f32>() {
                    scores.insert(label, score);
                }
            }
        }
        // Take only the first RANK: line
        break;
    }

    scores
}

/// Aggregate rankings from multiple reviewers into a consensus leaderboard.
/// Returns labels sorted by average score descending.
pub fn aggregate_rankings(reviews: &[crate::ModelReview]) -> Vec<(String, f32)> {
    let mut totals: HashMap<String, (f32, usize)> = HashMap::new();

    for review in reviews {
        for (label, score) in &review.rankings {
            let entry = totals.entry(label.clone()).or_insert((0.0, 0));
            entry.0 += score;
            entry.1 += 1;
        }
    }

    let mut averages: Vec<(String, f32)> = totals
        .into_iter()
        .map(|(label, (total, count))| (label, total / count as f32))
        .collect();

    averages.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    averages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rankings_basic() {
        let text = "Good responses overall.\nRANK: Model A=9, Model B=7, Model C=8\nSome commentary.";
        let scores = parse_rankings(text);
        assert_eq!(scores.get("Model A"), Some(&9.0));
        assert_eq!(scores.get("Model B"), Some(&7.0));
        assert_eq!(scores.get("Model C"), Some(&8.0));
    }

    #[test]
    fn test_parse_rankings_missing() {
        let text = "No ranking line here.";
        let scores = parse_rankings(text);
        assert!(scores.is_empty());
    }

    #[test]
    fn test_aggregate_rankings() {
        use crate::ModelReview;
        let reviews = vec![
            ModelReview {
                reviewer_id: "model-1".into(),
                review_text: "".into(),
                rankings: [("Model A".into(), 9.0), ("Model B".into(), 6.0)].into(),
            },
            ModelReview {
                reviewer_id: "model-2".into(),
                review_text: "".into(),
                rankings: [("Model A".into(), 7.0), ("Model B".into(), 8.0)].into(),
            },
        ];
        let leaderboard = aggregate_rankings(&reviews);
        // Model A avg=8.0, Model B avg=7.0 → A first
        assert_eq!(leaderboard[0].0, "Model A");
        assert_eq!(leaderboard[1].0, "Model B");
    }
}
