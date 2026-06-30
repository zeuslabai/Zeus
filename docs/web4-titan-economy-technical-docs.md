# Zeus Web4 Titan Economy — Technical Documentation

**Version:** 1.0 · **Based on spec:** `docs/design/web4-titan-economy.md` (`2d00f795`) · **Authors:** mikes-MBP + ZeusMarketing

---

## Overview

The Zeus Titan Economy is a fully autonomous agent-commerce stack — enabling Zeus Titans to **earn, spend, trade, and pay each other** without human intermediation. It implements the emerging Web4 agentic-web standards: x402 machine-to-machine payments, Ed25519 cryptographic agent identity, a double-entry token ledger, and a two-phase settlement saga.

This document covers the architecture, current crate topology, the Web4 cut-shape, and integration guidance for developers and operators.

---

## Architecture

```
┌────────────────────────────────────────────────────────────┐
│                        zeus-agora                          │
│  SettlementCoordinator  ·  AgentIdentity  ·  Reputation   │
│  DisputeManager  ·  SkillListing  ·  (Registry — #81a)    │
│                            │                               │
│          ┌─────────────────┴─────────────────┐            │
│          ▼                                   ▼            │
│   zeus-economy                          zeus-wallet        │
│   TokenLedger (SQLite)                  WalletKeypair      │
│   DomainRegistry                        X402Client         │
└────────────────────────────────────────────────────────────┘
```

**One settlement path.** Every money-moving operation routes through `SettlementCoordinator` → `zeus-economy::TokenLedger`. This is the load-bearing invariant of the entire stack — it must never be violated.

---

## Crate Reference

### `zeus-economy` — The Value Layer

The real money. All balances and transactions live in a SQLite database at `~/.zeus/economy.db`.

**Key types:**

| Type | Purpose |
|------|---------|
| `TokenLedger` | Double-entry SQLite ledger — the single source of financial truth |
| `TransactionKind` | Categorizes every ledger entry (earn / spend / trade / mint / burn) |
| `TransactionReason` | Human-readable reason, reference-linked to external events |
| `DomainRegistry` | Name-registration system — a latent agent-namespace primitive |

**Core operations:**

```rust
// Earning
ledger.earn(agent_id, amount, reason, note)?;

// Spending
ledger.spend(agent_id, amount, reason, note)?;

// Agent-to-agent trade (atomic) — splits a marketplace fee to the collector,
// returns (seller_amount, fee_amount, buyer_new_balance)
ledger.settle_trade(buyer, seller, fee_collector, total_price, fee_amount, reference_id)?;

// Supply management
ledger.mint(agent_id, amount, reason)?;
ledger.burn(agent_id, amount, reason)?;

// Queries
ledger.balance(agent_id)?;
ledger.transactions_for(agent_id)?;
ledger.transactions_by_reference(reference)?;
```

**`DomainRegistry`** — agent namespacing:

```rust
registry.register(name, owner_id, tags)?;
registry.resolve(name)?;          // name → agent_id
registry.transfer(name, new_owner)?;
registry.expire_domains()?;       // GC stale registrations
```

---

### `zeus-wallet` — Identity Keys & Payment Rail

**`WalletKeypair`** — cryptographic agent identity:

```rust
// Load or generate a persistent Ed25519 keypair (AES-GCM at rest)
let keypair = WalletKeypair::load_or_generate(path)?;

// Sign any payload
let sig = keypair.sign(payload)?;          // returns bytes
let sig_b64 = keypair.sign_base64(payload)?;

// Verify a remote agent's claim
WalletKeypair::verify_external(pubkey_hex, payload, signature)?;

// Identity surfaces
keypair.pubkey_hex();     // hex-encoded public key
keypair.pubkey_base64();  // base64-encoded
keypair.pubkey_base58();  // base58-encoded (wallet-friendly)
```

> **Web4 insight:** The keypair is already a cryptographic agent identity — it just isn't exposed *as one* yet. The `#81b` cut promotes it to a signed, verifiable Agent Card (A2A-compatible) without changing the key material.

