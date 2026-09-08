// Positive-control fixture for scripts/check-selected-node-exhaustive.sh --control.
//
// Not real source — a minimal excerpt reproducing the SHAPE of the
// pre-BA.26.A `is_space_overview` site (a `matches!` gate over
// `self.selected_node()`, spanning two lines) that the sweep's patterns
// exist to catch. The sweep must find a match here; if it does not, the
// instrument itself is broken, not the tree.
fn is_space_overview(&self) -> bool {
    matches!(
        self.selected_node(),
        SelectedNode::MissionControl | SelectedNode::Hq
    )
}
