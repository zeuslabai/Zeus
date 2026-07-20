#!/usr/bin/env bash
# release-local.sh — Local cross-compile release pipeline for Zeus.
#
# Builds the full release matrix from a single machine, generates SHA256SUMS,
# and optionally creates a GitHub release via `gh release create`.
#
# Targets:
#   linux-amd64, linux-arm64, macos-intel, macos-arm, freebsd-amd64
#   windows-amd64 — auto-enabled when ci/mingw-w64-x86_64.cmake exists (the
#   #308 enabler, on feat/windows-tier1 until merged), or selected explicitly
#   via --targets windows-amd64
#
# Usage:
#   scripts/release-local.sh --version 0.1.2              # dry-run (default)
#   scripts/release-local.sh --version 0.1.2 --publish    # create GH release
#   scripts/release-local.sh --version 0.1.2 --targets linux-arm64,linux-amd64
#   scripts/release-local.sh --version 0.1.2 --package deb,rpm,freebsd,macos
#   scripts/release-local.sh --version 0.1.2 --targets linux-amd64,linux-arm64 --package deb-nfpm,rpm-nfpm
#   scripts/release-local.sh --version 0.1.2 --targets windows-amd64 --package msi
#
# OS packages (opt-in, #396 P1 + P2 + P3 + P4): --package wires the existing
# scripts/packaging/*.sh scripts (deb/rpm/freebsd/nfpm/msi) and zeus-setup's
# Rust .pkg builder (macos) against the binaries built above.
#   deb, rpm       — need dpkg-deb / rpmbuild on the build host
#                    (macOS: brew install dpkg rpm; Linux: native package manager)
#   deb-nfpm,
#   rpm-nfpm       — nfpm (brew install nfpm), no native dpkg-deb/rpmbuild
#                    needed — packages ANY built linux-amd64/linux-arm64
#                    target regardless of this host's arch (unlike deb/rpm
#                    above, which are host-arch-locked).
#   freebsd        — needs a FreeBSD host for a true pkg(8) .txz; on other
#                    hosts build-port.sh transparently falls back to a plain
#                    tar.xz (a real cross-format .txz isn't buildable
#                    off-FreeBSD — no cross tool exists, verified during
#                    #395 research)
#   macos          — needs pkgbuild/productbuild (Xcode CLT), macOS-only, and
#                    a prior native build_target run (this Mac's
#                    target/release/). Unsigned by default; set
#                    ZEUS_PKG_SIGN_IDENTITY to sign.
#   msi            — needs wixl (brew install msitools), buildable from any
#                    host — no Windows box required. Needs a prior
#                    windows-amd64 target build. Custom action registers the
#                    ZeusGateway Task Scheduler service via the binary's own
#                    `zeus.exe daemon install` path (#308).
#
# Version stamping: if --version differs from the workspace Cargo.toml version,
# the script stamps Cargo.toml (+ syncs Cargo.lock) so binaries report the same
# version as the release tag. The stamp is left uncommitted for review;
# --publish refuses to run until it is committed.
#
# Requirements:
#   - Rust toolchain (rustup)
#   - cargo-zigbuild + zig (auto-installed if missing on macOS/Linux)
#   - mingw-w64 + cmake + zip (windows-amd64 only; brew-installed on macOS)
#   - gh CLI (authenticated, for --publish only)
#
# Environment overrides:
#   ZEUS_REPO_DIR    Repo root (default: parent of this script)
#   ZEUS_JOBS        Parallel cargo jobs (default: nproc)

set -euo pipefail

# ── Defaults ──────────────────────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="${ZEUS_REPO_DIR:-$(dirname "$SCRIPT_DIR")}"
VERSION=""
PUBLISH=false
DRY_RUN=true
ALL_TARGETS=("linux-amd64" "linux-arm64" "macos-intel" "macos-arm" "freebsd-amd64")
# windows-amd64 joins the default matrix only when the #308 cross-compile
# enabler is present (ci/mingw-w64-x86_64.cmake — on feat/windows-tier1 until
# merged). Explicit --targets windows-amd64 still works and fails loudly on
# trees without the enabler.
if [[ -f "${ZEUS_REPO_DIR:-$(dirname "$SCRIPT_DIR")}/ci/mingw-w64-x86_64.cmake" ]]; then
    ALL_TARGETS+=("windows-amd64")
fi
# Honor CARGO_TARGET_DIR (gate worktrees set it) — bin paths must follow cargo.
TARGET_ROOT="${CARGO_TARGET_DIR:-target}"
SELECTED_TARGETS=()
SELECTED_PACKAGES=()
JOBS="${ZEUS_JOBS:-$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)}"
DIST_DIR="$REPO_DIR/dist/release"
TIMESTAMP="$(date -u +%Y%m%d-%H%M%S)"