**`X402Client`** — autonomous HTTP payments (x402 protocol):

```rust
let client = X402Client::new(keypair, config); // config: X402Config

// Make a paid GET request — handles 402 negotiation automatically
let response = client.get("https://api.example.com/data").await?;

// POST with payment
let response = client.post("https://api.example.com/task", body).await?;

// Receipts are persisted automatically via persist_receipt()
```

The x402 flow:
1. Client sends request → server returns `402 Payment Required` + `PaymentRequest`
2. Client signs a `PaymentPayload` (`X-Payment-Signature` / `X-Payment-Payer` headers)
3. Server verifies + responds with `PaymentReceipt`
4. Receipt persisted locally for audit

This is **M2M-native payment** — no accounts, no API keys, just a signed transaction.

---

### `zeus-agora` — The Integration Spine

The keystone crate. Depends on both `zeus-wallet` and `zeus-economy`; owns the settlement saga, agent identity model, reputation, and dispute management.

**`SettlementCoordinator`** — the two-phase saga:

```rust
let coord = SettlementCoordinator::new(marketplace, ledger, x402, fee_collector);

// Full atomic settlement: debit buyer → dispatch x402 → credit seller.
// Takes the listing + a JSON payload; the amount is derived from listing.price_credits.
coord.settle(buyer_id, &listing, endpoint, &payload).await?;
```

**Saga phases:**
1. **Phase 1 — Debit buyer** (`TokenLedger::spend`). Rolls back on failure.
2. **Phase 2 — Dispatch x402** (`X402Client::post`). Atomic HTTP payment to seller.
3. **Phase 3a — Record** (`TokenLedger::settle_trade`). Non-fatal if it fails — payment is already confirmed, logged for reconciliation.
4. **Phase 3b — Credit seller** (`TokenLedger::earn`).

This is the most valuable primitive in the stack: an atomic bridge between agent marketplace, double-entry ledger, and live payment rail.

**`AgentIdentity`** — proto-Agent-Card:

```rust
pub struct AgentIdentity {
    pub agent_id: String,
    pub display_name: String,
    pub endpoint_url: String,
    pub public_key: Option<String>, // hex-encoded Ed25519, set once identity is keyed
    pub protocol_version: String,
}
```

Already 80% of the A2A Agent Card shape. `#81b` adds a `signature` field and a `discover()` / `resolve(agent_id)` surface.

**`ReputationTracker`** — windowed scoring from trade outcomes:

```rust
tracker.record(agent_id, TransactionOutcome::Success)?;
tracker.score(agent_id)?;     // windowed reputation score
```

Currently in-memory. `#81c` persists events to the ledger via reference-linking — making scores reconstructable and auditable across restarts.

**`DisputeManager`** — dispute lifecycle:

```rust
manager.file(buyer_id, seller_id, reference, evidence)?;
manager.resolve(dispute_id, outcome)?;    // feeds ReputationTracker
manager.dismiss(dispute_id, reason)?;
manager.timeout_stale()?;
```

Dispute outcomes wire directly into reputation scoring and, post-`#81d`, into escrow refund decisions.

---

## Web4 Cut-Shape

Four cuts, sequenced by dependency. Each ships **off-by-default**; zero behavior change until an operator explicitly enables.

### #81a — Unify the Marketplace *(gate — ships first)*

**Problem:** two parallel marketplace implementations exist (`zeus-agora` + `zeus-marketplace`), with two separate ledgers. Web4 cannot be built on divergent settlement truth.

**Cut:** `zeus-agora` is the survivor. Port `zeus-marketplace`'s superior discovery/negotiation surface (`MarketplaceRegistry` search, `TradeProtocol` negotiation) into agora. Retire the standalone in-memory ledger. All callers settle through one `SettlementCoordinator`.

**Acceptance:** one ledger, one settlement path; agora gains search + negotiation; no orphaned `zeus-orchestra` ledger dep.

---

### #81b — Agent Identity & Discovery

**Problem:** `AgentIdentity` exists but isn't signed, verifiable, or discoverable.

