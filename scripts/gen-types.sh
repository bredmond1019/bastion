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
#
# When the probe finds the binary, **use it and carry on** rather than refusing
# to run — see the companion comment in check-typeshare-drift.sh for why the
# earlier exit-1 form made this unrunnable from SDLC subagent shells.
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

# okf-core is the schema crate whose four BlockedBy payload structs
# (BlockDep, ExternalDep, OperatorDep, ApprovalDep) must also reach
# types/serve.ts — Cargo.toml already pins it as a sibling path dependency
# (`okf-core = { path = "../okf-core" }`), so this is the same sibling
# relationship, not a new one. Guard its presence explicitly: a missing
# directory must not silently generate a types/serve.ts that is quietly
# missing those four types (see planning/knowledge.md's scan-scope entry).
OKF_CORE_SRC="$REPO_ROOT/../okf-core/src"
if [ ! -d "$OKF_CORE_SRC" ]; then
    echo "error: expected okf-core source tree at $OKF_CORE_SRC, but it does not exist." >&2
    echo "typeshare must scan both src/serve and okf-core/src, or types/serve.ts would be" >&2
    echo "silently generated without the BlockDep/ExternalDep/OperatorDep/ApprovalDep interfaces." >&2
    exit 1
fi

typeshare "$REPO_ROOT/src/serve" "$OKF_CORE_SRC" \
    --lang typescript \
    --output-file "$OUTPUT_FILE" \
    --config-file "$REPO_ROOT/typeshare.toml"