# ── Colors ────────────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
BLUE='\033[0;34m'; BOLD='\033[1m'; NC='\033[0m'

log()  { echo -e "${BLUE}▸${NC} $*"; }
ok()   { echo -e "${GREEN}✔${NC} $*"; }
warn() { echo -e "${YELLOW}⚠${NC} $*"; }
err()  { echo -e "${RED}✖${NC} $*" >&2; }
die()  { err "$@"; exit 1; }

# ── Parse args ────────────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --version)   VERSION="$2"; shift 2 ;;
        --publish)   PUBLISH=true; DRY_RUN=false; shift ;;
        --dry-run)   DRY_RUN=true; shift ;;
        --targets)   IFS=',' read -ra SELECTED_TARGETS <<< "$2"; shift 2 ;;
        --package)   IFS=',' read -ra SELECTED_PACKAGES <<< "$2"; shift 2 ;;
        --jobs)      JOBS="$2"; shift 2 ;;
        --help|-h)
            sed -n '2,/^$/s/^# \?//p' "$0"
            exit 0
            ;;
        *) die "Unknown flag: $1" ;;
    esac
done

[[ -n "$VERSION" ]] || die "Missing --version X.Y.Z"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$ ]] || die "Invalid version format: $VERSION (expected X.Y.Z or X.Y.Z-tag)"

if [[ ${#SELECTED_TARGETS[@]} -eq 0 ]]; then
    SELECTED_TARGETS=("${ALL_TARGETS[@]}")
fi

# ── Target → Rust triple mapping ─────────────────────────────────────────────
target_to_triple() {
    case "$1" in
        linux-amd64)   echo "x86_64-unknown-linux-gnu" ;;
        linux-arm64)   echo "aarch64-unknown-linux-gnu" ;;
        macos-intel)   echo "x86_64-apple-darwin" ;;
        macos-arm)     echo "aarch64-apple-darwin" ;;
        freebsd-amd64) echo "x86_64-unknown-freebsd" ;;
        windows-amd64) echo "x86_64-pc-windows-gnu" ;;
        *) die "Unknown target: $1" ;;
    esac
}

# Validate every requested target up front. build_target calls this inside
# $(...) where 'die' only kills the subshell — a bad --targets name would
# otherwise sail through the whole run and exit 0 with zero artifacts.
for _t in "${SELECTED_TARGETS[@]}"; do
    target_to_triple "$_t" >/dev/null
done

# ── Preflight ─────────────────────────────────────────────────────────────────
log "Zeus release pipeline v${VERSION}"
log "Repo: $REPO_DIR"
log "Targets: ${SELECTED_TARGETS[*]}"
log "Mode: $(if $DRY_RUN; then echo 'DRY-RUN'; else echo 'PUBLISH'; fi)"
echo ""

cd "$REPO_DIR"

# Verify we're on a clean-ish tree
if [[ -n "$(git status --porcelain)" ]]; then
    warn "Working tree has uncommitted changes — building from current state"
fi

CURRENT_SHA="$(git rev-parse --short HEAD)"
log "Building from: $CURRENT_SHA"

