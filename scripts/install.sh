#!/usr/bin/env sh
set -eu

# ╔══════════════════════════════════════════════════════════════════════════╗
# ║  Zeus — Universal Installer v2                                          ║
# ║  Cyberpunk UI · OS detection → build → install → onboard → launch      ║
# ║  Works on macOS, FreeBSD, and Linux.                                    ║
# ╚══════════════════════════════════════════════════════════════════════════╝

# ── ANSI Theme (Cyberpunk Red/Dark) ─────────────────────────────────────────
C="\033[38;5;196m"    # red accent
CS="\033[38;5;203m"   # soft red
CD="\033[38;5;124m"   # dark rust accent
G="\033[38;5;46m"     # green
Y="\033[38;5;220m"    # yellow
P="\033[38;5;135m"    # purple
D="\033[38;5;240m"    # dim
W="\033[38;5;252m"    # white/text
B="\033[1m"           # bold
N="\033[0m"           # reset
DIM="\033[2m"

# Terminal width (fallback 70)
COLS=$(tput cols 2>/dev/null || echo 70)
[ "$COLS" -gt 100 ] && COLS=100

# ── UI Functions ────────────────────────────────────────────────────────────

banner() {
    printf "\n"
    printf "${C}${B}  ██████╗ ███████╗██╗   ██╗███████╗${N}\n"
    printf "${C}${B}  ╚════██╗██╔════╝██║   ██║██╔════╝${N}\n"
    printf "${CS}    ███╔═╝█████╗  ██║   ██║███████╗${N}\n"
    printf "${CS}   ██╔══╝ ██╔══╝  ██║   ██║╚════██║${N}\n"
    printf "${CD}${B}  ███████╗███████╗╚██████╔╝███████║${N}\n"
    printf "${CD}${B}  ╚══════╝╚══════╝ ╚═════╝ ╚══════╝${N}\n"
    printf "${D}  ─────────────────────────────────────${N}\n"
    printf "${D}  Universal Installer v0.1.0${N}\n"
    printf "${D}  zeuslab.ai${N}\n"
    printf "\n"
}

separator() {
    # $1 = label
    local label="$1"
    local llen=${#label}
    local pad=$((COLS - llen - 4))
    [ $pad -lt 4 ] && pad=4
    printf "${C}${B} %s ${D}" "$label"
    i=0; while [ $i -lt $pad ]; do printf "━"; i=$((i + 1)); done
    printf "${N}\n"
}

# Phase header with number and progress bar
PHASE_NUM=0
TOTAL_PHASES=9
phase() {
    PHASE_NUM=$((PHASE_NUM + 1))
    local label="$1"
    local filled=$((PHASE_NUM * 20 / TOTAL_PHASES))
    local empty=$((20 - filled))

    printf "\n"
    separator "$label"

    # Progress bar
    printf "${D}  [${N}"
    i=0; while [ $i -lt $filled ]; do printf "${C}█${N}"; i=$((i + 1)); done
    i=0; while [ $i -lt $empty ]; do printf "${D}░${N}"; i=$((i + 1)); done
    printf "${D}] ${W}${PHASE_NUM}/${TOTAL_PHASES}${N}\n\n"
}

ok()   { printf "  ${G}✓${N} ${W}%s${N}\n" "$1"; }
warn() { printf "  ${Y}!${N} ${Y}%s${N}\n" "$1"; }
fail() { printf "  ${C}✗${N} ${C}%s${N}\n" "$1"; exit 1; }
info() { printf "  ${CS}→${N} ${D}%s${N}\n" "$1"; }

# Spinner for long operations
spin() {
    local pid=$1 label=$2
    local chars='⣾⣽⣻⢿⡿⣟⣯⣷'
    local i=0
    while kill -0 "$pid" 2>/dev/null; do
        local c=$(printf '%s' "$chars" | cut -c$((i % 8 + 1)))
        printf "\r  ${C}%s${N} ${D}%s${N}" "$c" "$label"
        sleep 0.1
        i=$((i + 1))
    done
    printf "\r  ${G}✓${N} ${W}%s${N}\n" "$label"
    wait "$pid" 2>/dev/null
    return $?
}

# Timer
timer_start() { TIMER_START=$(date +%s); }
timer_elapsed() {
    local now=$(date +%s)
    local elapsed=$((now - TIMER_START))
    if [ $elapsed -ge 60 ]; then
        printf "%dm%ds" $((elapsed / 60)) $((elapsed % 60))
    else
        printf "%ds" $elapsed
    fi
}

# Summary table row
summary_row() {
    # $1 = label, $2 = value, $3 = color (optional, default W)
    local color="${3:-$W}"
    printf "  ${D}%-20s${N} ${color}%s${N}\n" "$1" "$2"
}

# Box drawing for final summary
box_top() {
    printf "${C}  ╔"
    i=0; while [ $i -lt $((COLS - 6)) ]; do printf "═"; i=$((i + 1)); done
    printf "╗${N}\n"
}
box_mid() {
    printf "${C}  ║${N} %-$((COLS - 8))s ${C}║${N}\n" "$1"
}
box_sep() {
    printf "${C}  ╠"
    i=0; while [ $i -lt $((COLS - 6)) ]; do printf "═"; i=$((i + 1)); done
    printf "╣${N}\n"
}
box_bot() {
    printf "${C}  ╚"
    i=0; while [ $i -lt $((COLS - 6)) ]; do printf "═"; i=$((i + 1)); done
    printf "╝${N}\n"
}

# ── macOS LaunchDaemon helpers (system domain) ──────────────────────────────
#
# install_launchd_plist <repo_root> <install_dir> <log_dir> [mode]
#   Substitutes placeholders in packaging/macos/com.zeus.gateway.plist.tmpl
#   and installs into /Library/LaunchDaemons (system domain — no GUI session
#   required). Migrates from any pre-existing user-agent at
#   ~/Library/LaunchAgents/com.zeus.gateway.plist by booting it out of the
#   gui/UID domain first. Idempotent: safe to call on fresh install or update.
#   mode (default "load"): bootstrap the plist this boot. Pass "write-only" to
#   install the plist for reboot-survival (RunAtLoad=true) WITHOUT loading it
#   now — used on fresh TUI onboard where AWAKEN spawns the live daemon and a
#   parallel launchd start would race it.
#
# install_newsyslog_conf <repo_root> <log_dir>
#   Substitutes placeholders in packaging/macos/newsyslog-zeus-gateway.conf.tmpl
#   and installs into /etc/newsyslog.d/com.zeus.gateway.conf. Rotates
#   zeus-gateway.out.log at 50 MB and zeus-gateway.err.log at 10 MB,
#   keeping 5 compressed archives each. Requires sudo.
#
#   REGRESSION FENCE: if you change log paths in the plist template above,
#   update this function's template path AND the legacy-cleanup stanza below.
#
# Echoes "installed" on success; non-zero exit on failure.
install_newsyslog_conf() {
    local repo_root="$1" log_dir="$2"
    local tmpl="$repo_root/packaging/macos/newsyslog-zeus-gateway.conf.tmpl"
    local dst="/etc/newsyslog.d/com.zeus.gateway.conf"
    local install_user="${SUDO_USER:-$USER}"

    [ -f "$tmpl" ] || { echo "newsyslog template missing: $tmpl" >&2; return 1; }

    local tmp; tmp="$(mktemp -t zeus-newsyslog.XXXXXX)" || return 1
    sed -e "s|__LOG_DIR__|$log_dir|g" \
        -e "s|__USER__|$install_user|g" \
        "$tmpl" > "$tmp"

    sudo mkdir -p /etc/newsyslog.d
    sudo cp "$tmp" "$dst"
    sudo chmod 644 "$dst"
    rm -f "$tmp"

    # HUP newsyslog so it picks up the new config without waiting for the
    # next cron interval. Non-fatal: rotation will still happen on schedule.
    sudo pkill -HUP newsyslog 2>/dev/null || true

    echo "installed"
}

# Echoes "loaded" on success; non-zero exit on failure.
install_launchd_plist() {
    local repo_root="$1" install_dir="$2" log_dir="$3"
    # $4 = bootstrap mode (default "load"). Pass "write-only" to render+install
    # the plist for reboot-survival (RunAtLoad=true) WITHOUT bootstrapping it
    # this boot — used on fresh TUI onboard where AWAKEN spawns the live daemon
    # (avoids a double-start race; launchd takes over on the next reboot).
    local mode="${4:-load}"
    local tmpl="$repo_root/packaging/macos/com.zeus.gateway.plist.tmpl"
    local sys_dst="/Library/LaunchDaemons/com.zeus.gateway.plist"
    local user_old="$HOME/Library/LaunchAgents/com.zeus.gateway.plist"
    local install_user="${SUDO_USER:-$USER}"

    [ -f "$tmpl" ] || { echo "plist template missing: $tmpl" >&2; return 1; }

    # Migration: bootout + remove any old user-agent plist.
    if [ -f "$user_old" ]; then
        launchctl bootout "gui/$(id -u "$install_user")" "$user_old" 2>/dev/null || true
        launchctl unload "$user_old" 2>/dev/null || true
        rm -f "$user_old" 2>/dev/null || true
    fi

    # Render template → temp file → install with root ownership.
    local tmp; tmp="$(mktemp -t zeus-gateway-plist.XXXXXX)" || return 1
    sed -e "s|__USER__|$install_user|g" \
        -e "s|__INSTALL_DIR__|$install_dir|g" \
        -e "s|__LOG_DIR__|$log_dir|g" \
        "$tmpl" > "$tmp" || { rm -f "$tmp"; return 1; }

    sudo install -o root -g wheel -m 644 "$tmp" "$sys_dst" || { rm -f "$tmp"; return 1; }
    rm -f "$tmp"

    # write-only: plist installed (RunAtLoad=true → survives reboot) but NOT
    # loaded this boot. AWAKEN's spawn brings the titan live now; launchd takes
    # over on next reboot. No bootstrap = no race with the live daemon.
    if [ "$mode" = "write-only" ]; then
        echo "written"
        return 0
    fi

    # Reload: bootout (best-effort, ignore "not loaded") then bootstrap.
    sudo launchctl bootout system "$sys_dst" 2>/dev/null || true
    sudo launchctl bootstrap system "$sys_dst" || return 1
    sudo launchctl enable system/com.zeus.gateway 2>/dev/null || true

    echo "loaded"
}

# ── Defaults ────────────────────────────────────────────────────────────────
RECONFIGURE=false
DO_BUILD=true
CLEAN_BUILD=false
DO_LAUNCH=true
UPDATE_ONLY=false
WITH_IDENTITY=false
CLASSIC_ONBOARD=false
WITH_WEBUI=false
MCP_ONLY=false
WEBUI_LISTEN="localhost"
ZEUS_HOME="${HOME}/.zeus"
INSTALL_DIR="/usr/local/bin"

# ── Parse flags ─────────────────────────────────────────────────────────────
while [ $# -gt 0 ]; do
    case "$1" in
        --reconfigure)     RECONFIGURE=true ;;
        --no-build)        DO_BUILD=false ;;
        --clean)           CLEAN_BUILD=true ;;
        --update)          UPDATE_ONLY=true ;;
        --with-identity)   WITH_IDENTITY=true ;;
        --no-launch)       DO_LAUNCH=false ;;
        --classic)         CLASSIC_ONBOARD=true ;;
        --with-webui)      WITH_WEBUI=true ;;
        --mcp-only)        MCP_ONLY=true; DO_LAUNCH=false ;;
        --webui-listen=*)  WEBUI_LISTEN="${1#*=}"
                           [ -n "$WEBUI_LISTEN" ] || { warn "--webui-listen requires a non-empty address; using localhost"; WEBUI_LISTEN="localhost"; } ;;
        --zeus-home)
            # Validate: requires a value, and that value must not be another flag.
            # Without this, `--zeus-home --update` silently sets ZEUS_HOME="--update",
            # and a trailing `--zeus-home` crashes on unbound $1 under `set -u`.
            if [ $# -lt 2 ] || [ -z "$2" ]; then
                fail "--zeus-home requires a directory argument (e.g. --zeus-home ~/.zeus)"
            fi
            case "$2" in
                -*) fail "--zeus-home requires a directory argument, got flag-like value: $2" ;;
            esac
            shift; ZEUS_HOME="$1"
            ;;
        --help|-h)
            banner
            printf "${W}Usage:${N} %s [flags]\n\n" "$(basename "$0")"
            printf "${D}  Builds the gateway, installs the binary to %s, onboards your\n" "$INSTALL_DIR"
            printf "  config, and launches Zeus — works on macOS, Linux, and FreeBSD.${N}\n\n"
            printf "${W}Flags:${N}\n"
            printf "  ${CS}--reconfigure${N}       Re-run onboarding on existing install\n"
            printf "  ${CS}--no-build${N}          Skip cargo build (use existing binary)\n"
            printf "  ${CS}--clean${N}             Clean build (cargo clean before build)\n"
            printf "  ${CS}--update${N}            Rebuild + install binary + restart (no config changes)\n"
            printf "  ${CS}--with-identity${N}     With --update: also refresh workspace identity templates\n"
            printf "  ${CS}--no-launch${N}         Don't start gateway after install\n"
            printf "  ${CS}--classic${N}           Use classic CLI onboarding (no TUI)\n"
            printf "  ${CS}--with-webui${N}        Build and install WebUI (trunk + WASM)\n"
            printf "  ${CS}--mcp-only${N}          Install only the MCP server (build + binary + Claude/Codex MCP config); skip gateway service, onboarding, and launch\n"
            printf "  ${CS}--webui-listen=ADDR${N} Gateway/WebUI listen address (default: localhost). Use 0.0.0.0 to expose on the LAN\n"
            printf "  ${CS}--zeus-home DIR${N}     Custom zeus home (default: ~/.zeus)\n"
            printf "  ${CS}-h, --help${N}          Show this help\n"
            printf "\n${W}Common usage:${N}\n"
            printf "  ${D}# fresh install + onboard${N}\n"
            printf "  ${CS}%s${N}\n" "$(basename "$0")"
            printf "  ${D}# rebuild + reinstall + restart${N}\n"
            printf "  ${CS}%s --update${N}\n" "$(basename "$0")"
            printf "  ${D}# update + refresh personas${N}\n"
            printf "  ${CS}%s --update --with-identity${N}\n" "$(basename "$0")"
            printf "  ${D}# build + serve WebUI (:8081)${N}\n"
            printf "  ${CS}%s --with-webui${N}\n" "$(basename "$0")"
            printf "  ${D}# re-run onboarding${N}\n"
            printf "  ${CS}%s --reconfigure${N}\n" "$(basename "$0")"
            exit 0
            ;;
        *) fail "Unknown flag: $1 (run with --help to see available flags)" ;;
    esac
    shift
