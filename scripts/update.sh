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
            printf "  --with-identity   Re-stamp workspace identity templates via deploy-identity.sh --force\n"
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
            # binary and it survives reboot.
            #
            # #333 invariants: supervised restart or LOUD failure — every
            # fallback names which check failed, and we never nohup while
            # launchd still owns a copy (bootout first, kickstart -k if a
            # loaded service is the reason bootstrap failed).
            DARWIN_PLIST=/Library/LaunchDaemons/com.zeus.gateway.plist
            if [ -f "$DARWIN_PLIST" ] && \
               $SUDO launchctl bootstrap system "$DARWIN_PLIST" 2>/dev/null; then
                $SUDO launchctl enable system/com.zeus.gateway 2>/dev/null || true
                ok "Gateway reloaded via launchd (com.zeus.gateway)"
            elif [ -f "$DARWIN_PLIST" ] && \
                 $SUDO launchctl print system/com.zeus.gateway >/dev/null 2>&1; then
                # bootstrap failed BECAUSE the service is already loaded
                # (Phase 4 bootout didn't take, or a concurrent load). launchd
                # owns a copy — restart it in place rather than spawning a
                # second writer.
                warn "launchctl bootstrap failed: service already loaded — restarting in place"
                if $SUDO launchctl kickstart -k system/com.zeus.gateway 2>/dev/null; then
                    ok "Gateway restarted via launchd kickstart (com.zeus.gateway)"
                else
                    fail "launchctl kickstart -k failed for loaded service com.zeus.gateway — refusing to nohup a duplicate; inspect with: sudo launchctl print system/com.zeus.gateway"
                fi
            else
                # Name WHICH check failed before falling back.
                if [ ! -f "$DARWIN_PLIST" ]; then
                    warn "launchd path unavailable: plist missing ($DARWIN_PLIST)"
                else
                    warn "launchd path unavailable: bootstrap failed and service not loaded (sudo denied or plist rejected — check: sudo launchctl bootstrap system $DARWIN_PLIST)"
                fi
                if zeus daemon start 2>/dev/null; then
                    ok "Gateway started via launchd (zeus daemon)"
                else
                    warn "zeus daemon start failed — falling back to unsupervised nohup"
                    # Ensure launchd does not still own a copy before spawning
                    # an unmanaged one (KeepAlive respawn = duplicate writer).
                    $SUDO launchctl bootout system "$DARWIN_PLIST" 2>/dev/null || true
                    nohup zeus gateway >"$ZEUS_HOME/logs/gateway.out.log" 2>"$ZEUS_HOME/logs/gateway.err.log" &
                    warn "Gateway started via nohup — UNSUPERVISED: no KeepAlive, will not survive reboot"
                fi
            fi
            ;;
        FreeBSD)
            # #333/#409 invariants: FreeBSD rc.d restart must be supervised
            # and must fail loud. Do not mask an rc.d failure with nohup: the
            # rc.d prestart creates root-owned log files, so a non-root start
            # can stop the old gateway and then silently fail to bring it back.
            FBSD_RCD=/usr/local/etc/rc.d/zeus_gateway
            if [ -f "$FBSD_RCD" ]; then
                if [ "$(id -u)" -ne 0 ] && [ -z "$SUDO" ]; then
                    fail "FreeBSD rc.d gateway restart requires root or sudo; sudo not found, refusing to leave gateway half-restarted"
                fi
                $SUDO sysrc zeus_gateway_enable=YES >/dev/null 2>&1 || \
                    fail "FreeBSD rc.d gateway restart failed: could not enable zeus_gateway (try: sudo sysrc zeus_gateway_enable=YES)"

                # Prefer explicit stop+sudo start over `service restart` so the
                # start half is definitely escalated and its failure is not
                # hidden by rc.d restart semantics.
                $SUDO service zeus_gateway stop >/dev/null 2>&1 || true
                if $SUDO service zeus_gateway start >/dev/null 2>&1; then
                    ok "Gateway restarted via rc.d (zeus_gateway)"
                else
                    fail "FreeBSD rc.d gateway start failed after update; gateway may be down (check: sudo service zeus_gateway status; sudo service zeus_gateway start)"
                fi
            else
                warn "rc.d path unavailable: rc script missing ($FBSD_RCD)"
                nohup zeus gateway >"$ZEUS_HOME/logs/gateway.out.log" 2>"$ZEUS_HOME/logs/gateway.err.log" &
                warn "Gateway started via nohup — UNSUPERVISED: no rc.d service, will not survive reboot"
            fi
            ;;
        Linux)
            # #333 invariants: stop the user unit before any nohup (a running
            # unit + nohup = two writers), and name which check failed —
            # including the headless/no-logind case where systemctl --user
            # has no bus.
            if ! command -v systemctl >/dev/null 2>&1; then
                warn "systemd path unavailable: systemctl not found"
                nohup zeus gateway >"$ZEUS_HOME/logs/gateway.out.log" 2>"$ZEUS_HOME/logs/gateway.err.log" &
                warn "Gateway started via nohup — UNSUPERVISED: no systemd, will not survive reboot"
            elif ! systemctl --user show-environment >/dev/null 2>&1; then
                # No user manager bus: headless/SSH session without lingering.
                warn "systemd path unavailable: no user manager session (headless/no-logind?)"
                info "Enable lingering so the user unit survives logout/reboot: loginctl enable-linger $(id -un)"
                nohup zeus gateway >"$ZEUS_HOME/logs/gateway.out.log" 2>"$ZEUS_HOME/logs/gateway.err.log" &
                warn "Gateway started via nohup — UNSUPERVISED: user unit unreachable this session"
            elif systemctl --user restart zeus-gateway 2>/dev/null; then
                ok "Gateway restarted via systemd (zeus-gateway)"
            else
                warn "systemd path unavailable: 'systemctl --user restart zeus-gateway' failed (unit missing? check: systemctl --user status zeus-gateway)"
                # Make sure the unit isn't left running/half-started before
                # spawning an unmanaged copy.
                systemctl --user stop zeus-gateway 2>/dev/null || true
                nohup zeus gateway >"$ZEUS_HOME/logs/gateway.out.log" 2>"$ZEUS_HOME/logs/gateway.err.log" &
                warn "Gateway started via nohup — UNSUPERVISED: no working systemd unit, will not survive reboot"
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
    HEALTH_OK=false
    echo "$HEALTH" | grep -q '"ok"' && HEALTH_OK=true

    if $HEALTH_OK; then
        # #333: count gateway processes — head -1 on the PID list masked
        # duplicate writers. >1 gateway = two processes sharing state dirs.
        # NOTE: ps-based matching, not pgrep -f — verified on macOS that
        # pgrep -f "zeus gateway" can return nothing while the gateway is
        # demonstrably running (ps sees it). ps -o pid=,args= is portable
        # across macOS/FreeBSD/Linux.
        GWPIDS=$(ps ax -o pid=,args= 2>/dev/null | awk '/[z]eus gateway/ {print $1}')
        GWCOUNT=$(printf '%s\n' "$GWPIDS" | grep -c . || true)
        if [ "$GWCOUNT" -gt 1 ]; then
            warn "DUPLICATE GATEWAYS: $GWCOUNT 'zeus gateway' processes running (PIDs: $(printf '%s' "$GWPIDS" | tr '\n' ' '))"
            warn "Two gateways sharing ~/.zeus state — kill the unmanaged one: pkill -f 'zeus gateway' then restart via the service manager"
            if [ "$OS" = "FreeBSD" ]; then
                fail "FreeBSD gateway restart failed invariant: duplicate zeus gateway processes after restart"
            fi
        else
            GWPID=$(printf '%s' "$GWPIDS" | head -1)
            if [ -z "$GWPID" ] && [ "$OS" = "FreeBSD" ]; then
                fail "FreeBSD gateway restart failed invariant: /health is OK but no 'zeus gateway' PID was found"
            fi
            [ -n "$GWPID" ] || GWPID="?"
            ok "Health check OK (gateway PID $GWPID)"
        fi
    else
        if [ "$OS" = "FreeBSD" ]; then
            GWPIDS=$(ps ax -o pid=,args= 2>/dev/null | awk '/[z]eus gateway/ {print $1}')
            GWCOUNT=$(printf '%s\n' "$GWPIDS" | grep -c . || true)
            fail "FreeBSD gateway restart failed: /health did not return ok after restart (gateway pid count: $GWCOUNT). Check: sudo service zeus_gateway status; tail -f $ZEUS_HOME/logs/gateway.err.log"
        else
            warn "Health check failed — gateway may still be starting"
            info "Logs: tail -f $ZEUS_HOME/logs/gateway.err.log"
        fi
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
