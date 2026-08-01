#!/usr/bin/env bash
# gen-contract-corpus.sh — regenerate the checked-in contract-corpus goldens
# under types/contract-corpus/ from the real serve handlers and the real
# serde serializer (spec `plan-contract-corpus-goldens`, ask A4).
#
# This is the single source of truth for the corpus-dump invocation. Both the
# documented "regenerate" instructions (docs/serve-api.md) and
# scripts/check-contract-corpus-drift.sh call through this script so the
# invocation can never diverge between the two — mirrors the
# scripts/gen-types.sh / scripts/check-typeshare-drift.sh pair for typeshare.
#
# Mechanics: the corpus is produced by the #[cfg(test)] scenario tests in
# src/serve/contract_corpus.rs's `*_scenarios` modules. With
# BASTION_DUMP_CORPUS=1 set, `dump()` (re)writes each golden to disk instead
# of verifying against it (see that module's doc comment for the full
# generate-vs-verify contract). Only the `*_scenarios::` tests are run here —
# `harness_tests::` is deliberately excluded: those tests exercise the
# harness itself against throwaway temp-dir corpora and must never touch the
# checked-in corpus.
#
# Usage:
#   scripts/gen-contract-corpus.sh              # writes types/contract-corpus/ in place
#   scripts/gen-contract-corpus.sh <output-dir>  # writes to a custom dir (used by the drift check)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

OUTPUT_DIR="${1:-$REPO_ROOT/types/contract-corpus}"

mkdir -p "$OUTPUT_DIR"

cd "$REPO_ROOT"

BASTION_DUMP_CORPUS=1 \
BASTION_CONTRACT_CORPUS_DIR="$OUTPUT_DIR" \
    cargo test --bin bastion "_scenarios::" --quiet
