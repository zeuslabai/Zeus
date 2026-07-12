use chrono::{Duration, TimeZone, Utc};
use tempfile::tempdir;
use zeus_core::soul::render_soul_md;
use zeus_skills::openclaw::{parse_frontmatter, parse_openclaw_skill};
use zeus_skills::soul_rewrite::{
    approve_soul_rewrite, cleanup_expired_proposals, load_proposal_state, propose_soul_rewrite,
    save_proposal_state, SoulRewriteError, SoulRewriteLimits, SoulRewriteRequest,
};

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0).unwrap()
}

fn request_for(soul_path: std::path::PathBuf, proposed: &str) -> SoulRewriteRequest {
    SoulRewriteRequest {
        agent_name: "zeus-titan".to_string(),
        soul_path,
        proposed_agent_soul: proposed.to_string(),
        rationale: "tighten voice/persona".to_string(),
        requested_by: "zeus-titan".to_string(),
        last_proposal_at: None,
        rate_limit_override_approved_by: None,
    }
}

#[test]
fn proposal_renders_diff_and_never_writes_soul() {
    let dir = tempdir().unwrap();
    let soul_path = dir.path().join("SOUL.md");
    let original = "# SOUL.md — zeus-titan\n\nCustom human-written soul.\n";
    std::fs::write(&soul_path, original).unwrap();

    let proposal = propose_soul_rewrite(
        request_for(
            soul_path.clone(),
            "Direct practical specialist. Low noise, high follow-through.",
        ),
        SoulRewriteLimits::default(),
        now(),
    )
    .unwrap();

    assert_eq!(std::fs::read_to_string(&soul_path).unwrap(), original);
    assert_eq!(
        proposal.rendered_candidate,
        render_soul_md(
            "zeus-titan",
            "Direct practical specialist. Low noise, high follow-through."
        )
    );
    assert!(proposal.unified_diff.contains("--- current/SOUL.md"));
    assert!(proposal.unified_diff.contains("+++ candidate/SOUL.md"));
    assert!(proposal
        .approval_instruction()
        .starts_with("approve soul zeus-titan-20260712120000 sha256:"));
    assert!(!proposal.validation.current_is_stub_or_missing);
    assert!(proposal.validation.proposed_sludge_check_passed);
    assert!(proposal.validation.rendered_sludge_check_passed);
}

#[test]
fn approval_is_operator_only_and_writes_via_canonical_onboarding_path() {
    let dir = tempdir().unwrap();
    let soul_path = dir.path().join("SOUL.md");
    std::fs::write(&soul_path, "# SOUL.md — zeus-titan\n\nCustom soul.\n").unwrap();

    let proposal = propose_soul_rewrite(
        request_for(
            soul_path.clone(),
            "Concise shipper. Verifies before claiming done.",
        ),
        SoulRewriteLimits::default(),
        now(),
    )
    .unwrap();

    let coordinator_result = approve_soul_rewrite(
        &proposal,
        &soul_path,
        &proposal.candidate_sha256,
        "Zeus100",
        &SoulRewriteLimits::default(),
        now(),
    );
    assert!(matches!(
        coordinator_result,
        Err(SoulRewriteError::UnauthorizedApprover { .. })
    ));
    assert!(std::fs::read_to_string(&soul_path)
        .unwrap()
        .contains("Custom soul"));

    let wrote = approve_soul_rewrite(
        &proposal,
        &soul_path,
        &proposal.candidate_sha256,
        "merakizzz",
        &SoulRewriteLimits::default(),
        now(),
    )
    .unwrap();
    assert!(wrote);

    let updated = std::fs::read_to_string(&soul_path).unwrap();
    assert!(updated.contains("# SOUL.md — zeus-titan"));
    assert!(updated.contains("Concise shipper. Verifies before claiming done."));
}

#[test]
fn approval_blocks_hash_mismatch_file_drift_and_expired_state() {
    let dir = tempdir().unwrap();
    let soul_path = dir.path().join("SOUL.md");
    std::fs::write(&soul_path, "# SOUL.md — zeus-titan\n\nCustom soul.\n").unwrap();

    let proposal = propose_soul_rewrite(
        request_for(soul_path.clone(), "Fast practical specialist."),
        SoulRewriteLimits::default(),
        now(),
    )
    .unwrap();

    assert!(matches!(
        approve_soul_rewrite(
            &proposal,
            &soul_path,
            "not-the-hash",
            "merakizzz",
            &SoulRewriteLimits::default(),
            now(),
        ),
        Err(SoulRewriteError::CandidateHashMismatch { .. })
    ));

    std::fs::write(
        &soul_path,
        "# SOUL.md — zeus-titan\n\nChanged while pending.\n",
    )
    .unwrap();
    assert!(matches!(
        approve_soul_rewrite(
            &proposal,
            &soul_path,
            &proposal.candidate_sha256,
            "merakizzz",
            &SoulRewriteLimits::default(),
            now(),
        ),
        Err(SoulRewriteError::CurrentFileDrift)
    ));

    std::fs::write(&soul_path, "# SOUL.md — zeus-titan\n\nCustom soul.\n").unwrap();
    assert!(matches!(
        approve_soul_rewrite(
            &proposal,
            &soul_path,
            &proposal.candidate_sha256,
            "merakizzz",
            &SoulRewriteLimits::default(),
            now() + Duration::days(8),
        ),
        Err(SoulRewriteError::ProposalExpired { .. })
    ));
}

