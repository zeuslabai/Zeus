#!/bin/bash
# Zeus Gateway Health-Poll Watchdog (single-shot)
# Polls /health endpoint ONCE and restarts via service manager if 3 consecutive failures.
# Designed to be run by a systemd timer (Type=oneshot) every 60s.
# Persists consecutive-failure count to state file so it survives reboots.
#
# Usage: zeus-watchdog.sh [host:port]
# Default: 127.0.0.1:8080

set -euo pipefail

HOST="${1:-127.0.0.1:8080}"
HEALTH_URL="http://${HOST}/health"
MAX_FAILURES=3
STATE_FILE="/var/run/zeus-watchdog.state"
LOG_TAG="zeus-watchdog"
LOG_FILE="/var/log/zeus/watchdog.log"

# Ensure log directory exists
mkdir -p "$(dirname "$LOG_FILE")"

log() {
    local level="$1"
    shift
    local msg="$*"
    local ts
    ts=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
    echo "[$ts] [$level] $msg" | tee -a "$LOG_FILE"
    logger -t "$LOG_TAG" "[$level] $msg" 2>/dev/null || true
}

check_health() {
    local response
    
    # Try to fetch health endpoint with 10s timeout
    if ! response=$(curl -sf --max-time 10 "$HEALTH_URL" 2>/dev/null); then
        return 1
    fi
    
    # Check if response contains "status":"ok"
    if echo "$response" | grep -q '"status":"ok"'; then
        return 0
    else
        return 1
    fi
}

restart_gateway() {
    local reason="$1"
    log "WARN" "Restarting gateway: $reason"
    
    # Detect OS and restart via service manager (never bare kill+nohup — #333 invariant)
    if [[ "$(uname)" == "FreeBSD" ]]; then
        service zeus_gateway restart 2>&1 | tee -a "$LOG_FILE"
    elif [[ -d /run/systemd/system ]]; then
        systemctl restart zeus-gateway.service 2>&1 | tee -a "$LOG_FILE"
    elif [[ "$(uname)" == "Darwin" ]]; then
        # macOS: kickstart the system daemon (authoritative plist)
        sudo launchctl kickstart -k system/com.zeus.gateway 2>&1 | tee -a "$LOG_FILE"
    else
        log "ERROR" "Unknown OS — cannot restart gateway via service manager"
        return 1
    fi
    
    log "INFO" "Gateway restart triggered"
}

# Read persisted failure count (default 0)
read_failure_count() {
    if [[ -f "$STATE_FILE" ]]; then
        cat "$STATE_FILE" 2>/dev/null || echo 0
    else
        echo 0
    fi
}

# Write failure count to state file
write_failure_count() {
    echo "$1" > "$STATE_FILE"
}

# Main — single health check, then exit
main() {
    local consecutive_failures
    consecutive_failures=$(read_failure_count)
    
    if check_health; then
        if [ "$consecutive_failures" -gt 0 ]; then
            log "INFO" "Health check recovered after $consecutive_failures failure(s)"
        fi
        write_failure_count 0
    else
        consecutive_failures=$((consecutive_failures + 1))
        log "WARN" "Health check failed ($consecutive_failures/$MAX_FAILURES)"
        write_failure_count "$consecutive_failures"
        
        if [ "$consecutive_failures" -ge "$MAX_FAILURES" ]; then
            restart_gateway "3 consecutive health check failures"
            write_failure_count 0
        fi
    fi
}

# Run if not sourced
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    main "$@"
fi