# ── Version stamp ─────────────────────────────────────────────────────────────
# --version is the release's single source of truth and must match what the
# binaries will report: the workspace Cargo.toml [workspace.package] version,
# which every crate inherits (version.workspace = true). If they differ, stamp
# the workspace version and sync Cargo.lock so the tag, artifact names, and
# `zeus --version` all agree. The stamp is left UNCOMMITTED for review;
# --publish refuses to run until it is committed (see Publish section).
workspace_version() {
    awk -F'"' '
        /^\[workspace\.package\]/ { in_wp = 1; next }
        /^\[/                     { in_wp = 0 }
        in_wp && /^version[[:space:]]*=/ { print $2; exit }
    ' Cargo.toml
}

CARGO_VERSION="$(workspace_version)"
[[ -n "$CARGO_VERSION" ]] || die "Could not read [workspace.package] version from Cargo.toml"
if [[ "$CARGO_VERSION" == "$VERSION" ]]; then
    ok "Version $VERSION matches workspace Cargo.toml"
else
    log "Stamping version: Cargo.toml $CARGO_VERSION → $VERSION"
    # Portable in-place sed (BSD + GNU via -i.bak), scoped to the
    # [workspace.package] section so only the workspace version line changes.
    _cv_re="${CARGO_VERSION//./\\.}"
    sed -i.zeus-release.bak \
        "/^\[workspace\.package\]/,/^\[/ s/^version[[:space:]]*=[[:space:]]*\"${_cv_re}\"/version = \"${VERSION}\"/" \
        Cargo.toml
    rm -f Cargo.toml.zeus-release.bak
    [[ "$(workspace_version)" == "$VERSION" ]] \
        || die "Version stamp failed — Cargo.toml still reads '$(workspace_version)'"
    # Sync Cargo.lock so --locked builds (install.sh, seat gates) keep working:
    # --workspace touches only the workspace members' own lock entries.
    log "Syncing Cargo.lock (cargo update --workspace)..."
    cargo update --workspace --offline >/dev/null 2>&1 \
        || cargo update --workspace >/dev/null 2>&1 \
        || die "cargo update --workspace failed — Cargo.lock not synced with $VERSION"
    ok "Stamped $VERSION into Cargo.toml + Cargo.lock (uncommitted — review before publishing)"
    warn "Commit before publishing: git add Cargo.toml Cargo.lock && git commit -m \"release: v${VERSION}\""
fi

# ── Toolchain setup ──────────────────────────────────────────────────────────
ensure_target() {
    local triple="$1"
    if ! rustup target list --installed | grep -q "^${triple}$"; then
        log "Installing Rust target: $triple"
        rustup target add "$triple"
    fi
}

ensure_zigbuild() {
    if ! command -v cargo-zigbuild &>/dev/null; then
        log "Installing cargo-zigbuild..."
        cargo install cargo-zigbuild
    fi
    if ! command -v zig &>/dev/null; then
        log "Installing zig..."
        local OS="$(uname -s)"
        local ARCH="$(uname -m)"
        case "$OS" in
            Darwin)
                brew install zig
                ;;
            Linux)
                # Download zig tarball (most reliable cross-distro method)
                local ZIG_VER="0.13.0"
                local ZIG_ARCH="$ARCH"
                [[ "$ARCH" == "aarch64" ]] && ZIG_ARCH="aarch64"
                [[ "$ARCH" == "x86_64" ]] && ZIG_ARCH="x86_64"
                local ZIG_URL="https://ziglang.org/download/${ZIG_VER}/zig-linux-${ZIG_ARCH}-${ZIG_VER}.tar.xz"
                local ZIG_TMP="/tmp/zig-install-$$"
                log "Downloading zig ${ZIG_VER} for ${ZIG_ARCH}..."
                mkdir -p "$ZIG_TMP"
                curl -fsSL "$ZIG_URL" | tar -xJ -C "$ZIG_TMP"
                sudo cp "$ZIG_TMP"/zig-linux-*/zig /usr/local/bin/zig
                sudo chmod +x /usr/local/bin/zig
                rm -rf "$ZIG_TMP"
                ;;
            *) die "Unsupported OS for zig install: $OS" ;;
        esac
    fi
    ok "cargo-zigbuild + zig ready"
}

ensure_mingw() {
    local missing=()
    command -v x86_64-w64-mingw32-gcc &>/dev/null || missing+=("mingw-w64")
    command -v cmake &>/dev/null || missing+=("cmake")
    command -v zip &>/dev/null || missing+=("zip")
    if [[ ${#missing[@]} -gt 0 ]]; then
        if [[ "$(uname -s)" == "Darwin" ]]; then
            log "Installing for windows-amd64: ${missing[*]} (brew)..."
            brew install "${missing[@]}"
        else
            die "Missing for windows-amd64: ${missing[*]} (Debian/Ubuntu: apt install mingw-w64 cmake zip; Fedora: dnf install mingw64-gcc cmake zip)"
        fi
    fi
    ok "mingw-w64 + cmake + zip ready"
}

# Check if a target needs cross-compilation on this host
needs_cross() {
    local triple="$1"
    local host_arch="$(uname -m)"
    local host_os="$(uname -s)"

    case "$triple" in
        aarch64-unknown-linux-gnu)
            [[ "$host_os" == "Linux" && "$host_arch" == "aarch64" ]] && return 1 || return 0 ;;
        x86_64-unknown-linux-gnu)
            [[ "$host_os" == "Linux" && "$host_arch" == "x86_64" ]] && return 1 || return 0 ;;
        aarch64-apple-darwin)
            [[ "$host_os" == "Darwin" && "$host_arch" == "arm64" ]] && return 1 || return 0 ;;
        x86_64-apple-darwin)
            [[ "$host_os" == "Darwin" && "$host_arch" == "x86_64" ]] && return 1 || return 0 ;;
        *) return 0 ;;
    esac
}

# ── Build matrix ─────────────────────────────────────────────────────────────
mkdir -p "$DIST_DIR"
BUILD_LOG="$DIST_DIR/build-${TIMESTAMP}.log"
RESULTS=()
FAILED=()

