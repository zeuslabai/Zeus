---
name: soul-self-rewrite
description: "Propose and explicitly approve a concise SOUL.md rewrite for the current agent."
version: 0.1.0
category: identity
approval_required: true
user-invocable: true
skillKey: soul_self_rewrite
read_when:
  - "rewrite my soul"
  - "soul self rewrite"
  - "propose a SOUL.md revision"
permissions:
  filesystem:
    read:
      - workspace/SOUL.md
    write:
      - proposal_state_only
  network: false
---

# SOUL Self-Rewrite

Proposal-only workflow for a seat that wants to revise its own `SOUL.md` voice/persona.

## Contract

1. Draft `proposed_agent_soul` as persona text only: short voice, taste, operating style. No mechanics, tool rules, escalation policy, or coordination doctrine.
2. Render the exact candidate with `zeus_core::soul::render_soul_md(agent_name, proposed_agent_soul)`.
3. Validate before posting:
   - current file state via `soul_is_stub_or_missing(path)`
   - proposed body sludge check via `soul_content_is_stub(proposed_agent_soul)`
   - rendered candidate sludge check via `soul_content_is_stub(rendered_candidate)`
   - one proposal per agent per 14 days unless merakizzz approves the override
   - diff cap: 80 changed lines or 4 KiB
   - lean SOUL: warn above ~200 words, reject above 500 words
   - proposal TTL: 7 days
4. Post in-channel: rationale, validation summary, unified diff, proposal id, candidate sha256, and the exact approval phrase.
5. Do not write `SOUL.md` during proposal — missing/stub/sludge files still require explicit approval.
6. Approval authority is merakizzz only. Coordinators may relay; they cannot approve a SOUL rewrite or rate-limit override.
7. On approval, reload stored proposal state, verify proposal id + sha256 + unchanged current-file hash + non-expired TTL, then perform the only allowed write:

```rust
write_onboarding_soul(path, agent_name, proposed_agent_soul, true)
```

## Approval post shape

```text
SOUL rewrite proposal: titan-20260712-001

Rationale:
- tighten voice/persona
- keep SOUL concise

Validation:
- current SOUL: custom | stub/missing
- proposed sludge check: pass
- rendered sludge check: pass
- words: 137 (warn above 200, reject above 500)
- diff size: pass
- rate limit: pass
- expires: 2026-07-19T00:00:00Z

Approve with:
approve soul titan-20260712-001 sha256:<hash>
```

## Guardrails

- SOUL is persona-only. Put workflows, mechanics, tool rules, escalation paths, channel policy, and coordination doctrine in `AGENTS.md`, not `SOUL.md`.
- No silent healing: even if the current SOUL is missing, blank, install-stub, or old sludge, this skill proposes first.
- Drift protection blocks stale approvals if `SOUL.md` changed after proposal generation.
- Expired proposals are ignored/cleaned and must be regenerated.
