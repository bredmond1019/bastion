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

# `command -v typeshare` failing does NOT mean "not installed": cargo installs
# into ~/.cargo/bin, which is not on PATH on every machine (cargo itself often
# resolves from Homebrew instead). Probe that directory before prescribing an
# install, so a PATH gap is never misreported as a missing binary.
if ! command -v typeshare >/dev/null 2>&1; then
    CARGO_BIN_TYPESHARE="${CARGO_HOME:-${HOME:-}/.cargo}/bin/typeshare"
    if [ -x "$CARGO_BIN_TYPESHARE" ]; then
        echo "error: 'typeshare' is installed at $CARGO_BIN_TYPESHARE, but that directory is not on PATH." >&2
        echo "do NOT reinstall it. Add the directory to PATH instead:" >&2
        echo "    export PATH=\"\$HOME/.cargo/bin:\$PATH\"   # add to ~/.zshrc to make it durable" >&2
        echo "or symlink the binary into a directory already on PATH:" >&2
        echo "    ln -s \"$CARGO_BIN_TYPESHARE\" ~/.local/bin/typeshare" >&2
        exit 1
    fi
    echo "error: 'typeshare' CLI not found on PATH." >&2
    echo "install it with: cargo install typeshare-cli --locked" >&2
    exit 1
fi

typeshare "$REPO_ROOT/src/serve" \
    --lang typescript \
    --output-file "$OUTPUT_FILE" \
    --config-file "$REPO_ROOT/typeshare.toml"
