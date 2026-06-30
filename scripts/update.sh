#!/usr/bin/env sh
set -u

# ═══════════════════════════════════════════════════════════════════════════════
# Zeus — Update Script
# Safe in-place update of an existing Zeus install.
# Works on macOS, FreeBSD, and Linux.
#
# Flow:  detect OS → pull → build (gateway stays up) → stop service +
#        kill ALL zeus processes → FORCE-REMOVE old binary → install new binary
#        → restart gateway (per-OS) → health check.
#
# The force-remove step is the important part: a plain `cp` over the live binary
# leaves the old inode in place and trips "Text file busy" (ETXTBSY) on FreeBSD
# and stale-binary issues elsewhere. This script kills every zeus process and
# `rm -f`s the binary before installing, so the swap is always clean.
#
# Usage:
#   ./scripts/update.sh                  # pull main, build, swap, restart
#   ./scripts/update.sh --branch dev     # update from a specific branch
#   ./scripts/update.sh --no-pull        # skip git pull (build current tree)
#   ./scripts/update.sh --no-build       # skip build (swap existing release binary)
#   ./scripts/update.sh --clean          # cargo clean before build
#   ./scripts/update.sh --no-restart     # swap binary but leave gateway stopped
#   ./scripts/update.sh --fresh          # also clear sessions after update
#   ./scripts/update.sh --with-identity  # also re-stamp workspace identity (deploy-identity.sh --force)
#
# ═══════════════════════════════════════════════════════════════════════════════

# ── Colors ────────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

step_num=0
total_steps=8

step() {
    step_num=$((step_num + 1))
    printf "\n${BOLD}${BLUE}[%d/%d]${NC} ${BOLD}%s${NC}\n" "$step_num" "$total_steps" "$1"
}

ok()   { printf "  ${GREEN}✓${NC} %s\n" "$1"; }
warn() { printf "  ${YELLOW}!${NC} %s\n" "$1"; }
fail() { printf "  ${RED}✗${NC} %s\n" "$1"; exit 1; }
info() { printf "  ${CYAN}→${NC} %s\n" "$1"; }

# ── Defaults ──────────────────────────────────────────────────────────────────
BRANCH="main"
DO_PULL=true
DO_BUILD=true
CLEAN_BUILD=false
DO_RESTART=true
FRESH=false
WITH_IDENTITY=false
ZEUS_HOME="${ZEUS_HOME:-${HOME}/.zeus}"
INSTALL_DIR="/usr/local/bin"
BINARY_NAME="zeus"

# ── Parse flags ───────────────────────────────────────────────────────────────
while [ $# -gt 0 ]; do
    case "$1" in
        --branch)      shift; BRANCH="${1:?--branch requires a name}" ;;
        --no-pull)     DO_PULL=false ;;
        --no-build)    DO_BUILD=false ;;
        --clean)       CLEAN_BUILD=true ;;
        --no-restart)  DO_RESTART=false ;;
        --fresh)       FRESH=true ;;
        --with-identity) WITH_IDENTITY=true ;;
        --zeus-home)   shift; ZEUS_HOME="${1:?--zeus-home requires a dir}" ;;
        -h|--help)
            printf "${BOLD}Zeus Update${NC} — safe in-place update (macOS / FreeBSD / Linux)\n\n"
            printf "Usage: %s [flags]\n" "$(basename "$0")"
            printf "  --branch NAME     Pull + build from this branch (default: main)\n"
            printf "  --no-pull         Skip git pull (build the current working tree)\n"
            printf "  --no-build        Skip cargo build (swap the existing release binary)\n"
            printf "  --clean           cargo clean before building\n"
            printf "  --no-restart      Install the new binary but leave the gateway stopped\n"
            printf "  --fresh           Clear sessions after the update\n"
            printf "  --with-identity   Re-stamp workspace identity (AGENTS.md/SOUL.md) via deploy-identity.sh --force\n"
            printf "  --zeus-home DIR   Zeus home directory (default: ~/.zeus)\n"
            printf "  -h, --help        Show this help\n"
            exit 0
            ;;
        *)
            warn "Unknown flag: $1"
            ;;
    esac
    shift
done

# --with-identity adds one extra step (workspace identity refresh).
$WITH_IDENTITY && total_steps=9

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
INSTALLED_BINARY="$INSTALL_DIR/$BINARY_NAME"
RELEASE_BINARY="$REPO_ROOT/target/release/$BINARY_NAME"

