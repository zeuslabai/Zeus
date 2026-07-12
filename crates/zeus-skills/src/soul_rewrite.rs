//! Proposal-only SOUL.md self-rewrite support.
//!
//! A seat may draft a concise persona body for its own SOUL.md, but the skill
//! never writes during proposal. Approval reloads the stored proposal, verifies
//! the operator/hash/current-file invariants, and then routes the only SOUL.md
//! write through `zeus_core::soul::write_onboarding_soul(..., true)`.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;
use zeus_core::soul::{
    render_soul_md, soul_content_is_stub, soul_is_stub_or_missing, write_onboarding_soul,
};

pub const DEFAULT_OPERATOR: &str = "merakizzz";
pub const DEFAULT_RATE_LIMIT_DAYS: i64 = 14;
pub const DEFAULT_PROPOSAL_TTL_DAYS: i64 = 7;
pub const DEFAULT_DIFF_CHANGED_LINES_CAP: usize = 80;
pub const DEFAULT_DIFF_BYTES_CAP: usize = 4 * 1024;
pub const DEFAULT_LEAN_SOUL_WARN_WORDS: usize = 200;
pub const DEFAULT_LEAN_SOUL_HARD_WORDS: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SoulRewriteLimits {
    pub operator: String,
    pub rate_limit_days: i64,
    pub proposal_ttl_days: i64,
    pub diff_changed_lines_cap: usize,
    pub diff_bytes_cap: usize,
    pub lean_soul_warn_words: usize,
    pub lean_soul_hard_words: usize,
}