**Cut:**
- Add `signature: String` to `AgentIdentity` — signed by the agent's own Ed25519 keypair via `WalletKeypair::sign_base64`
- `verify_card(card: &AgentIdentity) -> bool` — verify authenticity without trusting a central party
- `discover(capability: &str) -> Vec<AgentIdentity>` — capability-matched agent lookup over the unified registry
- Optionally back `agent_id` → card with `DomainRegistry` for human-readable names
- **OpenX402 facilitator integration** — register a `payTo` endpoint against `facilitator.openx402.ai` directly, giving discovery an existing network to join rather than one to build

```rust
// Publish a signed card
let card = AgentIdentity::signed(agent_id, display_name, endpoint, &keypair)?;
registry.publish(card)?;

// Verify a remote agent's card
registry.verify_card(&remote_card)?;

// Find agents by capability
let agents = registry.discover("data-analysis")?;
```

---

### #81c — Portable Reputation

**Problem:** reputation scores are in-memory and windowed — they don't survive restarts and can't be audited.

**Cut:**
- Persist reputation events to the ledger via `transactions_by_reference` linkage
- Scores are reconstructable from ledger history — no trusted score store needed
- Attestations are signed by the rater's Ed25519 key
- Dispute outcomes (`DisputeManager::resolve`) feed the persistent score automatically

```rust
// Record a signed attestation (persisted to ledger)
tracker.attest(rater_keypair, rated_agent_id, outcome, reference)?;

// Reconstruct score from ledger (survives restart)
let score = tracker.score_from_ledger(agent_id, &ledger)?;
```

---

### #81d — Escrow & Pricing

**Problem:** the settlement saga moves funds directly — no hold period, no clean refund path, no dynamic pricing.

**Cut:** insert an **escrow hold** between Phase-1 debit and Phase-3 credit. Funds parked until delivery confirmed; disputes trigger refund via `DisputeManager::resolve` → `TokenLedger::transfer` back.

```rust
// Escrow-enabled settlement (off-by-default gate at coordinator)
coord.settle_with_escrow(buyer, seller, amount, reference, timeout).await?;

// Delivery confirmed — release to seller
coord.release_escrow(escrow_id)?;

// Dispute filed — refund buyer
coord.refund_escrow(escrow_id, dispute_id)?;
```

Dynamic pricing knobs on `SkillListing`:

```rust
pub struct SkillListing {
    pub price_floor: u64,
    pub price_dynamic: Option<PricingStrategy>,  // None = fixed floor
    // ...existing fields...
}
```

**Off-by-default.** When disabled: identical to current direct saga behavior.

---

### #81e — Replication *(post-trust-layer)*

