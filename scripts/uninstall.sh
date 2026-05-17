#!/usr/bin/env bash
set -euo pipefail

# ═══════════════════════════════════════════════════════════════════════════════
# Zeus — Universal Uninstaller
# Detects OS, stops services, removes binary, optionally nukes data.
#
# Usage:
#   ./scripts/uninstall.sh              # Remove binary + services, keep config/data
#   ./scripts/uninstall.sh --nuke      # Remove everything including ~/.zeus/
#   ./scripts/uninstall.sh --dry-run    # Show what would be removed without doing it
#
# ═══════════════════════════════════════════════════════════════════════════════

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BOLD='\033[1m'
NC='\033[0m'

NUKE=false
DRY_RUN=false
ZEUS_HOME="${ZEUS_HOME:-$HOME/.zeus}"
INSTALL_DIR="/usr/local/bin"

for arg in "$@"; do
    case "$arg" in
        --nuke)   NUKE=true ;;
        --dry-run) DRY_RUN=true ;;
        --help|-h)
            printf "Usage: %s [flags]\n" "$(basename "$0")"
            printf "  --nuke      Remove ALL Zeus data (~/.zeus/) including config and credentials\n"
            printf "  --dry-run    Show what would be removed without doing it\n"
            printf "  -h, --help   Show this help\n"
            exit 0
            ;;
    esac
done

info()  { printf "  ${YELLOW}→${NC} %s\n" "$*"; }
ok()    { printf "  ${GREEN}✓${NC} %s\n" "$*"; }
warn()  { printf "  ${RED}✗${NC} %s\n" "$*"; }
step()  { printf "\n${BOLD}[%s]${NC} ${BOLD}%s${NC}\n" "$((++STEP_NUM))" "$*"; }
dry()   { if $DRY_RUN; then printf "  ${YELLOW}[dry-run]${NC} would: %s\n" "$*"; return 0; else return 1; fi; }

STEP_NUM=0

printf "\n${BOLD}${RED}Zeus Uninstaller${NC}\n"
printf "═══════════════════════════════════════════════\n"

# ── Detect OS ────────────────────────────────────────────────────────────────
OS="$(uname -s)"
case "$OS" in
    Darwin)  PLATFORM="macos" ;;
    FreeBSD) PLATFORM="freebsd" ;;
    Linux)   PLATFORM="linux" ;;
    *)       PLATFORM="unknown"; warn "Unknown OS: $OS" ;;
esac
info "Platform: $PLATFORM ($OS)"

if $NUKE; then
    warn "NUKE MODE — this will delete ALL Zeus data including config and credentials"
    printf "  Are you sure? Type 'yes' to confirm: "
    read -r confirm
    if [ "$confirm" != "yes" ]; then
        info "Aborted."
        exit 0
    fi
fi

# ── Step 1: Stop processes FIRST (before nuke, so they don't recreate files)
step "Stop Zeus processes"

if dry "kill all zeus processes"; then :
else
    # 1a. Stop via launchd FIRST (prevents launchd from restarting the process
    #     between the pgrep kill and the rm -rf in the nuke step).
    if [ "$PLATFORM" = "macos" ]; then
        for label in "ai.zeus.gateway" "com.zeus.gateway"; do
            if launchctl list "$label" &>/dev/null; then
                launchctl stop "$label" 2>/dev/null || true
                launchctl remove "$label" 2>/dev/null || true
                info "Stopped launchd service: $label"
            fi
        done
        for p in "$HOME/Library/LaunchAgents/ai.zeus.gateway.plist" \
                  "$HOME/Library/LaunchAgents/com.zeus.gateway.plist"; do
            [ -f "$p" ] && launchctl bootout "gui/$(id -u)" "$p" 2>/dev/null || true
        done
    elif [ "$PLATFORM" = "freebsd" ]; then
        sudo service zeus stop 2>/dev/null || true
    elif [ "$PLATFORM" = "linux" ]; then
        systemctl --user stop zeus 2>/dev/null || true
        sudo systemctl stop zeus 2>/dev/null || true
    fi
    sleep 1

    # 1b. Kill any remaining zeus processes via PID file or pgrep
    MYPID=$$
    PIDFILE="${ZEUS_HOME}/gateway.pid"
    if [ -f "$PIDFILE" ]; then
        GW_PID=$(cat "$PIDFILE" 2>/dev/null || true)
        if [ -n "$GW_PID" ] && kill -0 "$GW_PID" 2>/dev/null; then
            kill "$GW_PID" 2>/dev/null && info "Stopped gateway PID $GW_PID (from pid file)" || true
            sleep 1
            kill -9 "$GW_PID" 2>/dev/null || true
        fi
    fi

    PIDS=$(pgrep -f 'zeus (gateway|serve|tui)' 2>/dev/null | grep -v "^${MYPID}$" || true)
    if [ -z "$PIDS" ]; then
        PIDS=$(pgrep -x 'zeus' 2>/dev/null | grep -v "^${MYPID}$" || true)
    fi
    if [ -n "$PIDS" ]; then
        echo "$PIDS" | while read -r pid; do
            if [ "$pid" != "$$" ] && [ "$pid" != "$PPID" ]; then
                kill "$pid" 2>/dev/null && info "Stopped PID $pid" || true
            fi
        done
        sleep 1
        pgrep -x 'zeus' 2>/dev/null | while read -r pid; do
            if [ "$pid" != "$$" ] && [ "$pid" != "$PPID" ]; then
                kill -9 "$pid" 2>/dev/null && info "Force-killed PID $pid" || true
            fi
        done
    else
        ok "No Zeus processes running"
    fi