impl Default for SoulRewriteLimits {
    fn default() -> Self {
        Self {
            operator: DEFAULT_OPERATOR.to_string(),
            rate_limit_days: DEFAULT_RATE_LIMIT_DAYS,
            proposal_ttl_days: DEFAULT_PROPOSAL_TTL_DAYS,
            diff_changed_lines_cap: DEFAULT_DIFF_CHANGED_LINES_CAP,
            diff_bytes_cap: DEFAULT_DIFF_BYTES_CAP,
            lean_soul_warn_words: DEFAULT_LEAN_SOUL_WARN_WORDS,
            lean_soul_hard_words: DEFAULT_LEAN_SOUL_HARD_WORDS,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SoulRewriteRequest {
    pub agent_name: String,
    pub soul_path: PathBuf,
    pub proposed_agent_soul: String,
    pub rationale: String,
    pub requested_by: String,
    pub last_proposal_at: Option<DateTime<Utc>>,
    /// Operator-only approval to bypass the rate limit. Coordinators may relay
    /// requests, but only the configured operator can approve this override.
    pub rate_limit_override_approved_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SoulRewriteValidation {
    pub current_is_stub_or_missing: bool,
    pub proposed_word_count: usize,
    pub lean_warning: bool,
    pub proposed_sludge_check_passed: bool,
    pub rendered_sludge_check_passed: bool,
    pub diff_changed_lines: usize,
    pub diff_bytes: usize,
    pub rate_limit_passed: bool,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SoulRewriteProposal {
    pub proposal_id: String,
    pub agent_name: String,
    pub soul_path: PathBuf,
    pub proposed_agent_soul: String,
    pub rationale: String,
    pub requested_by: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub current_sha256: String,
    pub candidate_sha256: String,
    pub rendered_candidate: String,
    pub unified_diff: String,
    pub validation: SoulRewriteValidation,
}

impl SoulRewriteProposal {
    pub fn approval_instruction(&self) -> String {
        format!(
            "approve soul {} sha256:{}",
            self.proposal_id, self.candidate_sha256
        )
    }

    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now > self.expires_at
    }
}

#[derive(Debug, Error)]
pub enum SoulRewriteError {
    #[error("proposed SOUL body is boilerplate/sludge")]
    ProposedSludge,
    #[error("rendered SOUL candidate is boilerplate/sludge")]
    RenderedSludge,
    #[error("proposed SOUL body has {word_count} words; hard cap is {hard_cap}")]
    LeanSoulHardCap { word_count: usize, hard_cap: usize },
    #[error("diff is too large: {changed_lines} changed lines / {diff_bytes} bytes")]
    DiffTooLarge {
        changed_lines: usize,
        diff_bytes: usize,
    },
    #[error(
        "rate limit active until {next_allowed}; override must be approved by operator {operator}"
    )]
    RateLimited {
        next_allowed: DateTime<Utc>,
        operator: String,
    },
    #[error("approval rejected: {approver} is not operator {operator}")]
    UnauthorizedApprover { approver: String, operator: String },
    #[error("proposal expired at {expires_at}")]
    ProposalExpired { expires_at: DateTime<Utc> },
    #[error("candidate hash mismatch: expected {expected}, got {actual}")]
    CandidateHashMismatch { expected: String, actual: String },
    #[error("current SOUL.md changed since proposal")]
    CurrentFileDrift,
    #[error("stored proposal does not match approval target path")]
    ProposalPathMismatch,
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub fn propose_soul_rewrite(
    request: SoulRewriteRequest,
    limits: SoulRewriteLimits,
    now: DateTime<Utc>,
) -> Result<SoulRewriteProposal, SoulRewriteError> {
    let proposed_body = request.proposed_agent_soul.trim();
    let proposed_word_count = count_words(proposed_body);
    if proposed_word_count > limits.lean_soul_hard_words {
        return Err(SoulRewriteError::LeanSoulHardCap {
            word_count: proposed_word_count,
            hard_cap: limits.lean_soul_hard_words,
        });
    }

    if soul_content_is_stub(proposed_body) {
        return Err(SoulRewriteError::ProposedSludge);
    }

    let rendered_candidate = render_soul_md(&request.agent_name, proposed_body);
    if soul_content_is_stub(&rendered_candidate) {
        return Err(SoulRewriteError::RenderedSludge);
    }

    let rate_limit_passed = rate_limit_passed(&request, &limits, now)?;
    let current = fs::read_to_string(&request.soul_path).unwrap_or_default();
    let current_is_stub_or_missing = soul_is_stub_or_missing(&request.soul_path);
    let diff = unified_diff(
        &current,
        &rendered_candidate,
        "current/SOUL.md",
        "candidate/SOUL.md",
    );
    let diff_bytes = diff.text.len();
    if diff.changed_lines > limits.diff_changed_lines_cap || diff_bytes > limits.diff_bytes_cap {
        return Err(SoulRewriteError::DiffTooLarge {
            changed_lines: diff.changed_lines,
            diff_bytes,
        });
    }

    let created_at = now;
    let expires_at = now + Duration::days(limits.proposal_ttl_days);
    let candidate_sha256 = sha256_hex(&rendered_candidate);
    let proposal_id = format!(
        "{}-{}",
        slugify_agent(&request.agent_name),
        now.format("%Y%m%d%H%M%S")
    );

    Ok(SoulRewriteProposal {
        proposal_id,
        agent_name: request.agent_name,
        soul_path: request.soul_path,
        proposed_agent_soul: proposed_body.to_string(),
        rationale: request.rationale,
        requested_by: request.requested_by,
        created_at,
        expires_at,
        current_sha256: sha256_hex(&current),
        candidate_sha256,
        rendered_candidate,
        unified_diff: diff.text,
        validation: SoulRewriteValidation {
            current_is_stub_or_missing,
            proposed_word_count,
            lean_warning: proposed_word_count > limits.lean_soul_warn_words,
            proposed_sludge_check_passed: true,
            rendered_sludge_check_passed: true,
            diff_changed_lines: diff.changed_lines,
            diff_bytes,
            rate_limit_passed,
            expires_at,
        },
    })
}

pub fn save_proposal_state(
    proposal: &SoulRewriteProposal,
    state_dir: &Path,
) -> Result<PathBuf, SoulRewriteError> {
    fs::create_dir_all(state_dir)?;
    let path = state_dir.join(format!("{}.json", proposal.proposal_id));
    let bytes = serde_json::to_vec_pretty(proposal)?;
    fs::write(&path, bytes)?;
    Ok(path)
}

pub fn load_proposal_state(path: &Path) -> Result<SoulRewriteProposal, SoulRewriteError> {
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn cleanup_expired_proposals(
    state_dir: &Path,
    now: DateTime<Utc>,
) -> Result<usize, SoulRewriteError> {
    if !state_dir.exists() {
        return Ok(0);
    }

    let mut removed = 0;
    for entry in fs::read_dir(state_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Ok(proposal) = load_proposal_state(&path) else {
            continue;
        };
        if proposal.is_expired(now) {
            fs::remove_file(path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

pub fn approve_soul_rewrite(
    proposal: &SoulRewriteProposal,
    soul_path: &Path,
    expected_candidate_sha256: &str,
    approver: &str,
    limits: &SoulRewriteLimits,
    now: DateTime<Utc>,
) -> Result<bool, SoulRewriteError> {
    if approver != limits.operator {
        return Err(SoulRewriteError::UnauthorizedApprover {
            approver: approver.to_string(),
            operator: limits.operator.clone(),
        });
    }

    if proposal.is_expired(now) {
        return Err(SoulRewriteError::ProposalExpired {
            expires_at: proposal.expires_at,
        });
    }

    if proposal.soul_path != soul_path {
        return Err(SoulRewriteError::ProposalPathMismatch);
    }

    if proposal.candidate_sha256 != expected_candidate_sha256 {
        return Err(SoulRewriteError::CandidateHashMismatch {
            expected: proposal.candidate_sha256.clone(),
            actual: expected_candidate_sha256.to_string(),
        });
    }

    let rendered = render_soul_md(&proposal.agent_name, &proposal.proposed_agent_soul);
    let rendered_sha = sha256_hex(&rendered);
    if rendered_sha != proposal.candidate_sha256 {
        return Err(SoulRewriteError::CandidateHashMismatch {
            expected: proposal.candidate_sha256.clone(),
            actual: rendered_sha,
        });
    }

    let current = fs::read_to_string(soul_path).unwrap_or_default();
    if sha256_hex(&current) != proposal.current_sha256 {
        return Err(SoulRewriteError::CurrentFileDrift);
    }

    Ok(write_onboarding_soul(
        soul_path,
        &proposal.agent_name,
        &proposal.proposed_agent_soul,
        true,
    )?)
}

fn rate_limit_passed(
    request: &SoulRewriteRequest,
    limits: &SoulRewriteLimits,
    now: DateTime<Utc>,
) -> Result<bool, SoulRewriteError> {
    let Some(last) = request.last_proposal_at else {
        return Ok(true);
    };
    let next_allowed = last + Duration::days(limits.rate_limit_days);
    if now >= next_allowed {
        return Ok(true);
    }
    if request.rate_limit_override_approved_by.as_deref() == Some(limits.operator.as_str()) {
        return Ok(true);
    }
    Err(SoulRewriteError::RateLimited {
        next_allowed,
        operator: limits.operator.clone(),
    })
}

fn count_words(s: &str) -> usize {
    s.split_whitespace().count()
}

fn sha256_hex(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn slugify_agent(agent_name: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for c in agent_name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
            last_dash = false;
        } else if !last_dash && !slug.is_empty() {
            slug.push('-');
            last_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "agent".to_string()
    } else {
        slug
    }
}

struct DiffResult {
    text: String,
    changed_lines: usize,
}

fn unified_diff(old: &str, new: &str, old_label: &str, new_label: &str) -> DiffResult {
    if old == new {
        return DiffResult {
            text: format!("--- {old_label}\n+++ {new_label}\n"),
            changed_lines: 0,
        };
    }

    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let mut prefix = 0;
    while prefix < old_lines.len()
        && prefix < new_lines.len()
        && old_lines[prefix] == new_lines[prefix]
    {
        prefix += 1;
    }

    let mut suffix = 0;
    while suffix + prefix < old_lines.len()
        && suffix + prefix < new_lines.len()
        && old_lines[old_lines.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let old_mid_end = old_lines.len() - suffix;
    let new_mid_end = new_lines.len() - suffix;
    let old_mid = &old_lines[prefix..old_mid_end];
    let new_mid = &new_lines[prefix..new_mid_end];
    let changed_lines = old_mid.len() + new_mid.len();

    let mut text = String::new();
    text.push_str(&format!("--- {old_label}\n+++ {new_label}\n"));
    text.push_str(&format!(
        "@@ -{},{} +{},{} @@\n",
        prefix + 1,
        old_mid.len(),
        prefix + 1,
        new_mid.len()
    ));
    if prefix > 0 {
        text.push_str(&format!(" {}\n", old_lines[prefix - 1]));
    }
    for line in old_mid {
        text.push_str(&format!("-{line}\n"));
    }
    for line in new_mid {
        text.push_str(&format!("+{line}\n"));
    }
    if suffix > 0 {
        text.push_str(&format!(" {}\n", old_lines[old_mid_end]));
    }

    DiffResult {
        text,
        changed_lines,
    }
}
