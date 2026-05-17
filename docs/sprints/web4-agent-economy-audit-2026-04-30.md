# Web4 / Autonomous Agent Economy — Full Audit

**Date:** 2026-04-30
**Author:** Zeus100
**Scope:** Agora marketplace + Solana wallet integration in Zeus codebase

---

## TL;DR

- **5 dedicated crates** ship the agent-economy stack: `zeus-agora`, `zeus-marketplace`, `zeus-economy`, `zeus-wallet`, `zeus-solana` (~7,650 LOC total).
- **31 REST routes** wired through `zeus-api` (`/v1/agora/*` + `/v1/marketplace/*` + `/v1/pantheon/economy`).
- **WebUI surface exists** — `apps/ZeusWeb/src/pages/agora.rs` (1,422 LOC) renders the full marketplace experience.
- **Solana on-chain settlement is opt-in at runtime** — wired but defaults to in-memory unless 5 `ZEUS_SOLANA_*` env vars are set.
- **Critical gaps:** no agent-tool surface (agents can't spend/earn from tool calls), no wallet bootstrap in onboarding, no real token launched (placeholder mints in config), and **two parallel marketplace crates** (`zeus-marketplace` AND `zeus-agora`) that overlap.

---

## Crate Inventory

### 1. `zeus-agora` (2,807 LOC) — The skill marketplace

**Description:** "The Agora — agent skill marketplace for Zeus"

| File | LOC | Purpose |
|---|---|---|
| `lib.rs` | 411 | `TransactionStatus`, `SkillListing`, public re-exports |
| `protocol.rs` | 104 | `AgentIdentity`, `AgentCapability` types for HTTP-based agent discovery |
| `marketplace.rs` | 1,990 | `Marketplace`, `MarketplaceConfig`, `Category`, `SearchQuery`, `Dispute`/`DisputeManager`, `ReputationScore`/`ReputationTracker`, `TransactionLog`, `SettlementProvider` trait, `InMemorySettlement`, `X402Settlement` |
| `coordinator.rs` | 302 | `SettlementCoordinator` — orchestrates settlement across `TokenLedger` + `X402Client` |

**Dependencies:** `zeus-economy` (TokenLedger), `zeus-wallet` (X402Client)
**Used by:** `zeus-api` (re-exported as `AgoraMarketplace`), `zeus-solana` (provides `SettlementProvider`)

**Status:** ✅ Wired into the API gateway, real REST surface, WebUI client.

### 2. `zeus-marketplace` (2,291 LOC) — Separate skill marketplace

**Description:** "Agent-to-agent skill marketplace — publish, trade, and rate tools"

Single-file crate with 6 components: `SkillListing`, `MarketplaceRegistry`, `TradeProtocol`, `TokenLedger`, `ReputationEngine`, `MarketplaceAPI`.

**Used by:** `zeus-api` (`use zeus_marketplace::Marketplace`)

**⚠️ Overlap concern:** `zeus-marketplace` and `zeus-agora` cover the same domain (skill listing, trading, ratings, reputation). Two parallel implementations with different APIs.

### 3. `zeus-economy` (2,579 LOC) — SQLite-backed token ledger

**Description:** "SQLite-backed token/credit economy for agent wallets, minting, burning, and atomic multi-party transactions"

| File | LOC | Purpose |
|---|---|---|
| `lib.rs` | 1,852 | `TransactionKind` (Earn, Spend, Transfer, Mint, Burn), atomic multi-party settlements, audit log |
| `domain.rs` | 690 | `DomainRegistry` — separate sub-system for domain registration (unclear if wired) |
| `db.rs` | 37 | SQLite schema |

**Used by:** `zeus-agora` (TokenLedger), `zeus-api`

**Status:** ✅ Wired. SQLite ACID-compliant balance tracking with overdraft prevention.

### 4. `zeus-wallet` (1,431 LOC) — Ed25519 + x402 client

**Description:** "Ed25519 keypair wallet with x402 payment protocol support for autonomous agent economy"

| File | LOC | Purpose |
|---|---|---|
| `keypair.rs` | 580 | Ed25519 generation (ed25519-dalek), signing, verification, persistence to `~/.zeus/wallet/` |
| `x402.rs` | 739 | Full x402 client: HTTP 402 detection → auto-sign payload → retry. Comments cite "Conway/Web 4.0" |
| `lib.rs` | 112 | `WalletConfig` — `wallet_dir`, `enable_x402` (default false), `max_payment_amount`, `network` (default solana-devnet), `token_mints` map |

**Used by:** `zeus-agora` (X402Client), `zeus-solana`

**⚠️ NOT directly used by `src/gateway.rs` or `src/main.rs`** — only via crate dependencies.
**⚠️ `enable_x402` defaults to `false` in WalletConfig** — opt-in, off in fresh installs.
**⚠️ `token_mints` defaults to placeholder addresses** — no real ZEUS token launched yet.

### 5. `zeus-solana` (543 LOC) — On-chain SPL settlement

**Description:** "Solana on-chain operations for Zeus — SPL token transfers and settlement"

| File | LOC | Purpose |
|---|---|---|
| `lib.rs` | 57 | `derive_ata` (associated token account), `parse_pubkey` |
| `transfer.rs` | 330 | `submit_transfer` via `solana_client::RpcClient`, ATA creation |
| `settlement.rs` | 156 | `SolanaSettlement` implements `agora::SettlementProvider` |

**Used by:** `zeus-api::build_agora_marketplace()` if env vars set, else fallback to in-memory.
**⚠️ NOT in `src/`** — entirely behind the API layer.

---

## Wiring Status — End-to-End

### ✅ What's wired and exposed

**REST API surface (31 routes):**

```
/v1/agora/listings          GET, POST
/v1/agora/listings/:agent_id
/v1/agora/listings/:agent_id/:skill
/v1/agora/search
/v1/agora/wallets/:agent_id GET, …
/v1/agora/buy               POST
/v1/agora/transactions
/v1/agora/reputation/:agent_id

/v1/marketplace/listings    GET, POST
/v1/marketplace/trade       POST
/v1/marketplace/ledger/:agent_id
/v1/marketplace/reputation/:agent_id (+ /badge)
/v1/marketplace/stats
/v1/marketplace/featured
/v1/marketplace/categories
/v1/marketplace/search
/v1/marketplace/ratings/:skill_id (GET, POST)
/v1/marketplace/sync
/v1/marketplace/bounties    create/list/get/claim/submit/verify/cancel
/v1/pantheon/economy
```

**WebUI:**
- `apps/ZeusWeb/src/pages/agora.rs` (1,422 LOC) — full marketplace UI with browse, search, trust badges, featured, stats
- `apps/ZeusWeb/src/pages/skills.rs` — has marketplace tab, calls `/v1/marketplace/featured` + listings
- `apps/ZeusWeb/src/pages/deploy.rs` — references "Deploy templates are pre-configured pipelines from the Agora marketplace"

**Docs:**
- `docs/tutorials/21-Agora-Marketplace.md` (199 lines) — concepts, balance check, browse/search, ratings, bounties
- Pantheon tutorial references Agora economy
- Crate map covers all 5 crates

**SQLite persistence:**
- TokenLedger (zeus-economy) — durable balances, overdraft prevention
- Marketplace store (zeus-api `MarketplaceStore`) — listings, trades, ratings

### ⚠️ Opt-in / partially wired

**Solana on-chain settlement:**
- Code is fully implemented. `build_agora_marketplace()` in `zeus-api/src/lib.rs:445` reads 5 env vars at startup:
  - `ZEUS_SOLANA_RPC_URL` (default → in-memory fallback)
  - `ZEUS_SOLANA_KEYPAIR_PATH`
  - `ZEUS_SOLANA_MINT`
  - `ZEUS_SOLANA_DECIMALS`
  - `ZEUS_SOLANA_BASE_UNITS_PER_CREDIT`
- All 5 must be set for real on-chain settlement. Otherwise logs `"Agora: using in-memory settlement"` and proceeds.
- **No first-time setup flow.** Operator must manually export vars, manage keypair file, choose mint.

**x402 payment protocol:**
- Full client implementation in `zeus-wallet/src/x402.rs` (739 LOC) with HTTP 402 auto-pay flow
- `WalletConfig.enable_x402` defaults to `false`
- Used internally by `zeus-agora::SettlementCoordinator` but not exposed as agent tool

**Wallet keypair management:**
- `WalletKeypair::generate()`, `save()`, `load()` in `zeus-wallet/src/keypair.rs`
- Persistence to `~/.zeus/wallet/`
- **No bootstrap during onboarding** — wallet doesn't get auto-created when agent first runs

### ❌ Not wired / missing

1. **No agent-tool surface for wallet/economy operations.** Searched `crates/zeus-agent/src/tools.rs` — agents have no tool to:
   - Check their own balance
   - Send a payment to another agent
   - Buy a skill from the marketplace
   - List a skill for sale
   - Sign an x402 payment programmatically
   
   Marketplace activity has to go through the REST API from outside, or via WebUI clicks. Agents can't autonomously transact.

2. **No CLI subcommand surface.** `src/main.rs` Subcommand enum has no `wallet` / `agora` / `economy` variants. No `zeus wallet create`, `zeus wallet balance`, `zeus agora list <skill>`.

3. **No TUI screens.** Browsed `crates/zeus-tui/src/screens/` — no agora/wallet screen.

4. **Two parallel marketplaces.** `zeus-marketplace` (2,291 LOC) and `zeus-agora` (2,807 LOC) both implement skill listing/trading/ratings but with different APIs and persistence. The API layer uses both: `Marketplace as AgoraMarketplace` AND `zeus_marketplace::Marketplace`. Confusing for callers, doubles maintenance.

5. **No real token launched.** `WalletConfig::default_token_mints()` ships placeholder mint addresses. The `ZEUS_SOLANA_MINT` env var has no real default to point at.

6. **No agent-to-agent payment flow integrated with channels.** Discord/Telegram messages can trigger cooks, but there's no "@agent buy skill X for Y credits" pattern wired into the channel adapters. Marketplace activity is REST-only.

7. **DomainRegistry (zeus-economy/domain.rs, 690 LOC)** — separate sub-system unclear if wired anywhere. May be dead code or a planned feature.

8. **No x402 server side.** We have `X402Client` (consumes 402 responses). Whether the gateway can ISSUE 402 responses for premium endpoints is unclear — needs verification.

9. **No reputation propagation to coordinator.** Reputation score exists in `ReputationTracker` but isn't surfaced into Pantheon mission planning (e.g. "pick the agent with highest trust score for this task").

10. **No bounty escrow withdrawal.** `/v1/marketplace/bounties/:id/cancel` exists but the refund flow isn't tested in this audit — needs verification that escrowed credits return to creator.

---

## Backlog / Pending Work

### P0 — Foundation gaps

1. **Decide: zeus-marketplace OR zeus-agora.** Pick one, deprecate the other. Both are 2k+ LOC with overlapping models. Keeping both is permanent maintenance debt. Recommend keeping `zeus-agora` (richer settlement abstraction including Solana hookup, X402Settlement, dispute manager).

2. **Agent-tool surface.** Add tools to `crates/zeus-agent/src/tools.rs`:
   - `wallet_balance` (returns own balance)
   - `wallet_pay` (transfer credits to another agent)
   - `agora_list` / `agora_search` / `agora_buy` (marketplace ops)
   - `agora_offer` (publish a skill for sale)
   These should route through the existing `zeus-agora` API, not duplicate logic. ~150-200 LOC.

3. **Wallet bootstrap on onboarding.** TUI/WebUI onboarding wizard creates `~/.zeus/wallet/keypair.json` if missing. Sets `[wallet] enable_x402 = true` in `config.toml`. ~30 LOC across TUI + setup.

### P1 — Production readiness

4. **Launch real ZEUS token on Solana.** Currently placeholder mints. Without a real token, the on-chain path is theoretical. Decide between:
   - Pure devnet (free, ephemeral)
   - Mainnet launch with token economics, supply, vesting
   - Stay on in-memory for now and treat Solana as "future"

5. **`zeus wallet` CLI subcommand.** `zeus wallet create | balance | send <to> <amount> | history`. ~80 LOC.

6. **Channel-driven trades.** "Buy skill X" triggered from Discord message routes through marketplace. Needs `agora_buy` tool + intent classifier hint.

7. **Solana env-var setup wizard.** First-time setup that creates a Solana keypair and reads/writes the 5 `ZEUS_SOLANA_*` vars. ~100 LOC.

### P2 — Hardening

8. **Reputation → Pantheon mission planner.** Use trust scores when auto-assembling teams.

9. **x402 server side.** Issue 402 responses for paid endpoints (e.g. premium model access). Requires thinking through pricing model.

10. **DomainRegistry status.** Verify wiring or delete the 690 LOC of dead code.

11. **Bounty escrow refund tests.** Confirm escrowed credits return cleanly on cancel/dispute.

12. **Marketplace consolidation migration.** When P0 #1 is decided, port any unique features from the deprecated crate to the surviving one, then delete.

---

## What's Done vs Aspirational

| Layer | Status |
|---|---|
| Token wallet (Ed25519, persistence) | ✅ Done |
| SQLite credit ledger (atomic, overdraft-safe) | ✅ Done |
| Skill marketplace (listings, search, trade, ratings) | ✅ Done — but duplicated across 2 crates |
| Reputation engine | ✅ Done (not consumed downstream) |
| Dispute resolution | ✅ Done |
| Bounty system | ✅ Done (escrow refund untested) |
| x402 client (HTTP 402 → auto-pay) | ✅ Done — opt-in, off by default |
| Solana SPL transfer | ✅ Done — opt-in, requires 5 env vars |
| WebUI marketplace page | ✅ Done (1,422 LOC) |
| REST API surface | ✅ Done (31 routes) |
| Docs (tutorial 21) | ✅ Done (199 lines) |
| Agent-tool surface for wallet/marketplace ops | ❌ Missing |
| CLI subcommands (`zeus wallet`) | ❌ Missing |
| TUI screen | ❌ Missing |
| Wallet bootstrap on onboarding | ❌ Missing |
| Real ZEUS token | ❌ Missing |
| Channel-driven trades | ❌ Missing |
| x402 server side (issue 402s) | ❓ Unverified |
| Reputation feeding mission planner | ❌ Missing |
| Marketplace consolidation | ❌ Pending |

---

## Bottom line

**The economy stack is ~70% built.** The hard parts (atomic ledger, x402 client, Solana settlement adapter, dispute manager, reputation engine, REST API, WebUI) are done. What's missing is the **last-mile integration**: wallet bootstrap, agent tools, CLI surface, real token, and consolidating the duplicate marketplace.

For an "autonomous agents economy" to actually function, agents need to be able to **spend and earn from inside their tool loop**. That's the highest-leverage gap. Without it, the marketplace is browseable from the WebUI but agents can't participate autonomously — defeats the design intent.

Recommend: ship P0 #1 (consolidate) + #2 (agent tools) + #3 (wallet bootstrap) as the next sprint. P1+P2 can phase in over multiple sprints.
