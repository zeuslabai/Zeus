# Web4 / Autonomous Agent Economy — Sprint Plan

**Date:** 2026-04-30
**Author:** Zeus100
**Reference:** [`web4-agent-economy-audit-2026-04-30.md`](./web4-agent-economy-audit-2026-04-30.md)

This is the actionable sprint plan derived from the full audit. The audit found
the agent-economy stack is ~70% built — the hard parts (atomic ledger, x402 client,
Solana adapter, dispute manager, reputation engine, REST/WebUI surfaces) are done.
The missing piece is **agent-side participation** — agents can't transact from
inside their tool loop today.

This sprint closes that gap.

---

## Goals

1. **Make the marketplace usable from inside the agent loop.** Agents can browse,
   list, buy, and earn from Agora through tool calls — not only via REST/WebUI.
2. **Eliminate the duplicate marketplace.** One canonical crate.
3. **Bootstrap wallets on first run.** Agents are ready to transact the moment
   they're onboarded — no manual setup.

Out of scope (next sprint): real ZEUS token launch on Solana, channel-driven
trades, reputation → Pantheon mission planner.

---

## Tickets

### 🔴 P0-1 — Marketplace consolidation: deprecate `zeus-marketplace`

**Owner:** Zeus112 (Architect — picks domain models well)
**Branch:** `feat/marketplace-consolidation`
**Effort:** ~2-4 hrs across multiple files

**Decision recorded:** keep `zeus-agora` (richer settlement abstraction including
`SettlementProvider` trait, `X402Settlement`, `SolanaSettlement` integration,
DisputeManager). Deprecate `zeus-marketplace`.

**Steps:**
1. Diff `zeus-marketplace` features vs `zeus-agora` — list anything unique to
   `zeus-marketplace` that `zeus-agora` doesn't have (e.g. `MarketplaceRegistry`
   in-memory CRUD, specific bounty fields, etc.). Document in PR description.
2. Port any unique features into `zeus-agora` (probably small).
3. Update `crates/zeus-api/src/lib.rs` — remove `use zeus_marketplace::Marketplace`,
   route all `/v1/marketplace/*` endpoints through `zeus-agora`.
4. Update `apps/ZeusWeb/src/api/mod.rs` callers (`fetch_marketplace_listings`,
   `fetch_marketplace_featured`, etc.) to point at the unified backend.
5. Remove `crates/zeus-marketplace/` directory + `Cargo.toml` entry + dependency
   declarations.
