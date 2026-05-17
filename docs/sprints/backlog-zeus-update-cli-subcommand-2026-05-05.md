# Backlog — `zeus update` CLI Subcommand

**Status:** BACKLOG (sprint candidate, not launch-blocking)
**Origin:** merakizzz directive 2026-05-05 — "zeus update would be good to have later on, backlog it"

## Problem

The canonical update flow today is `./scripts/install.sh --update`. It works, but:

- Requires being in the repo directory or knowing the script path
- Requires the install.sh shell script to be present (not always the case post-installer-removal)
- Doesn't compose well from automation (operators have to remember the path)
- Two-name convention (binary + script) is unnecessary cognitive load when `zeus update` would feel native

## Scope

Add a top-level `zeus update` subcommand to the CLI in `src/main.rs` (the `Commands` enum).

### Behavior — match `scripts/install.sh --update` (lines 690-737):

1. **Resolve repo root** — either `--repo PATH` flag, current working directory if `.git` is present, or `~/Zeus` per fleet convention.
2. **Build** — run `cargo build --release` in the repo root (skippable via `--no-build` if binary already fresh).
3. **Verify binary exists** — `target/release/zeus` must exist after build (or error out cleanly).
4. **Stop gateway** — `pkill -f 'zeus gateway'` + 2s sleep.
5. **Install binary** — `sudo cp target/release/zeus /usr/local/bin/zeus`.
6. **Codesign on macOS** — same `sudo codesign --force --sign "${ZEUS_CODESIGN_IDENTITY:--}" ...` flow as install.sh.
7. **Restart gateway via platform service manager:**
   - macOS → launchd bootstrap into system domain (use the same `install_launchd_plist` helper logic — porting it into Rust as `crates/zeus-cli/src/update.rs::install_launchd_plist()`)
   - FreeBSD → `sudo service zeus_gateway restart`
   - Linux → `sudo systemctl restart zeus` if unit exists, else nohup fallback
8. **Health check** — curl `localhost:8080/health` for 3s grace, OK or warn.

### Flags

```bash
zeus update                      # default — rebuild + install + restart
zeus update --no-build           # skip cargo build (binary must already exist)
zeus update --no-restart         # install binary but don't restart (for staged rollouts)
zeus update --with-identity      # also re-stamp workspace AGENTS.md / SOUL.md per deploy-identity.sh
zeus update --from-git           # git pull origin main before build
zeus update --repo PATH          # explicit repo path (default: cwd or ~/Zeus)
zeus update --health-timeout 10  # health check grace window in seconds
```

### Output format

Same colored section headers as install.sh (`phase()`, `ok()`, `warn()`, `info()` patterns) — keeps the operator-facing UX consistent.

### Cross-cutting concerns

- **Permission**: `sudo cp` and `sudo codesign` need sudo — should detect non-sudo run + prompt for sudo elevation gracefully OR fail clean with a clear "rerun with sudo" message.
- **Concurrency**: if another `zeus gateway` is mid-restart, the pkill + sleep handles it. Add a config-guard hook? Optional — punt to follow-up.
- **Rollback**: keep the previous binary at `/usr/local/bin/zeus.prev` before `sudo cp` so `zeus update --rollback` (future flag) can restore.

## Files

- `src/main.rs` — add `Update { ... }` variant to the `Commands` enum + dispatch handler
- `crates/zeus-cli/src/update.rs` (new file or extension of existing CLI module) — porting the install.sh `--update` block as a Rust module
- Tests: integration tests are awkward for service-manager interactions; unit tests on the per-step helpers (build invocation, codesign call, health-check decoder).

## Estimate

~6-8h for a clean port of install.sh's --update logic into Rust, with platform-detection helpers + health-check + flag parsing. Most of the work is mirroring shell into Rust + adding the platform branches.

## Suggested owner

**fbsd2** — operator persona, has done channel parity + comfortable with the multi-platform service-manager surface (launchd / rc.d / systemd).

Or **zeus106** — would be a solid second; familiar with config / `Config::save()` / typed-section discipline that this update path needs to preserve.

## Branch convention

`feat/zeus-update-cli-subcommand` off `origin/main`.

## Verify-before-claim discipline

- After `zeus update` runs, the new binary at `/usr/local/bin/zeus` must report a different `--version` SHA from before (compare pre/post).
- Health check at 8080/health must return `"ok"` post-restart.
- `pgrep -f 'zeus gateway'` must show a fresh PID (not the pre-restart one).

## Out of scope

- `zeus update --rollback` (future flag, separate dispatch)
- Auto-update on `zeus daemon start` (tempting but adds boot-time complexity)
- Update-from-binary-release (no source build) — would need a release-tarball pipeline first
- Self-updating from GitHub releases (security implications — defer to a properly-signed release pipeline)

## Reference — current operator one-liner

merakizzz's canonical update flow as of 2026-05-05:

```bash
cd ~/Zeus && git checkout main && git pull && scripts/install.sh --update --with-identity && zeus daemon restart
```

Five operations:
1. `cd ~/Zeus` — canonical fleet repo path (per-Mac convention)
2. `git checkout main` — ensure on main branch
3. `git pull` — fetch + fast-forward
4. `scripts/install.sh --update --with-identity` — rebuild + install binary + codesign + restart gateway via platform service manager + refresh `~/.zeus/workspace/AGENTS.md` etc.
5. `zeus daemon restart` — belt-and-suspenders explicit restart (--update already restarts via launchd/systemd, but this gives a clean foreground restart for any state still hanging)

The eventual `zeus update` subcommand should subsume steps 3-5 by default (with `--from-git` flag for step 3). Step 1 (cd) and step 2 (checkout main) are operator pre-flight; the new command runs from any directory and doesn't need an explicit checkout if the repo is at the canonical `~/Zeus` path.

Equivalent eventual one-liner: `zeus update --from-git --with-identity` (one command vs five).