done

timer_start
banner

# ── Sudo check ──────────────────────────────────────────────────────────────
# The installer needs sudo to copy the binary to /usr/local/bin, install
# system dependencies, and set up OS services (launchd/rc.d).
printf "${CS}→${N} ${W}This installer needs sudo access to:${N}\n"
printf "  ${D}• Install the zeus binary to /usr/local/bin${N}\n"
printf "  ${D}• Install system packages (cmake, protobuf, etc.)${N}\n"
printf "  ${D}• Set up the gateway service (launchd/rc.d)${N}\n"
printf "\n"
if ! sudo -v 2>/dev/null; then
    fail "sudo access required. Run with a user that has sudo privileges."
fi
# Keep sudo alive for the duration of the install.
# Self-bound to the installer PID: the keepalive polls `kill -0 $MAIN_PID` and
# exits within one sleep-cycle (≤50s) once install.sh is gone — even if the
# parent dies via SIGKILL or an SSH disconnect (rebuild-fleet) skips the trap.
# This prevents an orphaned `sudo -n true` loop from lingering and, in the
# worst case, interfering with the gateway it was meant to (re)start.
MAIN_PID=$$
(while kill -0 "$MAIN_PID" 2>/dev/null; do sudo -n true 2>/dev/null; sleep 50; done) &
SUDO_KEEPALIVE_PID=$!
# Trap the common exit paths so the keepalive is reaped promptly on a clean run.
# HUP catches the SSH-disconnect case (rebuild-fleet); EXIT/INT/TERM cover the
# normal/cancelled paths. SIGKILL can't be trapped — the self-bind above is the
# backstop for that.
trap "kill $SUDO_KEEPALIVE_PID 2>/dev/null" EXIT INT TERM HUP

# ═══════════════════════════════════════════════════════════════════════════
# Phase 1: OS Detection
# ═══════════════════════════════════════════════════════════════════════════
phase "DETECT ENVIRONMENT"

OS="$(uname -s)"
ARCH="$(uname -m)"
HOSTNAME="$(hostname -s 2>/dev/null || hostname 2>/dev/null || echo 'zeus-node')"

case "$OS" in
    Darwin)
        OS_LABEL="macOS"
        CORES=$(sysctl -n hw.ncpu 2>/dev/null || echo 4)
        MEM=$(sysctl -n hw.memsize 2>/dev/null | awk '{printf "%.0f GB", $1/1024/1024/1024}')
        ;;
    FreeBSD)
        OS_LABEL="FreeBSD"
        CORES=$(sysctl -n hw.ncpu 2>/dev/null || echo 4)
        MEM=$(sysctl -n hw.physmem 2>/dev/null | awk '{printf "%.0f GB", $1/1024/1024/1024}')
        [ ! -w /dev/null ] && sudo chmod 666 /dev/null
        ;;
    Linux)
        OS_LABEL="Linux"
        CORES=$(nproc 2>/dev/null || echo 4)
        MEM=$(free -h 2>/dev/null | awk '/Mem:/{print $2}' || echo "?")
        ;;
    *)
        fail "Unsupported OS: $OS"
        ;;
esac

summary_row "OS:" "$OS_LABEL ($ARCH)"
summary_row "Hostname:" "$HOSTNAME"
summary_row "Cores:" "$CORES"
summary_row "Memory:" "$MEM"

# ═══════════════════════════════════════════════════════════════════════════
# Phase 2: Check dependencies + build
# ═══════════════════════════════════════════════════════════════════════════
phase "CHECK DEPENDENCIES"

# Install platform build deps FIRST (needed for curl, cmake, etc. before Rust install)
case "$OS" in
    Linux)
        if command -v apt-get >/dev/null 2>&1; then
            if ! command -v curl >/dev/null 2>&1 || ! command -v cc >/dev/null 2>&1; then
                info "Installing essential build tools (curl, build-essential, cmake...)"
                sudo apt-get update -qq
                sudo apt-get install -y curl build-essential cmake pkg-config libssl-dev protobuf-compiler libasound2-dev libopus-dev 2>/dev/null \
                    || warn "Some packages failed — build may still work"
                ok "Linux packages (apt)"
            fi
        elif command -v pacman >/dev/null 2>&1; then
            if ! command -v curl >/dev/null 2>&1 || ! command -v cc >/dev/null 2>&1; then
                info "Installing essential build tools..."
                sudo pacman -S --noconfirm base-devel cmake pkg-config openssl curl 2>/dev/null \
                    || warn "Some packages failed — build may still work"
                ok "Linux packages (pacman)"
            fi
        fi
        ;;
esac

# Check for Rust toolchain
# Minimum required rustc — bumped when deps (e.g. constant_time_eq) require it
RUST_MIN_VERSION="1.95.0"

# Compare semver-style X.Y.Z versions. Returns 0 if $1 >= $2, else 1.
version_ge() {
    # POSIX-compatible semver comparison (no bash arrays or here-strings)
    local a1 a2 a3 b1 b2 b3
    a1=$(echo "$1" | cut -d. -f1); a2=$(echo "$1" | cut -d. -f2); a3=$(echo "$1" | cut -d. -f3)
    b1=$(echo "$2" | cut -d. -f1); b2=$(echo "$2" | cut -d. -f2); b3=$(echo "$2" | cut -d. -f3)
    : "${a1:=0}" "${a2:=0}" "${a3:=0}" "${b1:=0}" "${b2:=0}" "${b3:=0}"
    [ "$a1" -gt "$b1" ] 2>/dev/null && return 0
    [ "$a1" -lt "$b1" ] 2>/dev/null && return 1
    [ "$a2" -gt "$b2" ] 2>/dev/null && return 0
    [ "$a2" -lt "$b2" ] 2>/dev/null && return 1
    [ "$a3" -gt "$b3" ] 2>/dev/null && return 0
    [ "$a3" -lt "$b3" ] 2>/dev/null && return 1
    return 0
}

if command -v cargo >/dev/null 2>&1; then
    RUST_VERSION=$(rustc --version 2>/dev/null | awk '{print $2}')
    if version_ge "$RUST_VERSION" "$RUST_MIN_VERSION"; then
        ok "Rust $RUST_VERSION (>= $RUST_MIN_VERSION required)"
    else
        warn "Rust $RUST_VERSION is older than required $RUST_MIN_VERSION — running rustup update"
        if command -v rustup >/dev/null 2>&1; then
            rustup update stable >/dev/null 2>&1 || true
            RUST_VERSION=$(rustc --version 2>/dev/null | awk '{print $2}')
            if version_ge "$RUST_VERSION" "$RUST_MIN_VERSION"; then
                ok "Rust updated to $RUST_VERSION"
            else
                fail "Rust $RUST_VERSION still below required $RUST_MIN_VERSION after update. Run: rustup update stable"
            fi
        else
            fail "Rust $RUST_VERSION below required $RUST_MIN_VERSION and rustup not found. Install rustup: https://rustup.rs"
        fi
    fi
else
    warn "Rust not found — installing via rustup"
    if command -v curl >/dev/null 2>&1; then
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        . "$HOME/.cargo/env"
        ok "Rust installed: $(rustc --version | awk '{print $2}')"
    else
        fail "Neither cargo nor curl found. Install Rust: https://rustup.rs"
    fi
fi

# Sanity check: warn loudly if rust-toolchain.toml pins an older version than the minimum.
# This was the exact trap on .234 (stale checkout with 1.93.1 pin → cargo ignored rustup 1.95).
# REPO_ROOT isn't set yet at this point — resolve from script location.
_SCRIPT_DIR="$(cd "$(dirname "$0")" 2>/dev/null && pwd)" || _SCRIPT_DIR=""
_TOOLCHAIN_FILE=""
if [ -n "$_SCRIPT_DIR" ] && [ -f "$_SCRIPT_DIR/../rust-toolchain.toml" ]; then
    _TOOLCHAIN_FILE="$_SCRIPT_DIR/../rust-toolchain.toml"
elif [ -f "$HOME/Zeus/rust-toolchain.toml" ]; then
    _TOOLCHAIN_FILE="$HOME/Zeus/rust-toolchain.toml"
fi
if [ -n "$_TOOLCHAIN_FILE" ]; then
    PINNED_CHANNEL=$(awk -F'"' '/^[[:space:]]*channel[[:space:]]*=/ {print $2; exit}' "$_TOOLCHAIN_FILE" 2>/dev/null)
    if [ -n "$PINNED_CHANNEL" ] && [ "$PINNED_CHANNEL" != "stable" ] && [ "$PINNED_CHANNEL" != "nightly" ] && [ "$PINNED_CHANNEL" != "beta" ]; then
        if ! version_ge "$PINNED_CHANNEL" "$RUST_MIN_VERSION"; then
            warn "rust-toolchain.toml pins channel=\"$PINNED_CHANNEL\" (< $RUST_MIN_VERSION). Build WILL fail. Run: git pull origin dev"
        fi
    fi
fi

# Check for git
if command -v git >/dev/null 2>&1; then
    ok "git $(git --version | awk '{print $3}')"
else
    case "$OS" in
        Darwin)  fail "git not found. Run: xcode-select --install" ;;
        FreeBSD) fail "git not found. Run: pkg install git" ;;
        Linux)   fail "git not found. Run: apt install git (or equivalent)" ;;
    esac
fi

