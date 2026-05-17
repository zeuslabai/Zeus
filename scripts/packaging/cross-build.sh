#!/usr/bin/env sh
set -eu

# ═══════════════════════════════════════════════════════════════════════════════
# Zeus — Cross-Platform Build Script
#
# Interactive platform selector for cross-compiling Zeus binaries.
# Based on install.sh patterns, outputs to builds/{platform}/zeus
#
# Usage:
#   ./scripts/packaging/cross-build.sh              # Interactive mode
#   ./scripts/packaging/cross-build.sh linux-amd64  # Build specific target
#   ./scripts/packaging/cross-build.sh all          # Build all platforms
#
# Supported targets:
#   linux-amd64, linux-arm64, linux-armv7
#   darwin-amd64, darwin-arm64
#   freebsd-amd64, freebsd-arm64
# ═══════════════════════════════════════════════════════════════════════════════

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# ── ANSI Theme (Cyberpunk Red/Dark) ─────────────────────────────────────────
C="\033[38;5;196m"    # red accent
CS="\033[38;5;203m"   # soft red
CD="\033[38;5;124m"   # dark rust accent
G="\033[38;5;46m"     # green
Y="\033[38;5;220m"    # yellow
D="\033[38;5;240m"    # dim
W="\033[38;5;252m"    # white/text
B="\033[1m"           # bold
N="\033[0m"           # reset

# ── All targets (order matters for interactive menu) ────────────────────────
ALL_TARGETS="linux-amd64 linux-arm64 linux-armv7 darwin-amd64 darwin-arm64 freebsd-amd64 freebsd-arm64"

# ── Lookup functions (bash 3.2 / POSIX compatible) ─────────────────────────
rust_target() {
    case "$1" in
        linux-amd64)    echo "x86_64-unknown-linux-gnu" ;;
        linux-arm64)    echo "aarch64-unknown-linux-gnu" ;;
        linux-armv7)    echo "armv7-unknown-linux-gnueabihf" ;;
        darwin-amd64)   echo "x86_64-apple-darwin" ;;
        darwin-arm64)   echo "aarch64-apple-darwin" ;;
        freebsd-amd64)  echo "x86_64-unknown-freebsd" ;;
        freebsd-arm64)  echo "aarch64-unknown-freebsd" ;;
        *) echo ""; return 1 ;;
    esac
}

target_name() {
    case "$1" in
        linux-amd64)    echo "Linux x86_64" ;;
        linux-arm64)    echo "Linux ARM64" ;;
        linux-armv7)    echo "Linux ARMv7" ;;
        darwin-amd64)   echo "macOS Intel" ;;
        darwin-arm64)   echo "macOS Apple Silicon" ;;
        freebsd-amd64)  echo "FreeBSD x86_64" ;;
        freebsd-arm64)  echo "FreeBSD ARM64" ;;
        *) echo "Unknown" ;;
    esac
}

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
    printf "${D}  Cross-Platform Build Tool${N}\n"
    printf "\n"
}

ok()   { printf "  ${G}✓${N} ${W}%s${N}\n" "$1"; }
warn() { printf "  ${Y}!${N} ${Y}%s${N}\n" "$1"; }
fail() { printf "  ${C}✗${N} ${C}%s${N}\n" "$1"; exit 1; }
info() { printf "  ${CS}→${N} ${D}%s${N}\n" "$1"; }

# ── Detect Host Platform ───────────────────────────────────────────────────
detect_host() {
    local os arch
    os="$(uname -s | tr '[:upper:]' '[:lower:]')"
    arch="$(uname -m)"

    case "$arch" in
        x86_64)  arch="amd64" ;;
        aarch64|arm64) arch="arm64" ;;
        armv7l)  arch="armv7" ;;
    esac

    case "$os" in
        darwin)  os="darwin" ;;
        linux)   os="linux" ;;
        freebsd) os="freebsd" ;;
    esac

    echo "${os}-${arch}"
}

