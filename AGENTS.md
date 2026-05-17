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