build_target() {
    local name="$1"
    local triple
    triple="$(target_to_triple "$name")"

    echo ""
    log "━━━ Building: ${BOLD}${name}${NC} (${triple}) ━━━"

    ensure_target "$triple"

    local bin_path=""
    local start_time=$SECONDS

    if [[ "$triple" == "x86_64-pc-windows-gnu" ]]; then
        # Proven #308 recipe: mingw-w64 + plain cargo (NOT zigbuild). The CMake
        # toolchain file cross-compiles the opus C dep; the ring rustls backend
        # (120442e7) removed the aws-lc-sys blocker — trees without it fail in
        # aws-lc-sys, expected until #308 merges.
        local tc_file="${ZEUS_REPO_DIR:-$REPO_DIR}/ci/mingw-w64-x86_64.cmake"
        if [[ ! -f "$tc_file" ]]; then
            FAILED+=("$name")
            err "windows-amd64 needs ci/mingw-w64-x86_64.cmake (the #308 enabler — on feat/windows-tier1 until merged to main)"
            return 1
        fi
        ensure_mingw
        log "Cross-compiling for Windows (mingw-w64)..."
        if ! env CMAKE_POLICY_VERSION_MINIMUM=3.5 \
                 CMAKE_TOOLCHAIN_FILE="$tc_file" \
                 CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
                 cargo build --release --target "$triple" --jobs "$JOBS" >> "$BUILD_LOG" 2>&1; then
            FAILED+=("$name")
            err "Cross-build failed: $name ($triple)"
            warn "Last 5 lines of build log:"
            tail -5 "$BUILD_LOG" | while read -r line; do warn "  $line"; done
            return 1
        fi
        bin_path="$TARGET_ROOT/${triple}/release/zeus.exe"
    elif needs_cross "$triple"; then
        # Cross-compile via cargo-zigbuild (handles linking for foreign targets)
        ensure_zigbuild
        log "Cross-compiling with cargo-zigbuild..."

        # Set up cross pkg-config sysroot for targets that need it
        local cross_env=""
        case "$triple" in
            x86_64-unknown-linux-gnu)
                cross_env="CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc"
                ;;
            aarch64-unknown-linux-gnu)
                cross_env="CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc"
                ;;
        esac

        if ! env $cross_env cargo zigbuild --release --target "$triple" --jobs "$JOBS" >> "$BUILD_LOG" 2>&1; then
            # Fallback: try cargo build with explicit linker
            warn "zigbuild failed, trying cargo build with cross-linker..."
            if ! env $cross_env cargo build --release --target "$triple" --jobs "$JOBS" >> "$BUILD_LOG" 2>&1; then
                FAILED+=("$name")
                err "Cross-build failed: $name ($triple)"
                warn "Hint: cross-compilation requires system dev libraries for the target."
                warn "For full matrix, use CI (release.yml) or build on native hosts."
                warn "Last 5 lines of build log:"
                tail -5 "$BUILD_LOG" | while read -r line; do warn "  $line"; done
                return 1
            fi
        fi
        bin_path="$TARGET_ROOT/${triple}/release/zeus"
    else
        # Native build
        log "Building natively..."
        cargo build --release --jobs "$JOBS" >> "$BUILD_LOG" 2>&1 || {
            FAILED+=("$name")
            err "Build failed: $name ($triple)"
            return 1
        }
        bin_path="$TARGET_ROOT/release/zeus"
    fi

    # Verify binary exists
    if [[ ! -f "$bin_path" ]]; then
        FAILED+=("$name")
        err "Binary not found: $bin_path"
        return 1
    fi

    # Package — .zip for Windows convention, .tar.gz everywhere else
    local pkg_name="zeus-${name}"
    local pkg_dir="$DIST_DIR/${pkg_name}"
    local pkg_file
    mkdir -p "$pkg_dir"
    if [[ "$bin_path" == *.exe ]]; then
        cp "$bin_path" "$pkg_dir/zeus.exe"
        pkg_file="${pkg_name}.zip"
        (cd "$DIST_DIR" && rm -f "$pkg_file" && zip -qr "$pkg_file" "$pkg_name")
    else
        cp "$bin_path" "$pkg_dir/zeus"
        chmod +x "$pkg_dir/zeus"
        pkg_file="${pkg_name}.tar.gz"
        (cd "$DIST_DIR" && tar czf "$pkg_file" "$pkg_name")
    fi
    rm -rf "$pkg_dir"

    local elapsed=$(( SECONDS - start_time ))
    local size
    size="$(du -h "$DIST_DIR/${pkg_file}" | cut -f1)"

    ok "${name}: ${pkg_file} (${size}, ${elapsed}s)"
    RESULTS+=("$name")
}

# Run builds
for target in "${SELECTED_TARGETS[@]}"; do
    build_target "$target" || true
done

