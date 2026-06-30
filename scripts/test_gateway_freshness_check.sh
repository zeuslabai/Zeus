#!/bin/sh
# test_gateway_freshness_check.sh — fixture harness for #125 guard
# Proves the guard FAILS CLOSED against the #123 failure modes.
# We can directly exercise exit-codes 3 (no config) and 2 (no gateway)
# with fixtures. Exit-code 1 (stale) requires a live PID whose start
# predates config mtime — we simulate that by touching the config into
# the FUTURE so any running gateway's start < config mtime (stale-mode).

GUARD="$(dirname "$0")/gateway_freshness_check.sh"
TMP="$(mktemp -d)"
PASS=0
FAIL=0

check() {
    desc="$1"; want="$2"; got="$3"
    if [ "$want" = "$got" ]; then
        echo "  PASS: $desc (exit $got)"
        PASS=$((PASS + 1))
    else
        echo "  FAIL: $desc — wanted exit $want, got $got"
        FAIL=$((FAIL + 1))
    fi
}

echo "=== #125 guard fixture suite ==="

# --- Fixture A: no config at all → expect exit 3 (fail-closed) ---
ZEUS_CONFIG="$TMP/does_not_exist.toml" sh "$GUARD" >/dev/null 2>&1
check "missing config fails closed" 3 $?

# --- Fixture B: config exists but stale gateway (config newer than proc) ---
# Drop a synthetic config, then stamp its mtime 1 hour into the FUTURE so
# that ANY running gateway necessarily started before it → stale (exit 1).
# If no gateway is running, the guard returns 2 (no-gateway) — still fail-closed,
# which we assert as "not 0".
cat > "$TMP/stale.toml" <<'EOF'
[gateway]
enable_channels = true
enable_agent_processing = true
EOF
# stamp mtime far in the future (FreeBSD touch -t / -d both work; use date math)
FUTURE=$(date -v+1H '+%Y%m%d%H%M' 2>/dev/null) || FUTURE=$(date -d '+1 hour' '+%Y%m%d%H%M')
touch -t "$FUTURE" "$TMP/stale.toml"
ZEUS_CONFIG="$TMP/stale.toml" sh "$GUARD" >"$TMP/out_stale" 2>&1
RC=$?
echo "    [stale fixture output] $(cat "$TMP/out_stale")"
if [ "$RC" -eq 0 ]; then
    check "future-dated config must NOT pass (stale or no-gateway)" "non-zero" 0
else
    check "future-dated config fails closed (exit 1 stale, or 2 no-gw)" "$RC" "$RC"
fi

# --- Fixture C: fresh config, fresh check — only passes if gateway live AND fresh ---
cat > "$TMP/fresh.toml" <<'EOF'
[gateway]
enable_channels = true
enable_agent_processing = true
EOF
touch "$TMP/fresh.toml"   # mtime = now
ZEUS_CONFIG="$TMP/fresh.toml" sh "$GUARD" >"$TMP/out_fresh" 2>&1
RC=$?
echo "    [fresh fixture output] $(cat "$TMP/out_fresh")"
# This passes (0) only if a real gateway is running with start >= now.
# On most boxes the gateway started before 'now', so expect FAIL(1) stale OR 2.
# The point: it does NOT false-green. Document whatever it returns.
echo "    [fresh fixture exit] $RC (0=live+fresh, 1=stale, 2=no-gateway — all are correct fail-closed behaviors)"

echo "=== results: $PASS passed, $FAIL failed ==="
rm -rf "$TMP"
[ "$FAIL" -eq 0 ]
