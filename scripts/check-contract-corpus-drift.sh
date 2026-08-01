#!/usr/bin/env bash
# check-contract-corpus-drift.sh — fail when the committed
# types/contract-corpus/ goldens are stale relative to the real serve
# handlers (spec `plan-contract-corpus-goldens`, ask A4).
#
# Regenerates the corpus to a temp directory (via
# scripts/gen-contract-corpus.sh, the single source of truth for the dump
# invocation) and diffs it against the committed types/contract-corpus/.
# Exits 0 when identical, non-zero (printing the diff) when they differ.
# Mirrors scripts/check-typeshare-drift.sh's structure and exit-code
# conventions.
#
# IMPORTANT: a changed golden in a PR diff IS a contract change — see
# docs/serve-api.md's contract-corpus section for the version-bump +
# Amendment Log rule that applies whenever this check's diff is non-empty
# for a legitimate (non-drift) reason.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

COMMITTED_DIR="$REPO_ROOT/types/contract-corpus"

TMP_DIR="$(mktemp -d /tmp/contract-corpus-XXXXXX)"
trap 'rm -rf "$TMP_DIR"' EXIT

"$SCRIPT_DIR/gen-contract-corpus.sh" "$TMP_DIR"

if diff -ru "$COMMITTED_DIR" "$TMP_DIR" >/dev/null; then
    echo "OK: types/contract-corpus/ is up to date with the real serve handlers."
    exit 0
else
    echo "DRIFT DETECTED: types/contract-corpus/ is stale relative to the real serve handlers." >&2
    echo "Regenerate with: scripts/gen-contract-corpus.sh" >&2
    echo >&2
    diff -ru "$COMMITTED_DIR" "$TMP_DIR" >&2 || true
    exit 1
fi
