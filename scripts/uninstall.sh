#!/usr/bin/env sh
set -u

# ═══════════════════════════════════════════════════════════════════════════════
# Zeus — Universal Uninstaller
# Detects OS → stops service → kills ALL zeus processes → force-removes binary →
# removes the service definition → optionally purges ~/.zeus and build artifacts.
# Works on macOS, FreeBSD, and Linux.
#
# Like update.sh, this kills every zeus process and force-removes the binary so
# the removal is always clean (no "Text file busy" / stale inode left behind).
#
# Usage:
#   ./scripts/uninstall.sh              # Remove binary + service, KEEP ~/.zeus data
#   ./scripts/uninstall.sh --purge      # Also remove ~/.zeus (config, creds, sessions)
#   ./scripts/uninstall.sh --dry-run    # Show what would be removed, do nothing
#   ./scripts/uninstall.sh --yes        # Skip the --purge confirmation prompt
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
total_steps=6

step() {
    step_num=$((step_num + 1))
    printf "\n${BOLD}${BLUE}[%d/%d]${NC} ${BOLD}%s${NC}\n" "$step_num" "$total_steps" "$1"
}
ok()   { printf "  ${GREEN}✓${NC} %s\n" "$1"; }
warn() { printf "  ${YELLOW}!${NC} %s\n" "$1"; }
fail() { printf "  ${RED}✗${NC} %s\n" "$1"; exit 1; }
info() { printf "  ${CYAN}→${NC} %s\n" "$1"; }
# In dry-run, announce the action and signal "skip the real work" (return 0).
dry()  { if $DRY_RUN; then printf "  ${YELLOW}[dry-run]${NC} would: %s\n" "$1"; return 0; fi; return 1; }

# ── Defaults ──────────────────────────────────────────────────────────────────
PURGE=false
DRY_RUN=false
ASSUME_YES=false
ZEUS_HOME="${ZEUS_HOME:-${HOME}/.zeus}"
INSTALL_DIR="/usr/local/bin"
BINARY_NAME="zeus"

# ── Parse flags ───────────────────────────────────────────────────────────────
while [ $# -gt 0 ]; do
    case "$1" in
        --purge)       PURGE=true ;;
        --dry-run)     DRY_RUN=true ;;
        --yes|-y)      ASSUME_YES=true ;;
        --zeus-home)   shift; ZEUS_HOME="${1:?--zeus-home requires a dir}" ;;
        -h|--help)
            printf "${BOLD}Zeus Uninstaller${NC} — remove Zeus (macOS / FreeBSD / Linux)\n\n"
            printf "Usage: %s [flags]\n" "$(basename "$0")"
            printf "  --purge        Also remove ~/.zeus (config, credentials, sessions)\n"
            printf "  --dry-run      Show what would be removed without doing it\n"
            printf "  --yes, -y      Skip the --purge confirmation prompt\n"
            printf "  --zeus-home DIR Zeus home directory (default: ~/.zeus)\n"
            printf "  -h, --help     Show this help\n"
            exit 0
            ;;
        *) warn "Unknown flag: $1" ;;
    esac
    shift
done

INSTALLED_BINARY="$INSTALL_DIR/$BINARY_NAME"

# sudo for root-owned paths ($INSTALL_DIR, system service files)
SUDO=""
if [ "$(id -u)" -ne 0 ] && command -v sudo >/dev/null 2>&1; then
    SUDO="sudo"
fi

printf "${BOLD}${RED}╔══════════════════════════════════════════╗${NC}\n"
printf "${BOLD}${RED}║${NC}        ${BOLD}Zeus — Uninstaller${NC}                ${BOLD}${RED}║${NC}\n"
printf "${BOLD}${RED}╚══════════════════════════════════════════╝${NC}\n"
$DRY_RUN && printf "  ${YELLOW}${BOLD}DRY RUN — nothing will be removed${NC}\n"

# ── Detect OS ─────────────────────────────────────────────────────────────────
OS="$(uname -s)"
case "$OS" in
    Darwin)  PLATFORM="macOS" ;;
    FreeBSD) PLATFORM="FreeBSD" ;;
    Linux)   PLATFORM="Linux" ;;
    *)       PLATFORM="unknown"; warn "Unrecognized OS: $OS (continuing best-effort)" ;;
