/// 3-stage LLM Council pipeline.
///
/// Stage 1 — Opinions:   All models answer the query in parallel
/// Stage 2 — Review:     Each model reviews anonymized peer responses and ranks them
/// Stage 3 — Synthesis:  Chairman produces the final answer using all inputs

use crate::{
    CouncilConfig, CouncilResult, CouncilSession, ModelResponse, ModelReview,
    anonymizer::{assign_labels, build_anonymized_context},
    ranking::parse_rankings,
};
use anyhow::{Context, Result};
use futures::future::join_all;
use std::time::Instant;
use tracing::{info, warn};
use zeus_core::{Message, Provider, Role};
use zeus_llm::LlmClient;

// ── helpers ──────────────────────────────────────────────────────────────────

/// Parse "provider/model" string into (Provider, model_name).
/// Falls back to Anthropic if the prefix is unrecognised.
fn parse_model_id(model_id: &str) -> (Provider, String) {
    if let Some(rest) = model_id.strip_prefix("anthropic/") {
        (Provider::Anthropic, rest.to_string())
    } else if let Some(rest) = model_id.strip_prefix("openai/") {
        (Provider::OpenAI, rest.to_string())
    } else if let Some(rest) = model_id.strip_prefix("google/") {
        (Provider::Google, rest.to_string())
    } else if let Some(rest) = model_id.strip_prefix("ollama/") {
        (Provider::Ollama, rest.to_string())
    } else if let Some(rest) = model_id.strip_prefix("openrouter/") {
        (Provider::OpenRouter, rest.to_string())
    } else {
        // No prefix — treat the whole string as model name, default to Anthropic
        (Provider::Anthropic, model_id.to_string())
    }
}

fn make_client(model_id: &str) -> Result<LlmClient> {
    let (provider, model) = parse_model_id(model_id);
    LlmClient::new(provider, model).context("Failed to create LlmClient")
}

fn user_message(content: impl Into<String>) -> Message {
    Message {
        role: Role::User,
        content: content.into(),
        tool_calls: vec![],
        tool_results: vec![],
        timestamp: chrono::Utc::now(),
        attachments: vec![],
        message_id: None,
        parent_id: None,
        thread_id: None,
        direction: Default::default(),
        channel_source: None, compaction_hint: Default::default(),
    }
}

// ── Stage 1 ───────────────────────────────────────────────────────────────────

/// Call all council models in parallel and collect their raw responses.
pub async fn stage1_opinions(
    query: &str,
    config: &CouncilConfig,
) -> Result<Vec<ModelResponse>> {
    info!("Council stage 1: gathering opinions from {} models", config.models.len());

    let tasks: Vec<_> = config
        .models
        .iter()
        .map(|model_id| {
            let model_id = model_id.clone();
            let query = query.to_string();
            tokio::spawn(async move {
                let t0 = Instant::now();
                let client = make_client(&model_id)?;
                let msgs = vec![user_message(&query)];
                let resp = client.complete(&msgs, &[], None).await?;
                let latency_ms = t0.elapsed().as_millis() as u64;
                Ok::<ModelResponse, anyhow::Error>(ModelResponse {
                    model_id: model_id.clone(),
                    label: String::new(), // assigned later
                    response: resp.content,
                    tokens: resp.input_tokens + resp.output_tokens,
                    latency_ms,
                })
            })
        })
        .collect();

    let mut responses = Vec::new();
    for (i, result) in join_all(tasks).await.into_iter().enumerate() {
        match result {
            Ok(Ok(r)) => responses.push(r),
            Ok(Err(e)) => warn!("Model {} failed in stage 1: {}", config.models[i], e),
            Err(e) => warn!("Task panic for model {}: {}", config.models[i], e),
        }
    }

    if responses.is_empty() {
        anyhow::bail!("All models failed in stage 1");
    }

    assign_labels(&mut responses);
    Ok(responses)
}

// ── Stage 2 ───────────────────────────────────────────────────────────────────

