#!/usr/bin/env bash
#
# deploy-poll.sh — B1 deploy-on-merge trigger for Zeus seats.
#
# Compares remote origin/main against the last deployed SHA in
# $ZEUS_HOME/deploy/last-deploy. If main moved, runs deploy-on-merge.sh.
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="${ZEUS_REPO:-$(cd "$SCRIPT_DIR/../.." && pwd)}"
BRANCH="${ZEUS_DEPLOY_BRANCH:-main}"
ZEUS_HOME="${ZEUS_HOME:-$HOME/.zeus}"
STATE_DIR="${ZEUS_DEPLOY_STATE_DIR:-$ZEUS_HOME/deploy}"
STAMP="$STATE_DIR/last-deploy"
TELEMETRY="$SCRIPT_DIR/fleet-telemetry.sh"
DEPLOY="$SCRIPT_DIR/deploy-on-merge.sh"

record_failure() {
    if [ -x "$TELEMETRY" ]; then
        "$TELEMETRY" record \
            --kind deploy_failure \
            --severity error \
            --source deploy-poll \
            --summary "$1" \
            --details "repo=$REPO branch=$BRANCH" >/dev/null 2>&1 || true
    fi
}

fail() { echo "[zeus-deploy-poll] FAIL: $*" >&2; record_failure "$*"; exit 1; }

[ -d "$REPO/.git" ] || fail "not a git checkout: $REPO"
[ -x "$DEPLOY" ] || fail "deploy script not executable: $DEPLOY"

REMOTE_FULL="$(git -C "$REPO" ls-remote "$(git -C "$REPO" remote get-url origin)" "refs/heads/$BRANCH" | awk '{print $1}')"
REMOTE="${REMOTE_FULL:0:8}"
[ -n "$REMOTE" ] || fail "could not resolve origin/$BRANCH"

LAST=""
[ -f "$STAMP" ] && LAST="$(awk -F= '/^sha=/{print $2; exit}' "$STAMP")"

if [ "$REMOTE" = "$LAST" ]; then
    exit 0
fi

echo "[zeus-deploy-poll] origin/$BRANCH moved: ${LAST:-none} -> $REMOTE"
exec "$DEPLOY"