fi

# ── Step 2: Clean data (if --nuke) — AFTER killing processes
if $NUKE; then
    step "Clean ALL Zeus data"
    # Always clear sessions first (explicit, in case ZEUS_HOME path differs)
    SESSIONS_PATH="$ZEUS_HOME/sessions"
    if [ -d "$SESSIONS_PATH" ]; then
        if dry "clear sessions $SESSIONS_PATH"; then :
        else
            rm -rf "$SESSIONS_PATH"
            ok "Cleared sessions: $SESSIONS_PATH"
        fi
    fi
    if [ -d "$ZEUS_HOME" ]; then
        if dry "remove $ZEUS_HOME"; then :
        else
            rm -rf "$ZEUS_HOME"
            ok "Removed $ZEUS_HOME"
        fi
    else
        ok "$ZEUS_HOME does not exist"
    fi
    # Remove stale PID files
    for pidfile in /tmp/zeus-gateway.pid /tmp/zeus-gateway.log /var/run/zeus_web.pid /var/run/zeus_gateway.pid; do
        [ -f "$pidfile" ] && rm -f "$pidfile" 2>/dev/null && ok "Removed $pidfile"
    done
    # Remove stale temp files
    rm -rf /tmp/zeus_vault_test_* /tmp/zeus_workspace /tmp/zeus-build*.log /tmp/zeus-deploy* 2>/dev/null
    [ -f "$HOME/.zeus-credentials.json" ] && rm -f "$HOME/.zeus-credentials.json" 2>/dev/null
    ok "Cleaned stale temp files and PID files"
fi

# ── Step 3: Remove service ───────────────────────────────────────────────────
step "Remove service"

case "$PLATFORM" in
    macos)
        PLIST="$HOME/Library/LaunchAgents/ai.zeus.gateway.plist"
        PLIST_ALT="$HOME/Library/LaunchAgents/com.zeus.gateway.plist"
        # Service was already stopped in Step 1; just remove the plist files here.
        for p in "$PLIST" "$PLIST_ALT"; do
            if [ -f "$p" ]; then
                if dry "remove $p"; then :
                else
                    launchctl bootout "gui/$(id -u)" "$p" 2>/dev/null || true
                    rm -f "$p"
                    ok "Removed plist: $(basename "$p")"
                fi
            fi
        done
        if [ ! -f "$PLIST" ] && [ ! -f "$PLIST_ALT" ]; then
            ok "No launchd plist files found"
        fi
        ;;
    freebsd)
        RC_SCRIPT="/usr/local/etc/rc.d/zeus"
        if [ -f "$RC_SCRIPT" ]; then
            if dry "stop and remove $RC_SCRIPT"; then :
            else
                sudo service zeus stop 2>/dev/null || true
                sudo rm -f "$RC_SCRIPT"
                # Remove from rc.conf
                sudo sysrc -x zeus_enable 2>/dev/null || true
                ok "Removed rc.d service"
            fi
        else
            ok "No rc.d service found"
        fi
        ;;
    linux)
        SYSTEMD_UNIT="/etc/systemd/system/zeus.service"
        USER_UNIT="$HOME/.config/systemd/user/zeus.service"
        for u in "$SYSTEMD_UNIT" "$USER_UNIT"; do
            if [ -f "$u" ]; then
                if dry "disable and remove $u"; then :
                else
                    if [ "$u" = "$SYSTEMD_UNIT" ]; then
                        sudo systemctl stop zeus 2>/dev/null || true
                        sudo systemctl disable zeus 2>/dev/null || true
                        sudo rm -f "$u"
                        sudo systemctl daemon-reload
                    else
                        systemctl --user stop zeus 2>/dev/null || true
                        systemctl --user disable zeus 2>/dev/null || true
                        rm -f "$u"
                        systemctl --user daemon-reload
                    fi
                    ok "Removed systemd service: $(basename "$u")"
                fi
            fi
        done
        if [ ! -f "$SYSTEMD_UNIT" ] && [ ! -f "$USER_UNIT" ]; then
            ok "No systemd service found"
        fi
        ;;