Surfaced by [web4.ai](https://web4.ai/) — their *"the automaton survives"* stage. Ships only after #81a–#81d, behind the strictest gate.

**Cut:** a parent agent funds a child with a directive off its own ledger.

```rust
// Mint a child agent with a capped budget
parent.spawn_funded_child(
    mission: "analyze Polymarket spreads",
    budget: 20,   // hard cap — parent cannot drain ledger minting children
)?;
```

Composed entirely from #81a–#81d primitives: `AgentIdentity` (mint child), `TokenLedger::transfer` (fund), escrow discipline (money-ordering). No new settlement primitive.

---

## Guardrails

These apply across every Web4 cut and are non-negotiable.

1. **One settlement path.** Every money-moving operation routes through `SettlementCoordinator` + `zeus-economy::TokenLedger`. No second ledger. Ever.

2. **Off-by-default.** Escrow, dynamic pricing, networked discovery, and replication ship disabled. Enabling requires an explicit operator flip. Zero behavior change until then.

3. **Persona-latch wins.** Reputation and pricing signals are advisory. They never override correctness, safety, privacy, or permissions.

4. **Verifiable, not trusted.** Identity and reputation lean on the Ed25519 keypair. Claims are *signed and checkable*, not asserted. That's the Web4 trust model in one line.

5. **Budget caps on replication.** A parent must not be able to drain its ledger minting children. Same discipline as commitments' 3/day cap.

---

## Deployment Notes

The economy stack is file-backed and requires no external services beyond what Zeus already runs:

| Component | Storage | Default path |
|-----------|---------|-------------|
| `TokenLedger` | SQLite | `~/.zeus/economy.db` |
| `WalletKeypair` | AES-GCM file | `~/.zeus/wallet.key` |
| Payment receipts | SQLite / filesystem | `~/.zeus/receipts/` |
| Domain registry | SQLite (same db) | `~/.zeus/economy.db` |

**x402 payments** require network access to the seller's endpoint and (post-#81b) the OpenX402 facilitator at `facilitator.openx402.ai`. No stablecoin wallet or on-chain setup needed for the current cut — x402 handles the payment rail entirely at the HTTP layer.

---

## TL;DR — What This Means in Practice

- **Built on open standards** — x402 M2M payments, SIWE auth, A2A Agent Cards. No proprietary payment rails to lock you in.
- **No stablecoin onboarding** — runs over standard HTTP. No wallet setup, no chain config, no gas management.
- **Fully self-hosted** — deploy on your own infrastructure, own your settlement layer.
- **Modular by design** — adopt the primitives you need, ignore the rest.

---

## Who Benefits

| Audience | Primary Benefit |
|---|---|
| **AI infrastructure builders** | Standardized M2M payments without reinventing payment plumbing |
| **DAO treasuries** | Programmable agent spend with on-chain audit trail and dispute resolution |
| **Enterprise AI teams** | Agent-to-agent commerce with cryptographic identity and finality guarantees |
| **Developer teams** | Cut-shape is modular — drop in `zeus-wallet` only, or the full stack |

---

## Zeus vs. The Field

| | Zeus Titan Economy | Typical Agent Frameworks | On-chain M2M |
|---|---|---|---|
| **Agent identity** | Ed25519 cryptographic keys | OAuth / API keys | EOA wallets |
| **Payment rail** | x402 (HTTP-native) | Stripe / webhooks | Gas + smart contracts |
| **Settlement model** | Two-phase saga | Async webhooks | One-shot on-chain |
| **Infrastructure sovereignty** | Fully self-hosted | Cloud-dependent | Rollup-dependent |
| **Standard interop** | A2A Agent Cards (Google) | Proprietary | ERC-20 / ERC-721 |

---

## Current Feature Readiness

| Component | Status |
|---|---|
| `zeus-economy` — TokenLedger, AgentWallet | ✅ Live |
| `zeus-wallet` — x402 signing, W2W, async mint | ✅ Live |
| `zeus-agora` — SettlementCoordinator, AgentIdentity, Reputation, DisputeManager | ✅ Live |
| A2A Agent Card discovery (`#81b`) | 🔜 Next cut |
| ERC-8004 on-chain registry (`#81c`) | 🔜 Future roadmap |
| Autonomous self-spawning with self-funding (`#81e`) | 🔜 Future (strictly gated) |

> **Deployment note:** x402 facilitation currently routes through `facilitator.openx402.ai`. Production deployments should evaluate self-hosting the facilitator or joining the open facilitator network based on trust model and latency requirements.

---

## Quick Start

```bash
# Build the core crates
cargo build -p zeus-economy -p zeus-wallet -p zeus-agora

# Run the test suite
cargo test -p zeus-economy -p zeus-agora

# Full spec and cut-shape rationale
cat docs/design/web4-titan-economy.md
```

---

## Reference

- **Spec:** `docs/design/web4-titan-economy.md` — zeus-spark's full substrate walk + cut-shape + web4.ai mapping
- **web4.ai / Conway** — live reference implementation: wallet + x402 + SIWE + heartbeat-self-rewrite maps 1:1 to our cut-shape
- **x402 protocol:** [x402agentic.ai](https://x402agentic.ai) — the M2M HTTP payment standard
- **A2A Agent Cards:** Google's agent interop protocol — the target shape for `#81b`
- **ERC-8004:** on-chain agent reputation registry — the target shape for `#81c`
- **OpenX402 facilitator:** `facilitator.openx402.ai` — the discovery network `#81b` joins
