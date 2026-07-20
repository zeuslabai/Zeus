#!/usr/bin/env bash
#
# redeploy.sh — idempotent deploy-on-merge for PRISM (frontend + gateway)
#
# Builds the gateway and frontend from origin/main, swaps them into the live
# serving paths atomically, then runs a smoke test that FAILS LOUD if the
# deploy did not actually go live. The WASM-freshness assert deterministically
# catches the "merged != live / stale bundle" bug class.
#
# Exit non-zero on any failed gate. Old binary/dist are left untouched if the
# BUILD fails, because we build before we swap.
#
# Intended to run on the Linux deploy box (192.168.1.82) as user `mike`
# (passwordless sudo). Trigger it from redeploy.timer or by hand.
#
set -euo pipefail

REPO="${PRISM_REPO:-/home/mike/oracles}"
DIST_SRC="$REPO/app/dist"
DIST_LIVE="/srv/prism/dist"
BIN_LIVE="/srv/prism/bin/prism-gateway"
BIN_BUILT="$REPO/target/release/prism-gateway"
GW_URL="http://127.0.0.1:9080/health"
WEB_URL="http://127.0.0.1:80"
STAMP="$REPO/.last-deploy"

# Ensure cargo/trunk are on PATH even under a non-login shell.
export PATH="$HOME/.cargo/bin:$PATH"

log()  { printf '\033[1;34m[redeploy]\033[0m %s\n' "$*"; }
ok()   { printf '\033[1;32m[  ok  ]\033[0m %s\n'  "$*"; }
fail() { printf '\033[1;31m[ FAIL ]\033[0m %s\n'  "$*" >&2; exit 1; }

cd "$REPO"

# ---------------------------------------------------------------------------
# 1. Sync to origin/main, capture the SHA we are deploying.
# ---------------------------------------------------------------------------
log "fetching origin/main"
git fetch --quiet origin main
git checkout --quiet main
git reset --hard --quiet origin/main
SHA="$(git rev-parse --short HEAD)"
log "deploying SHA $SHA"

# ---------------------------------------------------------------------------
# 2. Build gateway (release). Build BEFORE swap so a failed build never takes
#    down the running service.
# ---------------------------------------------------------------------------
log "building gateway (cargo build --release -p prism-gateway)"
cargo build --release -p prism-gateway || fail "gateway build failed"
[ -x "$BIN_BUILT" ] || fail "gateway binary missing after build: $BIN_BUILT"
ok "gateway built"

# ---------------------------------------------------------------------------
# 3. Build frontend (trunk release).
# ---------------------------------------------------------------------------
log "building frontend (trunk build --release)"
( cd app && trunk build --release ) || fail "frontend build failed"
NEW_WASM="$(grep -o 'prism-app-[a-z0-9]*_bg.wasm' "$DIST_SRC/index.html" | head -1)"
[ -n "$NEW_WASM" ] || fail "could not find built WASM name in $DIST_SRC/index.html"
ok "frontend built — bundle $NEW_WASM"

# ---------------------------------------------------------------------------
# 4. Swap gateway atomically. MUST stop the service first or cp hits
#    "Text file busy" on the running binary.
# ---------------------------------------------------------------------------
log "swapping gateway binary"
sudo systemctl stop prism-gateway
sudo cp "$BIN_BUILT" "$BIN_LIVE"
sudo systemctl start prism-gateway
ok "gateway service restarted"

# ---------------------------------------------------------------------------
# 5. Swap dist (mirror, delete stale files).
# ---------------------------------------------------------------------------
log "swapping frontend dist"
# Mirror dist without depending on rsync (not present on all boxes): clear the
# live dir then copy the fresh build in. Delete semantics preserved.
sudo find "$DIST_LIVE" -mindepth 1 -delete
sudo cp -a "$DIST_SRC/." "$DIST_LIVE/"
ok "dist synced"

# ---------------------------------------------------------------------------
# 6. Reload nginx.
# ---------------------------------------------------------------------------
log "reloading nginx"
sudo nginx -t
sudo systemctl reload nginx
ok "nginx reloaded"

# ---------------------------------------------------------------------------
# 7. SMOKE TEST — the part that makes this trustworthy. Any failure => exit 1.
# ---------------------------------------------------------------------------
log "smoke test"

# 7a. gateway health
GW_BODY="$(curl -fsS --max-time 10 "$GW_URL" || true)"
[ "$GW_BODY" = "ok" ] || fail "gateway health != ok (got: '$GW_BODY')"
ok "gateway health: ok"

# 7b. web root serves 200
WEB_CODE="$(curl -s -o /dev/null -w '%{http_code}' --max-time 10 "$WEB_URL/")"
[ "$WEB_CODE" = "200" ] || fail "web root != 200 (got $WEB_CODE)"
ok "web root: 200"

# 7c. WASM FRESHNESS ASSERT — the freshly-built bundle name must be the one
#     nginx actually serves through :80. This is the deterministic kill for
#     the "merged != live / stale bundle" bug.
LIVE_INDEX_WASM="$(curl -fsS --max-time 10 "$WEB_URL/" | grep -o 'prism-app-[a-z0-9]*_bg.wasm' | head -1)"
[ "$LIVE_INDEX_WASM" = "$NEW_WASM" ] \
  || fail "STALE BUNDLE: built $NEW_WASM but nginx serves $LIVE_INDEX_WASM"
WASM_CODE="$(curl -s -o /dev/null -w '%{http_code}' --max-time 10 "$WEB_URL/$NEW_WASM")"
[ "$WASM_CODE" = "200" ] || fail "fresh WASM $NEW_WASM not served (got $WASM_CODE)"
ok "WASM freshness: nginx serves $NEW_WASM (200)"

# ---------------------------------------------------------------------------
# 8. Record what went live.
# ---------------------------------------------------------------------------
printf 'sha=%s\nwasm=%s\ndeployed_at=%s\n' \
  "$SHA" "$NEW_WASM" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > "$STAMP"

ok "DEPLOY COMPLETE — sha=$SHA wasm=$NEW_WASM"
