#!/usr/bin/env bash
#
# deploy-on-merge.sh — Zeus seat deploy-on-merge, ported from PRISM.
#
# Invariant: build from origin/main BEFORE touching the live binary. Only after
# the release build and built-binary SHA assertion pass do we stop the supervised
# gateway, replace /usr/local/bin/zeus, restart through the platform supervisor,
# and run fail-loud post-deploy assertions.
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="${ZEUS_REPO:-$(cd "$SCRIPT_DIR/../.." && pwd)}"
BRANCH="${ZEUS_DEPLOY_BRANCH:-main}"
ZEUS_HOME="${ZEUS_HOME:-$HOME/.zeus}"
LOG_DIR="${ZEUS_LOG_DIR:-$ZEUS_HOME/logs}"
STATE_DIR="${ZEUS_DEPLOY_STATE_DIR:-$ZEUS_HOME/deploy}"
HEALTH_URL="${ZEUS_DEPLOY_HEALTH_URL:-http://127.0.0.1:8080/health}"
INSTALL_BIN="${ZEUS_DEPLOY_INSTALL_BIN:-/usr/local/bin/zeus}"
BUILT_BIN="${ZEUS_DEPLOY_BUILT_BIN:-$REPO/target/release/zeus}"
TELEMETRY="$SCRIPT_DIR/fleet-telemetry.sh"
STAMP="$STATE_DIR/last-deploy"
SANDBOX="${ZEUS_DEPLOY_SANDBOX:-0}"
NO_FETCH="${ZEUS_DEPLOY_NO_FETCH:-0}"
NO_BUILD="${ZEUS_DEPLOY_NO_BUILD:-0}"
NO_RESTART="${ZEUS_DEPLOY_NO_RESTART:-0}"
ALLOW_DIRTY="${ZEUS_DEPLOY_ALLOW_DIRTY:-0}"
REQUIRE_ADAPTER_CONNECT="${ZEUS_DEPLOY_REQUIRE_ADAPTER_CONNECT:-1}"
SERVICE_MODE=""
DEPLOY_SHA=""

if [ "$SANDBOX" = "1" ]; then
    INSTALL_BIN="${ZEUS_DEPLOY_INSTALL_BIN:-$ZEUS_HOME/bin/zeus}"
    REQUIRE_ADAPTER_CONNECT="${ZEUS_DEPLOY_REQUIRE_ADAPTER_CONNECT:-1}"
fi

export ZEUS_HOME

log()  { printf '\033[1;34m[zeus-deploy]\033[0m %s\n' "$*"; }
ok()   { printf '\033[1;32m[  ok  ]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[ warn ]\033[0m %s\n' "$*" >&2; }

record_event() {
    local kind="$1" severity="$2" summary="$3" details="${4:-}"
    if [ -x "$TELEMETRY" ]; then
        "$TELEMETRY" record \
            --kind "$kind" \
            --severity "$severity" \
            --source deploy-on-merge \
            --summary "$summary" \
            --sha "$DEPLOY_SHA" \
            --details "$details" >/dev/null 2>&1 || true
    fi
}

fail() {
    local msg="$*"
    printf '\033[1;31m[ FAIL ]\033[0m %s\n' "$msg" >&2
    record_event deploy_failure error "$msg"
    exit 1
}