# Platform-specific deps
case "$OS" in
    Darwin)
        if ! xcode-select -p >/dev/null 2>&1; then
            warn "Xcode CLT not found — installing"
            xcode-select --install 2>/dev/null || true
            info "Complete Xcode CLT installation and re-run"
            exit 1
        fi
        ok "Xcode CLT"
        if ! command -v cmake >/dev/null 2>&1; then
            info "Installing cmake via Homebrew"
            brew install cmake 2>/dev/null || fail "cmake required. Install: brew install cmake"
        fi
        ok "cmake $(cmake --version 2>/dev/null | head -1 | awk '{print $3}')"
        if ! command -v pkg-config >/dev/null 2>&1; then
            info "Installing pkg-config via Homebrew"
            brew install pkg-config 2>/dev/null || warn "pkg-config install failed — build may still work"
        fi
        ok "pkg-config"
        if ! brew list opus >/dev/null 2>&1; then
            info "Installing opus via Homebrew (required for voice)"
            brew install opus 2>/dev/null || warn "opus install failed -- voice features may not build"
        fi
        ok "opus"
        ;;
    Linux)
        info "Installing Linux build dependencies"
        if command -v apt-get >/dev/null 2>&1; then
            sudo apt-get update -qq
            sudo apt-get install -y build-essential cmake pkg-config libssl-dev protobuf-compiler libasound2-dev libopus-dev 2>/dev/null \
                || warn "Some packages failed — build may still work"
            ok "Linux packages (apt)"
        elif command -v dnf >/dev/null 2>&1; then
            sudo dnf install -y gcc cmake pkgconf openssl-devel protobuf-compiler alsa-lib-devel opus-devel 2>/dev/null \
                || warn "Some packages failed — build may still work"
            ok "Linux packages (dnf)"
        elif command -v pacman >/dev/null 2>&1; then
            sudo pacman -S --noconfirm base-devel cmake pkg-config openssl protobuf alsa-lib opus 2>/dev/null \
                || warn "Some packages failed — build may still work"
            ok "Linux packages (pacman)"
        else
            warn "Unknown package manager — ensure cmake, pkg-config, libssl-dev are installed"
        fi
        ;;
    FreeBSD)
        for pkg in pkgconf sqlite3 cmake; do
            if ! pkg info -e "$pkg" >/dev/null 2>&1; then
                info "Installing $pkg"
                sudo pkg install -y "$pkg" || warn "Failed to install $pkg"
            fi
        done
        ok "FreeBSD packages"
        ;;
esac

# ═══════════════════════════════════════════════════════════════════════════
# Phase 3: Locate/clone source
# ═══════════════════════════════════════════════════════════════════════════
phase "LOCATE SOURCE"

SCRIPT_DIR="$(cd "$(dirname "$0")" 2>/dev/null && pwd)" || SCRIPT_DIR=""
REPO_ROOT=""

if [ -n "$SCRIPT_DIR" ] && [ -f "$SCRIPT_DIR/../Cargo.toml" ]; then
    REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
    ok "Running from repo: $REPO_ROOT"
elif [ -d "$HOME/Zeus" ] && [ -f "$HOME/Zeus/Cargo.toml" ]; then
    REPO_ROOT="$HOME/Zeus"
    ok "Found Zeus at: $REPO_ROOT"
else
    info "Cloning Zeus repository"
    git clone git@github.com:zeuslabai/Zeus.git "$HOME/Zeus" 2>/dev/null \
        || git clone https://github.com/zeuslabai/Zeus.git "$HOME/Zeus"
    REPO_ROOT="$HOME/Zeus"
    ok "Cloned to: $REPO_ROOT"
fi

cd "$REPO_ROOT"
if [ -d .git ]; then
    CURRENT_BRANCH=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo "unknown")
    info "Building from branch '$CURRENT_BRANCH'"
    if git rev-parse --abbrev-ref --symbolic-full-name '@{u}' >/dev/null 2>&1; then
        git pull --ff-only 2>/dev/null && ok "Pulled latest $CURRENT_BRANCH" || warn "Pull failed (offline, or local commits ahead?) — building from local $CURRENT_BRANCH"
    else
        info "No upstream for '$CURRENT_BRANCH' — building as-is"
    fi
fi

# ═══════════════════════════════════════════════════════════════════════════
# Phase 4: Build
# ═══════════════════════════════════════════════════════════════════════════
if $DO_BUILD; then
    phase "BUILD"

    BUILD_JOBS="$CORES"

    # Detect stale binary: exists but older than newest source file
    STALE_BINARY=false
    if [ -f "target/release/zeus" ]; then
        NEWEST_SRC=$(find src crates -name "*.rs" -newer "target/release/zeus" 2>/dev/null | head -1)
        [ -n "$NEWEST_SRC" ] && STALE_BINARY=true && warn "Stale binary detected (source changed since last build) — cleaning"
    fi

    if $CLEAN_BUILD || [ ! -f "target/release/zeus" ] || $STALE_BINARY; then
        if [ -d "target" ]; then
            info "Cleaning build cache for fresh build..."
            cargo clean 2>/dev/null || true
        fi
        # Also clean WebUI artifacts for --with-webui
        if $WITH_WEBUI; then
            rm -rf "$REPO_ROOT/apps/ZeusWeb/dist" "$REPO_ROOT/apps/ZeusWeb/.trunk" 2>/dev/null
        fi
    fi

    BUILD_LOG="$ZEUS_HOME/logs/build.log"
    mkdir -p "$ZEUS_HOME/logs"
    BUILD_START=$(date +%s)

    # Run build in background, show spinner instead of cargo spam
    cargo build --release --locked -j"$BUILD_JOBS" > "$BUILD_LOG" 2>&1 &
    BUILD_PID=$!

    # Live spinner with elapsed time
    SPIN_CHARS='⣾⣽⣻⢿⡿⣟⣯⣷'
    SPIN_I=0
    while kill -0 "$BUILD_PID" 2>/dev/null; do
        NOW=$(date +%s)
        ELAPSED_B=$(( NOW - BUILD_START ))
        if [ $ELAPSED_B -ge 60 ]; then
            ELAPSED_STR=$(printf "%dm%02ds" $((ELAPSED_B / 60)) $((ELAPSED_B % 60)))
        else
            ELAPSED_STR=$(printf "%ds" $ELAPSED_B)
        fi
        SPIN_C=$(printf '%s' "$SPIN_CHARS" | cut -c$((SPIN_I % 8 + 1)))
        # Show last crate being compiled
        LAST_CRATE=$(tail -1 "$BUILD_LOG" 2>/dev/null | sed -n 's/.*Compiling \([^ ]*\).*/\1/p' | tail -1)
        [ -z "$LAST_CRATE" ] && LAST_CRATE="preparing..."
        printf "\r  ${C}%s${N} ${W}Building${N} ${D}(%s)${N} ${D}%s${N}    " "$SPIN_C" "$ELAPSED_STR" "$LAST_CRATE"
        sleep 0.2
        SPIN_I=$((SPIN_I + 1))
    done

    # Check build result
    set +e
    wait "$BUILD_PID"
    BUILD_RC=$?
    set -e
    BUILD_END=$(date +%s)
    BUILD_TIME=$(( BUILD_END - BUILD_START ))
    printf "\n"

    if [ $BUILD_RC -ne 0 ]; then
        printf "\n"
        printf "  ${C}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${N}\n"
        printf "  ${C}${B}  BUILD FAILED${N} ${D}(after %s)${N}\n" "${BUILD_TIME}s"
        printf "  ${C}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${N}\n"
        printf "\n"
        printf "  ${Y}Full log:${N} %s\n" "$BUILD_LOG"
        printf "\n"
        printf "  ${C}── Last 30 lines ──${N}\n"
        tail -30 "$BUILD_LOG" | while IFS= read -r line; do
            printf "  ${D}%s${N}\n" "$line"
        done
        printf "\n"
        printf "  ${Y}Tip:${N} Run ${W}cat %s${N} for the full build log.\n" "$BUILD_LOG"
        printf "\n"
        exit 1
    fi

    if [ $BUILD_TIME -ge 60 ]; then
        BUILD_TIME_STR=$(printf "%dm%02ds" $((BUILD_TIME / 60)) $((BUILD_TIME % 60)))
    else
        BUILD_TIME_STR="${BUILD_TIME}s"
    fi

    printf "\r  ${G}✓${N} ${W}Build complete${N} ${D}(%s)${N}                              \n" "$BUILD_TIME_STR"

    BINARY="$REPO_ROOT/target/release/zeus"
    if [ ! -f "$BINARY" ]; then
        fail "Build succeeded but binary not found at $BINARY"
    fi

    BINARY_SIZE=$(du -h "$BINARY" | awk '{print $1}')
    summary_row "Binary:" "$BINARY_SIZE"
    summary_row "Log:" "$BUILD_LOG"

    # On the --update path, defer the binary install to SAFE UPDATE (post-stop
    # cp at the update fast path below). cp onto the *live* binary raises
    # ETXTBSY ("Text file busy") on FreeBSD and has the same busy-text exposure
    # on Linux — the gateway is still running at this point. SAFE UPDATE stops
    # the gateway first, then does its own cp + codesign, so skipping here is
    # both safe and required for a live-box update.
    if $UPDATE_ONLY; then
        info "Skipping build-phase install (--update): SAFE UPDATE swaps the binary after gateway stop"
    else

    # Create install directory if it doesn't exist (fresh Apple Silicon Macs lack /usr/local/bin)
    if [ ! -d "$INSTALL_DIR" ]; then
        info "Creating $INSTALL_DIR (not present on this system)"
        sudo mkdir -p "$INSTALL_DIR"
    fi

    info "Installing to $INSTALL_DIR/zeus"
    if [ -w "$INSTALL_DIR" ]; then
        cp "$BINARY" "$INSTALL_DIR/zeus"
        chmod +x "$INSTALL_DIR/zeus"
    else
        sudo cp "$BINARY" "$INSTALL_DIR/zeus"
        sudo chmod +x "$INSTALL_DIR/zeus"
    fi

    # Codesign on macOS with stable identifier + entitlements.
    # ZEUS_CODESIGN_IDENTITY env var: set to your Developer ID for persistent TCC grants.
    # Falls back to ad-hoc (-) if not set. Ad-hoc causes TCC re-prompts on rebuild.
    if [ "$OS" = "Darwin" ]; then
        sudo xattr -cr "$INSTALL_DIR/zeus" 2>/dev/null || true
        SIGN_ID="${ZEUS_CODESIGN_IDENTITY:--}"
        ENTITLEMENTS="$REPO_ROOT/packaging/macos/entitlements.plist"
        # --deep signs nested code (libs, frameworks, helpers) — required for Rust binaries
        # with embedded resources. Without --deep, Gatekeeper rejects the binary on launch.
        if [ -f "$ENTITLEMENTS" ]; then
            sudo codesign --force --deep --sign "$SIGN_ID" --identifier com.zeus.agent --entitlements "$ENTITLEMENTS" "$INSTALL_DIR/zeus" \
                && ok "Codesigned (com.zeus.agent, identity: $SIGN_ID, --deep)" \
                || warn "Codesign failed (non-fatal) — run 'sudo codesign --force --deep --sign - $INSTALL_DIR/zeus' manually to diagnose"
        else
            sudo codesign --force --deep --sign "$SIGN_ID" --identifier com.zeus.agent "$INSTALL_DIR/zeus" \
                && ok "Codesigned (com.zeus.agent, identity: $SIGN_ID, --deep)" \
                || warn "Codesign failed (non-fatal) — run 'sudo codesign --force --deep --sign - $INSTALL_DIR/zeus' manually to diagnose"
        fi
        # Explicit quarantine removal AFTER codesign — covers macOS versions where
        # xattr -cr above didn't clear com.apple.quarantine, or where signing
        # didn't replace the quarantine attribute.
        sudo xattr -dr com.apple.quarantine "$INSTALL_DIR/zeus" 2>/dev/null || true
        if [ "$SIGN_ID" = "-" ]; then
            warn "Using ad-hoc signing — TCC will re-prompt on rebuild"
            info "Set ZEUS_CODESIGN_IDENTITY for persistent permissions"
        fi
    fi

    ok "Installed: $INSTALL_DIR/zeus ($BINARY_SIZE)"

    fi  # end !UPDATE_ONLY build-phase install (ETXTBSY guard)

    # ── macOS Full Disk Access reminder ──
    if [ "$(uname)" = "Darwin" ]; then
        info "Grant Full Disk Access for autonomous operation:"
        info "  System Settings → Privacy & Security → Full Disk Access → add zeus"
        info "  (one-time setup, persists across rebuilds)"
    fi

    # ── WebUI Build (optional) ──
    if $WITH_WEBUI; then
        info "Building WebUI (trunk + WASM)..."

        if ! rustup target list --installed 2>/dev/null | grep -q wasm32-unknown-unknown; then
            rustup target add wasm32-unknown-unknown
        fi
        command -v trunk >/dev/null 2>&1 || cargo install trunk

        # Derive the wasm-bindgen version from the WebUI's resolved Cargo.lock so the
        # CLI matches the library the build links against (the lib floats on a "0.2"
        # caret, so a bare `cargo install wasm-bindgen-cli` can skew newer → mismatch).
        WEBUI_DIR="$REPO_ROOT/apps/ZeusWeb"
        WASM_BINDGEN_VER=""
        if [ -d "$WEBUI_DIR" ]; then
            # Ensure the lock exists before we grep it — on a clean box it may not be
            # resolved yet, and a missing lock would silently yield an empty version.
            if [ ! -f "$WEBUI_DIR/Cargo.lock" ]; then
                ( cd "$WEBUI_DIR" && cargo generate-lockfile 2>/dev/null ) || true
            fi
            if [ -f "$WEBUI_DIR/Cargo.lock" ]; then
                # bash-3.2 / POSIX-safe extraction (install.sh runs on Mac seats too).
                WASM_BINDGEN_VER=$(grep -A1 '^name = "wasm-bindgen"$' "$WEBUI_DIR/Cargo.lock" \
                    | grep '^version' | head -1 | awk -F'"' '{print $2}')
            fi
        fi

        if [ -n "$WASM_BINDGEN_VER" ]; then
            # Pin the CLI to the resolved library version — deterministic, no lockstep.
            INSTALLED_VER=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}' || echo "")
            if [ "$INSTALLED_VER" != "$WASM_BINDGEN_VER" ]; then
                info "Installing wasm-bindgen-cli ${WASM_BINDGEN_VER} (pinned from Cargo.lock)"
                cargo install wasm-bindgen-cli --version "$WASM_BINDGEN_VER" --force
            fi
        else
            warn "Could not derive wasm-bindgen version from Cargo.lock; installing unpinned"
            command -v wasm-bindgen >/dev/null 2>&1 || cargo install wasm-bindgen-cli
        fi

        # Ensure trunk can find wasm-bindgen (FreeBSD has no prebuilt download).
        # Trunk looks in its own cache dir — symlink the cargo-installed binary there.
        WASM_BINDGEN_PATH=$(command -v wasm-bindgen 2>/dev/null || echo "")
        if [ -n "$WASM_BINDGEN_PATH" ]; then
            # Name the cache dir after the version trunk will actually look for — the
            # resolved library version when known, else the installed CLI's version.
            if [ -z "$WASM_BINDGEN_VER" ]; then
                WASM_BINDGEN_VER=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}' || echo "0.2.100")
            fi
            TRUNK_CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/trunk"
            TRUNK_BINDGEN_DIR="$TRUNK_CACHE/wasm-bindgen-${WASM_BINDGEN_VER}"
            if [ ! -f "$TRUNK_BINDGEN_DIR/wasm-bindgen" ]; then
                mkdir -p "$TRUNK_BINDGEN_DIR"
                ln -sf "$WASM_BINDGEN_PATH" "$TRUNK_BINDGEN_DIR/wasm-bindgen"
                ok "Linked wasm-bindgen to trunk cache"
            fi
        fi
        if ! command -v wasm-opt >/dev/null 2>&1; then
            case "$OS" in
                FreeBSD) sudo pkg install -y binaryen 2>/dev/null ;;
                Darwin)  brew install binaryen 2>/dev/null ;;
                Linux)   sudo apt-get install -y binaryen 2>/dev/null ;;
            esac
        fi

        WEBUI_DIR="$REPO_ROOT/apps/ZeusWeb"
        if [ -d "$WEBUI_DIR" ]; then
            cd "$WEBUI_DIR"
            if trunk build --release; then
                mkdir -p "$ZEUS_HOME/web"
                cp -r "$WEBUI_DIR/dist/"* "$ZEUS_HOME/web/"
                ok "WebUI installed to $ZEUS_HOME/web/"
                # Get local IP for URL
                LOCAL_IP=$(ipconfig getifaddr en0 2>/dev/null || hostname -I 2>/dev/null | awk '{print $1}' || echo "localhost")
                WEBUI_URL="http://${LOCAL_IP}:8081"
                WEBUI_SIZE=$(du -sh "$ZEUS_HOME/web/" 2>/dev/null | awk '{print $1}')
                printf "\n"
                printf "  ${CS}┌─────────────────────────────────────────────────┐${N}\n"
                printf "  ${CS}│${N}                                                 ${CS}│${N}\n"
                printf "  ${CS}│${N}   ${B}${CS}⚡ WebUI build complete${N}                        ${CS}│${N}\n"
                printf "  ${CS}│${N}                                                 ${CS}│${N}\n"
                printf "  ${CS}│${N}   ${D}Files:${N}     $ZEUS_HOME/web/                ${CS}│${N}\n"
                printf "  ${CS}│${N}   ${D}Size:${N}      $WEBUI_SIZE                             ${CS}│${N}\n"
                printf "  ${CS}│${N}   ${D}URL:${N}       ${B}${WEBUI_URL}${N}              ${CS}│${N}\n"
                printf "  ${CS}│${N}                                                 ${CS}│${N}\n"
                printf "  ${CS}│${N}   Open in browser after starting gateway.       ${CS}│${N}\n"
                printf "  ${CS}│${N}                                                 ${CS}│${N}\n"
                printf "  ${CS}└─────────────────────────────────────────────────┘${N}\n"
                printf "\n"
            else
                warn "WebUI build failed (non-fatal)"
            fi
            cd "$REPO_ROOT"
        fi
    fi

    # Detect stale binaries
    WHICH_ZEUS=$(which zeus 2>/dev/null || true)
    if [ -n "$WHICH_ZEUS" ] && [ "$WHICH_ZEUS" != "$INSTALL_DIR/zeus" ]; then
        warn "Stale zeus at $WHICH_ZEUS (shadows $INSTALL_DIR/zeus)"
    fi