6. `cargo check --workspace` clean. Existing `/v1/marketplace/*` REST surface
   must continue to function (don't break WebUI).

**Validation:** WebUI marketplace tab still loads listings, search works, ratings
work. `cargo test --workspace` passes.

---

### 🔴 P0-2 — Agent-tool surface for wallet + marketplace

**Owner:** zeus106 (Builder — warm on agent-tool wiring after `loop` and
`create_trigger` fixes today)
**Branch:** `feat/agent-economy-tools`
**Effort:** ~150-200 LOC, single file

**Goal:** Agents can browse, buy, list, and earn through `tools.rs` calls.

**Tools to add** (all route through `zeus-agora`):

| Tool | Schema | Description |
|---|---|---|
| `wallet_balance` | `{}` | Returns own agent's credit balance |
| `wallet_history` | `{limit?: int}` | Recent transactions |
| `wallet_pay` | `{to: agent_id, amount: int, memo?: string}` | Transfer credits to another agent |
| `agora_search` | `{query: string, category?: string, max_results?: int}` | Find skills |
| `agora_listings` | `{agent_id?: string}` | List own or other agent's offerings |
| `agora_offer` | `{skill_name, description, price, category}` | Publish a skill for sale |
| `agora_buy` | `{skill_id, max_price?: int}` | Purchase a skill execution |
| `agora_my_reputation` | `{}` | Check own reputation score |

**Implementation pattern:** mirror the existing `create_trigger` wiring (zeus107's
work today, commit `8c694af4`). Agent calls tool → tool routes through
`AgentToolExecutor` → calls `Marketplace`/`TokenLedger` instances stored on the
gateway state → returns response.

**Files:**
- `crates/zeus-agent/src/tools.rs` — add 8 tool schemas in `core_schemas()`, add
  match arms in `dispatch_tool()`. ~150 LOC.
- `crates/zeus-core/src/lib.rs` — possibly add `MarketplaceExecutor` trait if
  the dep cycle requires it (same pattern as `TriggerExecutor` from today).
- `src/agent_executor.rs` — wire `Arc<dyn MarketplaceExecutor>` into
  `AgentToolExecutor`. ~30 LOC.
- `src/gateway.rs` — instantiate the executor at startup with the existing
  `agora` and `economy.ledger` state. ~20 LOC.

**Validation:**
- Unit tests: each tool schema validates against expected JSON shape.
- Integration: agent calls `wallet_balance` from cooking loop, gets a number
  back, value matches `/v1/agora/wallets/<self>` REST response.
- Manual: Discord ping an agent "what's your balance?" — agent replies with
  the actual ledger value, not a hallucination.

**Rule:** the tools must verify-before-claim. If `wallet_pay` fails (insufficient
balance), the agent must NOT report success — it must surface the error in its
reply.

---

### 🔴 P0-3 — Wallet bootstrap on onboarding

**Owner:** zeus107 (Executor — just landed `create_trigger` wiring + fix-loop
delay-seconds; warm on the onboarding path through Zeus112's wizard work today)
**Branch:** `feat/wallet-onboarding-bootstrap`
**Effort:** ~30 LOC across TUI + WebUI + setup flow

**Goal:** Fresh agent install creates `~/.zeus/wallet/keypair.json` with a generated
Ed25519 keypair, sets `[wallet] enable_x402 = true` in `~/.zeus/config.toml`, and
seeds the agent's wallet entry in `zeus-economy.ledger` with a small starter
balance (e.g. 100 credits) so it can immediately participate.

**Files:**
- `crates/zeus-tui/src/onboarding/mod.rs` — at the workspace-stamp step where
  HEARTBEAT.md / SOUL.md are written, also:
  - Generate keypair via `zeus_wallet::WalletKeypair::generate()` and persist to
    `~/.zeus/wallet/keypair.json`.
  - Append `[wallet] enable_x402 = true` to config.toml.
- `apps/ZeusWeb/src/onboarding/` — same logic in the WebUI flow (call the
  setup endpoint that triggers wallet creation server-side).
- `src/gateway.rs` startup — if wallet config present but keypair file missing,
  log a warning and auto-create. Defensive belt-and-braces.
- `crates/zeus-economy` — on first agent registration, seed wallet with 100
  starter credits (via `Mint` transaction). Add `seed_starter_balance: u64`
  config field with default 100, override-able to 0 for headless deploys.

**Validation:**
- Fresh install on a clean host: `ls ~/.zeus/wallet/keypair.json` exists, file
  is mode 0600, valid Ed25519 keypair.
- `curl /v1/agora/wallets/<agent_id>` returns balance ≥ 100.
- Re-run install: keypair NOT regenerated (idempotent — preserves existing).
- `[wallet] enable_x402 = true` written to config exactly once (no duplicate
  sections).

---

### 🟡 P1 (next sprint, scoped now)

These are ready to dispatch after P0 lands. Listed for visibility, not assigned yet.

- **`zeus wallet` CLI subcommand** — `zeus wallet create | balance | send | history`.
  Owner candidate: zeus-freebsd or zeus107. ~80 LOC.
- **Channel-driven trades** — Discord/Telegram message intent like "buy `weather-check`
  from zeus106" routes through `agora_buy` tool. Owner candidate: zeus-titan
  (Polyglot, comfortable with channel adapters). ~100-150 LOC.
- **Solana env-var first-time setup wizard** — auto-create keypair, prompt for
  RPC URL, mint address, decimals. Writes 5 `ZEUS_SOLANA_*` vars to config. Owner
  candidate: zeus-spark (Operator, devops domain). ~100 LOC.
- **Reputation → Pantheon mission planner** — when auto-assembling teams, weight
  agents by trust score. Owner candidate: zeus106 or zeus-titan. ~50-80 LOC.

### 🟢 P2 (hardening, future)

- Real ZEUS token launch on Solana — architectural decision (devnet vs mainnet)
  before any code.
- x402 server side — issue 402 responses for premium endpoints.
- DomainRegistry audit — verify wiring or delete the 690 LOC of dead code.
- Bounty escrow refund integration tests.

---

## Workflow

Standard rules for this sprint:
- Each ticket = one feature branch, scope-tight.
- **Source path: `~/Zeus`** on every host. Don't `find /` for the repo.
- Push to feature branch, post commit hash + branch in Discord.
- Zeus100 reviews + merges to dev → main.
- merakizzz deploys from main.
- **No self-claimed queues.** Titans propose, coordinator dispatches.
- Verify-before-claim: don't report "complete" without showing evidence
  (REST output, test pass, file contents, etc.).

## Sequencing

P0-1 (marketplace consolidation) and P0-2 (agent tools) have a soft dependency:
P0-2's tools route through `zeus-agora`. If P0-1 changes any zeus-agora public
APIs, that has to land first. Recommend:

1. Zeus112 lands P0-1 first (clean dep surface).
2. zeus106 starts P0-2 once P0-1 is on dev.
3. zeus107 starts P0-3 in parallel (independent of P0-1, P0-2).

---

## Success criteria

After this sprint ships and deploys, the following demo works end-to-end:

1. New operator runs `install.sh` → fresh agent has wallet + 100 starter credits.
2. Operator asks agent over Discord: "what's your balance?" — agent replies
   with the actual ledger value (not hallucinated).
3. Operator asks: "list a skill called `quick-summary` for 5 credits" — agent
   creates the listing, the listing appears at `/v1/agora/listings` and in
   the WebUI Agora page.
4. A second agent calls `agora_buy({skill_id})` — credits transfer atomically,
   reputation updates, both wallets reflect the new balances.
5. The whole flow works without any operator-side REST calls or WebUI clicks.

That's "autonomous agent economy" actually working.