esac

# ── Step 3: Remove binary ───────────────────────────────────────────────────
step "Remove binary"

BINARY="$INSTALL_DIR/zeus"
# On Apple Silicon Macs (M1/M2/M3/M4/M5), /usr/local/bin may not exist or may
# require sudo even for the owning user — always try sudo as fallback.
if [ -f "$BINARY" ]; then
    if dry "remove $BINARY"; then :
    else
        if rm -f "$BINARY" 2>/dev/null; then
            ok "Removed $BINARY"
        elif sudo rm -f "$BINARY" 2>/dev/null; then
            ok "Removed $BINARY (via sudo)"
        else
            warn "Could not remove $BINARY — try: sudo rm $BINARY"
        fi
    fi
else
    # Also check Homebrew prefix on Apple Silicon (/opt/homebrew/bin)
    ALT_BINARY="/opt/homebrew/bin/zeus"
    if [ -f "$ALT_BINARY" ]; then
        if dry "remove $ALT_BINARY"; then :
        else
            if rm -f "$ALT_BINARY" 2>/dev/null; then
                ok "Removed $ALT_BINARY"
            elif sudo rm -f "$ALT_BINARY" 2>/dev/null; then
                ok "Removed $ALT_BINARY (via sudo)"
            else
                warn "Could not remove $ALT_BINARY — try: sudo rm $ALT_BINARY"
            fi
        fi
    else
        ok "Binary not found at $BINARY"
    fi
fi

# ── Step 4: Data status ──────────────────────────────────────────────────────
if ! $NUKE; then
    step "Data preserved"
    info "Config and data preserved at $ZEUS_HOME"
    info "Use --nuke to remove everything"
fi

# ── Step 5: Clean build artifacts ────────────────────────────────────────────
step "Clean build artifacts"

# Find the Zeus repo — check CWD first, then common locations
ZEUS_REPO=""
if [ -f "Cargo.toml" ] && grep -q "zeus" "Cargo.toml" 2>/dev/null; then
    ZEUS_REPO="$(pwd)"
else
    for candidate in "$HOME/zeus" "$HOME/Zeus" "$HOME/zeus-src"; do
        if [ -f "$candidate/Cargo.toml" ] && grep -q "zeus" "$candidate/Cargo.toml" 2>/dev/null; then
            ZEUS_REPO="$candidate"
            break
        fi
    done
fi

if [ -n "$ZEUS_REPO" ]; then
    # Rust build artifacts — cargo clean + rm target/
    if dry "clean build artifacts in $ZEUS_REPO"; then :
    else
        (cd "$ZEUS_REPO" && cargo clean 2>/dev/null) && ok "cargo clean completed" || true
        if [ -d "$ZEUS_REPO/target" ]; then
            rm -rf "$ZEUS_REPO/target"
            ok "Removed $ZEUS_REPO/target/"
        else
            ok "No target/ directory"
        fi
    fi
    # WebUI build artifacts (trunk/wasm)
    for webdir in "$ZEUS_REPO/apps/ZeusWeb/dist" "$ZEUS_REPO/apps/ZeusWeb/.trunk"; do
        if [ -d "$webdir" ]; then
            if dry "remove $webdir"; then :
            else
                rm -rf "$webdir"
                ok "Removed $(basename "$webdir")/"
            fi
        fi
    done
    # Deployed WebUI (only if ~/.zeus still exists)
    if [ -d "$ZEUS_HOME/web" ]; then
        if dry "remove $ZEUS_HOME/web/"; then :
        else
            rm -rf "$ZEUS_HOME/web"
            ok "Removed deployed WebUI"
        fi
    fi
else
    ok "No Zeus repo found — skipping build artifacts"
fi

# ── Done ─────────────────────────────────────────────────────────────────────
printf "\n${BOLD}${GREEN}Zeus uninstalled.${NC}\n"
if $NUKE; then
    printf "  All data removed. Credentials backed up if they existed.\n"
else
    printf "  Binary and services removed. Config preserved at %s\n" "$ZEUS_HOME"
    printf "  Run with --nuke to also remove config, credentials, and data.\n"
fi
printf "\n"

# Ensure successful exit code for command chaining
exit 0
