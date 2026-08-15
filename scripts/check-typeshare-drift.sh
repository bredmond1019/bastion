#!/usr/bin/env bash
# check-typeshare-drift.sh — fail when the committed types/serve.ts is stale
# relative to src/serve/dto.rs and okf-core's annotated state types.
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
#
# When the probe finds the binary, **use it and carry on** rather than refusing
# to run. The earlier form exited 1 here, which made this gate unrunnable from
# any shell that doesn't source the user's rc files — notably the
# non-interactive shells SDLC subagents get, which read neither ~/.zshrc nor
# ~/.zprofile no matter how durably a human "fixes" PATH in them. That cost
# wave 277 a bail on a machine where typeshare was installed, executable, and
# correct. Blocking a verifiable binary buys no safety; the warning below keeps
# the underlying env gap visible without holding the gate hostage to it.
if ! command -v typeshare >/dev/null 2>&1; then
    CARGO_BIN_TYPESHARE="${CARGO_HOME:-${HOME:-}/.cargo}/bin/typeshare"
    if [ -x "$CARGO_BIN_TYPESHARE" ]; then
        echo "warning: 'typeshare' is installed at $CARGO_BIN_TYPESHARE, but that directory is not on PATH." >&2
        echo "do NOT reinstall it — using the installed binary directly and continuing." >&2
        echo "to silence this, add the directory to PATH in the environment that runs this script:" >&2
        echo "    export PATH=\"\$HOME/.cargo/bin:\$PATH\"" >&2
        PATH="$(dirname "$CARGO_BIN_TYPESHARE"):$PATH"
        export PATH
    else
        echo "error: 'typeshare' CLI not found on PATH." >&2
        echo "install it with: cargo install typeshare-cli --locked" >&2
        exit 1
    fi
fi

TMP_FILE="$(mktemp /tmp/typeshare-serve-XXXXXX.ts)"
trap 'rm -f "$TMP_FILE"' EXIT

"$SCRIPT_DIR/gen-types.sh" "$TMP_FILE"

if diff -u "$COMMITTED_FILE" "$TMP_FILE" >/dev/null; then
    echo "OK: types/serve.ts is up to date with src/serve/dto.rs and okf-core/src."
    exit 0
else
    echo "DRIFT DETECTED: types/serve.ts is stale relative to src/serve/dto.rs and okf-core/src." >&2
    echo "Regenerate with: scripts/gen-types.sh" >&2
    echo >&2
    diff -u "$COMMITTED_FILE" "$TMP_FILE" >&2 || true
    exit 1
fi