#[test]
fn ttl_state_expires_and_cleanup_removes_stale_proposals() {
    let dir = tempdir().unwrap();
    let soul_path = dir.path().join("SOUL.md");
    let state_dir = dir.path().join("proposal-state");

    let proposal = propose_soul_rewrite(
        request_for(
            soul_path,
            "Direct specialist with a bias for verified delivery.",
        ),
        SoulRewriteLimits::default(),
        now(),
    )
    .unwrap();
    let state_path = save_proposal_state(&proposal, &state_dir).unwrap();
    assert!(state_path.exists());
    assert_eq!(load_proposal_state(&state_path).unwrap(), proposal);

    let removed = cleanup_expired_proposals(&state_dir, now() + Duration::days(8)).unwrap();
    assert_eq!(removed, 1);
    assert!(!state_path.exists());
}

#[test]
fn sludge_rate_limit_diff_and_lean_caps_are_enforced() {
    let dir = tempdir().unwrap();
    let soul_path = dir.path().join("SOUL.md");

    assert!(matches!(
        propose_soul_rewrite(
            request_for(soul_path.clone(), "an autonomous Zeus agent"),
            SoulRewriteLimits::default(),
            now(),
        ),
        Err(SoulRewriteError::ProposedSludge)
    ));

    let mut rate_limited = request_for(soul_path.clone(), "Precise operator. Keeps status short.");
    rate_limited.last_proposal_at = Some(now() - Duration::days(1));
    assert!(matches!(
        propose_soul_rewrite(rate_limited.clone(), SoulRewriteLimits::default(), now()),
        Err(SoulRewriteError::RateLimited { .. })
    ));

    rate_limited.rate_limit_override_approved_by = Some("Zeus100".to_string());
    assert!(matches!(
        propose_soul_rewrite(rate_limited.clone(), SoulRewriteLimits::default(), now()),
        Err(SoulRewriteError::RateLimited { .. })
    ));

    rate_limited.rate_limit_override_approved_by = Some("merakizzz".to_string());
    assert!(propose_soul_rewrite(rate_limited, SoulRewriteLimits::default(), now()).is_ok());

    let mut strict = SoulRewriteLimits::default();
    strict.diff_changed_lines_cap = 1;
    assert!(matches!(
        propose_soul_rewrite(
            request_for(soul_path.clone(), "Compact specialist."),
            strict,
            now(),
        ),
        Err(SoulRewriteError::DiffTooLarge { .. })
    ));

    let warning_body = vec!["voice"; 201].join(" ");
    let warning = propose_soul_rewrite(
        request_for(soul_path.clone(), &warning_body),
        SoulRewriteLimits::default(),
        now(),
    )
    .unwrap();
    assert!(warning.validation.lean_warning);

    let too_long = vec!["voice"; 501].join(" ");
    assert!(matches!(
        propose_soul_rewrite(
            request_for(soul_path, &too_long),
            SoulRewriteLimits::default(),
            now(),
        ),
        Err(SoulRewriteError::LeanSoulHardCap { .. })
    ));
}

#[test]
fn skill_manifest_documents_proposal_only_merakizzz_approval_and_state_only_write() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let skill_path = root.join("skills/soul-self-rewrite/SKILL.md");
    let content = std::fs::read_to_string(&skill_path).unwrap();
    let (frontmatter, _) = parse_frontmatter(&content);
    let skill = parse_openclaw_skill(&content, skill_path).unwrap();

    assert_eq!(skill.name, "soul-self-rewrite");
    assert_eq!(
        frontmatter.get("approval_required").map(String::as_str),
        Some("true")
    );
    assert!(content.contains("Approval authority is merakizzz only"));
    assert!(content.contains("proposal_state_only"));
    assert!(content.contains("network: false"));
    assert!(content.contains("write_onboarding_soul(path, agent_name, proposed_agent_soul, true)"));
    assert!(content.contains("Do not write `SOUL.md` during proposal"));
}

#[test]
fn static_regression_approval_path_has_only_canonical_soul_write() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source =
        std::fs::read_to_string(root.join("crates/zeus-skills/src/soul_rewrite.rs")).unwrap();

    assert!(source.contains("write_onboarding_soul("));
    assert!(source.contains("soul_content_is_stub(proposed_body)"));
    assert!(source.contains("soul_content_is_stub(&rendered_candidate)"));
    assert!(source.contains("proposal.is_expired(now)"));
    assert!(!source.contains("fs::write(soul_path"));
    assert!(!source.contains("std::fs::write(soul_path"));
    assert!(!source.contains("File::create(soul_path"));
}