printf "${BOLD}${BLUE}╔══════════════════════════════════════════╗${NC}\n"
printf "${BOLD}${BLUE}║${NC}        ${BOLD}Zeus — Update${NC}                     ${BOLD}${BLUE}║${NC}\n"
printf "${BOLD}${BLUE}╚══════════════════════════════════════════╝${NC}\n"

# ═════════════════════════════════════════════════════════════════════════════
# Phase 1: Detect OS
# ═════════════════════════════════════════════════════════════════════════════
step "Detect environment"

OS="$(uname -s)"
ARCH="$(uname -m)"
case "$OS" in
    Darwin)  OS_LABEL="macOS";   CORES=$(sysctl -n hw.ncpu 2>/dev/null || echo 4) ;;
    FreeBSD) OS_LABEL="FreeBSD"; CORES=$(sysctl -n hw.ncpu 2>/dev/null || echo 4) ;;
    Linux)   OS_LABEL="Linux";   CORES=$(nproc 2>/dev/null || echo 4) ;;
    *)       fail "Unsupported OS: $OS" ;;
esac
ok "OS: $OS_LABEL ($ARCH), $CORES cores"
info "Repo: $REPO_ROOT"
info "Binary: $INSTALLED_BINARY"

# sudo is needed to write/remove the binary in $INSTALL_DIR
SUDO=""
if [ "$(id -u)" -ne 0 ]; then
    if command -v sudo >/dev/null 2>&1; then
        SUDO="sudo"
    else
        warn "Not root and sudo not found — binary install may fail"
    fi
fi

# ═════════════════════════════════════════════════════════════════════════════
# Phase 2: Pull latest code
# ═════════════════════════════════════════════════════════════════════════════
step "Pull latest code"

cd "$REPO_ROOT" || fail "Cannot cd to repo root: $REPO_ROOT"

if $DO_PULL; then
    if ! command -v git >/dev/null 2>&1; then
        fail "git not found"
    fi
    info "git fetch + pull origin/$BRANCH"
    git fetch origin "$BRANCH" --quiet 2>/dev/null || warn "git fetch failed (offline?) — using local tree"
    if git checkout "$BRANCH" --quiet 2>/dev/null; then
        git pull --ff-only origin "$BRANCH" --quiet 2>/dev/null \
            && ok "Updated to origin/$BRANCH ($(git rev-parse --short HEAD))" \
            || warn "Pull not fast-forward — staying at $(git rev-parse --short HEAD)"
    else
        warn "Could not checkout $BRANCH — building current branch $(git rev-parse --abbrev-ref HEAD)"
    fi
else
    ok "Skipping pull (--no-pull) — building $(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo 'current tree')"
fi

# ═════════════════════════════════════════════════════════════════════════════
# Phase 3: Build (gateway stays up during this — no downtime yet)
# ═════════════════════════════════════════════════════════════════════════════
step "Build release binary"

if $DO_BUILD; then
    if ! command -v cargo >/dev/null 2>&1; then
        [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
    fi
    command -v cargo >/dev/null 2>&1 || fail "cargo not found — install Rust (https://rustup.rs)"

    if $CLEAN_BUILD; then
        info "cargo clean"
        cargo clean 2>/dev/null || true
    fi

    info "cargo build --release --locked --bin zeus (this can take several minutes)"
    if cargo build --release --locked --bin zeus; then
        ok "Build complete"
    else
        fail "Build failed — leaving the running gateway untouched"
    fi
else
    ok "Skipping build (--no-build)"
fi

[ -f "$RELEASE_BINARY" ] || fail "No release binary at $RELEASE_BINARY — run without --no-build first"

# ═════════════════════════════════════════════════════════════════════════════
# Phase 4: Stop the service + kill ALL zeus processes
# ═════════════════════════════════════════════════════════════════════════════
step "Stop gateway + kill zeus processes"

# 4a. Stop the managed service first so the supervisor does not respawn the
#     binary while we are swapping it out.
case "$OS" in
    Darwin)
        # Fully unload the launchd daemon so KeepAlive cannot respawn the stale
        # binary mid-swap. Primary install is the SYSTEM daemon com.zeus.gateway
        # (install.sh → /Library/LaunchDaemons); legacy calls kept as fallbacks.
        $SUDO launchctl bootout system /Library/LaunchDaemons/com.zeus.gateway.plist 2>/dev/null || true
        zeus daemon stop 2>/dev/null || true
        launchctl stop ai.zeus.gateway 2>/dev/null || true
        ;;
    FreeBSD)
        $SUDO service zeus_gateway stop 2>/dev/null || true
        ;;
    Linux)
        systemctl --user stop zeus-gateway 2>/dev/null || true
        ;;
