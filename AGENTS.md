# AGENTS.md

## Iron Laws
1. Test-Driven Development — no production code without a failing test first.
2. Verification Before Completion — run the actual command, read the output.
3. Systematic Debugging — no fixes without root cause investigation.
4. Deliver, Don't Reference — "post it on Discord" means attach the file via `send_file`, not post a path. File delivery ≠ file reference.
5. Fresh Backlog Rule — `git log origin/main` before publishing any backlog. Backlogs drift faster than assumptions.
6. Small pushes > monolithic pushes — phase = one commit + push. Cooks can hit the 1800s ceiling mid-iteration; phased pushes leave checkpoints on origin so the next wake picks up from a known-good state instead of restarting from zero. When a sprint has multiple sub-cooks, push after each sub completes, not at the end.

## How You Work
Execute tasks immediately. Ship working code in small commits.
Report progress as you work — never go silent.

## Engineering Standards — bounce-classes that cost us lands
1. `--lib` PASS ≠ suite PASS. Gate with FULL `cargo test -p <crate>` — `--lib` skips every `tests/*.rs` integration file.
2. Field added to a cross-crate struct/enum → `cargo build --workspace --locked`, never just `-p <crate>`. Dependents don't rebuild on `-p`.
3. Land with `--locked`. Dep adds commit Cargo.lock minimally (diff = lock only). Never regenerate the lock.
4. Clippy verdicts compare zero-new by warning TYPE + FILE, not file:line — line shifts create false positives. New files ship with zero warnings.
5. A ship = an origin SHA verified via `git ls-remote`. "Done", pasted code, or "branch ready" without a SHA is not shipped.
6. Diagnose against `git show origin/main:<path>` — local worktrees are chronically stale. A "missing/empty/regressed" finding from a bare local grep is invalid.
7. Auth, schema, or persistence surface → design note in-channel FIRST, build only on the explicit GO.
8. Code never goes in chat. Channel traffic = design notes and ship reports.
9. Build + clippy locally before every ship report; seat-local clippy is lenient, so compare by type against main.
10. Sweeps/cleanups: re-grep the whole target each round. "The files I looked at" ≠ done.
11. A tool "missing from my list" ≠ absent. Verify via the registry (`zeus tool <name>`) — a model's self-report of its own function list is not evidence; stale binary or stale long-lived process is the usual cause (binary swap ≠ process restart).
12. Verify-before-claim applies to yourself: never cite reports, approvals, or messages you cannot point to by message ID. Two phantom claims in a session = reset before continuing.
13. Presence checks grep case-insensitively; a zero-match means "my pattern didn't match", not "absent".
14. macOS has no `timeout` and >60s foreground calls die — background + poll + pkill.
