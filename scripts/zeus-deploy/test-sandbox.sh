#!/usr/bin/env bash
# Hermetic smoke test for Zeus deploy-on-merge + failure telemetry.
# Does not touch real ~/.zeus, /usr/local/bin/zeus, rc.d, launchd, or systemd.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$SCRIPT_DIR/../.." && pwd)"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/zeus-deploy-sandbox.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

SHA="$(git -C "$REPO" rev-parse --short=8 HEAD)"
ZEUS_HOME="$TMP/.zeus"
FAKE_BUILT="$TMP/fake-zeus-built"
HEALTH_FILE="$TMP/health.json"

mkdir -p "$ZEUS_HOME/logs" "$TMP/bin"
printf '{"status":"ok"}\n' > "$HEALTH_FILE"
printf 'ts=now target=adapter event="connected" adapter connected\n' > "$ZEUS_HOME/logs/gateway.log"

cat > "$FAKE_BUILT" <<FAKE
#!/usr/bin/env sh
if [ "\${1:-}" = "--version" ]; then
  echo "zeus 0.1.2 ($SHA)"
  exit 0
fi
echo "fake zeus"
FAKE
chmod +x "$FAKE_BUILT"

ZEUS_HOME="$ZEUS_HOME" \
ZEUS_REPO="$REPO" \
ZEUS_DEPLOY_SANDBOX=1 \
ZEUS_DEPLOY_NO_FETCH=1 \
ZEUS_DEPLOY_NO_BUILD=1 \
ZEUS_DEPLOY_ALLOW_DIRTY=1 \
ZEUS_DEPLOY_BUILT_BIN="$FAKE_BUILT" \
ZEUS_DEPLOY_HEALTH_URL="file://$HEALTH_FILE" \
"$SCRIPT_DIR/deploy-on-merge.sh"

INSTALLED="$ZEUS_HOME/bin/zeus"
[ -x "$INSTALLED" ] || { echo "ASSERT FAIL: sandbox binary not installed" >&2; exit 1; }
"$INSTALLED" --version | grep -F "$SHA" >/dev/null || { echo "ASSERT FAIL: installed binary SHA mismatch" >&2; exit 1; }

grep -F "sha=$SHA" "$ZEUS_HOME/deploy/last-deploy" >/dev/null || { echo "ASSERT FAIL: deploy stamp missing SHA" >&2; exit 1; }
grep -F '"kind":"deploy_success"' "$ZEUS_HOME/logs/fleet-failures.jsonl" >/dev/null || { echo "ASSERT FAIL: deploy_success telemetry missing" >&2; exit 1; }

ZEUS_HOME="$ZEUS_HOME" "$SCRIPT_DIR/fleet-telemetry.sh" record \
  --kind gate_bounce \
  --severity warn \
  --source test-sandbox \
  --summary "sandbox gate bounce" \
  --sha "$SHA"
ROLLUP="$(ZEUS_HOME="$ZEUS_HOME" "$SCRIPT_DIR/fleet-telemetry.sh" rollup)"
[ -f "$ROLLUP" ] || { echo "ASSERT FAIL: telemetry rollup missing" >&2; exit 1; }
grep -F "gate_bounce" "$ROLLUP" >/dev/null || { echo "ASSERT FAIL: rollup missing gate_bounce" >&2; exit 1; }

if [ -e /usr/local/bin/zeus.sandbox-test-should-not-exist ]; then
  echo "ASSERT FAIL: touched live install path sentinel" >&2
  exit 1
fi

printf 'ASSERT PASS: sandbox deploy installed %s under %s only\n' "$SHA" "$ZEUS_HOME"
printf 'ASSERT PASS: telemetry JSONL + rollup created under sandbox logs\n'
