// tokio event loop: keyboard navigation + DB poll every N seconds + redraw on change.

use anyhow::Result;
use crate::monitor::app::App;

pub async fn run_event_loop(_app: &mut App, _poll_secs: u64) -> Result<()> {
    todo!("Phase 1: crossterm KeyEvent handler + tokio interval for DB poll")
}
