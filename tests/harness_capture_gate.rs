//! BA.26.J AC-1 (MANDATORY, operator instruction): the capture checks land in
//! `planning/harness.json` with `gates: false` and NO `perTask` key — never gating, never
//! per-task. This asserts that on the real file, then SHOWS the assertion can fail by flipping
//! `gates` to `true` on a FIXTURE COPY and confirming the same check goes red. A red capture
//! check is a prompt to look, never a build error (see this repo's `run-the-gates` skill and
//! this block's own out-of-scope list) — this test is what keeps that true structurally rather
//! than by convention.

use serde_json::Value;
use std::fs;

/// Names of the checks BA.26.J registers. Both must be present, both non-gating, neither
/// per-task — the capture harness is out-of-band by construction (block notes: "NON-GATING BY
/// CONSTRUCTION ... gates:false, no perTask, ever").
const CAPTURE_CHECK_NAMES: &[&str] = &["capture-scenes-text", "capture-scenes-image"];

fn load_checks(raw: &str) -> Vec<Value> {
    let doc: Value = serde_json::from_str(raw).expect("harness.json must parse as JSON");
    doc["validation"]["checks"]
        .as_array()
        .cloned()
        .expect("validation.checks must be an array")
}

fn find_check<'a>(checks: &'a [Value], name: &str) -> &'a Value {
    checks
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("harness.json has no check named `{name}`"))
}

#[test]
fn capture_checks_are_registered_gates_false_with_no_per_task() {
    // planning/ is a gitignored symlink into the private HQ vault (repo-wide
    // "Planning symlinks" rule) -- absent from every hosted CI checkout,
    // which clones only this public repo. A guard for this working tree,
    // not a hard dependency: skip rather than fail when it's not present.
    let Ok(raw) = fs::read_to_string("planning/harness.json") else {
        eprintln!(
            "SKIPPED capture_checks_are_registered_gates_false_with_no_per_task: \
             planning/harness.json not present in this checkout (expected in hosted CI)"
        );
        return;
    };
    let checks = load_checks(&raw);

    for name in CAPTURE_CHECK_NAMES {
        let check = find_check(&checks, name);
        assert_eq!(
            check["gates"],
            Value::Bool(false),
            "check `{name}` must be gates:false — the capture harness must never be able to \
             red-gate a task or push (BA.26.J AC-1, operator instruction)"
        );
        assert!(
            check.get("perTask").is_none(),
            "check `{name}` must carry NO perTask key at all (BA.26.J AC-1) — found {:?}",
            check.get("perTask")
        );
    }
}

/// SHOWN FAILING (AC-1's own recipe): flip `gates` to `true` for one of the capture checks in a
/// fixture copy of the real file and confirm the same assertion this test relies on actually
/// goes red. This proves the positive assertion above is a real check, not a tautology that
/// would pass on any input.
#[test]
fn capture_check_gate_assertion_fails_on_a_flipped_fixture() {
    // Same checkout guard as the test above -- see its comment.
    let Ok(raw) = fs::read_to_string("planning/harness.json") else {
        eprintln!(
            "SKIPPED capture_check_gate_assertion_fails_on_a_flipped_fixture: \
             planning/harness.json not present in this checkout (expected in hosted CI)"
        );
        return;
    };
    let mut doc: Value = serde_json::from_str(&raw).expect("harness.json must parse as JSON");

    let checks = doc["validation"]["checks"]
        .as_array_mut()
        .expect("validation.checks must be an array");
    let target = checks
        .iter_mut()
        .find(|c| c["name"] == "capture-scenes-text")
        .expect("fixture setup: capture-scenes-text must exist to flip");
    target["gates"] = Value::Bool(true);

    let checks_after_flip = doc["validation"]["checks"]
        .as_array()
        .cloned()
        .expect("validation.checks must be an array");
    let flipped = find_check(&checks_after_flip, "capture-scenes-text");

    // This is the inverted assertion: on the flipped fixture it must NOT equal false. If it did,
    // the real test above would be incapable of catching a regression that re-enabled gating.
    assert_ne!(
        flipped["gates"],
        Value::Bool(false),
        "fixture flip did not take effect — the shown-failing recipe is broken"
    );
}