esac

# 4b. Kill every remaining zeus process. We match precisely so we never touch
#     this script, cargo, or the shell:
#       - pkill -x zeus       → processes whose command name is exactly "zeus"
#       - pkill -f 'zeus gateway' / 'zeus serve' / 'zeus tui'  → subcommands
KILLED=false
# Exact process-name match catches every flavour (zeus gateway/serve/tui share
# the command name "zeus") without ever touching this script, cargo, or the shell.
pkill -x zeus 2>/dev/null && KILLED=true || true
# Belt-and-suspenders for processes shown by full path; each pattern is ONE arg.
for sub in gateway serve tui daemon; do
    pkill -f "zeus $sub" 2>/dev/null && KILLED=true || true
done
if $KILLED; then
    info "Sent TERM to zeus processes — waiting for exit"
    sleep 2
fi

# 4c. Escalate to SIGKILL for anything that survived (so the binary is not held).
STILL=$(pgrep -x zeus 2>/dev/null || true)
if [ -n "$STILL" ]; then
    warn "Some zeus processes survived TERM — sending KILL"
    pkill -9 -x zeus 2>/dev/null || true
    pkill -9 -f 'zeus gateway' 2>/dev/null || true
    sleep 1
fi

if pgrep -x zeus >/dev/null 2>&1; then
    warn "zeus processes still present — binary removal may fail"
else
    ok "All zeus processes stopped"
fi
rm -f "$ZEUS_HOME/gateway.pid" 2>/dev/null || true

# ═════════════════════════════════════════════════════════════════════════════
# Phase 5: Force-remove the old binary
# ═════════════════════════════════════════════════════════════════════════════
step "Remove old binary"

if [ -e "$INSTALLED_BINARY" ]; then
    if $SUDO rm -f "$INSTALLED_BINARY" 2>/dev/null && [ ! -e "$INSTALLED_BINARY" ]; then
        ok "Removed $INSTALLED_BINARY"
    else
        # Last resort: rename out of the way (defeats ETXTBSY if anything holds it)
        $SUDO mv -f "$INSTALLED_BINARY" "${INSTALLED_BINARY}.old.$$" 2>/dev/null || true
        if [ ! -e "$INSTALLED_BINARY" ]; then
            $SUDO rm -f "${INSTALLED_BINARY}.old.$$" 2>/dev/null || true
            ok "Removed $INSTALLED_BINARY (via rename)"
        else
            fail "Could not remove $INSTALLED_BINARY — a process may still hold it"
        fi
    fi
else
    info "No existing binary at $INSTALLED_BINARY (fresh install path)"
fi

# ═════════════════════════════════════════════════════════════════════════════
# Phase 6: Install the new binary
# ═════════════════════════════════════════════════════════════════════════════
step "Install new binary"

$SUDO mkdir -p "$INSTALL_DIR" 2>/dev/null || true
$SUDO cp "$RELEASE_BINARY" "$INSTALLED_BINARY" || fail "Failed to copy binary to $INSTALLED_BINARY"
$SUDO chmod 0755 "$INSTALLED_BINARY" 2>/dev/null || true

if [ "$OS" = "Darwin" ]; then
    # Clear quarantine xattrs + ad-hoc codesign so Gatekeeper allows the binary
    $SUDO xattr -cr "$INSTALLED_BINARY" 2>/dev/null || true
    codesign -s - "$INSTALLED_BINARY" 2>/dev/null || true
fi

NEW_VER="$("$INSTALLED_BINARY" --version 2>/dev/null || echo 'unknown')"
ok "Installed: $INSTALLED_BINARY ($NEW_VER)"