usage() {
    cat <<'USAGE'
Usage: deploy-on-merge.sh [--no-fetch] [--no-build] [--no-restart] [--sandbox] [--skip-adapter-connect]

Environment:
  ZEUS_REPO                         repo checkout (default: script/../..)
  ZEUS_HOME                         seat home (default: ~/.zeus)
  ZEUS_DEPLOY_BRANCH                branch to deploy (default: main)
  ZEUS_DEPLOY_INSTALL_BIN           live binary (default: /usr/local/bin/zeus; sandbox: $ZEUS_HOME/bin/zeus)
  ZEUS_DEPLOY_HEALTH_URL            health URL (default: http://127.0.0.1:8080/health)
  ZEUS_DEPLOY_REQUIRE_ADAPTER_CONNECT 1/0, default 1
  ZEUS_DEPLOY_SANDBOX               1 disables supervisor/sudo and installs under ZEUS_HOME
USAGE
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --no-fetch) NO_FETCH=1 ;;
        --no-build) NO_BUILD=1 ;;
        --no-restart) NO_RESTART=1 ;;
        --sandbox) SANDBOX=1; INSTALL_BIN="${ZEUS_DEPLOY_INSTALL_BIN:-$ZEUS_HOME/bin/zeus}" ;;
        --skip-adapter-connect) REQUIRE_ADAPTER_CONNECT=0 ;;
        -h|--help) usage; exit 0 ;;
        *) fail "unknown argument: $1" ;;
    esac
    shift
done

priv() {
    if [ "$SANDBOX" = "1" ] || [ "$(id -u)" -eq 0 ]; then
        "$@"
    else
        command -v sudo >/dev/null 2>&1 || fail "sudo not found and privileged install is required"
        sudo "$@"
    fi
}

repo_clean_or_fail() {
    if [ "$ALLOW_DIRTY" = "1" ]; then
        warn "ZEUS_DEPLOY_ALLOW_DIRTY=1: skipping repo cleanliness assertion"
        return
    fi
    git -C "$REPO" diff --quiet || fail "repo has unstaged changes; refusing deploy"
    git -C "$REPO" diff --cached --quiet || fail "repo has staged changes; refusing deploy"
    [ -z "$(git -C "$REPO" ls-files --others --exclude-standard)" ] || fail "repo has untracked files; refusing deploy"
}

sync_repo() {
    [ -d "$REPO/.git" ] || fail "ZEUS_REPO is not a git checkout: $REPO"
    repo_clean_or_fail

    if [ "$NO_FETCH" = "1" ]; then
        ok "Skipping fetch (--no-fetch) at $(git -C "$REPO" rev-parse --short=8 HEAD)"
        return
    fi

    log "Sync origin/$BRANCH"
    git -C "$REPO" fetch origin "$BRANCH"
    if git -C "$REPO" show-ref --verify --quiet "refs/heads/$BRANCH"; then
        git -C "$REPO" checkout -q "$BRANCH"
    else
        git -C "$REPO" checkout -q -B "$BRANCH" "origin/$BRANCH"
    fi
    git -C "$REPO" merge --ff-only "origin/$BRANCH"
    repo_clean_or_fail
}

build_release() {
    DEPLOY_SHA="$(git -C "$REPO" rev-parse --short=8 HEAD)"
    export DEPLOY_SHA
    mkdir -p "$LOG_DIR" "$STATE_DIR"

    if [ "$NO_BUILD" = "1" ]; then
        ok "Skipping cargo build (--no-build): $BUILT_BIN"
    else
        command -v cargo >/dev/null 2>&1 || { [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"; }
        command -v cargo >/dev/null 2>&1 || fail "cargo not found"
        log "Build release binary for $DEPLOY_SHA"
        (cd "$REPO" && cargo build --release --locked --bin zeus)
    fi

    [ -x "$BUILT_BIN" ] || fail "built binary missing or not executable: $BUILT_BIN"
    local built_ver
    built_ver="$($BUILT_BIN --version 2>/dev/null || true)"
    printf '%s' "$built_ver" | grep -F "$DEPLOY_SHA" >/dev/null \
        || fail "built zeus --version does not contain deployed SHA $DEPLOY_SHA (got: $built_ver)"
    ok "Built binary SHA assertion: $built_ver"
}

stop_supervisor() {
    if [ "$SANDBOX" = "1" ] || [ "$NO_RESTART" = "1" ]; then
        ok "Sandbox/no-restart: skipping supervisor stop"
        return
    fi

    case "$(uname -s)" in
        Darwin)
            SERVICE_MODE="launchd-system"
            priv launchctl bootout system /Library/LaunchDaemons/com.zeus.gateway.plist 2>/dev/null || true
            ;;
        FreeBSD)
            SERVICE_MODE="freebsd-rcd"
            priv service zeus_gateway stop 2>/dev/null || true
            ;;
        Linux)
            if systemctl --user list-unit-files zeus-gateway.service >/dev/null 2>&1; then
                SERVICE_MODE="systemd-user"
                systemctl --user stop zeus-gateway 2>/dev/null || true
            elif command -v systemctl >/dev/null 2>&1 && priv systemctl list-unit-files zeus-gateway.service >/dev/null 2>&1; then
                SERVICE_MODE="systemd-system"
                priv systemctl stop zeus-gateway 2>/dev/null || true
            else
                fail "no supervised zeus-gateway systemd unit found; refusing unsupervised deploy"
            fi
            ;;
        *)
            fail "unsupported OS for supervised deploy: $(uname -s)"
            ;;
    esac
}

