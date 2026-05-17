#!/usr/bin/env sh
set -eu

# Zeus — Update Script
# Thin wrapper: git pull → install.sh --update
# Usage: ./scripts/update.sh [--fresh] [--with-identity] [--branch NAME]

FRESH=false
WITH_IDENTITY=""
BRANCH="main"

for arg in "$@"; do
    case "$arg" in
        --fresh)         FRESH=true ;;
        --with-identity) WITH_IDENTITY="--with-identity" ;;
        --branch)        shift; BRANCH="$1" ;;
        -h|--help)
            echo "Usage: update.sh [--fresh] [--with-identity] [--branch NAME]"
            echo "  --fresh         Clear sessions and restart after update"
            echo "  --with-identity Also refresh workspace identity templates (passed to install.sh)"
            echo "  --branch NAME   Pull from specific branch (default: main)"
            exit 0 ;;
    esac
done

cd "$(dirname "$0")/.." || exit 1

echo "==> Pulling origin/$BRANCH..."
git pull origin "$BRANCH"

echo "==> Running install.sh --update $WITH_IDENTITY..."
./scripts/install.sh --update $WITH_IDENTITY

if $FRESH; then
    echo "==> Clearing sessions (--fresh)..."
    rm -f ~/.zeus/sessions/*.jsonl 2>/dev/null || true
    zeus daemon restart 2>/dev/null || true
    echo "==> Sessions cleared, gateway restarted."
fi

echo "==> Done."
