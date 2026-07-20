# Zeus deploy-on-merge

Port of the PRISM deploy-on-merge pattern to Zeus seats. The goal is to close the `merged != live` failure class without handing rollout control to GitHub Actions or an external service.

## Invariants

`deploy-on-merge.sh` is intentionally fail-loud:

1. sync `origin/main`;
2. assert the repo is clean;
3. build `cargo build --release --locked --bin zeus` **before** touching the live binary;
4. assert the built binary's `zeus --version` contains the git SHA being deployed;
5. stop the native supervisor first;
6. sweep stray `zeus gateway`, `zeus serve`, and `zeus daemon` processes;
7. replace `/usr/local/bin/zeus` only after the build succeeded;
8. restart via the native supervisor — no silent `nohup` fallback;
9. assert the installed binary reports the deployed SHA;
10. assert gateway `/health` returns ok;
11. assert a #332 adapter-connect log line is present, unless explicitly disabled for a no-channel sandbox;
12. append success/failure telemetry to `$ZEUS_HOME/logs/fleet-failures.jsonl`.

Old binaries are not touched until the build and built-SHA assertion pass. A failed deploy leaves a JSONL event for morning-status rollup.

## Files

| File | Role |
|---|---|
| `deploy-on-merge.sh` | Main idempotent deploy script for Unix seats. |
| `deploy-poll.sh` | Compares remote `origin/main` to `$ZEUS_HOME/deploy/last-deploy`; runs deploy only when main moves. |
| `deploy-poll-loop.sh` | Simple loop wrapper for supervisors without timer semantics. |
| `deploy-poll.ps1` | Windows schtasks wrapper. It is fail-loud until native Windows deploy is implemented. |
| `fleet-telemetry.sh` | Append JSONL events and generate a Markdown rollup. |
| `test-sandbox.sh` | Hermetic `ZEUS_HOME` smoke test for deploy + telemetry behavior. |
| `units/systemd/*` | Linux poll timer/service templates. |
| `units/launchd/*` | macOS StartInterval poll template. |
| `units/freebsd/zeus_deploy_poll` | FreeBSD rc.d daemonized poll loop template. |
| `units/windows/zeus-deploy-poll.xml` | Windows Task Scheduler template. |
| `enable-deploy-on-merge.sh` | One-command install/enable/disable/status for the poll unit (#431). |

## Rollout model

The operator installs the right poll unit per seat and edits paths/users before enabling. Default template paths use `/home/zeus`, `/Users/zeus`, or `C:\Users\zeus`; do not assume those match a live seat.

Suggested cadence: 60s poll. Rollout remains operator-controlled: enable one seat, verify, then widen.

### One-command enable (#431)

`enable-deploy-on-merge.sh` detects the OS, renders the matching unit template
(substituting `__ZEUS_REPO__`/`__ZEUS_HOME__`/`__ZEUS_USER_HOME__` for the
current checkout and `$ZEUS_HOME`), and installs + enables it through the
platform's native service manager — no unsupervised nohup fallback, per the
#333 supervised-restart invariant:

```sh
# macOS: installs ~/Library/LaunchAgents/com.zeus.deploy-poll.plist via launchctl bootstrap
# Linux: installs a systemd --user timer+service under ~/.config/systemd/user
# FreeBSD: installs /usr/local/etc/rc.d/zeus_deploy_poll and flips zeus_deploy_poll_enable=YES via rc.conf/sysrc
scripts/zeus-deploy/enable-deploy-on-merge.sh            # install + enable
scripts/zeus-deploy/enable-deploy-on-merge.sh --status    # report enabled/disabled + last deployed sha
scripts/zeus-deploy/enable-deploy-on-merge.sh --disable   # stop + uninstall the unit
```

Windows has no native unit here yet — `deploy-poll.ps1` stays fail-loud (see
below) until a native Windows deploy path exists, so `enable-deploy-on-merge.sh`
does not attempt to install anything on Windows.

Enabling is still an operator (or explicitly-delegated) action, not something
a seat does to itself — this script only makes that action one command instead
of a multi-step manual install, per seat, per OS.

`zeus doctor` now reports deploy-on-merge state as part of its standard check
list (`Deploy-on-merge: enabled/disabled — last deployed sha=...`), so a
seat quietly drifting stale — the #390 empty-`deploy.db` failure class — shows
up in routine diagnostics instead of only being caught by a manual SHA audit.

## Useful environment

| Variable | Default | Purpose |
|---|---|---|
| `ZEUS_REPO` | repo containing this script | checkout to deploy |
| `ZEUS_HOME` | `$HOME/.zeus` | seat state/log directory |
| `ZEUS_DEPLOY_BRANCH` | `main` | branch to track |
| `ZEUS_DEPLOY_INSTALL_BIN` | `/usr/local/bin/zeus` | live binary path |
| `ZEUS_DEPLOY_HEALTH_URL` | `http://127.0.0.1:8080/health` | health assertion URL |
| `ZEUS_DEPLOY_REQUIRE_ADAPTER_CONNECT` | `1` | require adapter-connect evidence |
| `ZEUS_DEPLOY_SANDBOX` | `0` | install under `ZEUS_HOME/bin` and skip supervisor ops |

## Telemetry

`fleet-telemetry.sh record` appends normalized JSONL to `$ZEUS_HOME/logs/fleet-failures.jsonl`.

`fleet-telemetry.sh rollup` writes `$ZEUS_HOME/logs/fleet-failures-rollup.md` grouped by seat, kind, severity, and latest warn/error examples.

This is deliberately local-first: Discord or another telemetry channel can mirror alerts later, but local logs remain the evidence record.

## Windows note

The schtasks XML is included so the matrix has an explicit Windows poll-unit shape. `deploy-poll.ps1` currently records a warning/error and exits non-zero when main moves; it does **not** perform a native Windows binary swap. That keeps Windows fail-loud instead of pretending the Unix deploy path is safe there.