# ── Check Cross-Compilation Toolchain ───────────────────────────────────────
check_toolchain() {
    local target="$1"
    local rt
    rt=$(rust_target "$target")

    # Auto-install Rust target if missing
    if ! rustup target list --installed 2>/dev/null | grep -q "^${rt}$"; then
        info "Installing Rust target ${rt}..."
        if rustup target add "$rt" 2>/dev/null; then
            ok "Installed ${rt}"
        else
            warn "Failed to install Rust target ${rt}"
            return 1
        fi
    fi

    # Check for cross-compilation tools (non-native targets)
    local host
    host=$(detect_host)
    if [ "$target" != "$host" ]; then
        if ! command -v cargo-zigbuild >/dev/null 2>&1 && ! command -v cross >/dev/null 2>&1; then
            warn "No cross-compilation tool found"
            info "Install zigbuild: brew install zig && cargo install cargo-zigbuild"
            info "Or install cross: cargo install cross"
            return 1
        fi
    fi

    return 0
}

# ── Build Single Target ─────────────────────────────────────────────────────
build_target() {
    local target="$1"
    local rt tn output_dir build_log
    rt=$(rust_target "$target")
    tn=$(target_name "$target")
    output_dir="${REPO_ROOT}/builds/${target}"
    build_log="${REPO_ROOT}/builds/${target}.log"

    printf "\n${C}${B}▶ Building %s${N}\n" "$tn"
    printf "${D}  Target: %s${N}\n" "$rt"

    mkdir -p "$output_dir"

    # Determine build method based on target
    local build_cmd
    local host
    host=$(detect_host)
    if [ "$target" = "$host" ]; then
        # Native build — no cross-compilation needed
        build_cmd="cargo build --release --target ${rt}"
    elif command -v cargo-zigbuild >/dev/null 2>&1; then
        # Cross-compile with zigbuild (rustls-tls = pure Rust, no C deps)
        build_cmd="cargo zigbuild --release --target ${rt}"
    elif command -v cross >/dev/null 2>&1; then
        build_cmd="cross build --release --target ${rt}"
    else
        build_cmd="cargo build --release --target ${rt}"
    fi

    # Run build
    local start_time end_time duration
    start_time=$(date +%s)

    if eval "$build_cmd" > "$build_log" 2>&1; then
        end_time=$(date +%s)
        duration=$((end_time - start_time))

        # Copy binary to output dir
        local binary_path="${REPO_ROOT}/target/${rt}/release/zeus"
        if [ -f "$binary_path" ]; then
            cp "$binary_path" "${output_dir}/zeus"
            chmod +x "${output_dir}/zeus"

            local size
            size=$(du -h "${output_dir}/zeus" | awk '{print $1}')
            ok "Build complete (${duration}s) — ${size}"
            info "Output: ${output_dir}/zeus"
            return 0
        else
            warn "Binary not found at ${binary_path}"
            return 1
        fi
    else
        warn "Build failed — see ${build_log}"
        return 1
    fi
}

# ── Interactive Platform Selector ───────────────────────────────────────────
interactive_select() {
    local host_target
    host_target=$(detect_host)

    printf "\n${C}${B}Select platforms to build:${N}\n\n"

    local i=1
    for key in $ALL_TARGETS; do
        local marker=""
        [ "$key" = "$host_target" ] && marker=" ${G}(host)${N}"
        printf "  ${C}%d)${N} ${W}%-15s${N} ${D}— %s%b${N}\n" "$i" "$key" "$(target_name "$key")" "$marker"
        i=$((i + 1))
    done

    printf "\n  ${C}a)${N} ${W}%-15s${N} ${D}— Build all platforms${N}\n" "all"
    printf "  ${C}q)${N} ${W}%-15s${N} ${D}— Quit${N}\n" "quit"

    printf "\n${CS}Enter selection (e.g., 1 3 5 or 'all'):${N} "
    read -r selection
    echo "$selection"
}

