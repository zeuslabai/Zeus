# Dependency Pinning & the Telegram (grammers) Chain

> **TL;DR — always build with `cargo build --workspace --locked`.**
> Never run `cargo update` against the Telegram crypto chain. The committed
> `Cargo.lock` plus a vendored crate are the *only* thing keeping the
> workspace buildable, because upstream crates in this chain have been
> **yanked from crates.io**.

This page documents issue **#197**: why the Telegram (grammers) dependency
chain is fragile, exactly which versions are pinned, the vendoring workaround
in place, and why `--locked` is mandatory rather than optional.

## Why this chain is fragile

Telegram support (`zeus-channels`) pulls in the [`grammers`] family of crates.
Its crypto layer (`grammers-crypto`) depends on [`glass_pumpkin`], a
cryptographically-secure prime generator. Two yanks broke the normal
resolution path:

1. **`glass_pumpkin 1.10.0` was yanked from crates.io.** A yanked version
   still resolves *if* it's already named in a committed `Cargo.lock`, but it
   can never be freshly selected by the resolver.
2. **Every older `glass_pumpkin` release depends on `core2 ^0.4`, which is
   *also* fully yanked.** So there is no resolvable registry version of
   `glass_pumpkin` at all — not 1.10.0, not anything older.

Net effect: any `cargo update` that touches this sub-tree fails outright, and
a fresh checkout that ignores the lock (or regenerates it) cannot resolve a
valid graph. The committed `Cargo.lock` was, for a while, the single point of
failure holding builds together.

## The fix in place: vendoring

To remove the lock-as-crutch fragility, `glass_pumpkin 1.10.0` is **vendored**
into the repo and wired in via `[patch.crates-io]` in the workspace
`Cargo.toml`:

```toml
[patch.crates-io]
# ZEUS PATCH (#197): vendored glass_pumpkin 1.10.0 (Apache-2.0), used by the
# grammers (Telegram) crypto chain. Upstream yanked 1.10.0 from crates.io and
# every older release depends on core2 ^0.4 — which is ALSO fully yanked — so
# no resolvable registry version of glass_pumpkin exists at all.
glass_pumpkin = { path = "vendor/glass_pumpkin" }
```

The vendored source lives in [`vendor/glass_pumpkin/`](../../../vendor/glass_pumpkin)
(Apache-2.0 licensed — `LICENSE` and `MAINTAINERS.md` retained). It is the
byte-for-byte 1.10.0 release, so behaviour is identical to the yanked crate;
vendoring only changes *where* Cargo fetches it from.

> A second, unrelated vendored patch (`notify`) sits in the same
> `[patch.crates-io]` block — see its inline comment. It is not part of #197.

## Pinned versions

These are the exact versions the committed `Cargo.lock` resolves. Treat them
as load-bearing — do not bump them piecemeal.

| Crate                 | Pinned version | Notes                                              |
| --------------------- | -------------- | -------------------------------------------------- |
| `glass_pumpkin`       | **1.10.0**     | **Vendored** (`vendor/glass_pumpkin`). Yanked upstream. |
| `grammers-client`     | 0.6.0          | Declared `grammers-client = "0.6"` in `Cargo.toml`. |
| `grammers-session`    | 0.5.2          | Declared `grammers-session = "0.5"`.               |
| `grammers-crypto`     | 0.6.1          | Pulls `glass_pumpkin`.                              |
| `grammers-mtproto`    | 0.6.0          |                                                    |
| `grammers-mtsender`   | 0.5.1          |                                                    |
| `grammers-tl-gen`     | 0.6.0          |                                                    |
| `grammers-tl-parser`  | 1.2.1          |                                                    |
| `grammers-tl-types`   | 0.6.0          |                                                    |
| `getrandom` (for g_p) | 0.4.2          | Transitive, via vendored `glass_pumpkin`.          |

## Why `--locked` is the only safe path

- **`--locked` fails the build if `Cargo.lock` would change.** That is exactly
  the guard we want: it guarantees you are resolving the known-good graph
  above and not silently drifting onto a version the registry can no longer
  serve.
- **Without `--locked`, Cargo is free to re-resolve.** If anything nudges the
  resolver toward re-selecting `glass_pumpkin` (or its yanked `core2`
  dependency) from the registry, resolution fails — there is no live version
  to land on. The vendored `[patch]` covers `glass_pumpkin` itself, but
  `--locked` is what keeps the *rest* of the graph from drifting.
- **Do NOT regenerate the lock.** Running `cargo update` (or deleting and
  rebuilding `Cargo.lock`) against this chain will fail or produce an
  unbuildable graph. If you must update an unrelated dependency, update that
  single package explicitly (`cargo update -p <crate> --precise <ver>`) and
  confirm the grammers/`glass_pumpkin` entries are untouched.

### Canonical build / test / lint commands

```bash
cargo build --workspace --locked
cargo test  -p <crate>      --locked
cargo clippy --workspace    --locked
```

## When can this be removed?

Drop the vendored `glass_pumpkin` patch (and this caveat) once
`grammers-crypto` moves to a **live, non-yanked `glass_pumpkin` release
(2.0+)** that no longer depends on the yanked `core2 ^0.4`. At that point the
registry can resolve the chain normally and `[patch.crates-io]` for
`glass_pumpkin` can be deleted.
