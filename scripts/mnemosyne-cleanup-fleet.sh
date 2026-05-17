#!/usr/bin/env bash
# ╔══════════════════════════════════════════════════════════════════════════╗
# ║  mnemosyne-cleanup-fleet.sh — Run mnemosyne_cleanup --apply across fleet ║
# ║  Captures before/after row counts per node. Skips unreachable (10s).    ║
# ╚══════════════════════════════════════════════════════════════════════════╝
set -uo pipefail

SSH_TIMEOUT="${SSH_TIMEOUT:-10}"
SSH_OPTS=(
    -o BatchMode=yes
    -o StrictHostKeyChecking=accept-new
    -o ConnectTimeout="$SSH_TIMEOUT"
    -o ServerAliveInterval=15
    -o ServerAliveCountMax=2
)

# host:user:zeus_dir:db_path
MACOS_NODES=(
    "192.168.1.112:mike:/Users/mike/.zeus:/Users/mike/.zeus/mnemosyne.db"
    "192.168.1.106:mike:/Users/mike/.zeus:/Users/mike/.zeus/mnemosyne.db"
    "192.168.1.102:mike:/Users/mike/.zeus:/Users/mike/.zeus/mnemosyne.db"
    "192.168.1.107:mike:/Users/mike/.zeus:/Users/mike/.zeus/mnemosyne.db"
)

FREEBSD_NODES=(
    "192.168.1.224:mike:/home/mike/.zeus:/home/mike/.zeus/mnemosyne.db"
    "192.168.1.225:mike:/home/mike/.zeus:/home/mike/.zeus/mnemosyne.db"
    "192.168.1.226:mike:/home/mike/.zeus:/home/mike/.zeus/mnemosyne.db"
)

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

RESULT_NODES=(); RESULT_STATUS=(); RESULT_BEFORE=(); RESULT_AFTER=(); RESULT_DELETED=()

record() {
    RESULT_NODES+=("$1"); RESULT_STATUS+=("$2")
    RESULT_BEFORE+=("$3"); RESULT_AFTER+=("$4"); RESULT_DELETED+=("$5")
}

reachable() {
    ssh "${SSH_OPTS[@]}" "${2}@${1}" "true" >/dev/null 2>&1
}

# Count rows in mnemosyne DB via remote sqlite3
row_count() {
    local host="$1" user="$2" db="$3"
    ssh "${SSH_OPTS[@]}" "${user}@${host}" \
        "sqlite3 '$db' 'SELECT COUNT(*) FROM messages;' 2>/dev/null || echo -1" \
        2>/dev/null | tr -d '[:space:]'
}

cleanup_node() {
    local entry="$1"
    local host="${entry%%:*}"
    local rest="${entry#*:}"
    local user="${rest%%:*}"; rest="${rest#*:}"
    local zdir="${rest%%:*}"
    local db="${rest##*:}"

    log "→ ${B}${host}${N} (db: ${db})"

    if ! reachable "$host" "$user"; then
        err "${host} unreachable"
        record "$host" "UNREACHABLE" "-" "-" "-"
        return
    fi

    # Verify DB exists
    if ! ssh "${SSH_OPTS[@]}" "${user}@${host}" "test -f '$db'" 2>/dev/null; then
        err "${host} db missing: $db"
        record "$host" "NO_DB" "-" "-" "-"
        return
    fi

    local before
    before="$(row_count "$host" "$user" "$db")"
    [[ -z "$before" || "$before" == "-1" ]] && before="?"
    log "  before: ${before} rows"

    # Locate or build the binary, then run with --apply
    local cmd
    cmd="cd '$zdir' && \
        BIN=\"\$(find target/release -maxdepth 1 -name mnemosyne_cleanup -type f 2>/dev/null | head -1)\"; \
        if [ -z \"\$BIN\" ]; then \
            echo '[node] building mnemosyne_cleanup...' >&2; \
            cargo build --release --bin mnemosyne_cleanup -p zeus-mnemosyne >&2 || exit 91; \
            BIN=target/release/mnemosyne_cleanup; \
        fi; \
        \"\$BIN\" --db '$db' --apply"

    local logfile
    logfile="$(mktemp -t zeus-mcleanup-${host}.XXXXXX)"
    if ssh "${SSH_OPTS[@]}" "${user}@${host}" "$cmd" >"$logfile" 2>&1; then
        local after
        after="$(row_count "$host" "$user" "$db")"
        [[ -z "$after" || "$after" == "-1" ]] && after="?"
        local deleted="?"
        if [[ "$before" =~ ^[0-9]+$ && "$after" =~ ^[0-9]+$ ]]; then
            deleted=$((before - after))
        fi
        ok "${host} cleaned · ${before} → ${after} (Δ ${deleted})"
        record "$host" "OK" "$before" "$after" "$deleted"
    else
        local rc=$?
        local tail_line
        tail_line="$(tail -n 3 "$logfile" | tr '\n' '|' | head -c 120)"
        err "${host} cleanup failed (rc=${rc})"
        printf "${D}    %s${N}\n" "$tail_line"
        record "$host" "FAIL" "$before" "-" "rc=${rc}"
    fi
    rm -f "$logfile"
}

header "Mnemosyne Cleanup Rollout — $(date +'%Y-%m-%d %H:%M:%S')"
log "ssh timeout: ${SSH_TIMEOUT}s · nodes: $((${#MACOS_NODES[@]} + ${#FREEBSD_NODES[@]}))"

header "macOS nodes (${#MACOS_NODES[@]})"
for n in "${MACOS_NODES[@]}"; do cleanup_node "$n"; done

header "FreeBSD nodes (${#FREEBSD_NODES[@]})"
for n in "${FREEBSD_NODES[@]}"; do cleanup_node "$n"; done

header "Summary"
printf "${B}%-18s  %-12s  %10s  %10s  %10s${N}\n" "NODE" "STATUS" "BEFORE" "AFTER" "DELETED"
printf "${D}%s${N}\n" "──────────────────────────────────────────────────────────────────────"

ok_count=0; fail_count=0; skip_count=0; total_deleted=0
for i in "${!RESULT_NODES[@]}"; do
    node="${RESULT_NODES[$i]}"; status="${RESULT_STATUS[$i]}"
    before="${RESULT_BEFORE[$i]}"; after="${RESULT_AFTER[$i]}"; deleted="${RESULT_DELETED[$i]}"
    case "$status" in
        OK)          color="$G"; ok_count=$((ok_count + 1))
                     [[ "$deleted" =~ ^[0-9]+$ ]] && total_deleted=$((total_deleted + deleted)) ;;
        UNREACHABLE|NO_DB) color="$Y"; skip_count=$((skip_count + 1)) ;;
        *)           color="$R"; fail_count=$((fail_count + 1)) ;;
    esac
    printf "%-18s  ${color}%-12s${N}  %10s  %10s  %10s\n" \
        "$node" "$status" "$before" "$after" "$deleted"
done

printf "\n${B}Totals:${N} ${G}%d ok${N} · ${R}%d failed${N} · ${Y}%d skipped${N} · ${B}%d rows deleted${N}\n" \
    "$ok_count" "$fail_count" "$skip_count" "$total_deleted"

[[ "$fail_count" -eq 0 ]]
