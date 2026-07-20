#!/usr/bin/env bash
#
# enable-deploy-on-merge.sh — one-command install/enable for the #390
# deploy-on-merge poll timer, per #431.
#
# Detects the OS, installs the matching per-OS unit (launchd/systemd/rc.d),
# and enables + starts it through the platform's native service manager.
# Never falls back to an unsupervised nohup/background loop — that violates
# the #333 supervised-restart invariant the same way an unsupervised gateway
# restart would. If native service install fails, this fails loud and tells
# you exactly what to run by hand.
#
# Usage:
#   scripts/zeus-deploy/enable-deploy-on-merge.sh            # install + enable
#   scripts/zeus-deploy/enable-deploy-on-merge.sh --status    # report only
#   scripts/zeus-deploy/enable-deploy-on-merge.sh --disable   # uninstall + stop
#
# Env overrides (mirror deploy-on-merge.sh / deploy-poll.sh):
#   ZEUS_REPO        repo checkout to poll/deploy from (default: detected from this script's location)
#   ZEUS_HOME        zeus state dir (default: $HOME/.zeus)
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="${ZEUS_REPO:-$(cd "$SCRIPT_DIR/../.." && pwd)}"
ZEUS_HOME="${ZEUS_HOME:-$HOME/.zeus}"
UNITS_DIR="$SCRIPT_DIR/units"
MODE="enable"

for arg in "$@"; do
    case "$arg" in
        --status)  MODE="status" ;;
        --disable) MODE="disable" ;;
        --enable)  MODE="enable" ;;
        -h|--help)
            sed -n '2,20p' "${BASH_SOURCE[0]}"
            exit 0
            ;;
        *)
            echo "unknown argument: $arg" >&2
            exit 2
            ;;
    esac
done

log()  { printf '[enable-deploy-on-merge] %s\n' "$1"; }
fail() { printf '[enable-deploy-on-merge] FAIL: %s\n' "$1" >&2; exit 1; }

detect_os() {
    case "$(uname -s)" in
        Darwin) echo "macos" ;;
        Linux)  echo "linux" ;;
        FreeBSD) echo "freebsd" ;;
        *) fail "unsupported OS: $(uname -s) — no unit shape for this platform yet" ;;
    esac
}

render() {
    # render <template> <dest> — substitute __ZEUS_REPO__ / __ZEUS_HOME__ /
    # __ZEUS_USER_HOME__ placeholders and write the result to dest.
    local template="$1" dest="$2"
    sed \
        -e "s|__ZEUS_REPO__|$REPO|g" \
        -e "s|__ZEUS_HOME__|$ZEUS_HOME|g" \
        -e "s|__ZEUS_USER_HOME__|$HOME|g" \
        "$template" > "$dest"
}

# ---------------------------------------------------------------------------
# macOS (launchd)
# ---------------------------------------------------------------------------

macos_plist_dest() { echo "$HOME/Library/LaunchAgents/com.zeus.deploy-poll.plist"; }

macos_status() {
    local dest; dest="$(macos_plist_dest)"
    if [ -f "$dest" ] && launchctl list com.zeus.deploy-poll >/dev/null 2>&1; then
        echo "enabled"
    else
        echo "disabled"
    fi
}

macos_enable() {
    command -v launchctl >/dev/null 2>&1 || fail "launchctl not found"
    local dest; dest="$(macos_plist_dest)"
    mkdir -p "$(dirname "$dest")" "$ZEUS_HOME/logs"
    render "$UNITS_DIR/launchd/com.zeus.deploy-poll.plist" "$dest"
    chmod 644 "$dest"
    launchctl bootout "gui/$(id -u)" "$dest" >/dev/null 2>&1 || true
    launchctl bootstrap "gui/$(id -u)" "$dest" \
        || fail "launchctl bootstrap failed for $dest — inspect with: launchctl print gui/$(id -u)/com.zeus.deploy-poll"
    launchctl enable "gui/$(id -u)/com.zeus.deploy-poll" 2>/dev/null || true
    log "installed and bootstrapped $dest"
}

macos_disable() {
    local dest; dest="$(macos_plist_dest)"
    launchctl bootout "gui/$(id -u)" "$dest" >/dev/null 2>&1 || true
    rm -f "$dest"
    log "removed $dest and booted out the agent"
}

# ---------------------------------------------------------------------------
# Linux (systemd --user)
# ---------------------------------------------------------------------------

linux_unit_dir() { echo "$HOME/.config/systemd/user"; }

linux_status() {
    if systemctl --user is-enabled zeus-deploy-poll.timer >/dev/null 2>&1; then
        echo "enabled"
    else
        echo "disabled"
    fi
}