else
    phase "SKIP BUILD"
    command -v zeus >/dev/null 2>&1 || fail "zeus binary not found and --no-build specified"
    ok "Using existing: $(which zeus)"
fi

# ═══════════════════════════════════════════════════════════════════════════
# --update fast path
# ═══════════════════════════════════════════════════════════════════════════
if $UPDATE_ONLY; then
    phase "SAFE UPDATE"
    BINARY="$REPO_ROOT/target/release/zeus"
    [ ! -f "$BINARY" ] && fail "No binary found — run without --update first"

    # Stop the gateway and WAIT for it to actually exit before swapping the
    # binary. A fixed 'sleep 2' loses the race against graceful shutdown
    # (channel disconnects, state flush can take >2s) — cp onto a still-running
    # binary raises ETXTBSY on FreeBSD. Poll up to 30s, then escalate to KILL.
    #
    # FIRST stop the managing supervisor, so launchd KeepAlive / systemd
    # Restart=always / rc.d can't RESPAWN the old binary mid-kill. That race is
    # what leaves a respawned-old + freshly-bootstrapped-new pair = DUPLICATE
    # gateways, which collectively hammer the API → 429 rate-limit (looks like
    # "out of tokens" but it's stray instances). With the supervisor stopped,
    # the TERM→poll→KILL below reaps cleanly and the restart section below
    # bootstraps exactly one.
    case "$OS" in
        Darwin)  sudo launchctl bootout system /Library/LaunchDaemons/com.zeus.gateway.plist 2>/dev/null || true ;;
        FreeBSD) sudo service zeus_gateway stop 2>/dev/null || true ;;
        Linux)   systemctl --user stop zeus-gateway 2>/dev/null || sudo systemctl stop zeus-gateway 2>/dev/null || true ;;
    esac
    # Sweep stray serve/daemon instances too, not just 'gateway'.
    pkill -f 'zeus gateway' 2>/dev/null || true
    pkill -f 'zeus serve'   2>/dev/null || true
    pkill -f 'zeus daemon'  2>/dev/null || true
    WAIT_I=0
    while pgrep -f 'zeus gateway' >/dev/null 2>&1; do
        WAIT_I=$((WAIT_I + 1))
        if [ $WAIT_I -ge 30 ]; then
            warn "Gateway still running after 30s — escalating to SIGKILL"
            pkill -9 -f 'zeus gateway' 2>/dev/null || true
            sleep 1
            break
        fi
        sleep 1
    done
    pgrep -f 'zeus gateway' >/dev/null 2>&1 && fail "Gateway refuses to exit — cannot swap binary (ETXTBSY)"
    ok "Gateway stopped (waited ${WAIT_I}s)"

    sudo cp "$BINARY" "$INSTALL_DIR/zeus"
    if [ "$OS" = "Darwin" ]; then
        sudo xattr -cr "$INSTALL_DIR/zeus" 2>/dev/null || true
        SIGN_ID="${ZEUS_CODESIGN_IDENTITY:--}"
        ENTITLEMENTS="$REPO_ROOT/packaging/macos/entitlements.plist"
        # --deep signs nested code; quarantine cleared explicitly after.
        if [ -f "$ENTITLEMENTS" ]; then
            sudo codesign --force --deep --sign "$SIGN_ID" --identifier com.zeus.agent --entitlements "$ENTITLEMENTS" "$INSTALL_DIR/zeus" || true
        else
            sudo codesign --force --deep --sign "$SIGN_ID" --identifier com.zeus.agent "$INSTALL_DIR/zeus" || true
        fi
        sudo xattr -dr com.apple.quarantine "$INSTALL_DIR/zeus" 2>/dev/null || true
    fi
    ok "Binary installed"

    rm -f "$ZEUS_HOME/gateway.pid"
    case "$OS" in
        Darwin)
            # System LaunchDaemon: install (or refresh) and bootstrap into system domain.
            # Survives logout/reboot — no GUI session required.
            if install_launchd_plist "$REPO_ROOT" "$INSTALL_DIR" "$ZEUS_HOME/logs" >/dev/null 2>&1; then
                ok "Gateway restarted via launchd"
            else
                warn "launchd plist install failed — gateway may not auto-start on boot"
            fi

            # newsyslog rotation config: prevents unbounded log growth.
            # See install_newsyslog_conf() doc-comment for threshold details.
            if install_newsyslog_conf "$REPO_ROOT" "$ZEUS_HOME/logs" >/dev/null 2>&1; then
                ok "Log rotation configured (newsyslog)"
            else
                warn "newsyslog config install failed — logs may grow unbounded"
            fi

            # Cleanup: delete stale 0-byte legacy log files that predate
            # the newsyslog-managed naming convention. Harmless on fresh installs.
            for stale in "$ZEUS_HOME/logs"/gateway.out.log "$ZEUS_HOME/logs"/gateway.err.log; do
                if [ -f "$stale" ] && [ ! -s "$stale" ]; then
                    rm -f "$stale" && info "Removed stale 0-byte legacy log: $stale"
                fi
            done

            # Restart the service so it picks up any plist changes.
            if sudo launchctl bootstrap system /Library/LaunchDaemons/com.zeus.gateway.plist 2>/dev/null; then
                ok "Gateway service bootstrapped"
            elif sudo launchctl load -w /Library/LaunchDaemons/com.zeus.gateway.plist 2>/dev/null; then
                ok "Gateway service loaded"
            else
                warn "launchd bootstrap failed — falling back to nohup"
                nohup zeus gateway > "$ZEUS_HOME/logs/gateway.out.log" 2> "$ZEUS_HOME/logs/gateway.err.log" &
                ok "Gateway started via nohup"
            fi ;;
        FreeBSD)
            # Register/refresh the rc.d service + rc.conf enable before restart.
            # `zeus daemon install` is idempotent and stale-aware: it rewrites
            # /usr/local/etc/rc.d/zeus_gateway whenever the content drifted
            # (old pidfile semantics, hardcoded user, moved binary), so we run
            # it unconditionally — a stale script that merely *exists* was the
            # #211 silent-failure mode. Errors are surfaced, not swallowed.
            info "Registering/refreshing zeus_gateway rc.d service..."
            if daemon_install_out=$(sudo zeus daemon install 2>&1); then
                ok "rc.d service registered + enabled in rc.conf"
            else
                warn "zeus daemon install failed — service won't be registered:"
                printf '%s\n' "$daemon_install_out" | sed 's/^/    /'
            fi
            # NOTE: do NOT capture this with $(...). The rc.d script spawns the
            # gateway via /usr/sbin/daemon, which inherits the pipe's write fd;
            # the $() read then blocks until the *gateway* exits — install.sh
            # --update hangs forever at this step (#223). Redirect to a temp
            # file instead: the fd is a regular file, nothing waits on EOF.
            restart_log=$(mktemp -t zeus_restart) || restart_log="/tmp/zeus_restart.$$"
            if sudo service zeus_gateway restart >"$restart_log" 2>&1; then
                ok "Gateway restarted via rc.d service"
            else
                warn "service zeus_gateway restart failed:"
                sed 's/^/    /' "$restart_log"
                warn "Falling back to nohup"
                nohup zeus gateway > "$ZEUS_HOME/logs/gateway.out.log" 2> "$ZEUS_HOME/logs/gateway.err.log" &
                ok "Gateway started via nohup (run 'sudo zeus daemon install' to register rc.d service)"
            fi
            rm -f "$restart_log" ;;
        Linux)
            # Ensure systemd user unit is registered before service restart.
            # `zeus daemon install` writes ~/.config/systemd/user/zeus-gateway.service
            # and runs `systemctl --user daemon-reload`. Skip if already installed.
            if command -v systemctl >/dev/null 2>&1 && [ ! -f "$HOME/.config/systemd/user/zeus-gateway.service" ]; then
                info "Registering zeus-gateway systemd user unit..."
                zeus daemon install 2>/dev/null && ok "systemd user unit installed" \
                    || warn "zeus daemon install failed — service won't be registered"
            fi
            if command -v systemctl >/dev/null 2>&1 && [ -f "$HOME/.config/systemd/user/zeus-gateway.service" ]; then
                systemctl --user daemon-reload 2>/dev/null
                systemctl --user enable zeus-gateway 2>/dev/null
                if systemctl --user restart zeus-gateway 2>/dev/null; then
                    ok "Gateway restarted via systemd user unit"
                else
                    warn "systemctl --user restart failed — falling back to nohup"
                    nohup zeus gateway > "$ZEUS_HOME/logs/gateway.out.log" 2> "$ZEUS_HOME/logs/gateway.err.log" &
                    ok "Gateway started via nohup (run 'zeus daemon install' to register systemd unit)"
                fi
            else
                nohup zeus gateway > "$ZEUS_HOME/logs/gateway.out.log" 2> "$ZEUS_HOME/logs/gateway.err.log" &
                ok "Gateway started via nohup"
            fi ;;
        *)
            nohup zeus gateway > "$ZEUS_HOME/logs/gateway.out.log" 2> "$ZEUS_HOME/logs/gateway.err.log" &
            ok "Gateway started via nohup" ;;
    esac

    # Verify the gateway actually came back after the restart. A silent failure
    # here is the worst outcome of --update: the old process was killed, the new
    # one never bound, and the box is left with NO gateway. Poll the health
    # endpoint for up to ~30s before declaring a hard failure so the operator
    # gets a clear, actionable signal instead of a quiet warning.
    GW_UP=false
    for _attempt in $(seq 1 15); do
        if curl -s --max-time 5 http://127.0.0.1:8080/health 2>/dev/null | grep -q '"ok"'; then
            GW_UP=true
            break
        fi
        sleep 2
    done
    if $GW_UP; then
        ok "Health check: gateway is up (http://127.0.0.1:8080/health)"
    elif pgrep -f 'zeus gateway' >/dev/null 2>&1; then
        warn "Gateway process is running but /health did not return ok within 30s — check $ZEUS_HOME/logs/gateway.err.log"
    else
        warn "Gateway did NOT come back after --update (no process, /health unreachable)."
        warn "The update copied the new binary but the service failed to start."
        warn "Inspect: $ZEUS_HOME/logs/gateway.err.log"
        case "$OS" in
            Darwin)  warn "Retry: sudo launchctl bootstrap system /Library/LaunchDaemons/com.zeus.gateway.plist" ;;
            FreeBSD) warn "Retry: sudo service zeus_gateway restart" ;;
            Linux)   warn "Retry: systemctl --user restart zeus-gateway" ;;
        esac
        fail "Update left the gateway down — see remediation above."
    fi

    # Freshness guard (#125): health says "gateway is up", but a gateway that
    # came up reading a STALE in-memory config (the #123 failure class — process
    # survived a re-onboard / config rewrite without restarting) still passes
    # /health while serving the wrong credentials. Assert the gateway's start
    # time is AT OR AFTER the last config.toml write so post-onboard installs
    # can't silently leave a stale process bound. Non-fatal: warn + remediate.
    FRESHNESS_GUARD="$REPO_ROOT/scripts/gateway_freshness_check.sh"
    if [ -f "$FRESHNESS_GUARD" ]; then
        if FRESH_OUT=$(ZEUS_CONFIG="$ZEUS_HOME/config.toml" sh "$FRESHNESS_GUARD" 2>&1); then
            ok "Config freshness: gateway is running the current config"
        else
            warn "Config freshness check failed: $FRESH_OUT"
            warn "The gateway is up but may be serving a STALE config (pre-onboard state)."
            case "$OS" in
                Darwin)  warn "Restart: sudo launchctl bootstrap system /Library/LaunchDaemons/com.zeus.gateway.plist" ;;
                FreeBSD) warn "Restart: sudo service zeus_gateway restart" ;;
                Linux)   warn "Restart: systemctl --user restart zeus-gateway" ;;
                *)       warn "Restart the zeus gateway process to load the current config." ;;
            esac
        fi
    fi

    if $WITH_IDENTITY; then
        DEPLOY_IDENTITY="$REPO_ROOT/scripts/deploy-identity.sh"
        if [ -f "$DEPLOY_IDENTITY" ]; then
            info "Refreshing workspace identity templates..."
            chmod +x "$DEPLOY_IDENTITY" 2>/dev/null || true
            if "$DEPLOY_IDENTITY" --force; then
                ok "Workspace identity templates refreshed"
            else
                warn "deploy-identity.sh exited non-zero — workspace may be partially refreshed"
            fi
        else
            warn "deploy-identity.sh not found at $DEPLOY_IDENTITY — skipping identity refresh"
        fi
    fi

    printf "\n"
    box_top
    box_mid "$(printf "${G}Zeus updated successfully.${N}  ($(timer_elapsed))")"
    box_bot
    printf "\n"
    exit 0
