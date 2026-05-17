#!/bin/bash
#
# Fleet Deployment Script for Zeus Agent Platform
# Deploys and manages the Zeus agent fleet across hosts
#

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
ZEUS_HOME="${ZEUS_HOME:-$HOME/.zeus}"
WORKSPACE="${ZEUS_HOME}/workspace"
CONFIG_FILE="${ZEUS_HOME}/config.toml"
LOG_DIR="${ZEUS_HOME}/logs"
AGENT_FLEET=("zeus100" "zeus106" "zeus107" "fbsd2" "raspizeus" "ZeusMarketing")

# Command usage
usage() {
    cat << EOF
Fleet Deployment Script for Zeus Agent Platform

Usage: $0 [COMMAND] [OPTIONS]

Commands:
    status              Show status of all agents in the fleet
    deploy [agent]      Deploy specific agent or all agents
    restart [agent]     Restart specific agent or all agents
    stop [agent]        Stop specific agent or all agents
    logs [agent]        Show logs for specific agent
    update              Update all agents to latest version
    health              Run health check on fleet
    init                Initialize fleet configuration

Options:
    -h, --help          Show this help message
    -v, --verbose       Verbose output
    -q, --quiet         Quiet mode (errors only)

Examples:
    $0 status                    # Show fleet status
    $0 deploy                    # Deploy all agents
    $0 deploy zeus100            # Deploy only zeus100
    $0 restart raspizeus         # Restart raspizeus
    $0 logs zeus107              # Show logs for zeus107
EOF
}

# Logging functions
log_info() { echo -e "${BLUE}[INFO]${NC} $*"; }
log_success() { echo -e "${GREEN}[OK]${NC} $*"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $*"; }
log_error() { echo -e "${RED}[ERROR]${NC} $*"; }

# Check if Zeus is installed
check_zeus() {
    if [[ ! -d "$ZEUS_HOME" ]]; then
        log_error "Zeus not found at $ZEUS_HOME"
        log_info "Run install script first or set ZEUS_HOME"
        exit 1
    fi
    
    if [[ ! -f "$CONFIG_FILE" ]]; then
        log_warn "Config file not found: $CONFIG_FILE"
    fi
}

# Get agent status
get_agent_status() {
    local agent="$1"
    local pid_file="${ZEUS_HOME}/${agent}.pid"
    
    if [[ -f "$pid_file" ]]; then
        local pid
        pid=$(cat "$pid_file" 2>/dev/null)
        if kill -0 "$pid" 2>/dev/null; then
            echo "running"
        else
            echo "dead"
        fi
    else
        echo "stopped"
    fi
}

# Get agent task from HEARTBEAT.md
get_agent_task() {
    local agent="$1"
    local heartbeat_file="${ZEUS_HOME}/workspace/HEARTBEAT.md"
    
    if [[ -f "$heartbeat_file" ]]; then
        # Extract CURRENT TASK section
        grep -A 20 "## CURRENT TASK" "$heartbeat_file" 2>/dev/null | head -10
    else
        echo "No HEARTBEAT.md found"
    fi
}

# Show fleet status
cmd_status() {
    log_info "Fleet Status Report"
    echo "==================="
    printf "%-15s %-10s %-20s\n" "AGENT" "STATUS" "PID"
    echo "-------------------------------------------"
    
    local running=0
    local stopped=0
    local dead=0
    
    for agent in "${AGENT_FLEET[@]}"; do
        local status
        status=$(get_agent_status "$agent")
        local pid="-"
        local pid_file="${ZEUS_HOME}/${agent}.pid"
        
        if [[ -f "$pid_file" ]]; then
            pid=$(cat "$pid_file" 2>/dev/null || echo "?")
        fi
        
        case "$status" in
            running)
                printf "%-15s ${GREEN}%-10s${NC} %-20s\n" "$agent" "$status" "$pid"
                ((running++))
                ;;
            dead)
                printf "%-15s ${RED}%-10s${NC} %-20s\n" "$agent" "$status" "$pid"
                ((dead++))
                ;;
            stopped)
                printf "%-15s ${YELLOW}%-10s${NC} %-20s\n" "$agent" "$status" "$pid"
                ((stopped++))
                ;;
        esac
    done
    
    echo "-------------------------------------------"
    log_info "Running: $running | Stopped: $stopped | Dead: $dead"
}

# Deploy agent
cmd_deploy() {
    local target="${1:-all}"
    
    if [[ "$target" == "all" ]]; then
        log_info "Deploying entire fleet..."
        for agent in "${AGENT_FLEET[@]}"; do
            deploy_agent "$agent"
        done
    else
        deploy_agent "$target"
    fi
}

# Deploy single agent
deploy_agent() {
    local agent="$1"
    log_info "Deploying $agent..."
    
    # Check if already running
    local status
    status=$(get_agent_status "$agent")
    
    if [[ "$status" == "running" ]]; then
        log_warn "$agent is already running, restarting..."
        stop_agent "$agent"
        sleep 2
    fi
    
    # Start agent
    start_agent "$agent"
}