# ── OS packages (opt-in, #396 P1) ───────────────────────────────────────────
# Wires the existing scripts/packaging/*.sh against the binaries built above.
# deb/rpm need a linux-* target already built; freebsd needs freebsd-amd64.
# These scripts weren't written for cross-arch invocation — they always stamp
# uname -m of the *build host*, so packaging only makes sense against a target
# that matches this host's native arch (no fake cross-arch .deb/.rpm labels).
package_target() {
    local pkg="$1"
    local host_arch host_triple bin_path
    host_arch="$(uname -m)"

    case "$pkg" in
        macos)
            # .pkg is built by zeus-setup (crates/zeus-setup/src/ops/package.rs),
            # not scripts/packaging/ — it's a Rust binary, not a shell script,
            # because it drives pkgbuild/productbuild's multi-component tree
            # (8 components: CLI, setup, desktop, gateway, workspace, mcp, web,
            # completions) which needs real control flow, not just templating.
            # Requires the native macOS build (this Mac's target/release/, the
            # same path build_target()'s "Native build" branch already uses)
            # to exist for both `zeus` and `zeus-setup` — cargo build --release
            # at the workspace root builds both bins together (zeus-setup is
            # a workspace member with its own [[bin]]), so a prior native
            # build_target run (no --targets filter, or macos-intel/macos-arm
            # matching this host's arch) satisfies it.
            if [[ "$(uname -s)" != "Darwin" ]]; then
                err "--package macos: .pkg installers require pkgbuild/productbuild (Xcode CLT), macOS-only. Not buildable on $(uname -s)."
                FAILED+=("pkg:macos"); return 1
            fi
            bin_path="$TARGET_ROOT/release/zeus"
            local setup_bin="$TARGET_ROOT/release/zeus-setup"
            if [[ ! -f "$bin_path" || ! -f "$setup_bin" ]]; then
                err "--package macos: needs native zeus + zeus-setup binaries at $TARGET_ROOT/release/ (run without --targets, or with macos-intel/macos-arm matching this host's arch, first)"
                FAILED+=("pkg:macos"); return 1
            fi
            if ! command -v pkgbuild >/dev/null 2>&1 || ! command -v productbuild >/dev/null 2>&1; then
                err "--package macos: pkgbuild/productbuild not found — install Xcode Command Line Tools (xcode-select --install)"
                FAILED+=("pkg:macos"); return 1
            fi
            log "Packaging .pkg via zeus-setup (unsigned — pass ZEUS_PKG_SIGN_IDENTITY to sign)..."
            local sign_args=()
            [[ -n "${ZEUS_PKG_SIGN_IDENTITY:-}" ]] && sign_args=(--sign "$ZEUS_PKG_SIGN_IDENTITY")
            # bash 3.2 (macOS default /bin/bash) treats "${arr[@]}" on an EMPTY
            # array as an unbound-variable error under `set -u` (bash 4+ does
            # not). The ${arr[@]+"${arr[@]}"} idiom is the portable guard.
            if "$setup_bin" package --skip-build --non-interactive --version "$VERSION" \
                    --dist-dir "$DIST_DIR/macos-pkg" ${sign_args[@]+"${sign_args[@]}"} >> "$BUILD_LOG" 2>&1; then
                ok "Packaged: macos .pkg → $DIST_DIR/macos-pkg/ (see $BUILD_LOG for exact filename)"
                RESULTS+=("pkg:macos")
            else
                err "--package macos failed — see $BUILD_LOG"
                tail -5 "$BUILD_LOG" | while read -r line; do warn "  $line"; done
                FAILED+=("pkg:macos")
            fi
            ;;
        deb|rpm)
            case "$host_arch" in
                x86_64)  host_triple="x86_64-unknown-linux-gnu" ;;
                aarch64|arm64) host_triple="aarch64-unknown-linux-gnu" ;;
                *) err "--package $pkg: unsupported host arch $host_arch"; FAILED+=("pkg:$pkg"); return 1 ;;
            esac
            if [[ "$(uname -s)" != "Linux" ]]; then
                warn "--package $pkg: cross-packaging a Linux .${pkg} from $(uname -s) is untested by these scripts (they call dpkg-deb/rpmbuild locally, not a container) — building anyway, verify the artifact on a real Linux host before shipping it"
            fi
            bin_path="$TARGET_ROOT/${host_triple}/release/zeus"
            # No fallback to $TARGET_ROOT/release/zeus here: on a non-Linux
            # build host that path is a Mach-O/other-platform binary, and
            # silently packaging the wrong-platform binary into a .deb/.rpm
            # would be a correctness bug, not a convenience.
            if [[ ! -f "$bin_path" ]]; then
                err "--package $pkg: no linux binary found at $bin_path (build linux-amd64 or linux-arm64 first, matching this host's arch: $host_arch)"
                FAILED+=("pkg:$pkg"); return 1
            fi
            local tool="dpkg-deb"; [[ "$pkg" == "rpm" ]] && tool="rpmbuild"
            if ! command -v "$tool" >/dev/null 2>&1; then
                err "--package $pkg: $tool not found. macOS: brew install ${pkg}; Linux: install ${pkg}-build tooling"
                FAILED+=("pkg:$pkg"); return 1
            fi
            log "Packaging .${pkg} from ${bin_path}..."
            if ZEUS_VERSION="$VERSION" "$REPO_DIR/scripts/packaging/build-${pkg}.sh" "$bin_path" >> "$BUILD_LOG" 2>&1; then
                ok "Packaged: ${pkg} → $REPO_DIR/dist/ (see $BUILD_LOG for exact filename)"
                RESULTS+=("pkg:$pkg")
            else
                err "--package $pkg failed — see $BUILD_LOG"
                tail -5 "$BUILD_LOG" | while read -r line; do warn "  $line"; done
                FAILED+=("pkg:$pkg")
            fi
            ;;
        deb-nfpm|rpm-nfpm)
            # #396 P2: nfpm has no host-arch lock (unlike dpkg-deb/rpmbuild
            # above) — it can package ANY built linux-* target regardless of
            # this host's native arch. So instead of a single host_triple
            # guess, walk every linux-amd64/linux-arm64 binary that actually
            # got built this run and package each one found.
            local nfpm_pkg="${pkg%-nfpm}"
            if ! command -v nfpm >/dev/null 2>&1; then
                err "--package $pkg: nfpm not found. macOS: brew install nfpm; Linux: see https://nfpm.goreleaser.com/install/"
                FAILED+=("pkg:$pkg"); return 1
            fi
            local nfpm_triple nfpm_arch nfpm_bin nfpm_built=0
            for nfpm_triple in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
                nfpm_bin="$TARGET_ROOT/${nfpm_triple}/release/zeus"
                [[ -f "$nfpm_bin" ]] || continue
                case "$nfpm_triple" in
                    x86_64-unknown-linux-gnu)  nfpm_arch="amd64" ;;
                    aarch64-unknown-linux-gnu) nfpm_arch="arm64" ;;
                esac
                log "Packaging .${nfpm_pkg} (nfpm) from ${nfpm_bin} [${nfpm_arch}]..."
                if ZEUS_VERSION="$VERSION" ZEUS_NFPM_ARCH="$nfpm_arch" \
                        "$REPO_DIR/scripts/packaging/build-nfpm.sh" "$nfpm_pkg" "$nfpm_bin" >> "$BUILD_LOG" 2>&1; then
                    ok "Packaged (nfpm): ${nfpm_pkg} [${nfpm_arch}] → $REPO_DIR/dist/ (see $BUILD_LOG for exact filename)"
                    RESULTS+=("pkg:$pkg:$nfpm_arch")
                    nfpm_built=$((nfpm_built + 1))
                else
                    err "--package $pkg [${nfpm_arch}] failed — see $BUILD_LOG"
                    tail -5 "$BUILD_LOG" | while read -r line; do warn "  $line"; done
                    FAILED+=("pkg:$pkg:$nfpm_arch")
                fi
            done
            if [[ "$nfpm_built" -eq 0 ]]; then
                err "--package $pkg: no linux-amd64 or linux-arm64 binary found under $TARGET_ROOT/*/release/zeus (build at least one linux-* target first)"
                FAILED+=("pkg:$pkg"); return 1
            fi
            ;;
        freebsd)
            bin_path="$TARGET_ROOT/x86_64-unknown-freebsd/release/zeus"
            # Native-host fallback only valid when this host IS FreeBSD —
            # on any other host $TARGET_ROOT/release/zeus is a wrong-platform
            # binary and must not be silently packaged as a FreeBSD artifact.
            if [[ "$(uname -s)" == "FreeBSD" ]]; then
                [[ -f "$bin_path" ]] || bin_path="$TARGET_ROOT/release/zeus"
            fi
            if [[ ! -f "$bin_path" ]]; then
                err "--package freebsd: no freebsd-amd64 binary found at $bin_path (build freebsd-amd64 first)"
                FAILED+=("pkg:freebsd"); return 1
            fi
            if [[ "$(uname -s)" != "FreeBSD" ]]; then
                warn "--package freebsd: no pkg(8) on $(uname -s) — build-port.sh falls back to a plain .tar.xz, not a real .txz (verified during #395: no cross tool exists for FreeBSD packages off-FreeBSD). Native-host-only for a true pkg(8) artifact — the freebsd box should run this, not this Mac."
            fi
            log "Packaging freebsd port from ${bin_path}..."
            if ZEUS_VERSION="$VERSION" "$REPO_DIR/scripts/packaging/freebsd/build-port.sh" "$bin_path" >> "$BUILD_LOG" 2>&1; then
                ok "Packaged: freebsd"
                RESULTS+=("pkg:freebsd")
            else
                err "--package freebsd failed — see $BUILD_LOG"
                tail -5 "$BUILD_LOG" | while read -r line; do warn "  $line"; done
                FAILED+=("pkg:freebsd")
            fi
            ;;
        msi)
            # #396 P4: wixl (msitools) builds a real .msi cross-platform, no
            # Windows box required (verified: brew install msitools, wixl
            # 0.106) — same "no native host tooling" story as P2's nfpm, but
            # for Windows instead of Linux. Needs the windows-amd64 cross
            # build (mingw-w64 target, #308's recipe) already present.
            bin_path="$TARGET_ROOT/x86_64-pc-windows-gnu/release/zeus.exe"
            if [[ ! -f "$bin_path" ]]; then
                err "--package msi: no windows-amd64 binary found at $bin_path (build --targets windows-amd64 first)"
                FAILED+=("pkg:msi"); return 1
            fi
            if ! command -v wixl >/dev/null 2>&1; then
                err "--package msi: wixl not found. macOS: brew install msitools; Debian/Ubuntu: apt install msitools; Fedora: dnf install msitools"
                FAILED+=("pkg:msi"); return 1
            fi
            log "Packaging .msi from ${bin_path}..."
            if ZEUS_VERSION="$VERSION" "$REPO_DIR/scripts/packaging/build-msi.sh" "$bin_path" >> "$BUILD_LOG" 2>&1; then
                ok "Packaged: msi → $REPO_DIR/dist/ (see $BUILD_LOG for exact filename)"
                RESULTS+=("pkg:msi")
            else
                err "--package msi failed — see $BUILD_LOG"
                tail -5 "$BUILD_LOG" | while read -r line; do warn "  $line"; done
                FAILED+=("pkg:msi")
            fi
            ;;
        *)
            die "Unknown --package value: $pkg (supported: deb, rpm, deb-nfpm, rpm-nfpm, freebsd, macos, msi)"
            ;;
    esac
}

