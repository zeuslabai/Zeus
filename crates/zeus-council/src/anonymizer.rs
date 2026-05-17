/// Assigns anonymous labels (Model A, B, C…) to model responses
/// so reviewers can't be biased by knowing which model wrote what.

use crate::ModelResponse;

const LABELS: &[&str] = &[
    "Model A", "Model B", "Model C", "Model D", "Model E",
    "Model F", "Model G", "Model H",
];

/// Assign anonymous labels to a slice of responses in-place.
/// Labels are assigned in the order models appear (deterministic).
pub fn assign_labels(responses: &mut Vec<ModelResponse>) {
    for (i, resp) in responses.iter_mut().enumerate() {
        resp.label = LABELS
            .get(i)
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("Model {}", (b'A' + i as u8) as char));
    }
}

/// Build the anonymized context block shown to reviewers in stage 2.
/// Each entry is: "### Model A\n<response text>\n"
pub fn build_anonymized_context(responses: &[ModelResponse]) -> String {
    responses
        .iter()
        .map(|r| format!("### {}\n{}\n", r.label, r.response))
        .collect::<Vec<_>>()
        .join("\n")
}