fi

# --mcp-only stops here for the gateway-oriented phases: directory setup,
# skills/personas, config onboarding, agent identity, and daemon-service install
# are all gateway concerns. The MCP server is just `zeus mcp` on the installed
# binary, so we skip straight to the MCP Configuration block below.
if ! $MCP_ONLY; then

# ═══════════════════════════════════════════════════════════════════════════
# Phase 5: Setup directories
# ═══════════════════════════════════════════════════════════════════════════
phase "SETUP DIRECTORIES"

mkdir -p "$ZEUS_HOME/workspace/memory" "$ZEUS_HOME/workspace/daily" \
         "$ZEUS_HOME/workspace/goals" "$ZEUS_HOME/sessions" "$ZEUS_HOME/logs" \
         "$ZEUS_HOME/completions"
chmod 0700 "$ZEUS_HOME/workspace"
ok "Directory structure: $ZEUS_HOME"

# Clone persona library
AGENTS_DIR="$ZEUS_HOME/agents"
if [ -d "$AGENTS_DIR/.git" ]; then
    git -C "$AGENTS_DIR" pull --ff-only -q 2>/dev/null && ok "Persona library updated" || warn "Update failed (offline?)"
elif command -v git >/dev/null 2>&1; then
    info "Fetching persona library..."
    git clone --depth=1 -q https://github.com/anthropics/skills.git "$AGENTS_DIR" 2>/dev/null \
        && ok "Persona library installed" \
        || warn "Could not fetch personas (offline?)"
fi

# Workspace stubs
for stub_file in SOUL.md AGENTS.md HEARTBEAT.md USER.md TOOLS.md; do
    [ ! -f "$ZEUS_HOME/workspace/$stub_file" ] && echo "# $stub_file — Run 'zeus onboard' to configure" > "$ZEUS_HOME/workspace/$stub_file"
done
[ ! -f "$ZEUS_HOME/workspace/memory/MEMORY.md" ] && echo "# Memory" > "$ZEUS_HOME/workspace/memory/MEMORY.md"
ok "Workspace stubs ready"

# ═══════════════════════════════════════════════════════════════════════════
# Phase 6: Skills & Personas
# ═══════════════════════════════════════════════════════════════════════════
phase "SKILLS & PERSONAS"

CLAUDE_AGENTS_DIR="$HOME/.claude/agents"
ZEUS_SKILLS_DIR="$ZEUS_HOME/skills"
ZEUS_WS_SKILLS_DIR="$ZEUS_HOME/workspace/skills"
mkdir -p "$ZEUS_SKILLS_DIR" "$ZEUS_WS_SKILLS_DIR"

# Seed the persona library to the canonical runtime path. The TUI onboarding
# loader (zeus-tui screens/agent.rs load_personas) searches
# $ZEUS_HOME/personalities — seed it from the repo so every box (including a
# bare deployed titan with no repo clone reading this dir later) surfaces all
# personas during onboarding. Idempotent: overwrites/updates on re-install.
if [ -d "$REPO_ROOT/personalities" ]; then
    ZEUS_PERSONAS_DIR="$ZEUS_HOME/personalities"
    mkdir -p "$ZEUS_PERSONAS_DIR"
    cp -R "$REPO_ROOT/personalities/." "$ZEUS_PERSONAS_DIR/" 2>/dev/null \
        && ok "Persona library seeded → $ZEUS_PERSONAS_DIR" \
        || warn "Could not seed persona library"
fi
SKILLS_TMP="$ZEUS_HOME/.skills_tmp"
ANTHROPIC_SKILLS_REPO="https://github.com/anthropics/skills.git"

# Clone/update Anthropic skills
if [ -d "$SKILLS_TMP/.git" ]; then
    git -C "$SKILLS_TMP" pull --ff-only --quiet 2>/dev/null && ok "Anthropic skills updated" || warn "Update failed"
else
    rm -rf "$SKILLS_TMP"
    git clone --depth=1 --quiet "$ANTHROPIC_SKILLS_REPO" "$SKILLS_TMP" 2>/dev/null \
        && ok "Anthropic skills cloned" \
        || { warn "Could not clone skills (offline?)"; SKILLS_TMP=""; }
fi

# Community skills
COMMUNITY_SKILLS_DIR="$ZEUS_HOME/.community_skills"
mkdir -p "$COMMUNITY_SKILLS_DIR"
COMMUNITY_REPOS="
coreyhaines31/marketingskills
kepano/obsidian-skills
199-biotechnologies/claude-deep-research-skill
AgriciDaniel/claude-seo
obra/superpowers
"
COMMUNITY_COUNT=0
for repo in $COMMUNITY_REPOS; do
    repo_name=$(echo "$repo" | cut -d/ -f2)
    repo_dir="$COMMUNITY_SKILLS_DIR/$repo_name"
    if [ -d "$repo_dir/.git" ]; then
        git -C "$repo_dir" pull --ff-only --quiet 2>/dev/null
    else
        git clone --depth=1 --quiet "https://github.com/$repo.git" "$repo_dir" 2>/dev/null && COMMUNITY_COUNT=$((COMMUNITY_COUNT + 1))
    fi