esac
info "Platform: $PLATFORM"
info "Binary:   $INSTALLED_BINARY"
info "Data:     $ZEUS_HOME"

# ── Purge confirmation ────────────────────────────────────────────────────────
if $PURGE && ! $DRY_RUN && ! $ASSUME_YES; then
    printf "\n  ${RED}${BOLD}PURGE MODE${NC} — this deletes ALL Zeus data including config + credentials.\n"
    printf "  Type 'yes' to confirm: "
    read -r confirm
    if [ "$confirm" != "yes" ]; then
        info "Aborted — nothing removed."
        exit 0
    fi
fi

# ═════════════════════════════════════════════════════════════════════════════
# Phase 1: Stop the managed service
# ═════════════════════════════════════════════════════════════════════════════
step "Stop service"

if dry "stop the $PLATFORM gateway service"; then :
else
    case "$PLATFORM" in
        macOS)
            zeus daemon stop 2>/dev/null || true
            launchctl stop ai.zeus.gateway 2>/dev/null || true
            launchctl stop com.zeus.gateway 2>/dev/null || true
            ;;
        FreeBSD)
            $SUDO service zeus_gateway stop 2>/dev/null || true
            ;;
        Linux)
            systemctl --user stop zeus-gateway 2>/dev/null || true
            $SUDO systemctl stop zeus-gateway 2>/dev/null || true
            ;;
    esac
    ok "Service stop signalled"
fi

# ═════════════════════════════════════════════════════════════════════════════
# Phase 2: Kill ALL zeus processes
# ═════════════════════════════════════════════════════════════════════════════
step "Kill zeus processes"

if dry "kill all zeus processes (pkill -x zeus + subcommands, escalate to KILL)"; then :
else
    KILLED=false
    # Exact name match catches gateway/serve/tui (all run as command "zeus")
    # without ever matching this script (command name "sh"), cargo, or the shell.
    pkill -x zeus 2>/dev/null && KILLED=true || true
    for sub in gateway serve tui daemon; do
        pkill -f "zeus $sub" 2>/dev/null && KILLED=true || true
    done
    if $KILLED; then
        info "Sent TERM — waiting for exit"
        sleep 2
    fi
    if pgrep -x zeus >/dev/null 2>&1; then
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
fi

# ═════════════════════════════════════════════════════════════════════════════
# Phase 3: Remove the service definition
# ═════════════════════════════════════════════════════════════════════════════
step "Remove service definition"

case "$PLATFORM" in
    macOS)
        REMOVED=false
        for p in "$HOME/Library/LaunchAgents/ai.zeus.gateway.plist" \
                 "$HOME/Library/LaunchAgents/com.zeus.gateway.plist"; do
            [ -f "$p" ] || continue
            if dry "bootout + remove $p"; then REMOVED=true; continue; fi
            launchctl bootout "gui/$(id -u)" "$p" 2>/dev/null || true
            rm -f "$p" && { ok "Removed $(basename "$p")"; REMOVED=true; }
        done
        $REMOVED || ok "No launchd service found"
        ;;
    FreeBSD)
        RC="/usr/local/etc/rc.d/zeus_gateway"
        if [ -f "$RC" ]; then
            if dry "remove $RC + sysrc -x zeus_gateway_enable"; then :
            else
                $SUDO rm -f "$RC" || true
                $SUDO sysrc -x zeus_gateway_enable >/dev/null 2>&1 || true
                ok "Removed rc.d service (zeus_gateway)"
            fi
        else
            ok "No rc.d service found"
        fi
        ;;
    Linux)
        REMOVED=false
        USER_UNIT="$HOME/.config/systemd/user/zeus-gateway.service"
        SYS_UNIT="/etc/systemd/system/zeus-gateway.service"
        if [ -f "$USER_UNIT" ]; then
            if dry "disable + remove $USER_UNIT"; then REMOVED=true; else
                systemctl --user disable zeus-gateway 2>/dev/null || true
                rm -f "$USER_UNIT"
                systemctl --user daemon-reload 2>/dev/null || true
                ok "Removed systemd user unit (zeus-gateway)"; REMOVED=true
            fi
        fi
        if [ -f "$SYS_UNIT" ]; then
            if dry "disable + remove $SYS_UNIT"; then REMOVED=true; else
                $SUDO systemctl disable zeus-gateway 2>/dev/null || true
                $SUDO rm -f "$SYS_UNIT"
                $SUDO systemctl daemon-reload 2>/dev/null || true
                ok "Removed systemd system unit (zeus-gateway)"; REMOVED=true
            fi
        fi
        $REMOVED || ok "No systemd service found"
        ;;
