#!/usr/bin/env bash
# ╔══════════════════════════════════════════════════════════════════════════╗
# ║  rebuild-fleet.sh — Deploy latest Zeus binary to all fleet nodes via SSH ║
# ║  Detects OS per node, runs install.sh --update (macOS) or update.sh     ║
# ║  (FreeBSD), reports per-node status. Skips unreachable nodes (10s).     ║
# ╚══════════════════════════════════════════════════════════════════════════╝
set -uo pipefail

# ── Config ──────────────────────────────────────────────────────────────────
SSH_TIMEOUT="${SSH_TIMEOUT:-10}"
SSH_OPTS=(
    -o BatchMode=yes
    -o StrictHostKeyChecking=accept-new
    -o ConnectTimeout="$SSH_TIMEOUT"
    -o ServerAliveInterval=15
    -o ServerAliveCountMax=2
)

# Node lists. Format: "host:user:zeus_dir"
MACOS_NODES=(
    "192.168.1.112:mike:/Users/mike/.zeus"
    "192.168.1.106:mike:/Users/mike/.zeus"
    "192.168.1.102:mike:/Users/mike/.zeus"
    "192.168.1.107:mike:/Users/mike/.zeus"
)

FREEBSD_NODES=(
    "192.168.1.224:mike:/home/mike/.zeus"
    "192.168.1.225:mike:/home/mike/.zeus"
    "192.168.1.226:mike:/home/mike/.zeus"
)

# ── Theme ───────────────────────────────────────────────────────────────────
if [[ -t 1 ]]; then
    R="\033[38;5;196m"; G="\033[38;5;46m"; Y="\033[38;5;220m"
    D="\033[38;5;240m"; B="\033[1m"; N="\033[0m"
else
    R=""; G=""; Y=""; D=""; B=""; N=""
fi

log()    { printf "${D}[%s]${N} %b\n" "$(date +%H:%M:%S)" "$*"; }
ok()     { printf "${G}✔${N} %b\n" "$*"; }
warn()   { printf "${Y}⚠${N} %b\n" "$*"; }
err()    { printf "${R}✘${N} %b\n" "$*"; }
header() { printf "\n${B}${R}━━━ %s ━━━${N}\n" "$*"; }

# ── Results table (parallel arrays for bash 3.x compatibility) ──────────────
RESULT_NODES=()
RESULT_STATUS=()
RESULT_DETAIL=()

record() {
    RESULT_NODES+=("$1")
    RESULT_STATUS+=("$2")
    RESULT_DETAIL+=("$3")
}

# ── Reachability probe ──────────────────────────────────────────────────────
reachable() {
    local host="$1" user="$2"
    ssh "${SSH_OPTS[@]}" "${user}@${host}" "true" >/dev/null 2>&1
}

# ── Detect OS via uname ─────────────────────────────────────────────────────
detect_os() {
    local host="$1" user="$2"
    ssh "${SSH_OPTS[@]}" "${user}@${host}" "uname -s" 2>/dev/null
}

# ── Update one node ─────────────────────────────────────────────────────────
update_node() {
    local entry="$1" expected_os="$2"
    local host="${entry%%:*}"
    local rest="${entry#*:}"
    local user="${rest%%:*}"
    local zdir="${rest##*:}"

    log "→ ${B}${host}${N} (${user}, ${zdir})"

    if ! reachable "$host" "$user"; then
        err "${host} unreachable (timeout ${SSH_TIMEOUT}s)"
        record "$host" "UNREACHABLE" "ssh timeout"
        return
    fi

    local actual_os
    actual_os="$(detect_os "$host" "$user" || true)"
    log "  os: ${actual_os:-unknown} (expected ${expected_os})"

    local cmd
    case "$actual_os" in
        Darwin)
            cmd="cd '$zdir' && git pull --ff-only && ./scripts/install.sh --update"
            ;;
        FreeBSD)
            # FreeBSD nodes use update.sh (or install.sh --update if available)
            cmd="cd '$zdir' && git pull --ff-only && \
                 if [ -x scripts/update.sh ]; then ./scripts/update.sh; \
                 else ./scripts/install.sh --update; fi"
            ;;
        Linux)
            cmd="cd '$zdir' && git pull --ff-only && ./scripts/install.sh --update"
            ;;
        *)
            err "${host} unknown OS: '${actual_os}'"
            record "$host" "FAIL" "unknown OS: ${actual_os:-empty}"
            return
            ;;
    esac

    local logfile
    logfile="$(mktemp -t zeus-rebuild-${host}.XXXXXX)"
    if ssh "${SSH_OPTS[@]}" "${user}@${host}" "$cmd" >"$logfile" 2>&1; then
        local tail_line
        tail_line="$(tail -n 1 "$logfile" | tr -d '\r' | head -c 80)"
        ok "${host} updated · ${D}${tail_line}${N}"
        record "$host" "OK" "${actual_os}"
    else
        local rc=$?
        local tail_line
        tail_line="$(tail -n 3 "$logfile" | tr '\n' '|' | head -c 120)"
        err "${host} update failed (rc=${rc})"
        printf "${D}    %s${N}\n" "$tail_line"
        record "$host" "FAIL" "rc=${rc}: ${tail_line}"
    fi
    rm -f "$logfile"
}

# ── Main ────────────────────────────────────────────────────────────────────
header "Zeus Fleet Rebuild — $(date +'%Y-%m-%d %H:%M:%S')"
log "ssh timeout: ${SSH_TIMEOUT}s · nodes: $((${#MACOS_NODES[@]} + ${#FREEBSD_NODES[@]}))"

header "macOS nodes (${#MACOS_NODES[@]})"
for n in "${MACOS_NODES[@]}"; do update_node "$n" "Darwin"; done

header "FreeBSD nodes (${#FREEBSD_NODES[@]})"
for n in "${FREEBSD_NODES[@]}"; do update_node "$n" "FreeBSD"; done

# ── Summary table ───────────────────────────────────────────────────────────
header "Summary"
printf "${B}%-18s  %-12s  %s${N}\n" "NODE" "STATUS" "DETAIL"
printf "${D}%s${N}\n" "─────────────────────────────────────────────────────────────────"

ok_count=0; fail_count=0; skip_count=0
for i in "${!RESULT_NODES[@]}"; do
    node="${RESULT_NODES[$i]}"
    status="${RESULT_STATUS[$i]}"
    detail="${RESULT_DETAIL[$i]}"
    case "$status" in
        OK)          color="$G"; ok_count=$((ok_count + 1)) ;;
        UNREACHABLE) color="$Y"; skip_count=$((skip_count + 1)) ;;
        *)           color="$R"; fail_count=$((fail_count + 1)) ;;
    esac
    printf "%-18s  ${color}%-12s${N}  ${D}%s${N}\n" "$node" "$status" "$detail"
done

printf "\n${B}Totals:${N} ${G}%d ok${N} · ${R}%d failed${N} · ${Y}%d skipped${N}\n" \
    "$ok_count" "$fail_count" "$skip_count"

# Exit non-zero if any node failed (skipped is not a failure)
[[ "$fail_count" -eq 0 ]]
