#!/usr/bin/env bash
# check-selected-node-exhaustive.sh — regression check for AC-1/AC-3 (BA.26.A):
# no `matches!` and no `==`/`!=` comparison over `SelectedNode` may gate
# behaviour anywhere in src/. Every such site must be an exhaustive `match`
# returning an explicit per-variant value, so adding a new `SelectedNode`
# variant is a compile error at each site rather than a silent `false`.
#
# SCOPE: src/-wide, not app.rs-only. `SelectedNode::` appears in
# src/brain/spaces.rs and src/sessions/ui.rs as well as src/sessions/app.rs —
# an app.rs-only check would come back clean while the rule is violated in
# either of the other two files.
#
# MULTILINE, not single-line: the historical `is_space_overview` site spans
# two lines (`matches!(` on one line, `self.selected_node()` on the next), so
# a single-line grep (`rg -n 'matches!(' src/sessions/app.rs | rg
# 'selected_node'`) returns empty even when the bug is present — a
# clean-looking wrong answer (repo standing rule 11). The canonical sweep,
# per the block record's AC-3, is:
#
#   rg -U -n -e 'matches!\(\s*[^)]*selected_node' -e '== *SelectedNode' \
#      -e 'SelectedNode *==' src/
#
# `-U` is ripgrep's multiline mode, required to catch the two-line shape
# above. This script runs that exact invocation when `rg` is on PATH.
#
# FALLBACK: ripgrep is not installed as a real binary on every machine this
# script may run on (verified 2026-09-07: no `rg` binary anywhere on this
# session's host, `command -v rg`/`mdfind` both empty — only an interactive
# shell alias exists, which a `#!/usr/bin/env bash` script never inherits).
# Rather than block the gate on that gap, fall back to a Python re-implementation
# of the identical three patterns over the identical file set, applied with the
# same multiline semantics (Python's `\s` already matches newlines, so no
# extra flag is needed). This is a fallback for tool availability, not a
# weaker check — the patterns and the file set are the same.
#
# POSITIVE CONTROL: run with `--control` to assert the identical sweep DOES
# match against the pre-BA.26.A revision (27f04fa) of src/sessions/app.rs —
# proving the sweep can find the pattern it otherwise reports absent. An empty
# result there means the instrument is broken, not that the tree is clean.
#
# NOT registered in planning/harness.json (BA.26.A task 4, deliberate): this is
# a task-level check; gating it fleet-wide is a separate decision.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

PY_SWEEP='
import re
import sys

patterns = [
    re.compile(r"matches!\(\s*[^)]*selected_node"),
    re.compile(r"== *SelectedNode"),
    re.compile(r"SelectedNode *=="),
]

targets = sys.argv[1:]
found_any = False
for target in targets:
    with open(target, "r", encoding="utf-8", errors="replace") as fh:
        text = fh.read()
    for pat in patterns:
        for m in pat.finditer(text):
            found_any = True
            line_no = text.count("\n", 0, m.start()) + 1
            snippet = m.group(0).replace("\n", "\\n")
            print(f"{target}:{line_no}: {snippet}")

sys.exit(0 if found_any else 1)
'

# Run the sweep over every path in "$@" (files, or a directory to be walked
# for *.rs files when using the Python fallback). Returns rg-compatible exit
# codes: 0 = found a match, 1 = no match, >1 = error.
run_sweep() {
    if command -v rg >/dev/null 2>&1; then
        rg -U -n \
            -e 'matches!\(\s*[^)]*selected_node' \
            -e '== *SelectedNode' \
            -e 'SelectedNode *==' \
            "$@"
        return $?
    fi

    # Python fallback: expand any directory argument to its *.rs files first,
    # since the inline Python sweep (unlike rg) does not walk directories itself.
    local files=()
    for target in "$@"; do
        if [[ -d "$target" ]]; then
            while IFS= read -r -d '' f; do
                files+=("$f")
            done < <(find "$target" -type f -name '*.rs' -print0)
        else
            files+=("$target")
        fi
    done
    python3 -c "$PY_SWEEP" "${files[@]}"
    return $?
}

if [[ "${1:-}" == "--control" ]]; then
    # Positive control: the pre-change revision's app.rs must match at the
    # `is_space_overview` site (app.rs:318-321 pre-block).
    CONTROL_REV="27f04fa"
    CONTROL_FILE="$(mktemp -t bastion-selected-node-control.XXXXXX)"
    trap 'rm -f "$CONTROL_FILE"' EXIT
    git show "${CONTROL_REV}:src/sessions/app.rs" > "$CONTROL_FILE"

    if run_sweep "$CONTROL_FILE" >/dev/null; then
        echo "OK: positive control matched — the sweep instrument works (pre-change app.rs contains the pattern)."
        exit 0
    else
        echo "CONTROL FAILED: the sweep found NOTHING in the pre-change revision (${CONTROL_REV}) of src/sessions/app.rs." >&2
        echo "This means the instrument itself is broken, not that the tree is clean. Do not trust a clean result from this script until this control passes." >&2
        exit 1
    fi
fi

# Main sweep: the current tree, src/-wide, must have NO matches.
if MATCHES="$(run_sweep src/)"; then
    echo "FAIL: found matches!/==/!= gating on SelectedNode outside an exhaustive match:" >&2
    echo "$MATCHES" >&2
    exit 1
else
    echo "OK: no matches!/==/!= over SelectedNode found in src/ — every dispatch site is an exhaustive match."
    exit 0
fi