done
[ $COMMUNITY_COUNT -gt 0 ] && ok "$COMMUNITY_COUNT community skills installed"

# Symlink community skills
for repo_dir in "$COMMUNITY_SKILLS_DIR"/*/; do
    repo_name=$(basename "$repo_dir")
    if [ -d "$repo_dir/skills" ]; then
        for skill_dir in "$repo_dir/skills"/*/; do
            skill_name=$(basename "$skill_dir")
            [ -f "$skill_dir/SKILL.md" ] && [ ! -e "$ZEUS_SKILLS_DIR/$skill_name" ] && ln -sf "$skill_dir" "$ZEUS_SKILLS_DIR/$skill_name" 2>/dev/null
        done
    elif [ -f "$repo_dir/SKILL.md" ] && [ ! -e "$ZEUS_SKILLS_DIR/$repo_name" ]; then
        ln -sf "$repo_dir" "$ZEUS_SKILLS_DIR/$repo_name" 2>/dev/null
    fi
done

# Remove skills that cause robotic agent behavior when triggered by keyword matching.
# These inject rigid verification/TDD rules on every message including casual chat.
for bad_skill in verification-before-completion test-driven-development systematic-debugging verify tdd; do
    rm -f "$ZEUS_SKILLS_DIR/$bad_skill" 2>/dev/null
done

# MCP servers
MCP_SERVERS_DIR="$ZEUS_HOME/.mcp_servers"
mkdir -p "$MCP_SERVERS_DIR"
MCP_REPOS="
tavily-ai/tavily-mcp
upstash/context7
executeautomation/mcp-playwright
"
for repo in $MCP_REPOS; do
    repo_name=$(echo "$repo" | cut -d/ -f2)
    repo_dir="$MCP_SERVERS_DIR/$repo_name"
    if [ -d "$repo_dir/.git" ]; then
        git -C "$repo_dir" pull --ff-only --quiet 2>/dev/null
    else
        git clone --depth=1 --quiet "https://github.com/$repo.git" "$repo_dir" 2>/dev/null && ok "MCP: $repo_name"
    fi
done