const REVIEW_SYSTEM: &str = "\
You are a careful evaluator reviewing multiple model responses to the same question. \
Your job is to rank each response on accuracy, clarity, and completeness (1–10). \
At the END of your review, output exactly one line in this format (no extra text on that line): \
RANK: Model A=<score>, Model B=<score>, ...";

/// Each model reviews the anonymized peer responses and ranks them.
pub async fn stage2_review(
    query: &str,
    responses: &[ModelResponse],
    _config: &CouncilConfig,
) -> Result<Vec<ModelReview>> {
    info!("Council stage 2: peer review by {} models", responses.len());

    let context = build_anonymized_context(responses);
    let review_prompt = format!(
        "Original question:\n{query}\n\n\
         Peer responses:\n{context}\n\n\
         Review each response and rank them. Remember to end with a RANK: line."
    );

    let tasks: Vec<_> = responses
        .iter()
        .map(|resp| {
            let reviewer_id = resp.model_id.clone();
            let prompt = review_prompt.clone();
            tokio::spawn(async move {
                let client = make_client(&reviewer_id)?;
                let msgs = vec![user_message(&prompt)];
                let r = client.complete(&msgs, &[], Some(REVIEW_SYSTEM)).await?;
                let rankings = parse_rankings(&r.content);
                Ok::<ModelReview, anyhow::Error>(ModelReview {
                    reviewer_id,
                    review_text: r.content,
                    rankings,
                })
            })
        })
        .collect();

    let mut reviews = Vec::new();
    for result in join_all(tasks).await {
        match result {
            Ok(Ok(r)) => reviews.push(r),
            Ok(Err(e)) => warn!("Model failed in stage 2: {}", e),
            Err(e) => warn!("Task panic in stage 2: {}", e),
        }
    }

    Ok(reviews)
}

// ── Stage 3 ───────────────────────────────────────────────────────────────────

const SYNTHESIS_SYSTEM: &str = "\
You are the chairman of a model council. You have received multiple independent answers \
to a question, along with peer rankings. Your job is to synthesize the best possible \
final answer — drawing on the strongest elements from each response, correcting any errors, \
and presenting a clear, authoritative conclusion.";

/// Chairman synthesizes the final answer from all opinions and reviews.
pub async fn stage3_synthesize(
    query: &str,
    responses: &[ModelResponse],
    reviews: &[ModelReview],
    config: &CouncilConfig,
) -> Result<String> {
    info!("Council stage 3: chairman synthesis via {}", config.chairman);

    let opinions_block = build_anonymized_context(responses);

    let reviews_block = reviews
        .iter()
        .map(|r| format!("Reviewer ({}):\n{}\n", r.reviewer_id, r.review_text))
        .collect::<Vec<_>>()
        .join("\n---\n");

    let synthesis_prompt = format!(
        "Original question:\n{query}\n\n\
         ## Model Opinions\n{opinions_block}\n\n\
         ## Peer Reviews\n{reviews_block}\n\n\
         Please synthesize the definitive final answer."
    );

    let chairman = make_client(&config.chairman)?;
    let msgs = vec![user_message(&synthesis_prompt)];
    let resp = chairman
        .complete(&msgs, &[], Some(SYNTHESIS_SYSTEM))
        .await
        .context("Chairman synthesis failed")?;

    Ok(resp.content)
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Run the full 3-stage council pipeline and return the result.
pub async fn run_council(query: &str, config: CouncilConfig) -> Result<CouncilResult> {
    let mut session = CouncilSession::new(config.clone());

    // Stage 1
    let responses = stage1_opinions(query, &config).await?;
    session.results = responses.clone();

    // Stage 2
    let reviews = stage2_review(query, &responses, &config).await?;
    session.reviews = reviews.clone();

    // Stage 3
    let final_answer = stage3_synthesize(query, &responses, &reviews, &config).await?;
    session.final_answer = final_answer.clone();
    session.finished_at = Some(chrono::Utc::now());

    Ok(CouncilResult { final_answer, session })
}
