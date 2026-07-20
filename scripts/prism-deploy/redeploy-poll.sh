#!/usr/bin/env bash
#
# redeploy-poll.sh — B1 trigger for deploy-on-merge.
#
# Compares the remote origin/main SHA against the last-deployed SHA in
# .last-deploy. If they differ (a merge landed), runs redeploy.sh. Otherwise
# exits quietly. Driven by redeploy.timer (default: every 60s). Zero GitHub
# config — the whole trigger lives on the box.
#
set -euo pipefail

REPO="${PRISM_REPO:-/home/mike/oracles}"
STAMP="$REPO/.last-deploy"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

export PATH="$HOME/.cargo/bin:$PATH"

# Remote SHA of main (short, to match .last-deploy).
REMOTE_FULL="$(git ls-remote "$(git -C "$REPO" remote get-url origin)" refs/heads/main | awk '{print $1}')"
REMOTE="${REMOTE_FULL:0:7}"
[ -n "$REMOTE" ] || { echo "[poll] could not resolve remote main SHA" >&2; exit 1; }

# Last deployed SHA (empty on first run => force deploy).
LAST=""
[ -f "$STAMP" ] && LAST="$(awk -F= '/^sha=/{print $2}' "$STAMP")"

if [ "$REMOTE" = "$LAST" ]; then
  # Up to date — nothing to do.
  exit 0
fi

echo "[poll] main moved: $LAST -> $REMOTE, redeploying"
exec "$SCRIPT_DIR/redeploy.sh"