sweep_strays() {
    if [ "$SANDBOX" = "1" ] || [ "$NO_RESTART" = "1" ]; then
        ok "Sandbox/no-restart: skipping zeus process sweep"
        return
    fi

    pkill -x zeus 2>/dev/null || true
    pkill -f '[z]eus gateway' 2>/dev/null || true
    pkill -f '[z]eus serve' 2>/dev/null || true
    pkill -f '[z]eus daemon' 2>/dev/null || true

    local i
    for i in 1 2 3 4 5; do
        if ! pgrep -x zeus >/dev/null 2>&1 \
            && ! pgrep -f '[z]eus gateway' >/dev/null 2>&1 \
            && ! pgrep -f '[z]eus serve' >/dev/null 2>&1 \
            && ! pgrep -f '[z]eus daemon' >/dev/null 2>&1; then
            ok "No stray zeus processes"
            return
        fi
        sleep 1
    done

    warn "zeus processes survived polite sweep; escalating to KILL"
    pkill -9 -x zeus 2>/dev/null || true
    pkill -9 -f '[z]eus gateway' 2>/dev/null || true
    pkill -9 -f '[z]eus serve' 2>/dev/null || true
    pkill -9 -f '[z]eus daemon' 2>/dev/null || true
}

install_binary() {
    log "Install $DEPLOY_SHA to $INSTALL_BIN"
    mkdir -p "$STATE_DIR"
    cp "$BUILT_BIN" "$STATE_DIR/zeus.$DEPLOY_SHA"
    chmod 0755 "$STATE_DIR/zeus.$DEPLOY_SHA"

    if [ "$SANDBOX" = "1" ]; then
        mkdir -p "$(dirname "$INSTALL_BIN")"
        rm -f "$INSTALL_BIN"
        cp "$STATE_DIR/zeus.$DEPLOY_SHA" "$INSTALL_BIN"
        chmod 0755 "$INSTALL_BIN"
    else
        priv mkdir -p "$(dirname "$INSTALL_BIN")"
        priv rm -f "$INSTALL_BIN"
        priv cp "$STATE_DIR/zeus.$DEPLOY_SHA" "$INSTALL_BIN"
        priv chmod 0755 "$INSTALL_BIN"
    fi

    local installed_ver
    installed_ver="$($INSTALL_BIN --version 2>/dev/null || true)"
    printf '%s' "$installed_ver" | grep -F "$DEPLOY_SHA" >/dev/null \
        || fail "installed zeus --version does not contain deployed SHA $DEPLOY_SHA (got: $installed_ver)"
    ok "Installed binary SHA assertion: $installed_ver"
}

