#!/usr/bin/env bash
# check-serve-api-version.sh — fail when docs/serve/serve-api.md's version
# header and its §21 Versioning policy statement disagree, or when either
# cannot be found (spec `BA.ticket.serve-auth-boundary-freeze` task 5).
#
# Extracts two facts from the contract doc:
#   1. the header version — the line matching `^\*\*Version:\*\*\s*(vX.Y.Z)`
#   2. the §21-stated version — the sentence
#      `The current contract is **vX.Y.Z**.`
# and exits non-zero when they differ, OR when either pattern is absent from
# the file. A missing file is likewise a FAILURE, never a silent pass.
#
# The header pattern is line-ANCHORED (`^\*\*Version:\*\*`) on purpose — an
# unanchored substring match is how a prose sentence elsewhere in the doc
# that merely mentions a version gets mistaken for the authoritative header
# line. The §21 pattern matches the full distinctive sentence "The current
# contract is **vX.Y.Z**." (not line-anchored, since that sentence shares a
# line with preceding prose in the real doc) — the phrase itself is specific
# enough that no unrelated sentence can be mistaken for it.
#
# Usage:
#   scripts/check-serve-api-version.sh              # check the real doc
#   scripts/check-serve-api-version.sh --self-test   # prove the check can fail
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Extract the header version (`**Version:** vX.Y.Z`) from a file.
# Prints nothing (and returns non-zero) when the anchored pattern is absent.
extract_header_version() {
    local file="$1"
    grep -m1 -E '^\*\*Version:\*\*[[:space:]]*v[0-9]+\.[0-9]+(\.[0-9]+)?' "$file" \
        | sed -E 's/^\*\*Version:\*\*[[:space:]]*(v[0-9]+\.[0-9]+(\.[0-9]+)?).*/\1/'
}

# Extract the §21-stated version (`The current contract is **vX.Y.Z**.`) from a file.
#
# Not line-anchored (the sentence can share a line with preceding prose, e.g.
# "`bastion-ui` MUST pin to a specific version tag.  The current contract is
# **v0.27**.") — the distinctive full phrase "The current contract is
# **vX.Y.Z**." is itself specific enough to rule out an unrelated prose
# sentence being mistaken for it.
extract_section21_version() {
    local file="$1"
    grep -m1 -oE 'The current contract is \*\*v[0-9]+\.[0-9]+(\.[0-9]+)?\*\*\.' "$file" \
        | sed -E 's/^The current contract is \*\*(v[0-9]+\.[0-9]+(\.[0-9]+)?)\*\*\.$/\1/'
}

# Run the check against a given file. Prints a verdict line and returns
# 0 (versions agree) or 1 (missing pattern(s), or a disagreement).
check_file() {
    local file="$1"

    if [ ! -f "$file" ]; then
        echo "error: $file does not exist" >&2
        return 1
    fi

    local header_version section21_version
    header_version="$(extract_header_version "$file" || true)"
    section21_version="$(extract_section21_version "$file" || true)"

    if [ -z "$header_version" ]; then
        echo "error: no '**Version:**' header line found in $file" >&2
        return 1
    fi

    if [ -z "$section21_version" ]; then
        echo "error: no '§21 The current contract is **vX.Y.Z**.' sentence found in $file" >&2
        return 1
    fi

    if [ "$header_version" != "$section21_version" ]; then
        echo "error: version mismatch in $file — header says $header_version, §21 says $section21_version" >&2
        return 1
    fi

    echo "OK: $file agrees on $header_version (header and §21 Versioning policy)"
    return 0
}

# ── Self-test: prove the check can fail before it is trusted ───────────────
#
# A committed red fixture would leave a registered harness gate red for every
# concurrent lane, which is forbidden — so this builds two throwaway fixtures
# under mktemp at runtime, asserts PASS on the agreeing one and FAIL on the
# disagreeing one, then cleans up.
run_self_test() {
    local tmp_dir
    tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/serve-api-version-selftest-XXXXXX")"
    # `trap ... RETURN` is a shell-wide setting, not scoped to this function —
    # left registered, it would fire again (referencing an out-of-scope
    # $tmp_dir under `set -u`) when the NEXT function returns. Clear it
    # explicitly on every exit path instead of relying on it to self-clear.
    trap 'rm -rf "$tmp_dir"; trap - RETURN' RETURN

    local agree_file="$tmp_dir/agree.md"
    local disagree_file="$tmp_dir/disagree.md"

    cat >"$agree_file" <<'EOF'
# serve-api — v1.0.0 Contract

**Version:** v1.0.0

## 21. Versioning policy

`bastion-ui` MUST pin to a specific version tag.  The current contract is **v1.0.0**.
EOF

    cat >"$disagree_file" <<'EOF'
# serve-api — v1.0.0 Contract

**Version:** v1.0.0

## 21. Versioning policy

`bastion-ui` MUST pin to a specific version tag.  The current contract is **v1.0.1**.
EOF

    local failures=0

    if check_file "$agree_file" >/dev/null 2>&1; then
        echo "self-test: PASS on agreeing fixture — ok"
    else
        echo "self-test: FAILED — expected PASS on agreeing fixture, got FAIL" >&2
        failures=1
    fi

    if check_file "$disagree_file" >/dev/null 2>&1; then
        echo "self-test: FAILED — expected FAIL on disagreeing fixture, got PASS" >&2
        failures=1
    else
        echo "self-test: FAIL on disagreeing fixture — ok"
    fi

    local missing_file="$tmp_dir/missing.md"
    cat >"$missing_file" <<'EOF'
# serve-api — no version markers here
EOF
    if check_file "$missing_file" >/dev/null 2>&1; then
        echo "self-test: FAILED — expected FAIL when both patterns are absent, got PASS" >&2
        failures=1
    else
        echo "self-test: FAIL when both patterns absent — ok"
    fi

    if [ "$failures" -ne 0 ]; then
        echo "self-test: FAILED — the check did not fail when it should have" >&2
        return 1
    fi

    echo "self-test: all cases behaved as expected — the check can fail."
    return 0
}

main() {
    if [ "${1:-}" = "--self-test" ]; then
        run_self_test
        exit $?
    fi

    # Default invocation runs the self-test first, so the check is shown
    # capable of failing before its verdict on the real doc is trusted.
    run_self_test

    check_file "$REPO_ROOT/docs/serve/serve-api.md"
}

main "$@"