# ═════════════════════════════════════════════════════════════════════════════
# Phase 6b: Refresh workspace identity (optional, --with-identity)
# ═════════════════════════════════════════════════════════════════════════════
# Re-stamp AGENTS.md / SOUL.md / HEARTBEAT.md / etc. from the (now-updated) repo's
# deploy-identity.sh. Runs BEFORE the restart so the gateway comes up with fresh
# identity. The binary swap alone does NOT refresh workspace templates — mirrors
# install.sh's --with-identity behaviour.
if $WITH_IDENTITY; then
    step "Refresh workspace identity"
    DEPLOY_IDENTITY="$REPO_ROOT/scripts/deploy-identity.sh"
    if [ -f "$DEPLOY_IDENTITY" ]; then
        info "Running deploy-identity.sh --force"
        chmod +x "$DEPLOY_IDENTITY" 2>/dev/null || true
        if "$DEPLOY_IDENTITY" --force; then
            ok "Workspace identity refreshed"
        else
            warn "deploy-identity.sh exited non-zero — identity may be partially refreshed"
        fi
    else
        warn "deploy-identity.sh not found at $DEPLOY_IDENTITY — skipping identity refresh"
    fi
fi

# ═════════════════════════════════════════════════════════════════════════════
# Phase 7: Restart the gateway (per-OS)
# ═════════════════════════════════════════════════════════════════════════════
step "Restart gateway"

if ! $DO_RESTART; then
    ok "Skipping restart (--no-restart) — start it with 'zeus daemon start' or 'service zeus_gateway start'"
else
    mkdir -p "$ZEUS_HOME/logs" 2>/dev/null || true
    case "$OS" in
        Darwin)
            # Symmetric with Phase 4 bootout: reload the SYSTEM daemon
            # (com.zeus.gateway) so launchd manages + KeepAlive-respawns the NEW
            # binary and it survives reboot. Fall back to the binary's own daemon
            # manager, then a bare nohup, for non-standard installs.
            if [ -f /Library/LaunchDaemons/com.zeus.gateway.plist ] && \
               $SUDO launchctl bootstrap system /Library/LaunchDaemons/com.zeus.gateway.plist 2>/dev/null; then
                $SUDO launchctl enable system/com.zeus.gateway 2>/dev/null || true
                ok "Gateway reloaded via launchd (com.zeus.gateway)"
            elif zeus daemon start 2>/dev/null; then
                ok "Gateway started via launchd (zeus daemon)"
            else
                nohup zeus gateway >"$ZEUS_HOME/logs/gateway.out.log" 2>"$ZEUS_HOME/logs/gateway.err.log" &
                ok "Gateway started via nohup (launchd unavailable)"
            fi
            ;;
        FreeBSD)
            $SUDO sysrc zeus_gateway_enable=YES >/dev/null 2>&1 || true
            if [ -f /usr/local/etc/rc.d/zeus_gateway ] && $SUDO service zeus_gateway restart >/dev/null 2>&1; then
                ok "Gateway restarted via rc.d (zeus_gateway)"
            else
                nohup zeus gateway >"$ZEUS_HOME/logs/gateway.out.log" 2>"$ZEUS_HOME/logs/gateway.err.log" &
                ok "Gateway started via nohup (no rc.d service)"
            fi
            ;;
        Linux)
            if command -v systemctl >/dev/null 2>&1 && systemctl --user restart zeus-gateway 2>/dev/null; then
                ok "Gateway restarted via systemd (zeus-gateway)"
            else
                nohup zeus gateway >"$ZEUS_HOME/logs/gateway.out.log" 2>"$ZEUS_HOME/logs/gateway.err.log" &
                ok "Gateway started via nohup (no systemd unit)"
            fi
            ;;
    esac

    # Health check
    sleep 3
    HEALTH=""
    i=0
    while [ "$i" -lt 5 ]; do
        HEALTH=$(curl -s --max-time 5 http://127.0.0.1:8080/health 2>/dev/null || echo "")
        echo "$HEALTH" | grep -q '"ok"' && break
        sleep 2
        i=$((i + 1))
    done
    if echo "$HEALTH" | grep -q '"ok"'; then
        GWPID=$(pgrep -x zeus 2>/dev/null | head -1 || echo "?")
        ok "Health check OK (gateway PID $GWPID)"
    else
        warn "Health check failed — gateway may still be starting"
        info "Logs: tail -f $ZEUS_HOME/logs/gateway.err.log"
    fi
fi

# ═════════════════════════════════════════════════════════════════════════════
# Phase 8: Optional session reset
# ═════════════════════════════════════════════════════════════════════════════
step "Finish"

if $FRESH; then
    rm -f "$ZEUS_HOME"/sessions/*.jsonl 2>/dev/null || true
    ok "Sessions cleared (--fresh)"
fi

printf "\n${BOLD}${GREEN}Zeus updated successfully.${NC}  %s\n\n" "$NEW_VER"
