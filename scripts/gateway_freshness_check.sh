#!/bin/sh
# gateway_freshness_check.sh — #125 hygiene guard
# Asserts the gateway process started AT OR AFTER the last config.toml write.
# Catches the "stale in-memory config after re-onboard" failure class.
# Exit 0 = healthy, 1 = stale gateway, 2 = gateway not running, 3 = no config.
# FreeBSD-native (ps -o lstart, stat -f %m). Portable fallback for Linux below.

CONFIG="${ZEUS_CONFIG:-$HOME/.zeus/config.toml}"

[ -f "$CONFIG" ] || { echo "FAIL(3): config not found: $CONFIG"; exit 3; }

# config mtime (epoch). FreeBSD: stat -f %m ; Linux: stat -c %Y
if stat -f %m "$CONFIG" >/dev/null 2>&1; then
    CFG_MTIME=$(stat -f %m "$CONFIG")
else
    CFG_MTIME=$(stat -c %Y "$CONFIG")
fi

# gateway pid (the 'zeus gateway' process, not the agent)
# pgrep -f is unreliable on this FreeBSD build; use ps directly.
PID=$(ps -axo pid,command | grep 'zeus gateway' | grep -v grep | awk '{print $1}' | head -n1)
[ -n "$PID" ] || { echo "FAIL(2): no 'zeus gateway' process running"; exit 2; }

# process start epoch. FreeBSD: ps -o lstart -> parse; use etimes for robustness.
# etimes = elapsed seconds since start; start_epoch = now - etimes.
ETIMES=$(ps -o etimes= -p "$PID" 2>/dev/null | tr -d ' ')
if [ -n "$ETIMES" ]; then
    NOW=$(date +%s)
    PROC_START=$((NOW - ETIMES))
else
    echo "FAIL(2): cannot read start time for PID $PID"; exit 2
fi

if [ "$PROC_START" -ge "$CFG_MTIME" ]; then
    echo "OK: gateway PID $PID started ${PROC_START} >= config mtime ${CFG_MTIME} (fresh)"
    exit 0
else
    DELTA=$((CFG_MTIME - PROC_START))
    echo "FAIL(1): STALE gateway PID $PID started ${PROC_START} < config mtime ${CFG_MTIME} (config is ${DELTA}s newer than process — restart required)"
    exit 1
fi