for pkg in ${SELECTED_PACKAGES[@]+"${SELECTED_PACKAGES[@]}"}; do
    package_target "$pkg" || true
done

# ── SHA256SUMS ────────────────────────────────────────────────────────────────
echo ""
log "Generating SHA256SUMS..."
artifact_files=()
for f in "$DIST_DIR"/zeus-*.tar.gz "$DIST_DIR"/zeus-*.zip; do
    [[ -f "$f" ]] && artifact_files+=("${f##*/}")
done
if [[ ${#artifact_files[@]} -gt 0 ]]; then
    (cd "$DIST_DIR" && sha256sum "${artifact_files[@]}" > "SHA256SUMS-${VERSION}")
    ok "SHA256SUMS-${VERSION}:"
    cat "$DIST_DIR/SHA256SUMS-${VERSION}"
else
    warn "No artifacts to checksum"
fi
echo ""

# ── Summary ───────────────────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}═══ Release Build Summary ═══${NC}"
echo "Version:  $VERSION"
echo "Commit:   $CURRENT_SHA"
echo "Artifacts: $DIST_DIR/"
echo ""
echo "Built:"
# ${arr[@]+...} idiom: empty-array "${arr[@]}" is 'unbound variable' under
# set -u on macOS bash 3.2 — killed zero-artifact runs before the honest die.
for r in ${RESULTS[@]+"${RESULTS[@]}"}; do
    echo -e "  ${GREEN}✔${NC} $r"
done
if [[ ${#FAILED[@]} -gt 0 ]]; then
    echo ""
    echo "Failed:"
    for f in "${FAILED[@]}"; do
        echo -e "  ${RED}✖${NC} $f"
    done
fi

echo ""
ls -lh "$DIST_DIR"/*.tar.gz "$DIST_DIR"/SHA256SUMS-* 2>/dev/null

# ── Publish ───────────────────────────────────────────────────────────────────
if $PUBLISH; then
    if ! command -v gh &>/dev/null; then
        die "gh CLI not found — install: https://cli.github.com/"
    fi

    # Refuse to publish an uncommitted version stamp: the release tag must
    # point at a commit whose Cargo.toml already carries this version, or the
    # tagged source won't reproduce the shipped binaries.
    if [[ -n "$(git status --porcelain -- Cargo.toml Cargo.lock)" ]]; then
        die "Cargo.toml/Cargo.lock have uncommitted changes (version stamp?) — commit them before --publish"
    fi

    # Refuse to publish a partial matrix: a release with silently-missing
    # platforms is worse than no release. Re-run failed targets (or drop them
    # with --targets) until the build set is complete.
    if [[ ${#FAILED[@]} -gt 0 ]]; then
        die "Refusing to publish: ${#FAILED[@]} target(s) failed (${FAILED[*]})"
    fi

    echo ""
    log "Creating GitHub release: v${VERSION}"

    # Generate release notes ('local' is only legal inside functions — this
    # block is top-level, and 'local' here killed every --publish run)
    notes_file="$DIST_DIR/release-notes-${VERSION}.md"
    cat > "$notes_file" <<NOTES
## Zeus ${VERSION}

**Commit:** \`${CURRENT_SHA}\`

### Installation

\`\`\`bash
# Linux (amd64)
curl -fsSL https://github.com/zeuslabai/Zeus/releases/download/v${VERSION}/zeus-linux-amd64.tar.gz | tar xz
sudo mv zeus-linux-amd64/zeus /usr/local/bin/zeus

# Linux (arm64)
curl -fsSL https://github.com/zeuslabai/Zeus/releases/download/v${VERSION}/zeus-linux-arm64.tar.gz | tar xz
sudo mv zeus-linux-arm64/zeus /usr/local/bin/zeus

# macOS (Apple Silicon)
curl -fsSL https://github.com/zeuslabai/Zeus/releases/download/v${VERSION}/zeus-macos-arm.tar.gz | tar xz
sudo mv zeus-macos-arm/zeus /usr/local/bin/zeus

# macOS (Intel)
curl -fsSL https://github.com/zeuslabai/Zeus/releases/download/v${VERSION}/zeus-macos-intel.tar.gz | tar xz
sudo mv zeus-macos-intel/zeus /usr/local/bin/zeus
\`\`\`

### Verify checksums
\`\`\`bash
sha256sum -c SHA256SUMS-${VERSION}
\`\`\`

### Artifacts
| Target | File |
|--------|------|
| Linux x86_64 | \`zeus-linux-amd64.tar.gz\` |
| Linux aarch64 | \`zeus-linux-arm64.tar.gz\` |
| macOS Intel | \`zeus-macos-intel.tar.gz\` |
| macOS Apple Silicon | \`zeus-macos-arm.tar.gz\` |
| FreeBSD x86_64 | \`zeus-freebsd-amd64.tar.gz\` |
NOTES

    if [[ -f "$DIST_DIR/zeus-windows-amd64.zip" ]]; then
        cat >> "$notes_file" <<NOTES
| Windows x86_64 | \`zeus-windows-amd64.zip\` |

### Windows
Download \`zeus-windows-amd64.zip\`, extract, and run \`zeus.exe\` from Windows Terminal.
NOTES
    fi

    # Collect artifact paths ('local' is only legal inside functions — same
    # top-level-'local' class of bug that once killed --publish via notes_file)
    artifacts=()
    for f in "$DIST_DIR"/zeus-*.tar.gz "$DIST_DIR"/zeus-*.zip "$DIST_DIR"/SHA256SUMS-*; do
        [[ -f "$f" ]] && artifacts+=("$f")
    done

    if [[ ${#FAILED[@]} -gt 0 ]]; then
        warn "Some builds failed — creating as draft"
        gh release create "v${VERSION}" \
            --repo zeuslabai/Zeus \
            --title "Release ${VERSION}" \
            --notes-file "$notes_file" \
            --draft \
            "${artifacts[@]}"
    else
        gh release create "v${VERSION}" \
            --repo zeuslabai/Zeus \
            --title "Release ${VERSION}" \
            --notes-file "$notes_file" \
            "${artifacts[@]}"
    fi

    ok "Release published: https://github.com/zeuslabai/Zeus/releases/tag/v${VERSION}"
elif $DRY_RUN; then
    echo ""
    echo -e "${YELLOW}═══ DRY-RUN — no release created ═══${NC}"
    echo "To publish:  scripts/release-local.sh --version ${VERSION} --publish"
    echo "To publish:  gh release create v${VERSION} --repo zeuslabai/Zeus dist/release/zeus-*.tar.gz dist/release/SHA256SUMS-*"
fi

echo ""
# Honest exit status: a run that built nothing (or partially failed) must not
# report success — CI, operators, and wrapper scripts key off the exit code.
if [[ ${#RESULTS[@]} -eq 0 ]]; then
    die "No targets built successfully — see $BUILD_LOG"
fi
if [[ ${#FAILED[@]} -gt 0 ]]; then
    warn "Completed with ${#FAILED[@]} failed target(s): ${FAILED[*]}"
    exit 1
fi
ok "Done."