# ── Resolve selection to target list ────────────────────────────────────────
resolve_selection() {
    local selection="$1"

    case "$selection" in
        q|quit|exit) exit 0 ;;
        a|all)       echo "$ALL_TARGETS" ;;
        *)
            # Parse space-separated numbers into target names
            local result=""
            for num in $selection; do
                case "$num" in
                    [0-9]*)
                        local i=1
                        for key in $ALL_TARGETS; do
                            if [ "$i" -eq "$num" ]; then
                                result="$result $key"
                                break
                            fi
                            i=$((i + 1))
                        done
                        ;;
                esac
            done
            echo "$result"
            ;;
    esac
}

# ── Main ────────────────────────────────────────────────────────────────────
banner

# Check we're in a repo with Cargo.toml
if [ ! -f "${REPO_ROOT}/Cargo.toml" ]; then
    fail "Cargo.toml not found — run from Zeus repo root"
fi

# Check for Rust
command -v cargo >/dev/null 2>&1 || {
    [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
}
command -v cargo >/dev/null 2>&1 || fail "Rust/Cargo not found — install from https://rustup.rs"

SELECTED=""

# Handle command-line arg or interactive
if [ $# -eq 0 ]; then
    selection=$(interactive_select)
    SELECTED=$(resolve_selection "$selection")
elif [ "$1" = "all" ]; then
    SELECTED="$ALL_TARGETS"
elif [ "$1" = "-h" ] || [ "$1" = "--help" ]; then
    printf "${W}Usage:${N} cross-build.sh [TARGET|all]\n\n"
    printf "${W}Targets:${N}\n"
    for key in $ALL_TARGETS; do
        printf "  ${CS}%-15s${N} ${D}%s  (%s)${N}\n" "$key" "$(target_name "$key")" "$(rust_target "$key")"
    done
    printf "\n  ${CS}all${N}             ${D}Build all platforms${N}\n"
    exit 0
elif rust_target "$1" >/dev/null 2>&1; then
    SELECTED="$1"
else
    fail "Unknown target: $1 — run with --help for valid targets"
fi

# Count targets
target_count=0
for _ in $SELECTED; do target_count=$((target_count + 1)); done

if [ "$target_count" -eq 0 ]; then
    warn "No valid targets selected"
    exit 1
fi

# Build each target
printf "\n${C}${B}Building %d target(s)...${N}\n" "$target_count"

success=0
failed=0

for target in $SELECTED; do
    if check_toolchain "$target"; then
        if build_target "$target"; then
            success=$((success + 1))
        else
            failed=$((failed + 1))
        fi
    else
        failed=$((failed + 1))
    fi
done

# Summary
COLS=$(tput cols 2>/dev/null || echo 70)
[ "$COLS" -gt 100 ] && COLS=100

printf "\n"
printf "${C}  ╔════════════════════════════════════════════════╗${N}\n"
printf "${C}  ║${N}   ${B}${G}⚡ Build Complete${N}                            ${C}║${N}\n"
printf "${C}  ╠════════════════════════════════════════════════╣${N}\n"
printf "${C}  ║${N}   ${D}Successful:${N}  ${G}%-2d${N}                             ${C}║${N}\n" "$success"
printf "${C}  ║${N}   ${D}Failed:${N}      ${C}%-2d${N}                             ${C}║${N}\n" "$failed"
printf "${C}  ║${N}   ${D}Output:${N} ${W}builds/{platform}/zeus${N}              ${C}║${N}\n"
printf "${C}  ╚════════════════════════════════════════════════╝${N}\n"

# List outputs
if [ "$success" -gt 0 ]; then
    printf "\n${W}Built binaries:${N}\n"
    for target in $SELECTED; do
        bin_path="${REPO_ROOT}/builds/${target}/zeus"
        if [ -f "$bin_path" ]; then
            size=$(du -h "$bin_path" | awk '{print $1}')
            printf "  ${G}✓${N} ${W}%-20s${N} ${D}%s${N}\n" "$target" "$size"
        fi
    done
    printf "\n"
fi

[ "$failed" -eq 0 ] && exit 0 || exit 1
