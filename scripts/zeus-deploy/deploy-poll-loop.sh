#!/usr/bin/env bash
#
# deploy-poll-loop.sh — simple poll loop for supervisors without native timers.
# FreeBSD rc.d and Windows scheduled-task wrappers can run this safely; systemd
# and launchd prefer their native timer/StartInterval units.
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INTERVAL="${ZEUS_DEPLOY_POLL_INTERVAL:-60}"
POLL="$SCRIPT_DIR/deploy-poll.sh"

[ -x "$POLL" ] || { echo "deploy-poll.sh not executable: $POLL" >&2; exit 1; }

while :; do
    "$POLL" || true
    sleep "$INTERVAL"
done