linux_enable() {
    command -v systemctl >/dev/null 2>&1 || fail "systemctl not found"
    local dir; dir="$(linux_unit_dir)"
    mkdir -p "$dir" "$ZEUS_HOME/logs"
    render "$UNITS_DIR/systemd/zeus-deploy-poll.service" "$dir/zeus-deploy-poll.service"
    cp "$UNITS_DIR/systemd/zeus-deploy-poll.timer" "$dir/zeus-deploy-poll.timer"
    systemctl --user daemon-reload \
        || fail "systemctl --user daemon-reload failed — is a user session/lingering session active? (loginctl enable-linger $USER)"
    systemctl --user enable --now zeus-deploy-poll.timer \
        || fail "systemctl --user enable --now zeus-deploy-poll.timer failed — inspect with: systemctl --user status zeus-deploy-poll.timer"
    log "installed and enabled zeus-deploy-poll.timer (systemd --user)"
}

linux_disable() {
    local dir; dir="$(linux_unit_dir)"
    systemctl --user disable --now zeus-deploy-poll.timer >/dev/null 2>&1 || true
    rm -f "$dir/zeus-deploy-poll.service" "$dir/zeus-deploy-poll.timer"
    systemctl --user daemon-reload 2>/dev/null || true
    log "disabled and removed zeus-deploy-poll.timer"
}

# ---------------------------------------------------------------------------
# FreeBSD (rc.d)
# ---------------------------------------------------------------------------

freebsd_rc_dest() { echo "/usr/local/etc/rc.d/zeus_deploy_poll"; }

freebsd_status() {
    if service zeus_deploy_poll status >/dev/null 2>&1; then
        echo "enabled"
    else
        echo "disabled"
    fi
}

freebsd_enable() {
    command -v service >/dev/null 2>&1 || fail "service(8) not found"
    command -v sudo >/dev/null 2>&1 || fail "sudo not found and rc.d install requires root"
    local dest; dest="$(freebsd_rc_dest)"
    sudo install -m 0555 "$UNITS_DIR/freebsd/zeus_deploy_poll" "$dest" \
        || fail "failed to install rc.d script to $dest"
    if ! grep -q '^zeus_deploy_poll_enable=' /etc/rc.conf 2>/dev/null; then
        {
            echo "zeus_deploy_poll_enable=\"YES\""
            echo "zeus_deploy_poll_user=\"$(id -un)\""
            echo "zeus_deploy_poll_home=\"$HOME\""
            echo "zeus_deploy_poll_repo=\"$REPO\""
        } | sudo tee -a /etc/rc.conf >/dev/null \
            || fail "failed to append zeus_deploy_poll_enable to /etc/rc.conf"
    else
        sudo sysrc zeus_deploy_poll_enable="YES" >/dev/null \
            || fail "sysrc failed to set zeus_deploy_poll_enable=YES"
    fi
    sudo service zeus_deploy_poll start \
        || fail "service zeus_deploy_poll start failed — inspect with: service zeus_deploy_poll status"
    log "installed $dest and started via rc.d"
}

freebsd_disable() {
    sudo service zeus_deploy_poll stop >/dev/null 2>&1 || true
    sudo sysrc zeus_deploy_poll_enable="NO" >/dev/null 2>&1 || true
    sudo rm -f "$(freebsd_rc_dest)"
    log "stopped and disabled zeus_deploy_poll rc.d service"
}

# ---------------------------------------------------------------------------
# dispatch
# ---------------------------------------------------------------------------

OS="$(detect_os)"

status_line() {
    local state="$1"
    local stamp="$ZEUS_HOME/deploy/last-deploy"
    local sha="none"
    [ -f "$stamp" ] && sha="$(awk -F= '/^sha=/{print $2; exit}' "$stamp")"
    [ -n "$sha" ] || sha="none"
    echo "deploy-on-merge: $state (os=$OS, last deploy sha=$sha)"
}

case "$MODE" in
    status)
        case "$OS" in
            macos)   status_line "$(macos_status)" ;;
            linux)   status_line "$(linux_status)" ;;
            freebsd) status_line "$(freebsd_status)" ;;
        esac
        ;;
    enable)
        [ -x "$SCRIPT_DIR/deploy-on-merge.sh" ] || fail "deploy-on-merge.sh missing or not executable at $SCRIPT_DIR"
        [ -x "$SCRIPT_DIR/deploy-poll.sh" ] || fail "deploy-poll.sh missing or not executable at $SCRIPT_DIR"
        mkdir -p "$ZEUS_HOME/logs" "$ZEUS_HOME/deploy"
        case "$OS" in
            macos)   macos_enable ;;
            linux)   linux_enable ;;
            freebsd) freebsd_enable ;;
        esac
        log "deploy-on-merge is now enabled — polling origin/main every 60s from $REPO"
        ;;
    disable)
        case "$OS" in
            macos)   macos_disable ;;
            linux)   linux_disable ;;
            freebsd) freebsd_disable ;;
        esac
        log "deploy-on-merge disabled"
        ;;
esac
