# ZeusWeb — Tailwind CSS Migration Guide

**Status:** Prototype landed (`d14d86a` on `feat/zeusweb-tailwind-migration`, off main `cd0899f`)
**Owner of migration:** team (coord-led)
**Owner of deployment:** @QTUMFULLSTACK
**Frontend stack:** Leptos 0.8.16 (CSR) + Trunk/WASM — **no framework change**, CSS layer only.

---

## 0. Why / Goal

Replace the hand-rolled 36KB `input.css` (`.z-*` design system) with **Tailwind CSS v4 utilities**.
Outcome: structured, purge-driven CSS instead of a monolithic stylesheet. The Leptos/Trunk/WASM
architecture stays exactly as-is — this is a **CSS-system swap, not a frontend rewrite**.

---

## 1. Toolchain (FreeBSD — all binaries now installed)

| Binary | Version | Source | Role |
|--------|---------|--------|------|
| `trunk` | 0.21.14 | `pkg install trunk` | Rust WASM bundler (build/serve) |
| `node`  | 20.x    | `pkg install node20` | Runtime for Tailwind CLI |
| `npm`   | bundled | with node | Pulls `@tailwindcss/cli` |
| `@tailwindcss/cli` | v4.x | `npx` (in `node_modules`) | Compiles utilities from `.rs` view! macros |

> **FreeBSD note #1 (Tailwind):** the Tailwind *standalone* Linux binary aborts under
> linuxulator. Native `node20` + `npx @tailwindcss/cli` is the working path. Verified.
>
> **FreeBSD note #2 (wasm-bindgen) — INFRA BLOCKER:** `trunk build` tries to auto-download
> the `wasm-bindgen` release archive and fails with `unsupported OS` on FreeBSD. The Rust
> compile itself is GREEN (verified, 2m58s) — only the wasm-bindgen download step fails.
> **Fix:** install the CLI natively at the **exact** version the project pins (`Cargo.lock`):
> ```sh
> cargo install wasm-bindgen-cli --version 0.2.122   # match Cargo.lock exactly
> ```
> pkg ships `wasm-bindgen-cli-0.2.120` — **version mismatch**, trunk requires exact match,
> so `cargo install --version <locked>` is the route. `wasm32-unknown-unknown` target is
> already installed. Once the local CLI matches, trunk uses it instead of downloading.

---

## 2. How the build wires together

`Trunk.toml` runs Tailwind as a **pre_build hook** before the WASM compile:

```toml
[[hooks]]
stage = "pre_build"
command = "npx"
command_arguments = ["@tailwindcss/cli", "-i", "tailwind.css", "-o", "dist/tailwind.css", "--minify"]
```

Flow:
1. `trunk build` fires the pre_build hook.
2. Tailwind scans `@source "./src/**/*.rs"` — reads `class="..."` strings **inside Leptos `view!` macros**.
3. Emits minified `dist/tailwind.css` containing only used utilities (purge).
4. Trunk compiles WASM and bundles `dist/tailwind.css` into the page.

`index.html` links the generated file (NOT the legacy `input.css`) once migration completes.

---

## 3. Design-system port (`tailwind.css` `@theme`)

The entire `:root` design system from `input.css` is ported into Tailwind v4 `@theme` variables,
so `.z-*` semantics survive as utilities:

| Legacy CSS var | Tailwind theme var | Utility |
|----------------|-------------------|---------|
| `--z-surface`  | `--color-z-surface` | `bg-z-surface` |
| `--z-border-active` | `--color-z-border-active` | `border-z-border-active` |
| `--z-accent` (`#ff3c14`) | `--color-z-accent` | `text-z-accent`, `bg-z-accent` |
| Orbitron font  | `--font-orbitron` | `font-orbitron` |
| Rajdhani font  | `--font-rajdhani` | `font-rajdhani` |

Full ember theme palette (z-bg / z-surface / z-border / z-text / z-green/yellow/red/blue) is in
`tailwind.css` `@theme` block. Build verified GREEN (12.8KB output, 95ms).

---

## 4. Migration pattern (proof-of-pattern landed)

Migrate **one `.z-*` class cluster at a time**. Reference cut: onboarding Name-input cluster.

**Before:**
```rust
view! { <label class="z-orb-field-label" style="margin-bottom:8px">"Name"</label> }
```

**After:**
```rust
view! { <label class="font-rajdhani text-z-text-dim text-sm mb-2">"Name"</label> }
```

### Per-class checklist
1. Grep usage: `git grep 'z-CLASSNAME' apps/ZeusWeb/src/`
2. Map the CSS rule → equivalent Tailwind utilities (use the `@theme` vars above).
3. Replace `class=` strings in each `view!` macro. Fold inline `style=` into utilities too.
4. Delete the `.z-CLASSNAME` rule from `input.css`.
5. `trunk build` → confirm GREEN + visual parity.
6. Repeat for next cluster.

---

## 5. Substrate flags (honest scope)

- **Surface is bigger than 47 `.z-*` classes.** Codebase mixes `.z-*` classes **+** inline `style=` attrs **+** legacy `onboarding-*` classes. All three must purge for a clean cut.
- `.z-*` classes are used **~153×** across **79** `.rs` source files — incremental, not big-bang.
- Full `trunk build` WASM compile not yet run at prototype time — **next gate** (run now that trunk is installed).

---

## 6. Verify gates (before any merge to main)

1. `cd apps/ZeusWeb && trunk build` → must be GREEN (Rust-side + Tailwind-side).
2. Visual parity check per migrated cluster (before/after).
3. Confirm `dist/tailwind.css` contains expected utilities (purge correct).
4. **3-seat ratify** (PRIMARY cut-titan + COORD + SECONDARY cross-clone) before ff-push to `origin/main`.

---

## 7. Ownership split

| Area | Owner |
|------|-------|
| Migration cuts (`.z-*` → utilities) | team, coord-dispatched + ratify-chained |
| `trunk build` / WASM verify | cut-titan PRIMARY self-verify |
| 3-seat ratify + ff-push to main | COORD (.100) drives mechanical push |
| **Deployment** (host/service/health) | **@QTUMFULLSTACK** |

Deployment target details (host, user, path, service, health endpoint) live with @QTUMFULLSTACK.
This doc stops at the merge-to-main gate; @QTUMFULLSTACK owns everything after.
