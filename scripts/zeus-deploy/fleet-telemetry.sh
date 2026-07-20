#!/usr/bin/env bash
#
# fleet-telemetry.sh — append-only failure telemetry for Zeus seats.
#
# Source of truth is local JSONL under $ZEUS_HOME/logs/. A collector can roll
# multiple seat logs into one morning-status artifact, but event writes never
# require network access and never block gateway operation.
#
set -euo pipefail

ZEUS_HOME="${ZEUS_HOME:-$HOME/.zeus}"
LOG_DIR="${ZEUS_LOG_DIR:-$ZEUS_HOME/logs}"
EVENT_LOG="${ZEUS_FLEET_FAILURE_LOG:-$LOG_DIR/fleet-failures.jsonl}"
ROLLUP_OUT="${ZEUS_FLEET_FAILURE_ROLLUP:-$LOG_DIR/fleet-failures-rollup.md}"

usage() {
    cat <<'USAGE'
Usage:
  fleet-telemetry.sh record --kind KIND --severity LEVEL --source SRC --summary TEXT [--sha SHA] [--details TEXT] [--seat NAME]
  fleet-telemetry.sh rollup [LOG ...]

Kinds: cook_timeout, adapter_flap, gate_bounce, deploy_failure, deploy_success
Levels: info, warn, error

Environment:
  ZEUS_HOME                    default: ~/.zeus
  ZEUS_FLEET_FAILURE_LOG       default: $ZEUS_HOME/logs/fleet-failures.jsonl
  ZEUS_FLEET_FAILURE_ROLLUP    default: $ZEUS_HOME/logs/fleet-failures-rollup.md
USAGE
}

utc_now() { date -u +%Y-%m-%dT%H:%M:%SZ; }
host_name() { hostname 2>/dev/null || uname -n 2>/dev/null || printf 'unknown'; }
seat_name() {
    if [ -n "${ZEUS_SEAT:-}" ]; then printf '%s' "$ZEUS_SEAT"; return; fi
    if [ -f "$ZEUS_HOME/IDENTITY.md" ]; then
        awk -F: '/^- \*\*Name\*\*/{gsub(/^[[:space:]]+|[[:space:]]+$/, "", $2); print $2; exit}' "$ZEUS_HOME/IDENTITY.md" 2>/dev/null && return
    fi
    host_name
}

json_escape() {
    python3 -c 'import json,sys; print(json.dumps(sys.stdin.read())[1:-1])' 2>/dev/null || \
        sed 's/\\/\\\\/g; s/"/\\"/g; s/	/\\t/g'
}

write_event() {
    local kind="" severity="" source="" summary="" sha="" details="" seat=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --kind) kind="${2:-}"; shift 2 ;;
            --severity) severity="${2:-}"; shift 2 ;;
            --source) source="${2:-}"; shift 2 ;;
            --summary) summary="${2:-}"; shift 2 ;;
            --sha) sha="${2:-}"; shift 2 ;;
            --details) details="${2:-}"; shift 2 ;;
            --seat) seat="${2:-}"; shift 2 ;;
            -h|--help) usage; exit 0 ;;
            *) echo "unknown record arg: $1" >&2; usage >&2; exit 2 ;;
        esac
    done

    [ -n "$kind" ] || { echo "missing --kind" >&2; exit 2; }
    [ -n "$severity" ] || { echo "missing --severity" >&2; exit 2; }
    [ -n "$source" ] || { echo "missing --source" >&2; exit 2; }
    [ -n "$summary" ] || { echo "missing --summary" >&2; exit 2; }
    [ -n "$seat" ] || seat="$(seat_name)"

    mkdir -p "$(dirname "$EVENT_LOG")"

    local ts host esc_seat esc_host esc_kind esc_sev esc_source esc_summary esc_sha esc_details
    ts="$(utc_now)"
    host="$(host_name)"
    esc_seat="$(printf '%s' "$seat" | json_escape)"
    esc_host="$(printf '%s' "$host" | json_escape)"
    esc_kind="$(printf '%s' "$kind" | json_escape)"
    esc_sev="$(printf '%s' "$severity" | json_escape)"
    esc_source="$(printf '%s' "$source" | json_escape)"
    esc_summary="$(printf '%s' "$summary" | json_escape)"
    esc_sha="$(printf '%s' "$sha" | json_escape)"
    esc_details="$(printf '%s' "$details" | json_escape)"

    printf '{"ts":"%s","seat":"%s","host":"%s","kind":"%s","severity":"%s","source":"%s","summary":"%s","sha":"%s","details":"%s"}\n' \
        "$ts" "$esc_seat" "$esc_host" "$esc_kind" "$esc_sev" "$esc_source" "$esc_summary" "$esc_sha" "$esc_details" >> "$EVENT_LOG"
}

rollup() {
    mkdir -p "$(dirname "$ROLLUP_OUT")"
    if [ "$#" -eq 0 ]; then
        set -- "$EVENT_LOG"
    fi

    awk '
        BEGIN {
            print "# Fleet Failure Telemetry Rollup";
            cmd = "date -u +%Y-%m-%dT%H:%M:%SZ"; cmd | getline now; close(cmd);
            print "";
            print "Generated: " now;
            print "";
        }
        function field(name,   pat, s) {
            pat = "\\\"" name "\\\":\\\"[^\\\"]*\\\"";
            if (match($0, pat)) {
                s = substr($0, RSTART, RLENGTH);
                sub("^\\\"" name "\\\":\\\"", "", s);
                sub("\\\"$", "", s);
                return s;
            }
            return "";
        }
        FNR == 1 { files_seen++ }
        NF {
            total++;
            seat = field("seat"); kind = field("kind"); sev = field("severity"); summary = field("summary"); ts = field("ts"); sha = field("sha");
            if (seat == "") seat = "unknown";
            if (kind == "") kind = "unknown";
            if (sev == "") sev = "unknown";
            by_seat[seat]++;
            by_kind[kind]++;
            by_sev[sev]++;
            if ((sev == "warn" || sev == "error") && examples < 10) {
                examples++;
                ex[examples] = "- " ts " [" sev "] " seat " " kind " " sha " — " summary;
            }
        }
        END {
            print "Events: " total;
            print "Input logs: " files_seen;
            print "";
            print "## By seat";
            for (k in by_seat) print "- " k ": " by_seat[k];
            print "";
            print "## By kind";
            for (k in by_kind) print "- " k ": " by_kind[k];
            print "";
            print "## By severity";
            for (k in by_sev) print "- " k ": " by_sev[k];
            print "";
            print "## Latest warn/error examples";
            if (examples == 0) print "- none";
            for (i = 1; i <= examples; i++) print ex[i];
        }
    ' "$@" > "$ROLLUP_OUT"

    printf '%s\n' "$ROLLUP_OUT"
}

cmd="${1:-}"
case "$cmd" in
    record) shift; write_event "$@" ;;
    rollup) shift; rollup "$@" ;;
    -h|--help|help|"") usage ;;
    *) echo "unknown command: $cmd" >&2; usage >&2; exit 2 ;;
esac