restart_supervisor() {
    if [ "$NO_RESTART" = "1" ]; then
        ok "Skipping restart (--no-restart)"
        return
    fi
    if [ "$SANDBOX" = "1" ]; then
        ok "Sandbox: skipping supervisor restart"
        return
    fi

    local restart_log="$LOG_DIR/deploy-restart.log"
    case "$SERVICE_MODE" in
        launchd-system)
            [ -f /Library/LaunchDaemons/com.zeus.gateway.plist ] \
                || fail "missing launchd plist /Library/LaunchDaemons/com.zeus.gateway.plist"
            priv launchctl bootstrap system /Library/LaunchDaemons/com.zeus.gateway.plist 2>"$restart_log" || true
            priv launchctl kickstart -k system/com.zeus.gateway >>"$restart_log" 2>&1 \
                || fail "launchctl kickstart failed; see $restart_log"
            ;;
        freebsd-rcd)
            priv service zeus_gateway restart >"$restart_log" 2>&1 \
                || fail "service zeus_gateway restart failed; see $restart_log"
            ;;
        systemd-user)
            systemctl --user restart zeus-gateway >"$restart_log" 2>&1 \
                || fail "systemctl --user restart zeus-gateway failed; see $restart_log"
            ;;
        systemd-system)
            priv systemctl restart zeus-gateway >"$restart_log" 2>&1 \
                || fail "systemctl restart zeus-gateway failed; see $restart_log"
            ;;
        *)
            fail "unknown service mode '$SERVICE_MODE'"
            ;;
    esac
    ok "Restarted gateway via $SERVICE_MODE"
}

assert_health() {
    if [ "$NO_RESTART" = "1" ]; then
        ok "Skipping health assert (--no-restart)"
        return
    fi
    command -v curl >/dev/null 2>&1 || fail "curl not found for health assertion"

    local i body=""
    for i in 1 2 3 4 5 6 7 8 9 10; do
        body="$(curl -fsS --max-time 5 "$HEALTH_URL" 2>/dev/null || true)"
        if printf '%s' "$body" | grep -Eq '"status"[[:space:]]*:[[:space:]]*"ok"|\bok\b'; then
            ok "Gateway health assertion passed: $HEALTH_URL"
            return
        fi
        sleep 2
    done
    fail "gateway /health did not return ok at $HEALTH_URL (last body: $body)"
}

assert_adapter_connected() {
    if [ "$REQUIRE_ADAPTER_CONNECT" != "1" ] || [ "$NO_RESTART" = "1" ]; then
        ok "Skipping adapter-connect assertion"
        return
    fi

    local files=()
    [ -f "$LOG_DIR/gateway.log" ] && files+=("$LOG_DIR/gateway.log")
    [ -f "$LOG_DIR/gateway.out.log" ] && files+=("$LOG_DIR/gateway.out.log")
    [ -f "$LOG_DIR/gateway.err.log" ] && files+=("$LOG_DIR/gateway.err.log")
    [ -f /var/log/zeus_gateway.log ] && files+=(/var/log/zeus_gateway.log)

    if [ "${#files[@]}" -eq 0 ]; then
        fail "adapter-connect assertion requested but no gateway logs found under $LOG_DIR"
    fi

    if tail -n 1200 "${files[@]}" 2>/dev/null | grep -E 'adapter connected|event=("connected"|connected).*adapter|target=("adapter"|adapter).*connected' >/dev/null; then
        ok "Adapter-connect log assertion passed"
        return
    fi
    fail "no adapter-connect log line found in gateway logs (${files[*]})"
}

stamp_success() {
    mkdir -p "$STATE_DIR"
    printf 'sha=%s\ndeployed_at=%s\nrepo=%s\nbranch=%s\ninstall_bin=%s\n' \
        "$DEPLOY_SHA" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$REPO" "$BRANCH" "$INSTALL_BIN" > "$STAMP"
    record_event deploy_success info "deployed $DEPLOY_SHA" "install_bin=$INSTALL_BIN health=$HEALTH_URL"
    ok "DEPLOY COMPLETE — sha=$DEPLOY_SHA"
}

sync_repo
build_release
stop_supervisor
sweep_strays
install_binary
restart_supervisor
assert_health
assert_adapter_connected
stamp_success