# ── Interactive: MCP server selection ──
if [ -d "$MCP_SERVERS_DIR" ] && [ -t 0 ]; then
    printf "\n  ${CS}→${N} ${W}Enable optional MCP servers?${N}\n\n"
    MCP_IDX=0
    MCP_NAMES=""
    for repo_dir in "$MCP_SERVERS_DIR"/*/; do
        [ -d "$repo_dir" ] || continue
        name=$(basename "$repo_dir")
        case "$name" in
            tavily-mcp)     desc="AI search (needs TAVILY_API_KEY)" ;;
            context7)       desc="Live library docs (free)" ;;
            mcp-playwright) desc="Browser automation (free)" ;;
            *)              desc="MCP server" ;;
        esac
        MCP_IDX=$((MCP_IDX + 1))
        printf "    ${C}%d${N}${D})${N} ${W}%s${N} ${D}— %s${N}\n" "$MCP_IDX" "$name" "$desc"
        MCP_NAMES="$MCP_NAMES $name"
    done
    printf "\n  ${D}Enter numbers (e.g. 1 2 3), all, or none${N} ${C}[none]:${N} "
    read -r MCP_CHOICE 2>/dev/null || MCP_CHOICE="none"
    MCP_CHOICE="${MCP_CHOICE:-none}"

    if [ "$MCP_CHOICE" != "none" ]; then
        SETTINGS_JSON="$HOME/.claude/settings.json"
        [ "$MCP_CHOICE" = "all" ] && MCP_CHOICE="1 2 3 4 5 6 7 8 9"

        # Detect python3
        _P3=""
        for p in python3 python3.13 python3.12 python3.11 python3.10 python; do
            command -v "$p" >/dev/null 2>&1 && { _P3="$p"; break; }
        done

        for idx in $MCP_CHOICE; do
            name=$(echo $MCP_NAMES | tr " " "\n" | sed -n "${idx}p")
            [ -z "$name" ] && continue

            case "$name" in
                tavily-mcp)
                    printf "  ${CS}→${N} Tavily API key (tavily.com, Enter to skip): "
                    read -r TAVILY_KEY 2>/dev/null || TAVILY_KEY=""
                    if [ -n "$TAVILY_KEY" ] && [ -n "$_P3" ]; then
                        "$_P3" -c "
import json, os
path = '$SETTINGS_JSON'
data = json.load(open(path)) if os.path.exists(path) else {}
data.setdefault('mcpServers', {})
data['mcpServers']['tavily'] = {
    'command': 'npx',
    'args': ['-y', 'tavily-mcp@latest'],
    'env': {'TAVILY_API_KEY': '$TAVILY_KEY'}
}
with open(path, 'w') as f:
    json.dump(data, f, indent=2)
    f.write('\n')
" 2>/dev/null && ok "Tavily MCP enabled"
                    else
                        info "Skipped Tavily (no API key)"
                    fi
                    ;;
                context7)
                    if [ -n "$_P3" ]; then
                        "$_P3" -c "
import json, os
path = '$SETTINGS_JSON'
data = json.load(open(path)) if os.path.exists(path) else {}
data.setdefault('mcpServers', {})
data['mcpServers']['context7'] = {
    'command': 'npx',
    'args': ['-y', '@upstash/context7-mcp@latest']
}
with open(path, 'w') as f:
    json.dump(data, f, indent=2)
    f.write('\n')
" 2>/dev/null && ok "Context7 MCP enabled"
                    fi
                    ;;
                mcp-playwright)
                    if [ -n "$_P3" ]; then
                        "$_P3" -c "
import json, os
path = '$SETTINGS_JSON'
data = json.load(open(path)) if os.path.exists(path) else {}
data.setdefault('mcpServers', {})
data['mcpServers']['playwright'] = {
    'command': 'npx',
    'args': ['-y', '@anthropic/mcp-playwright@latest']
}
with open(path, 'w') as f:
    json.dump(data, f, indent=2)
    f.write('\n')
" 2>/dev/null && ok "Playwright MCP enabled"
                    fi
                    ;;
            esac
        done
    else
        info "No optional MCP servers enabled (add later in ~/.claude/settings.json)"
    fi
fi

# ── Interactive: Skill selection ──
if [ -n "$SKILLS_TMP" ] && [ -d "$SKILLS_TMP" ] && [ -t 0 ]; then
    AVAILABLE_SKILLS=""
    [ -d "$SKILLS_TMP/skills" ] && AVAILABLE_SKILLS=$(ls "$SKILLS_TMP/skills/" 2>/dev/null | grep -v '^\.' || true)
    [ -z "$AVAILABLE_SKILLS" ] && AVAILABLE_SKILLS=$(ls "$SKILLS_TMP/" 2>/dev/null | grep -v '^\.\|README\|LICENSE\|\.git' || true)

    if [ -n "$AVAILABLE_SKILLS" ]; then
        printf "\n  ${C}${B}Available Skills:${N}\n"
        i=1
        for skill in $AVAILABLE_SKILLS; do
            printf "    ${C}%2d${N}${D}.${N} ${W}%s${N}\n" "$i" "$skill"
            i=$((i + 1))
        done
        printf "\n  ${D}Enter numbers, all, or none${N} ${C}[all]:${N} "
        read -r SKILL_CHOICE 2>/dev/null || SKILL_CHOICE="all"
        SKILL_CHOICE="${SKILL_CHOICE:-all}"

        mkdir -p "$CLAUDE_AGENTS_DIR"
        if [ "$SKILL_CHOICE" = "none" ]; then
            info "Skipping skills"
        elif [ "$SKILL_CHOICE" = "all" ]; then
            SKILL_SRC="${SKILLS_TMP}/skills"
            [ -d "$SKILL_SRC" ] || SKILL_SRC="$SKILLS_TMP"
            cp -r "$SKILL_SRC/"* "$CLAUDE_AGENTS_DIR/" 2>/dev/null || true
            cp -r "$SKILL_SRC/"* "$ZEUS_SKILLS_DIR/" 2>/dev/null || true
            cp -r "$SKILL_SRC/"* "$ZEUS_WS_SKILLS_DIR/" 2>/dev/null || true
            ok "All skills installed"
        else
            SKILL_LIST="$AVAILABLE_SKILLS"
            for num in $SKILL_CHOICE; do
                CHOSEN=$(echo "$SKILL_LIST" | awk "NR==$num")
                if [ -n "$CHOSEN" ]; then
                    SKILL_SRC="${SKILLS_TMP}/skills/$CHOSEN"
                    [ -d "$SKILL_SRC" ] || SKILL_SRC="$SKILLS_TMP/$CHOSEN"
                    if [ -d "$SKILL_SRC" ]; then
                        cp -r "$SKILL_SRC" "$CLAUDE_AGENTS_DIR/$CHOSEN" 2>/dev/null || true
                        cp -r "$SKILL_SRC" "$ZEUS_SKILLS_DIR/$CHOSEN" 2>/dev/null || true
                        cp -r "$SKILL_SRC" "$ZEUS_WS_SKILLS_DIR/$CHOSEN" 2>/dev/null || true
                        ok "Skill: $CHOSEN"
                    fi
                fi
            done
        fi
    fi
fi

# ── #224: Seed workspace/skills on fresh installs ──
# Non-interactive installs (curl | sh, no TTY) skip the selection block above,
# leaving workspace/skills/ empty — which makes the WebUI onboarding skills
# step a blank page. If it's still empty here, seed it with the full skill set.
if [ -z "$(ls -A "$ZEUS_WS_SKILLS_DIR" 2>/dev/null)" ]; then
    if [ -n "$SKILLS_TMP" ] && [ -d "$SKILLS_TMP" ]; then
        SKILL_SEED_SRC="$SKILLS_TMP/skills"
        [ -d "$SKILL_SEED_SRC" ] || SKILL_SEED_SRC="$SKILLS_TMP"
        cp -r "$SKILL_SEED_SRC/"* "$ZEUS_WS_SKILLS_DIR/" 2>/dev/null || true
    elif [ -n "$(ls -A "$ZEUS_SKILLS_DIR" 2>/dev/null)" ]; then
        # Offline fallback: mirror the global skill library
        cp -rL "$ZEUS_SKILLS_DIR/"* "$ZEUS_WS_SKILLS_DIR/" 2>/dev/null || true
    fi
    [ -n "$(ls -A "$ZEUS_WS_SKILLS_DIR" 2>/dev/null)" ] && ok "Workspace skills seeded"
fi

# ── Interactive: Persona selection ──
if [ -n "$SKILLS_TMP" ] && [ -d "$SKILLS_TMP" ] && [ -t 0 ]; then
    PERSONA_SRC=""
    [ -d "$SKILLS_TMP/personas" ] && PERSONA_SRC="$SKILLS_TMP/personas"
    [ -z "$PERSONA_SRC" ] && [ -d "$SKILLS_TMP/agents" ] && PERSONA_SRC="$SKILLS_TMP/agents"

    if [ -n "$PERSONA_SRC" ]; then
        AVAILABLE_PERSONAS=$(ls "$PERSONA_SRC/"*.md 2>/dev/null | xargs -n1 basename 2>/dev/null | sed 's/\.md$//' || true)
        if [ -n "$AVAILABLE_PERSONAS" ]; then
            printf "\n  ${C}${B}Available Personas:${N}\n"
            i=1
            for persona in $AVAILABLE_PERSONAS; do
                printf "    ${C}%2d${N}${D}.${N} ${W}%s${N}\n" "$i" "$persona"
                i=$((i + 1))
            done
            printf "\n  ${D}Enter numbers, all, or none${N} ${C}[all]:${N} "
            read -r PERSONA_CHOICE 2>/dev/null || PERSONA_CHOICE="all"
            PERSONA_CHOICE="${PERSONA_CHOICE:-all}"

            mkdir -p "$CLAUDE_AGENTS_DIR"
            if [ "$PERSONA_CHOICE" = "none" ]; then
                info "Skipping personas"
            elif [ "$PERSONA_CHOICE" = "all" ]; then
                cp "$PERSONA_SRC/"*.md "$CLAUDE_AGENTS_DIR/" 2>/dev/null || true
                ok "All personas installed"
            else
                PERSONA_LIST="$AVAILABLE_PERSONAS"
                for num in $PERSONA_CHOICE; do
                    CHOSEN=$(echo "$PERSONA_LIST" | awk "NR==$num")
                    if [ -n "$CHOSEN" ]; then
                        cp "$PERSONA_SRC/$CHOSEN.md" "$CLAUDE_AGENTS_DIR/$CHOSEN.md" 2>/dev/null \
                            && ok "Persona: $CHOSEN" \
                            || warn "Could not install persona: $CHOSEN"
                    fi
                done
            fi
        fi
    else
        info "No personas directory found — skipped"
    fi
fi

# ═══════════════════════════════════════════════════════════════════════════
# Phase 7: Config
# ═══════════════════════════════════════════════════════════════════════════
phase "CONFIGURATION"

if $RECONFIGURE; then
    [ -f "$ZEUS_HOME/config.toml" ] && {
        cp "$ZEUS_HOME/config.toml" "$ZEUS_HOME/config.toml.pre-install"
        chmod 0600 "$ZEUS_HOME/config.toml.pre-install"
        ok "Backed up config"
    }
    rm -f "$ZEUS_HOME/config.toml"
    info "Config removed — wizard will run on next launch"
fi

if [ -f "$ZEUS_HOME/config.toml" ]; then
    ok "Config exists: $ZEUS_HOME/config.toml"
elif $WITH_WEBUI; then
    # WebUI-first onboarding runs the browser wizard against a LIVE gateway, so we
    # seed a minimal bootstrap config here so that gateway can start. A normal TUI
    # install must NOT pre-create config: `zeus` only launches the TUI onboarding
    # wizard when ~/.zeus has no config.toml. A pre-seeded config with a model set
    # trips needs_onboarding's legacy "model set = done" path and silently SKIPS
    # onboarding (the bug merakizzz hit 2026-06-19). --with-webui only.
    AGENT_HOSTNAME=$(hostname -s 2>/dev/null || hostname 2>/dev/null || echo "zeus-agent")
    cat > "$ZEUS_HOME/config.toml" <<CFGEOF
model = "anthropic/claude-sonnet-4-6"
workspace = "$ZEUS_HOME/workspace"
sessions = "$ZEUS_HOME/sessions"
max_iterations = 20
onboarding_complete = false
verbosity = "normal"

[tui]
theme = "dark"
vim_mode = false

[auth]
use_oauth = false

[prometheus]
enable_heartbeat = true
enable_cognitive = true

[gateway]
host = "$WEBUI_LISTEN"
port = 8080
enable_channels = true
enable_heartbeat = true

[credentials]

[aegis]
level = "standard"

[mnemosyne]
db_path = "$ZEUS_HOME/memory.db"
enable_fts = true

[agent]
persona = "The Herald"
name = "$AGENT_HOSTNAME"
CFGEOF
    chmod 0600 "$ZEUS_HOME/config.toml"
    ok "Created minimal bootstrap config.toml for the WebUI gateway"
else
    info "No config yet — 'zeus' will launch the TUI onboarding wizard on first run"
    info "(config pre-seed is --with-webui only; a pre-seeded config would skip TUI onboarding)"
fi

# ═══════════════════════════════════════════════════════════════════════════
# Phase 8: Agent Identity + MCP + Daemon
# ═══════════════════════════════════════════════════════════════════════════
phase "AGENT IDENTITY & MCP"

AGENT_NAME=$(hostname -s 2>/dev/null || hostname 2>/dev/null || echo "zeus-agent")
info "Agent: $AGENT_NAME"

# Deploy identity
DEPLOY_ID="$REPO_ROOT/scripts/deploy-identity.sh"
if [ -f "$DEPLOY_ID" ] && command -v bash >/dev/null 2>&1; then
    bash "$DEPLOY_ID" --agent "$AGENT_NAME" --home "$ZEUS_HOME" --force \
        && ok "Identity stamped" \
        || warn "deploy-identity.sh failed"
else
    if "$INSTALL_DIR/zeus" tool generate_workspace '{"name":"'"$AGENT_NAME"'","force":false}' 2>/dev/null; then
        ok "Workspace generated via zeus binary"
    else
        # Last resort: write minimal IDENTITY.md
        HN=$(hostname 2>/dev/null || echo "unknown")
        mkdir -p "$ZEUS_HOME/workspace/memory"
        printf "# IDENTITY.md — %s\n- **Name**: %s\n- **Host**: %s\n- **Role**: Zeus fleet agent\n" \
            "$AGENT_NAME" "$AGENT_NAME" "$HN" > "$ZEUS_HOME/workspace/IDENTITY.md"
        ok "Minimal identity written (run 'zeus onboard' for full templates)"
    fi
fi

# Daemon
zeus daemon install 2>/dev/null && ok "Daemon service installed" || warn "Daemon install failed"

fi  # end: if ! $MCP_ONLY (gateway-oriented phases 5-8)

if $MCP_ONLY; then
    phase "MCP-ONLY INSTALL"
    info "Skipping gateway service, onboarding, and launch (--mcp-only)"
    ok "Binary installed: $INSTALL_DIR/zeus — configuring MCP server only"
fi

# ── MCP Configuration ──────────────────────────────────────────────────────
# Detect python3
PYTHON3=""
for p in python3 python3.13 python3.12 python3.11 python3.10 python; do
    command -v "$p" >/dev/null 2>&1 && { PYTHON3="$p"; break; }
done

ZEUS_BIN="$INSTALL_DIR/zeus"
CLAUDE_DIR="$HOME/.claude"
mkdir -p "$CLAUDE_DIR"

ZEUS_TOOLS='[
      "mcp__zeus__read_file",
      "mcp__zeus__write_file",
      "mcp__zeus__edit_file",
      "mcp__zeus__list_dir",
      "mcp__zeus__shell",
      "mcp__zeus__web_fetch",
      "mcp__zeus__web_search",
      "mcp__zeus__deep_research",
      "mcp__zeus__spawn",
      "mcp__zeus__collect_spawns",
      "mcp__zeus__message",
      "mcp__zeus__send_file",
      "mcp__zeus__apply_patch",
      "mcp__zeus__link_understanding",
      "mcp__zeus__media_understanding",
      "mcp__zeus__auto_reply",
      "mcp__zeus__polls",
      "mcp__zeus__loop",
      "mcp__zeus__gmail_pubsub",
      "mcp__zeus__list_agents",
      "mcp__zeus__spawn_agent",
      "mcp__zeus__agent_status",
      "mcp__zeus__memory_graph",
      "mcp__zeus__memory_communities",
      "mcp__zeus__memory_graph_search",
      "mcp__zeus__send_p2p_message",
      "mcp__zeus__broadcast_p2p",
      "mcp__zeus__call_remote_agent",
      "mcp__zeus__*"
    ]'

write_mcp_json() {
    cat > "$1" << MCP_JSON_EOF
{
  "mcpServers": {
    "zeus": {
      "command": "$ZEUS_BIN",
      "args": ["mcp"],
      "env": {
        "HOME": "$HOME",
        "PATH": "$PATH"
      }
    }
  }
}
MCP_JSON_EOF
}

MCP_ENV_SCRIPT='
import os, json
env_block = {"HOME": os.environ["HOME"], "PATH": os.environ.get("PATH", "")}
try:
    import tomllib
    cfg_path = os.path.join(os.environ["HOME"], ".zeus", "config.toml")
    if os.path.exists(cfg_path):
        with open(cfg_path, "rb") as cf:
            cfg = tomllib.load(cf)
        dc = cfg.get("channels", {}).get("discord", {})
        if dc.get("token"):
            env_block["DISCORD_BOT_TOKEN"] = dc["token"]
        for acct in dc.get("accounts", {}).values():
            if isinstance(acct, dict) and acct.get("token"):
                env_block.setdefault("DISCORD_BOT_TOKEN", acct["token"])
        bindings = cfg.get("bindings", [])
        if isinstance(bindings, list):
            ch_ids = [b["channel_id"] for b in bindings if isinstance(b, dict) and b.get("channel_id")]
            if ch_ids:
                env_block["DISCORD_RELAY_CHANNEL_IDS"] = ",".join(ch_ids)
except Exception:
    pass
'

# ── ~/.claude/settings.json ──
SETTINGS_JSON="$CLAUDE_DIR/settings.json"
if [ -n "$PYTHON3" ]; then
    "$PYTHON3" -c "
import json, os
${MCP_ENV_SCRIPT}
_zeus_env = env_block
path = '$SETTINGS_JSON'
try:
    data = json.load(open(path))
except Exception:
    data = {}
data.setdefault('permissions', {})
allow = data['permissions'].get('allow', [])
zeus_tools = $ZEUS_TOOLS
for t in zeus_tools:
    if t not in allow:
        allow.append(t)
if 'Bash(*)' not in allow:
    allow.append('Bash(*)')
data['permissions']['allow'] = allow
data.setdefault('mcpServers', {})
data['mcpServers']['zeus'] = {
    'command': '$ZEUS_BIN',
    'args': ['mcp'],
    'env': _zeus_env
}
with open(path, 'w') as f:
    json.dump(data, f, indent=2)
    f.write('\n')
" 2>/dev/null && ok "Claude Code settings.json configured" \
               || warn "Could not write $SETTINGS_JSON via python3"
else
    if [ ! -f "$SETTINGS_JSON" ]; then
        cat > "$SETTINGS_JSON" << SETTINGS_EOF
{
  "permissions": {
    "allow": $ZEUS_TOOLS
  },
  "mcpServers": {
    "zeus": {
      "command": "$ZEUS_BIN",
      "args": ["mcp"],
      "env": {
        "HOME": "$HOME",
        "PATH": "$PATH"
      }
    }
  }
}
SETTINGS_EOF
        ok "Claude Code settings.json written (pure-sh fallback)"
    else
        warn "settings.json exists but python3 unavailable — cannot merge"
        info "Add 'mcp__zeus__*' to permissions.allow manually"
    fi
fi

# ── ~/.claude/mcp.json ──
MCP_JSON="$CLAUDE_DIR/mcp.json"
if [ ! -f "$MCP_JSON" ]; then
    write_mcp_json "$MCP_JSON"
    ok "~/.claude/mcp.json created"
else
    if [ -n "$PYTHON3" ]; then
        "$PYTHON3" -c "
import json, os
${MCP_ENV_SCRIPT}
_zeus_env = env_block
path = '$MCP_JSON'
try:
    data = json.load(open(path))
except Exception:
    data = {}
data.setdefault('mcpServers', {})
data['mcpServers']['zeus'] = {
    'command': '$ZEUS_BIN',
    'args': ['mcp'],
    'env': _zeus_env
}
with open(path, 'w') as f:
    json.dump(data, f, indent=2)
    f.write('\n')
" 2>/dev/null && ok "~/.claude/mcp.json updated" || ok "~/.claude/mcp.json exists"
    else
        ok "~/.claude/mcp.json exists"
    fi
fi

# ── ~/.claude.json (legacy) ──
CLAUDE_JSON="$HOME/.claude.json"
if [ -n "$PYTHON3" ]; then
    "$PYTHON3" -c "
import json, os
${MCP_ENV_SCRIPT}
_zeus_env = env_block
path = '$CLAUDE_JSON'
try:
    data = json.load(open(path))
except Exception:
    data = {}
data.setdefault('mcpServers', {})
data['mcpServers']['zeus'] = {
    'command': '$ZEUS_BIN',
    'args': ['mcp'],
    'env': _zeus_env
}
with open(path, 'w') as f:
    json.dump(data, f, indent=2)
    f.write('\n')
" 2>/dev/null && ok "~/.claude.json configured" \
               || info "Could not write ~/.claude.json (non-fatal)"
else
    if [ ! -f "$CLAUDE_JSON" ]; then
        write_mcp_json "$CLAUDE_JSON"
        ok "~/.claude.json created (pure-sh fallback)"
    else
        ok "~/.claude.json exists"
    fi
fi

ok "MCP setup complete — Zeus tools available as mcp__zeus__*"
info "Restart Claude Code to pick up the new MCP server"

# ═══════════════════════════════════════════════════════════════════════════
# macOS: Full Disk Access reminder
# ═══════════════════════════════════════════════════════════════════════════
# macOS FDA: just a reminder, no interactive prompt
if [ "$OS" = "Darwin" ]; then
    info "Tip: Grant Full Disk Access for autonomous operation:"
    info "  System Settings → Privacy & Security → Full Disk Access → add zeus"
fi

# ═══════════════════════════════════════════════════════════════════════════
# Phase 9: Launch
# ═══════════════════════════════════════════════════════════════════════════
if $DO_LAUNCH; then
    phase "LAUNCH GATEWAY"

    # With --with-webui, gateway MUST start even without config.toml
    # (the onboarding wizard running on 8081 creates the config)
    if ! $WITH_WEBUI && [ ! -f "$ZEUS_HOME/config.toml" ]; then
        warn "No config.toml — run 'zeus' for setup wizard first"
        DO_LAUNCH=false
    fi

    # Fresh install (onboarding not done): only start gateway for --with-webui
    # (WebUI needs the gateway running for browser-based onboarding).
    # TUI onboarding runs offline and starts the gateway after saving credentials.
    if $DO_LAUNCH && grep -q 'onboarding_complete = false' "$ZEUS_HOME/config.toml" 2>/dev/null; then
        if ! $WITH_WEBUI; then
            # Reboot-survival: write the system LaunchDaemon plist now (install.sh
            # already holds sudo) so launchd's RunAtLoad=true restarts the titan on
            # every future reboot. We do NOT bootstrap it this boot — AWAKEN spawns
            # the live daemon after onboarding saves credentials, and a parallel
            # launchd start would race it. write-only = plist on disk, dormant until
            # next reboot. Headless servers (SSH, no GUI login) need system-domain
            # boot-survival; a user LaunchAgent would never load without a GUI session.
            if [ "$(uname -s)" = "Darwin" ]; then
                if install_launchd_plist "$REPO_ROOT" "$INSTALL_DIR" "$ZEUS_HOME/logs" write-only >/dev/null 2>&1; then
                    info "LaunchDaemon plist written (reboot-survival) — not started this boot"
                else
                    warn "Could not write LaunchDaemon plist — reboot-survival unavailable"
                fi
            elif [ "$(uname -s)" = "FreeBSD" ]; then
                # Mirror the macOS write-only arm: register the rc.d service +
                # sysrc-enable now (install.sh already holds sudo), but do NOT
                # start it — AWAKEN spawns the live daemon after onboarding saves
                # credentials. Dormant until next reboot, exactly like the plist.
                if daemon_install_out=$(sudo zeus daemon install 2>&1); then
                    info "rc.d service registered + enabled (reboot-survival) — not started this boot"
                else
                    warn "zeus daemon install failed — reboot-survival unavailable:"
                    printf '%s\n' "$daemon_install_out" | sed 's/^/    /'
                fi
            fi
            info "Skipping gateway start — TUI onboarding will start it after setup"
            DO_LAUNCH=false
        fi
    fi
fi

if $DO_LAUNCH; then
    # Stop existing gateway (PID file + process scan)
    GATEWAY_WAS_RUNNING=false

    if [ -f "$ZEUS_HOME/gateway.pid" ]; then
        OLD_PID=$(cat "$ZEUS_HOME/gateway.pid" 2>/dev/null || echo "")
        if [ -n "$OLD_PID" ] && kill -0 "$OLD_PID" 2>/dev/null; then
            kill "$OLD_PID" 2>/dev/null || true
            GATEWAY_WAS_RUNNING=true
            info "Stopped old gateway (PID $OLD_PID)"
        fi
        rm -f "$ZEUS_HOME/gateway.pid"
    fi

    RUNNING_PIDS=$(pgrep -f 'zeus gateway' 2>/dev/null || true)
    if [ -n "$RUNNING_PIDS" ]; then
        echo "$RUNNING_PIDS" | while read -r pid; do
            kill "$pid" 2>/dev/null && GATEWAY_WAS_RUNNING=true
        done
    fi

    # Always sleep after killing — KeepAlive=true daemons (macOS launchd)
    # respawn immediately, so the old binary may still be settling even if
    # GATEWAY_WAS_RUNNING is false (PID file was stale/missing).
    sleep 2
    $GATEWAY_WAS_RUNNING && ok "Old gateway stopped — restarting" || info "Cleared stale gateway state"

    case "$OS" in
        Darwin)
            # System LaunchDaemon (system domain) — survives logout/reboot, no GUI session.
            # Migrates from any pre-existing user-agent at ~/Library/LaunchAgents/.
            GATEWAY_STARTED=false
            mkdir -p "$ZEUS_HOME/logs"
            if install_launchd_plist "$REPO_ROOT" "$INSTALL_DIR" "$ZEUS_HOME/logs" >/dev/null 2>&1; then
                GATEWAY_STARTED=true
                ok "Gateway started via launchd (system domain)"
            else
                warn "launchd bootstrap failed — falling back to nohup"
            fi
            # Fallback: nohup if launchd didn't work
            if ! $GATEWAY_STARTED; then
                mkdir -p "$ZEUS_HOME/logs"
                nohup zeus gateway > "$ZEUS_HOME/logs/gateway.out.log" 2> "$ZEUS_HOME/logs/gateway.err.log" &
                sleep 1
                if pgrep -f 'zeus gateway' > /dev/null 2>&1; then
                    ok "Gateway started via nohup"
                else
                    warn "Gateway failed to start — check $ZEUS_HOME/logs/gateway.err.log"
                fi
            fi ;;
        FreeBSD)
            if [ -f "$REPO_ROOT/scripts/freebsd/zeus_gateway" ]; then
                sudo install -d -o root -g wheel -m 0755 /usr/local/etc/rc.d
                sudo install -o root -g wheel -m 0755 "$REPO_ROOT/scripts/freebsd/zeus_gateway" /usr/local/etc/rc.d/zeus_gateway
            fi
            if [ -f /usr/local/etc/rc.d/zeus_gateway ]; then
                sudo sysrc zeus_gateway_enable=YES 2>/dev/null || true
                sudo sysrc zeus_gateway_user="$(id -un)" 2>/dev/null || true
                sudo sysrc zeus_gateway_home="$HOME" 2>/dev/null || true
                sudo service zeus_gateway restart 2>/dev/null && ok "Gateway via rc.d (zeus_gateway)" || {
                    nohup zeus gateway > "$ZEUS_HOME/logs/gateway.out.log" 2> "$ZEUS_HOME/logs/gateway.err.log" &
                    ok "Gateway via nohup"
                }
            else
                nohup zeus gateway > "$ZEUS_HOME/logs/gateway.out.log" 2> "$ZEUS_HOME/logs/gateway.err.log" &
                ok "Gateway via nohup"
            fi ;;
        Linux)
            if command -v systemctl >/dev/null 2>&1 && [ -f "$HOME/.config/systemd/user/zeus-gateway.service" ]; then
                systemctl --user daemon-reload 2>/dev/null; systemctl --user enable zeus-gateway 2>/dev/null
                systemctl --user restart zeus-gateway 2>/dev/null && ok "Gateway via systemd" || {
                    nohup zeus gateway > "$ZEUS_HOME/logs/gateway.out.log" 2> "$ZEUS_HOME/logs/gateway.err.log" &
                    ok "Gateway via nohup"
                }
            else
                nohup zeus gateway > "$ZEUS_HOME/logs/gateway.out.log" 2> "$ZEUS_HOME/logs/gateway.err.log" &
                ok "Gateway via nohup"
            fi ;;
    esac

    sleep 3
    GATEWAY_PID=$(pgrep -f 'zeus gateway' 2>/dev/null | head -1 || echo "")
    if [ -n "$GATEWAY_PID" ] && kill -0 "$GATEWAY_PID" 2>/dev/null; then
        ok "Gateway running (PID $GATEWAY_PID)"

        if $WITH_WEBUI && [ -d "$ZEUS_HOME/web" ] && [ -f "$ZEUS_HOME/web/index.html" ]; then
            ok "WebUI at http://localhost:8081/"
            # Bootstrap mode serves on 8081 — check that port
            HEALTH=$(curl -s --max-time 5 http://127.0.0.1:8081/health 2>/dev/null || echo "")
            if echo "$HEALTH" | grep -q '"ok"'; then
                ok "Health check (8081): ${G}OK${N}"
            else
                # Gateway may still be initialising — not fatal
                info "Gateway started; WebUI onboarding at http://localhost:8081/"
            fi
        else
            # Try port 8080 for standard gateway mode
            HEALTH=$(curl -s --max-time 5 http://127.0.0.1:8080/health 2>/dev/null || echo "")
            if echo "$HEALTH" | grep -q '"ok"'; then
                ok "Health check: ${G}OK${N}"
            else
                warn "Health check failed — gateway may still be starting"
            fi
        fi
    else
        warn "Gateway failed to start"
        info "Logs: tail -f $ZEUS_HOME/logs/gateway.err.log"
    fi
else
    phase "SKIP LAUNCH"
    if $MCP_ONLY; then
        ok "MCP server ready — restart Claude Code / Codex to pick it up"
    else
        ok "Run manually: zeus gateway"
    fi
fi

# ═══════════════════════════════════════════════════════════════════════════
# Final Summary
# ═══════════════════════════════════════════════════════════════════════════
ELAPSED=$(timer_elapsed)

printf "\n"
box_top
box_mid ""
box_mid "$(printf "  ${C}${B}⚡ Zeus installation complete${N}  ${D}(%s)${N}" "$ELAPSED")"
box_mid ""
box_sep
box_mid "$(printf "  ${D}Binary:${N}     ${W}%s/zeus${N}" "$INSTALL_DIR")"
box_mid "$(printf "  ${D}Config:${N}     ${W}%s/config.toml${N}" "$ZEUS_HOME")"
box_mid "$(printf "  ${D}Logs:${N}       ${W}%s/logs/${N}" "$ZEUS_HOME")"
box_mid "$(printf "  ${D}Skills:${N}     ${W}%s/skills/${N}" "$ZEUS_HOME")"
if $WITH_WEBUI; then
    LOCAL_IP=$(ipconfig getifaddr en0 2>/dev/null || hostname -I 2>/dev/null | awk '{print $1}' || echo "localhost")
    WEBUI_FINAL_URL="http://${LOCAL_IP}:8081"
    box_mid ""
    box_sep
    box_mid "$(printf "  ${CS}${B}⚡ WebUI:${N}    ${W}%s${N}" "$WEBUI_FINAL_URL")"
fi
box_mid ""
box_sep
box_mid "$(printf "  ${W}zeus${N}              ${D}Launch TUI (wizard on first run)${N}")"
box_mid "$(printf "  ${W}zeus onboard${N}      ${D}Re-run setup wizard${N}")"
box_mid "$(printf "  ${W}zeus gateway${N}      ${D}Start API server${N}")"
box_mid "$(printf "  ${W}zeus doctor${N}       ${D}Run diagnostics${N}")"
box_mid ""
if [ "$OS" = "Darwin" ]; then
    box_sep
    box_mid "$(printf "  ${Y}${B}macOS:${N} Grant Full Disk Access in")"
    box_mid "$(printf "  ${D}System Settings → Privacy & Security → Full Disk Access${N}")"
    box_mid "$(printf "  ${D}for ${W}%s/zeus${N}" "$INSTALL_DIR")"
    box_mid ""
fi
box_bot
printf "\n"
