#!/usr/bin/env bash
# gen-types.sh — regenerate the TypeScript type definitions for serve/dto.rs.
#
# This is the single source of truth for the typeshare invocation. Both the
# documented "regenerate" command and scripts/check-typeshare-drift.sh call
# through this script so the invocation can never diverge between the two.
#
# Usage:
#   scripts/gen-types.sh              # writes types/serve.ts in place
#   scripts/gen-types.sh <output-file> # writes to a custom path (used by the drift check)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

OUTPUT_FILE="${1:-$REPO_ROOT/types/serve.ts}"

if ! command -v typeshare >/dev/null 2>&1; then
    echo "error: 'typeshare' CLI not found on PATH." >&2
    echo "install it with: cargo install typeshare-cli --locked" >&2
    exit 1
fi

typeshare "$REPO_ROOT/src/serve" \
    --lang typescript \
    --output-file "$OUTPUT_FILE" \
    --config-file "$REPO_ROOT/typeshare.toml"
