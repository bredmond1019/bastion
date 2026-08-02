#!/usr/bin/env bash
# check-typeshare-drift.sh — fail when the committed types/serve.ts is stale
# relative to src/serve/dto.rs.
#
# Regenerates the TypeScript types to a temp file (via scripts/gen-types.sh, the
# single source of truth for the typeshare invocation) and diffs it against the
# committed types/serve.ts. Exits 0 when identical, non-zero (printing the diff)
# when they differ.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

COMMITTED_FILE="$REPO_ROOT/types/serve.ts"

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

TMP_FILE="$(mktemp /tmp/typeshare-serve-XXXXXX.ts)"
trap 'rm -f "$TMP_FILE"' EXIT

"$SCRIPT_DIR/gen-types.sh" "$TMP_FILE"

if diff -u "$COMMITTED_FILE" "$TMP_FILE" >/dev/null; then
    echo "OK: types/serve.ts is up to date with src/serve/dto.rs."
    exit 0
else
    echo "DRIFT DETECTED: types/serve.ts is stale relative to src/serve/dto.rs." >&2
    echo "Regenerate with: scripts/gen-types.sh" >&2
    echo >&2
    diff -u "$COMMITTED_FILE" "$TMP_FILE" >&2 || true
    exit 1
fi