esac

# ═════════════════════════════════════════════════════════════════════════════
# Phase 4: Force-remove the binary
# ═════════════════════════════════════════════════════════════════════════════
step "Remove binary"

if [ ! -e "$INSTALLED_BINARY" ]; then
    ok "No binary at $INSTALLED_BINARY"
elif dry "force-remove $INSTALLED_BINARY"; then :
else
    if $SUDO rm -f "$INSTALLED_BINARY" 2>/dev/null && [ ! -e "$INSTALLED_BINARY" ]; then
        ok "Removed $INSTALLED_BINARY"
    else
        # Rename out of the way (defeats ETXTBSY if anything still holds it)
        $SUDO mv -f "$INSTALLED_BINARY" "${INSTALLED_BINARY}.old.$$" 2>/dev/null || true
        $SUDO rm -f "${INSTALLED_BINARY}.old.$$" 2>/dev/null || true
        if [ ! -e "$INSTALLED_BINARY" ]; then
            ok "Removed $INSTALLED_BINARY (via rename)"
        else
            warn "Could not remove $INSTALLED_BINARY — a process may still hold it"
        fi
    fi
fi

# ═════════════════════════════════════════════════════════════════════════════
# Phase 5: Remove data (--purge only)
# ═════════════════════════════════════════════════════════════════════════════
step "Data"

if $PURGE; then
    if [ -d "$ZEUS_HOME" ]; then
        if dry "back up credentials + remove $ZEUS_HOME"; then :
        else
            # Preserve credentials/config before deleting, just in case
            for c in "$ZEUS_HOME/credentials.json" "$ZEUS_HOME/config.toml"; do
                [ -f "$c" ] || continue
                B="$HOME/.zeus-$(basename "$c").backup.$(date +%Y%m%d%H%M%S 2>/dev/null || echo bak)"
                cp "$c" "$B" 2>/dev/null && chmod 600 "$B" 2>/dev/null && info "Backed up $(basename "$c") → $B"
            done
            rm -rf "$ZEUS_HOME"
            ok "Removed $ZEUS_HOME"
        fi
    else
        ok "$ZEUS_HOME does not exist"
    fi
    # The Zeus MCP server entry in ~/.claude/settings.json is left untouched
    # (shared Claude config) — remove the "zeus" entry by hand if desired.
    [ -f "$HOME/.claude/settings.json" ] && info "Note: 'zeus' MCP entry in ~/.claude/settings.json left intact — remove manually if wanted"
else
    info "Config + data preserved at $ZEUS_HOME (use --purge to remove)"
fi

# ═════════════════════════════════════════════════════════════════════════════
# Phase 6: Build artifacts
# ═════════════════════════════════════════════════════════════════════════════
step "Build artifacts"

REPO_ROOT="$(cd "$(dirname "$0")/.." 2>/dev/null && pwd || echo "")"
if [ -n "$REPO_ROOT" ] && [ -f "$REPO_ROOT/Cargo.toml" ] && [ -d "$REPO_ROOT/target" ]; then
    if dry "remove $REPO_ROOT/target"; then :
    else
        rm -rf "$REPO_ROOT/target" && ok "Removed $REPO_ROOT/target"
    fi
else
    ok "No build artifacts to remove"
fi

# ── Done ──────────────────────────────────────────────────────────────────────
if $DRY_RUN; then
    printf "\n${BOLD}${YELLOW}Dry run complete — nothing was removed.${NC}\n\n"
elif $PURGE; then
    printf "\n${BOLD}${GREEN}Zeus fully uninstalled.${NC} All data removed (credentials/config backed up if present).\n\n"
else
    printf "\n${BOLD}${GREEN}Zeus uninstalled.${NC} Binary + service removed; data kept at %s (--purge to remove).\n\n" "$ZEUS_HOME"
fi
