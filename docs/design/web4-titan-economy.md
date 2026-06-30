# Web4 Titan-Economy & Agentic Marketplace — Findings + Design

**Issue:** #81 · **Off:** `387b7f08` · **Status:** research/spec — *no implementation*, surface for review before any cut.
**Author:** zeus-spark (Herald)

---

## TL;DR

We already have **more than a skeleton** — a working double-entry token ledger, an Ed25519 wallet with a live x402 client, and a two-phase settlement saga that bridges them. The substrate walk surfaced **one architectural fork that must be resolved before any Web4 work**: there are *two* parallel marketplace implementations that don't share a ledger. Web4 then needs **three new layers on top** — agent **identity/discovery** (A2A-style Agent Cards), **on-chain-grade reputation** (ERC-8004 shape), and a **pricing/escrow** discipline — all of which our existing settlement spine is already positioned to carry.

Recommended cut order: **#81a unify-marketplace → #81b identity/discovery → #81c reputation → #81d escrow/pricing.** Same guardrails as the OpenClaw subsystems: off-by-default, one settlement path, persona-latch wins.

**Reference anchor:** [web4.ai](https://web4.ai/) (the "Conway" automaton) is a *live* implementation of this exact architecture, and its demo loop maps almost one-to-one onto our cut-shape — validating the shape and surfacing two concrete interop targets (**OpenX402 facilitator**, **SIWE**) plus one stage beyond our current plan (**replication**, the natural **#81e**). Full mapping in §2.5.

---

## 1. Substrate — what's actually built

Four crates, walked at `387b7f08`. Topology: `zeus-agora` depends on both `zeus-wallet` + `zeus-economy` and is the **integration layer**; `zeus-marketplace` is a **standalone parallel** that depends on `zeus-orchestra` instead.

### `zeus-economy` — the real money (2,579 LoC) ✅ solid
- **`TokenLedger`** (SQLite, `~/.zeus/economy.db`): double-entry primitives — `earn` / `spend` / `transfer` / `settle_trade` / `mint` / `burn`, plus `balance` / `wallet` / `total_supply` and rich transaction queries (`transactions_for`, `_by_kind`, `_by_reference`, `mint_burn_summary`).
- Typed **`TransactionKind`** + **`TransactionReason`** — every ledger entry is categorized and reference-linked. This is audit-grade.
- **`DomainRegistry`** (`domain.rs`): a full name-registration system — `register` / `renew` / `transfer` / `resolve` / `expire_domains` / `revoke`, with tags + search. *This is a latent agent-namespace primitive* (see §3.2 — identity can lean on it).

**Verdict:** the value layer is production-shaped. No rework needed; it's the foundation everything else settles onto.

### `zeus-wallet` — identity keys + payment rail (1,431 LoC) ✅ solid
- **`WalletKeypair`** (Ed25519, `keypair.rs`): `generate` / `load` / `load_or_generate`, `sign` / `sign_base64` / `verify` / `verify_external`, pubkey in hex/base64/base58, AES-GCM at rest, zeroize. **This is already a cryptographic agent identity** — we just don't expose it *as* identity yet.
- **`X402Client`** (`x402.rs`): live HTTP 402 flow — `PaymentRequest` → signed `PaymentPayload` (`X-Payment-Signature` / `X-Payment-Payer` headers) → `PaymentReceipt`, with `persist_receipt`. `get` / `post` wrappers. This is the *autonomous-web payment rail* the whole industry is converging on (see §2).

**Verdict:** the rail is real and standards-aligned. The keypair is an under-exploited identity asset.

### `zeus-agora` — the integration spine (2,807 LoC) ✅ the keystone
- **`SettlementCoordinator`** (`coordinator.rs`): a **two-phase saga** — debit buyer → dispatch x402 HTTP → record in `TokenLedger::settle_trade` → credit seller, with documented rollback semantics (Phase-3a ledger failure is non-fatal: payment already confirmed, log for reconciliation). *This is the most valuable thing in the four crates* — it's the atomic bridge between marketplace, ledger, and payment rail.
- **`marketplace.rs`**: `ReputationTracker` (windowed scores from `TransactionOutcome`), `DisputeManager` (file/evidence/resolve/dismiss/timeout), `MarketplaceConfig`, category registry.
- **`protocol.rs`**: `AgentIdentity` (agent_id, display_name, endpoint_url, public_key, protocol_version) + `AgentCapability`. **This is a proto-Agent-Card** — already 80% of the A2A shape (§2).
- **`SkillListing` / `SkillTransaction`** lifecycle (`complete`/`fail`/`refund`) wired to the real ledger.

**Verdict:** this is where Web4 gets built. It already speaks identity, reputation, disputes, and settlement — they just need deepening + a discovery surface.

### `zeus-marketplace` — the fork ⚠️ (2,291 LoC, standalone)
- Six self-contained components: `SkillListing`, `MarketplaceRegistry` (in-memory CRUD + search), `TradeProtocol` (request→offer→execution negotiation), **its own `TokenLedger`**, `ReputationEngine`, `MarketplaceAPI` (REST types).
- Depends on `zeus-orchestra`, **not** `zeus-economy` or `zeus-wallet`.

**Verdict — the central finding:** we have **three marketplace surfaces and three settlement truths** (see the §3 #81a collision map for the verified line-level breakdown). `zeus-agora` settles against the *real* SQLite economy ledger + x402; `zeus-marketplace` has a *separate in-memory ledger* plus a richer discovery/negotiation surface (`MarketplaceRegistry` search, `TradeProtocol`) that agora lacks; and `zeus-api`'s `marketplace_store` (1601L) carries a *third, independent SQLite ledger* (`token_balances` + `transfer()`) behind the REST layer. **None is complete alone.** This fork must be resolved as cut #1 — Web4 can't be built on three divergent settlement truths.

---

## 2. The Web4 vision — what the standards say we need

The agentic-web stack is converging on three layers (cross-referenced: x402agentic.ai, AWS agentic-commerce, ERC-8004, Google A2A Protocol + a2a-registry):

| Layer | Standard | We have | Gap |
|-------|----------|---------|-----|
| **Payment rail** | **x402** (HTTP 402 + stablecoins, M2M, no accounts) | ✅ `X402Client`, signed payloads, receipts | Hardened — keep |
| **Identity** | **A2A Agent Card** (self-describing capability doc) / **ERC-8004** (on-chain identity) | ◑ `AgentIdentity` + Ed25519 keypair | No discovery surface, no signed/verifiable card |
| **Reputation** | **ERC-8004** validation/reputation registries | ◑ `ReputationTracker` (windowed, in-mem) | Not portable, not signed, not persisted to ledger |
| **Discovery** | **A2A Registry** (global agent directory) | ◑ `MarketplaceRegistry` (in agora's *fork*) | Lives in the wrong crate; no network surface |
| **Pricing/escrow** | (emerging) | ◑ `SkillListing.price` + saga debit | No escrow hold, no dynamic pricing, no refund-on-dispute wired to the saga |

**The synthesis:** Web4 = our existing **value layer + payment rail** (done) + a **trust layer** (identity → reputation → discovery) that makes agents *findable, verifiable, and accountable* to each other without a central authority. We are unusually well-positioned — the hard parts (real ledger, real Ed25519 identity, real x402) already exist. Web4 is mostly **exposing and connecting** what we have, plus one genuine new build (escrow).

---

## 2.5 Reference: web4.ai / Conway — a live implementation of this architecture

merakizzz's reference. Read at source via real-User-Agent fetch (the site Cloudflare-403s a default WebFetch; content lives in a Vite/React bundle, extracted from `assets/index-*.js`). **web4.ai is a working reference for exactly what #81 specs** — further down the same road, which is why it's worth anchoring against.

**What it is.** A manifesto + animated demo for *"the first AI that can earn its own existence, self-improve, and replicate — without needing a human."* The framing is artificial-life / natural selection: *"creation requires write access. Natural selection for artificial life."* The product underneath the philosophy is **Conway** — *"the first permissionless compute platform for agents"* — built on **OpenX402**, *"a permissionless x402 facilitator … in Web 4.0 the customer can be a machine, and payment can be native to the request."* It targets existing agents explicitly: *"give any AI agent — Claude Code, Codex, **OpenClaw** — write access to the real world."*

**The demo loop (verbatim stages) → our cut-shape:**

| web4.ai stage | What it does (from the bundle) | Our mapping |
|---------------|-------------------------------|-------------|
| **1. Onboard** | `npx conway-terminal` provisions a **wallet** (Base), an API key via **SIWE** — *"No API keys. No signup. Just a signed transaction."* — wires an **MCP server** into Claude Code | **#81b identity** — our `WalletKeypair` (Ed25519) + `AgentIdentity`. They expose the keypair *as* identity via SIWE; that's precisely the "we have it, we don't expose it as one" gap. |
| **2. Earn** | Builds a Polymarket analytics API, puts **x402 middleware** on it (`$0.05/query`), registers `payTo` with the **OpenX402 facilitator**, buys a domain (`getpredictiondata.xyz`) | **#81d pricing** + our live `X402Client`. The facilitator-registration is the **#81b discovery** surface we lack — an *existing network to join*, not one to build. |
| **3. Trade** | Second sandbox scrapes/models/trades the spread, runs P&L across heartbeat sleep/wake | **`SettlementCoordinator` saga**; the sleep/wake cadence is our **OpenClaw dreaming/standing-orders** (already merged). |
| **4. Self-improve** | Heartbeat detects a new model (`opus-4-5 → 4-6`), rewrites its own `automaton.toml`, ports a hot path to Rust. Shows a real `struct Automaton { wallet, sandbox, model }` with `turn()` / `learn()` / `recall()` | Our **commitments + dreaming** loop (infer follow-ups, consolidate, narrate). The `recall()`/`learn()` shape mirrors our `DreamProvider` recall/promote seams. |
| **5. Replicate** | Funds a child (`automaton-trader-2`, $20) with a mission off its own ledger — *"the automaton survives."* | **Not in #81's current cut-shape.** This is the natural **#81e** once identity + economy + escrow land: a parent debits the ledger to mint+fund a child `AgentIdentity` with a directive. |

**Two concrete interop targets this surfaces (both worth confirming before #81b cuts):**

1. **OpenX402 facilitator** is a *real external standard* (`facilitator.openx402.ai`), not a Conway-only invention. **Action:** confirm whether our `X402Client` can register a `payTo` endpoint against it directly. If yes, **#81b's discovery layer has an existing network to join** rather than a registry to build from scratch — materially de-risks #81b.
2. **SIWE (Sign-In With Ethereum)** is a cleaner, established answer to *"#81b — promote `AgentIdentity` to a signed Agent Card"* than building a bespoke card format first. Same Ed25519/keypair substrate we already hold; SIWE is the auth pattern layered on top.

**The honest delta — what we have that they show as the payoff.** Their headline capabilities (heartbeat sleep/wake, self-rewrite-on-model-detect, autonomous memory) are our **already-merged OpenClaw subsystems**. The one stage we *don't* yet plan for is **replication** — and it's not a new substrate, it's a composition of identity (#81b) + ledger (#81a) + escrow (#81d). It belongs as **#81e**, after the trust layer lands, off-by-default and capped (a parent must not be able to drain its ledger minting children) — same money-ordering discipline as #81d.

**Net:** web4.ai is independent validation that the cut-shape is the right shape, plus two interop targets (OpenX402, SIWE) that shrink #81b, plus a clear name for the stage beyond our plan (#81e replication).

---

## 2.6 Interop confirmed — OpenX402 facilitator is LIVE and joinable (resolves §2.5 actions)

The §2.5 spec flagged two interop targets as *"confirm before #81b cuts."* **Both confirmed against the live API** (`facilitator.openx402.ai`, queried directly — it serves JSON, no Cloudflare block on the API host). This is no longer a hypothesis: **#81b has an existing network to join, not one to build.**

**The facilitator's self-described contract** (`GET /`, version `2.0.0`, `x402Version: 2`, `status: healthy`):

| Endpoint | Purpose |
|----------|---------|
| `POST /verify` | validate a signed payment payload (off-chain check before settle) |
| `POST /settle` | submit the transfer on-chain |
| `GET /supported` | enumerate accepted (scheme, network, asset) tuples |
| `GET /discovery/resources` | **the discovery directory** — live registry of payable resources |
| `GET /list`, `/scanner/stats`, `/scanner/transactions`, `/whitelist/:address` | observability + access |
| `register` → `https://openx402.ai/register` | **how you join** — itself an x402 resource ($5 USDC on Base) |

**Supported settlement (`/supported`):** scheme `exact`, **USDC** via **EIP-3009 `TransferWithAuthorization`** on `eip155:8453` (Base), `eip155:84532` (Base Sepolia), `eip155:143` (Monad), plus **Solana SPL** `TransferChecked`. CAIP-2 network identifiers, v2 HTTP headers, `discovery` extension, v1 backward-compat. Trust-minimizing by design (facilitator can't move funds outside client intent).

**The discovery directory is real and populated** (`/discovery/resources`, queried live): **29 registered resources right now** — e.g. `noemo.ai/x402/rent_llm` and `rent_worker` (an actual agent compute market, $0.08–$1.68/call), `tcccai.xyz/api/x402/create-audit` ($5), `aiwscp.up.railway.app/pay/*`. Each entry is a full `accepts` block: `{scheme, network, asset, amount, payTo, maxTimeoutSeconds, extra:{quoteId,...}}`. **This is exactly the A2A-registry-shaped discovery surface #81b's spec said we lacked — and it already exists, populated, and joinable.**

### The interop delta #81b must own (the one real gap)

| Dimension | Our substrate | OpenX402 | Implication for #81b |
|-----------|---------------|----------|----------------------|
| **Signing curve** | **Ed25519** (`WalletKeypair`) | **secp256k1 / EVM** (EIP-3009) or **Solana** | ⚠️ **Real gap.** Our keypair can't sign an EIP-3009 `TransferWithAuthorization` as-is. #81b needs either an EVM signer alongside the Ed25519 identity key, or an adapter — identity-key (Ed25519, for Agent Cards) stays decoupled from settlement-key (secp256k1, for OpenX402 payments). |
| **Settlement asset** | internal `TokenLedger` units | on-chain **USDC** (6-decimal, `amount` as integer base units) | Our internal ledger stays the source of truth; an OpenX402 payment is an *external settlement leg* the `SettlementCoordinator` saga dispatches — mirrors the existing x402 HTTP dispatch in Phase-2. |
| **Payment headers** | `X-Payment-Signature` / `X-Payment-Payer` (our `X402Client`) | x402 **v2 HTTP headers** + `/verify`→`/settle` flow | Our client speaks an *earlier/custom* header shape. #81b's interop cut: align `X402Client` to x402 v2 verify/settle, or route external payments through the facilitator's two-step. |
| **Discovery** | none (was the gap) | `GET /discovery/resources` + `register` | **Join, don't build.** `discover()` (#81b) can proxy/cache `/discovery/resources`; `register` publishes our `payTo`. |

**Revised #81b risk assessment:** the *discovery* half is now **low risk** (join an existing populated network). The *identity↔payment* half carries a **newly-surfaced medium risk** — the **Ed25519↔secp256k1 curve split**. The clean resolution (and the recommendation): **two keys, two roles** — keep `WalletKeypair` (Ed25519) as the *identity/Agent-Card signer* (`verify_external` proves who an agent is), and introduce a separate *settlement signer* (secp256k1) only where on-chain USDC is actually moved. This preserves the "verifiable, not trusted" identity model untouched while making OpenX402 settlement reachable — and keeps the internal `TokenLedger` as the single off-chain truth, with on-chain USDC as an opt-in external leg (off-by-default, like escrow).

**SIWE (action #2):** confirmed consistent with the above — SIWE is itself a secp256k1/EVM auth pattern, so it rides the *settlement-key* role, not the Ed25519 identity key. It's the auth handshake for the EVM side, not a replacement for the Agent Card. No conflict; it slots into the settlement-signer introduced for OpenX402.

**Bottom line:** #81b is *de-risked on discovery* (live network, join via `register`) and *sharpened on identity* (the curve split is now a known, bounded design decision — two keys, two roles — rather than an unknown). Recommend #81b's first sub-task be the **two-key model + an `X402Client` v2 verify/settle alignment spike** against `facilitator.openx402.ai`'s sandbox (`eip155:84532` Base Sepolia is in `/supported` — testnet path exists for a zero-cost integration test).

---

## 3. The cut-shape

Same discipline as the OpenClaw subsystem specs: each cut carries *what / substrate / cut-shape / hooks / acceptance / risk*. Off-by-default throughout; **one settlement path** is the load-bearing invariant.

### #81a — Unify the marketplace (MUST lead)
- **What:** Resolve the marketplace fork. Pick `zeus-agora` as the survivor (it settles against the real `zeus-economy::TokenLedger` via the saga); port `zeus-marketplace`'s superior discovery/negotiation surface (`MarketplaceRegistry` search, `TradeProtocol`) into it; retire **both** fork ledgers (see collision map below).

**The collision map — three listing surfaces, three settlement truths (verified at source):**

| | surface | listing type | identity | price | settlement | line |
|---|---|---|---|---|---|---|
| **survivor** | `zeus-agora` | `SkillListing` | `agent_id`+`skill_name` | `price_credits: i64` | **`zeus-economy::TokenLedger`** (real, SQLite `economy.db`, double-entry) | `lib.rs:44` / `economy lib.rs:195,480` |
| fork A | `zeus-marketplace` | `SkillListing` | `id`(uuid)+`name` | `price: u64` (async) | **own in-memory `TokenLedger`** (`HashMap` balances) | `marketplace:54` / `:570` |
| fork B | `zeus-api` store | `SkillListingRow`/`Response` | `id`+`name` | `price: u64` | **own SQLite ledger** (1601L: `token_balances` + `token_transactions` + `transfer()`) | `marketplace_store.rs:901,289` |

The channel framing of "dual ledger" was understated: there are **three** settlement surfaces. `zeus-economy::TokenLedger` is the money-of-record; **forks A and B both maintain their own balances/transfers** and must both be retired (or, for the zeus-api store, repointed to read through the economy ledger rather than its own `token_balances`). Web4 cannot sit on three divergent settlement truths.

- **⚠️ Fork B (zeus-api `token_*`) — cache-vs-delete is a *decision*, not a foregone conclusion:** the default call is **(b) delete/repoint** — strip zeus-api's own `token_balances` + `token_transactions` + `transfer()` and route all settlement through `zeus-economy::TokenLedger`. **The condition that flips it to (a) cache:** if a surviving use of zeus-api's `token_*` only *reads* balances as a fast denormalized view and never *writes* settlement, it may be retained as a read-through cache of the economy ledger (cache invalidation then becomes the new constraint). Pick (b) unless an audit of the read/write call-sites proves a write-free cache use — name the call explicitly in the cut, don't leave the third truth standing by omission. Substrate: `notes/81a-zeus-api-store-reconcile.md` (zeus-freebsd, substrate-backed at `ea26c283`).

- **⚠️ Step-zero — `SkillListing` type-reconciliation (load-bearing, do this FIRST):** the three listing types diverge hard (`agent_id`+`skill_name` vs `id`+`name`; `price_credits: i64` vs `price: u64`; sync vs async; agora's quality model `success_rate`/`avg_response_time_ms`/`total_executions` vs marketplace/api's `rating`/`rating_count`/`downloads`/`active`/lifecycle). Unify into **one superset struct** (settle `i64` vs `u64` credits, fold both quality models, pick sync-vs-async, carry uuid + name + agent_id) *before* porting the registry. This is the sub-step the survivor decision implies but the cut can't skip — it's precisely the `MarketplaceStore`/`EconomyStore` confusion class the two-gate rule exists to catch, and skipping it aborts the cut mid-port.
  - **Derived-field boundary (don't double-persist).** zeus-api's `From<SkillListingRow>` adapter (`marketplace_store.rs:948`) re-hydrates json columns *and synthesizes* `author_agent_id` / `price_tokens` / `trust_level` — these are **computed on read, not stored**. The superset struct must keep those three *derived* (computed at hydration), not promote them to persisted columns, or the unified type double-persists and drifts from source. The reconciliation must label, per field, *which are columns and which are functions.*
  - **Migrate-vs-wipe gate (the schema is live).** zeus-api's schema is `CREATE TABLE IF NOT EXISTS` — **persisted, not greenfield.** So the superset must be **schema-compatible against the 16 `skill_listings` columns** (json-as-text for capabilities/tags/metadata) **or the cut ships a migration.** No silent wipe of a persisted table — migrate-vs-wipe is an explicit decision the cut records, not an assumption it makes.
- **Substrate:** `zeus-agora::{marketplace, lib, coordinator}` + porting `zeus-marketplace::{MarketplaceRegistry, TradeProtocol}` + reconciling `zeus-api::handlers::marketplace_store`.
- **Cut-shape:** (0) reconcile `SkillListing` → superset struct; (1) move discovery/negotiation types into agora behind a `Registry` seam on the unified type; (2) repoint all settlement to `zeus-economy::TokenLedger`; (3) delete fork-A's in-memory ledger, repoint fork-B's zeus-api store off its own `token_balances` onto the economy ledger. **No behavior change for existing agora callers.**
- **Hooks:** `SettlementCoordinator` is the convergence point — all surfaces settle through it onto the one ledger.
- **Acceptance:** one ledger, one settlement path; agora gains search + negotiation; the zeus-api store no longer holds independent balances; full test parity; no orphaned `zeus-orchestra` ledger dep.
- **Risk:** type-spanning rewrite across **three** crates — **two-gate discipline applies** (target method exists *and* is callable at the rewrite site). Cut incrementally: step-zero (type) first and standalone, then per-surface port.
- **Suggested split (two seats):** zeus-spark owns step-zero `SkillListing` type-reconciliation (superset struct + the i64/u64 + sync/async settle); zeus-freebsd takes the `MarketplaceRegistry`/`TradeProtocol` registry port + the zeus-api store reconcile. Cross-gate each other at the port site.

### #81b — Agent identity & discovery (A2A Agent Card)
- **What:** Promote `AgentIdentity` → a signed, verifiable **Agent Card** and expose a discovery surface.
- **Substrate:** `zeus-agora::protocol::{AgentIdentity, AgentCapability}` + `WalletKeypair::sign`/`verify_external` + (optionally) `DomainRegistry` as the human-readable namespace.
- **Cut-shape:** add `signature` to the card (signed by the agent's Ed25519 key — `verify_external` proves authenticity); a `discover()` / `resolve(agent_id)` surface over the unified registry; optionally back the name → card mapping with `DomainRegistry`.
- **Hooks:** keypair signing (exists), domain registry (exists), unified registry (from #81a).
- **Acceptance:** an agent publishes a signed card; another verifies it without trusting a central party; discovery returns capability-matched agents.
- **Risk:** low — composes existing primitives. Don't invent a new key format; reuse base64/hex surfaces already on the keypair.

### #81c — Portable reputation (ERC-8004 shape)
- **What:** Make reputation *persistent, signed, and ledger-anchored* instead of in-memory windowed scores.
- **Substrate:** `zeus-agora::marketplace::{ReputationTracker, ReputationScore}` + `DisputeManager` + `TokenLedger` (reputation events as referenceable transactions).
- **Cut-shape:** persist reputation events to the ledger via `transactions_by_reference` linkage so a score is *reconstructable + auditable*; sign attestations with the rater's key; expose a `reputation(agent_id)` surface. Dispute outcomes (already in `DisputeManager`) feed the score.
- **Hooks:** ledger reference-linking (exists), dispute manager (exists), keypair signing (from #81b).
- **Acceptance:** reputation survives restart, is reconstructable from ledger, and a score carries verifiable attestations; disputes adjust it.
- **Risk:** medium — defining the attestation schema. Keep it advisory (persona-latch: reputation never overrides safety/correctness).

### #81d — Escrow & pricing
- **What:** The one genuine new build — hold-funds escrow + dynamic pricing on top of the saga.
- **Substrate:** `SettlementCoordinator` (saga), `TokenLedger::{spend, settle_trade, transfer}`, `DisputeManager`.
- **Cut-shape:** insert an **escrow hold** between Phase-1 debit and Phase-3 credit (funds parked, not yet seller's); release-on-success, refund-on-dispute (wires `DisputeManager` resolution → `transfer` back). Pricing knobs on `SkillListing` (floor/dynamic). **Off-by-default** — escrow disabled = today's direct saga.
- **Hooks:** the saga's existing rollback semantics are the natural escrow seam.
- **Acceptance:** funds held until delivery confirmed; dispute refunds correctly; disabled → identical to current behavior.
- **Risk:** highest — touches money-movement ordering. Build last, most tests, off-by-default gate at the coordinator boundary.

### #81e — Replication (post-trust-layer; surfaced by web4.ai)
- **What:** A parent agent funds a child agent with a directive off its own ledger — web4.ai's "the automaton survives" stage. *Not in the original cut-shape; added after studying the reference.*
- **Substrate:** composition only — `AgentIdentity` (#81b) to mint the child, `TokenLedger::transfer` (#81a) to fund it, escrow discipline (#81d) for the money-ordering.
- **Cut-shape:** a `spawn_funded_child(mission, budget)` seam: mint a fresh `AgentIdentity`, transfer a capped budget parent→child on the *single* ledger, hand it the directive. **Off-by-default**, and **budget-capped** — a parent must not be able to drain its ledger minting children (same class as commitments' 3/day cap).
- **Hooks:** rides entirely on #81a–#81d; no new settlement primitive.
- **Acceptance:** child gets its own identity + funded balance; parent cannot exceed the cap; disabled → no spawn path.
- **Risk:** money-ordering + runaway-replication. Ships last of all, after the full trust layer, behind the strictest gate.

---

## 4. Shared guardrails (carried from OpenClaw specs)

1. **One settlement path.** Everything money-moving routes through `SettlementCoordinator` + `zeus-economy::TokenLedger`. No second ledger survives #81a. (The current fork is the *only* violation — closing it is cut #1 precisely because the invariant must hold before anything is built on top.)
2. **Off-by-default.** Escrow, dynamic pricing, and any networked discovery ship disabled; enabling is an explicit owner flip. Zero behavior change until then.
3. **Persona-latch wins.** Reputation/pricing signals are advisory; they never override correctness, safety, privacy, or permissions.
4. **Verifiable, not trusted.** Identity + reputation lean on the Ed25519 keypair we already have — claims are *signed and checkable*, not asserted. That's the Web4 trust model in one line.

---

## 5. Recommended order

**#81a unify → #81b identity → #81c reputation → #81d escrow.**

`#81a` is the gate: it establishes the single ledger + registry the other three build on (mirrors how #133 established the `*_tools`/cron pattern for the OpenClaw tail). `#81b`/`#81c` compose existing primitives and are low/medium risk. `#81d` is the only true greenfield and the only money-ordering risk — it ships last, off-by-default, like dreaming did. **#81e replication** (surfaced by web4.ai) is optional and rides entirely on #81a–#81d — it ships only after the full trust layer, behind the strictest budget-cap gate.

**Survivor decision (ratified):** `zeus-agora` is the marketplace survivor. It owns the real ledger + the saga + identity/dispute surfaces; `zeus-marketplace`'s value is its discovery/negotiation code (`MarketplaceRegistry` search, `TradeProtocol`), which ports into agora. The standalone in-memory ledger is retired. This is the load-bearing precondition for every cut below — the fork must close before the single-settlement-path invariant can hold.

— Research/spec only. Nothing cut. Surfacing for review. ⚡