# Start agent
start_agent() {
    local agent="$1"
    local log_file="${LOG_DIR}/${agent}.log"
    local pid_file="${ZEUS_HOME}/${agent}.pid"
    
    mkdir -p "$LOG_DIR"
    
    log_info "Starting $agent..."
    
    # This is a placeholder - actual implementation depends on Zeus architecture
    # In real deployment, this would:
    # 1. Source agent environment
    # 2. Start agent process
    # 3. Write PID file
    # 4. Verify startup
    
    # Simulated for now
    echo "$$" > "$pid_file"
    log_success "$agent started (PID: $$)"
}

# Stop agent
stop_agent() {
    local agent="$1"
    local pid_file="${ZEUS_HOME}/${agent}.pid"
    
    if [[ -f "$pid_file" ]]; then
        local pid
        pid=$(cat "$pid_file")
        log_info "Stopping $agent (PID: $pid)..."
        
        if kill "$pid" 2>/dev/null; then
            rm -f "$pid_file"
            log_success "$agent stopped"
        else
            log_error "Failed to stop $agent"
            rm -f "$pid_file"
        fi
    else
        log_warn "$agent is not running"
    fi
}

# Restart agent
cmd_restart() {
    local target="${1:-all}"
    
    if [[ "$target" == "all" ]]; then
        log_info "Restarting entire fleet..."
        for agent in "${AGENT_FLEET[@]}"; do
            stop_agent "$agent"
        done
        sleep 2
        for agent in "${AGENT_FLEET[@]}"; do
            start_agent "$agent"
        done
    else
        stop_agent "$target"
        sleep 2
        start_agent "$target"
    fi
}

# Stop agents
cmd_stop() {
    local target="${1:-all}"
    
    if [[ "$target" == "all" ]]; then
        log_info "Stopping entire fleet..."
        for agent in "${AGENT_FLEET[@]}"; do
            stop_agent "$agent"
        done
    else
        stop_agent "$target"
    fi
}

# Show logs
cmd_logs() {
    local agent="${1:-}"
    
    if [[ -z "$agent" ]]; then
        log_error "Please specify an agent name"
        exit 1
    fi
    
    local log_file="${LOG_DIR}/${agent}.log"
    
    if [[ -f "$log_file" ]]; then
        tail -f "$log_file"
    else
        log_error "No logs found for $agent"
        exit 1
    fi
}

# Update all agents
cmd_update() {
    log_info "Updating fleet to latest version..."
    
    # Pull latest changes
    if [[ -d "$WORKSPACE/.git" ]]; then
        cd "$WORKSPACE"
        git fetch origin
        git pull origin main
        log_success "Workspace updated"
    fi
    
    # Restart fleet to pick up changes
    cmd_restart all
}

# Health check
cmd_health() {
    log_info "Running fleet health check..."
    
    local issues=0
    
    for agent in "${AGENT_FLEET[@]}"; do
        local status
        status=$(get_agent_status "$agent")
        
        if [[ "$status" != "running" ]]; then
            log_warn "$agent is not running (status: $status)"
            ((issues++))
        fi
        
        # Check heartbeat file age
        local heartbeat="${ZEUS_HOME}/workspace/HEARTBEAT.md"
        if [[ -f "$heartbeat" ]]; then
            local age
            age=$(($(date +%s) - $(stat -c %Y "$heartbeat" 2>/dev/null || stat -f %m "$heartbeat")))
            if [[ $age -gt 7200 ]]; then  # 2 hours
                log_warn "$agent heartbeat is stale (${age}s old)"
                ((issues++))
            fi
        fi
    done
    
    if [[ $issues -eq 0 ]]; then
        log_success "Fleet is healthy"
    else
        log_error "Found $issues issue(s)"
        exit 1
    fi
}

# Initialize fleet
cmd_init() {
    log_info "Initializing fleet configuration..."
    
    mkdir -p "$LOG_DIR"
    mkdir -p "$WORKSPACE"
    
    # Create agent directories
    for agent in "${AGENT_FLEET[@]}"; do
        mkdir -p "${ZEUS_HOME}/${agent}"
    done
    
    log_success "Fleet initialized"
}

# Main
main() {
    local command="${1:-}"
    local target="${2:-all}"
    
    case "$command" in
        status)
            check_zeus
            cmd_status
            ;;
        deploy)
            check_zeus
            cmd_deploy "$target"
            ;;
        restart)
            check_zeus
            cmd_restart "$target"
            ;;
        stop)
            check_zeus
            cmd_stop "$target"
            ;;
        logs)
            cmd_logs "$target"
            ;;
        update)
            check_zeus
            cmd_update
            ;;
        health)
            check_zeus
            cmd_health
            ;;
        init)
            cmd_init
            ;;
        -h|--help|help)
            usage
            exit 0
            ;;
        *)
            usage
            exit 1
            ;;
    esac
}

main "$@"
