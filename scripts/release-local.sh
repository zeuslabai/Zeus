#!/usr/bin/env bash
# release-local.sh — Local cross-compile release pipeline for Zeus.
#
# Builds the full release matrix from a single machine, generates SHA256SUMS,
# and optionally creates a GitHub release via `gh release create`.
#
# Targets:
#   linux-amd64, linux-arm64, macos-intel, macos-arm, freebsd-amd64
#   (windows deferred to 0.1.3 pending #308)
#
# Usage:
#   scripts/release-local.sh --version 0.1.2              # dry-run (default)
#   scripts/release-local.sh --version 0.1.2 --publish    # create GH release
#   scripts/release-local.sh --version 0.1.2 --targets linux-arm64,linux-amd64
#
# Version stamping: if --version differs from the workspace Cargo.toml version,
# the script stamps Cargo.toml (+ syncs Cargo.lock) so binaries report the same
# version as the release tag. The stamp is left uncommitted for review;
# --publish refuses to run until it is committed.
#
# Requirements:
#   - Rust toolchain (rustup)
#   - cargo-zigbuild + zig (auto-installed if missing on macOS/Linux)
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
SELECTED_TARGETS=()
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

    if needs_cross "$triple"; then
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
        bin_path="target/${triple}/release/zeus"
    else
        # Native build
        log "Building natively..."
        cargo build --release --jobs "$JOBS" >> "$BUILD_LOG" 2>&1 || {
            FAILED+=("$name")
            err "Build failed: $name ($triple)"
            return 1
        }
        bin_path="target/release/zeus"
    fi

    # Verify binary exists
    if [[ ! -f "$bin_path" ]]; then
        FAILED+=("$name")
        err "Binary not found: $bin_path"
        return 1
    fi

    # Package
    local pkg_name="zeus-${name}"
    local pkg_dir="$DIST_DIR/${pkg_name}"
    mkdir -p "$pkg_dir"
    cp "$bin_path" "$pkg_dir/zeus"
    chmod +x "$pkg_dir/zeus"

    # Create tarball
    (cd "$DIST_DIR" && tar czf "${pkg_name}.tar.gz" "$pkg_name")
    rm -rf "$pkg_dir"

    local elapsed=$(( SECONDS - start_time ))
    local size
    size="$(du -h "$DIST_DIR/${pkg_name}.tar.gz" | cut -f1)"

    ok "${name}: ${pkg_name}.tar.gz (${size}, ${elapsed}s)"
    RESULTS+=("$name")
}

# Run builds
for target in "${SELECTED_TARGETS[@]}"; do
    build_target "$target" || true
done

# ── SHA256SUMS ────────────────────────────────────────────────────────────────
echo ""
log "Generating SHA256SUMS..."
tarballs=("$DIST_DIR"/zeus-*.tar.gz)
if [[ ${#tarballs[@]} -gt 0 && -f "${tarballs[0]}" ]]; then
    (cd "$DIST_DIR" && sha256sum zeus-*.tar.gz > "SHA256SUMS-${VERSION}")
else
    warn "No tarballs to checksum"
fi
ok "SHA256SUMS-${VERSION}:"
cat "$DIST_DIR/SHA256SUMS-${VERSION}"
echo ""

# ── Summary ───────────────────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}═══ Release Build Summary ═══${NC}"
echo "Version:  $VERSION"
echo "Commit:   $CURRENT_SHA"
echo "Artifacts: $DIST_DIR/"
echo ""
echo "Built:"
for r in "${RESULTS[@]}"; do
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

    # Collect artifact paths
    local artifacts=()
    for f in "$DIST_DIR"/zeus-*.tar.gz "$DIST_DIR"/SHA256SUMS-*; do
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
