// `bastion abort <run>` — operator-facing abort switch.
//
// Thin shell over `api::client::abort_run` (task 4). Per D25, bastion only
// *triggers* an abort — it never cancels a run itself, writes the `events`
// row, or touches Celery/Redis.
//
// Naming: this is the block's authored `src/run/kill.rs`, renamed to
// `src/run/abort.rs` because `bastion kill <session>` already ships for tmux
// session control — `abort` matches the endpoint it calls and avoids a name
// collision with a shipped surface. See the spec's *Naming deviation*.
//
// Degrades gracefully on every outcome — including a missing config or a
// connection failure reaching the engine — following the established
// `costs::run` style: an actionable operator message and a graceful `Ok(())`
// return, never a panic.

use std::io::{self, Write};
use std::time::Instant;

use anyhow::Result;

use crate::api::client::{AbortOutcome, ApiClient};
use crate::config::Config;
use crate::observ::{self, errors::ConsoleError};

// ── Pure: confirmation prompt/parse ─────────────────────────────────────────

/// Build the confirmation prompt shown before sending an abort request.
///
/// Pure function — no I/O — so it is unit-testable without stdin/stdout.
pub fn confirm_prompt(run_id: &str) -> String {
    format!("About to abort run '{run_id}'. This cannot be undone. Continue? [y/N] ")
}

/// Parse a line of operator input into a yes/no confirmation.
///
/// Accepts `"y"` or `"yes"` (case-insensitive, surrounding whitespace
/// ignored) as confirmation; everything else — including empty input —
/// declines. Pure function — no I/O.
pub fn parse_confirmation(input: &str) -> bool {
    matches!(input.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

// ── Pure: outcome rendering ─────────────────────────────────────────────────

/// Render the outcome of an abort attempt as a distinct, actionable operator
/// message.
///
/// One branch per outcome the pinned contract (and the transport layer)
/// produce: accepted, unknown/finished run, auth failure, and engine
/// unreachable (a connection failure or a missing `engine_api_key`, both
/// surfaced as `Err` by [`ApiClient::abort_run`] since neither produced a
/// pinned response to classify). Every failure branch carries its `C0xx`
/// code via `ConsoleError`'s `Display` (which embeds it) — the accepted
/// branch carries none by construction, since a code is by definition an
/// error/degradation signal (`observ/errors.rs`).
///
/// The unreachable-engine message points at `bastion serve` (which mounts
/// the engine's route table per task 2), never at the orchestrator stack.
///
/// Pure function — no I/O — so every outcome is unit-testable without a live
/// server (Rule 6).
pub fn render_outcome(run_id: &str, result: &Result<AbortOutcome, ConsoleError>) -> String {
    match result {
        Ok(AbortOutcome::Accepted {
            run_id: accepted_run_id,
            status,
        }) => {
            format!("abort accepted: run {accepted_run_id} is now '{status}'\n")
        }
        Ok(AbortOutcome::NotFound(err)) => {
            format!("abort failed: run '{run_id}' not found or already finished\n{err}\n")
        }
        Ok(AbortOutcome::Unauthorized(err)) => {
            format!("abort failed: engine rejected the request (bad or missing X-API-Key)\n{err}\n")
        }
        Err(err) => {
            format!(
                "abort failed: could not reach the engine for run '{run_id}'\n{err}\n\
                 Is `bastion serve` running? The abort endpoint is served by engine-serve, \
                 which `bastion serve` mounts — not the orchestrator stack.\n"
            )
        }
    }
}

/// Return the `C0xx` code string for an abort outcome, or `None` for a
/// successful (`Accepted`) outcome — mirrors [`render_outcome`]'s branches.
///
/// Pure function — no I/O.
fn error_code_for(result: &Result<AbortOutcome, ConsoleError>) -> Option<String> {
    match result {
        Ok(AbortOutcome::Accepted { .. }) => None,
        Ok(AbortOutcome::NotFound(err)) => Some(err.code().to_string()),
        Ok(AbortOutcome::Unauthorized(err)) => Some(err.code().to_string()),
        Err(err) => Some(err.code().to_string()),
    }
}

// ── Thin I/O shell ───────────────────────────────────────────────────────────

/// Print [`confirm_prompt`] and read a line of operator input from stdin.
/// Thin I/O shell over [`confirm_prompt`] / [`parse_confirmation`].
fn prompt_confirmation(run_id: &str) -> io::Result<bool> {
    print!("{}", confirm_prompt(run_id));
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(parse_confirmation(&input))
}

/// Run `bastion abort <run>`.
///
/// Prompts for operator confirmation before sending unless `yes` is set
/// (`--yes`, for scripted use — "silent auto-kill" itself is out of scope;
/// `--yes` only skips the interactive prompt, the operator/script still
/// chose to invoke the command). Emits a structured `observ` event for the
/// attempt (`emit_start`) and for its outcome (`emit_outcome`, carrying the
/// outcome's `C0xx` code when it degrades).
pub async fn run(run_id: String, yes: bool) -> Result<()> {
    let t0 = Instant::now();
    observ::emit_start("abort");

    if !yes {
        let confirmed = match prompt_confirmation(&run_id) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("bastion abort: could not read confirmation: {e}");
                observ::emit_outcome("abort", t0.elapsed().as_millis() as u64, None);
                return Ok(());
            }
        };
        if !confirmed {
            println!("abort cancelled (not confirmed)");
            observ::emit_outcome("abort", t0.elapsed().as_millis() as u64, None);
            return Ok(());
        }
    }

    let config = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("bastion abort: {e}");
            observ::emit_outcome("abort", t0.elapsed().as_millis() as u64, None);
            return Ok(());
        }
    };

    let client =
        ApiClient::new(&config.api_base_url).with_engine_api_key(config.engine_api_key.clone());
    let result = client.abort_run(&run_id).await;

    let message = render_outcome(&run_id, &result);
    match &result {
        Ok(AbortOutcome::Accepted { .. }) => print!("{message}"),
        _ => eprint!("{message}"),
    }

    let code = error_code_for(&result);
    observ::emit_outcome("abort", t0.elapsed().as_millis() as u64, code.as_deref());

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── confirm_prompt ───────────────────────────────────────────────────────

    #[test]
    fn confirm_prompt_contains_run_id() {
        let p = confirm_prompt("run-abc");
        assert!(p.contains("run-abc"), "got: {p}");
    }

    #[test]
    fn confirm_prompt_warns_irreversible() {
        let p = confirm_prompt("run-abc");
        assert!(p.contains("cannot be undone"), "got: {p}");
    }

    // ── parse_confirmation ───────────────────────────────────────────────────

    #[test]
    fn parse_confirmation_lowercase_y_is_true() {
        assert!(parse_confirmation("y"));
    }

    #[test]
    fn parse_confirmation_lowercase_yes_is_true() {
        assert!(parse_confirmation("yes"));
    }

    #[test]
    fn parse_confirmation_uppercase_y_is_true() {
        assert!(parse_confirmation("Y"));
    }

    #[test]
    fn parse_confirmation_uppercase_yes_is_true() {
        assert!(parse_confirmation("YES"));
    }

    #[test]
    fn parse_confirmation_trims_whitespace_and_newline() {
        assert!(parse_confirmation("  yes\n"));
        assert!(parse_confirmation("y\r\n"));
    }

    #[test]
    fn parse_confirmation_empty_is_false() {
        assert!(!parse_confirmation(""));
        assert!(!parse_confirmation("\n"));
    }

    #[test]
    fn parse_confirmation_no_is_false() {
        assert!(!parse_confirmation("n"));
        assert!(!parse_confirmation("no"));
    }

    #[test]
    fn parse_confirmation_garbage_is_false() {
        assert!(!parse_confirmation("sure"));
        assert!(!parse_confirmation("yeah"));
    }

    // ── render_outcome — Accepted ────────────────────────────────────────────

    #[test]
    fn render_outcome_accepted_message() {
        let outcome = Ok(AbortOutcome::Accepted {
            run_id: "run-1".to_string(),
            status: "aborting".to_string(),
        });
        let msg = render_outcome("run-1", &outcome);
        assert!(msg.contains("accepted"), "got: {msg}");
        assert!(msg.contains("run-1"), "got: {msg}");
        assert!(msg.contains("aborting"), "got: {msg}");
    }

    #[test]
    fn render_outcome_accepted_carries_no_c0xx_code() {
        let outcome = Ok(AbortOutcome::Accepted {
            run_id: "run-1".to_string(),
            status: "aborting".to_string(),
        });
        let msg = render_outcome("run-1", &outcome);
        assert!(
            !msg.contains("C0"),
            "accepted message should carry no error code: {msg}"
        );
        assert!(error_code_for(&outcome).is_none());
    }

    // ── render_outcome — NotFound ────────────────────────────────────────────

    #[test]
    fn render_outcome_not_found_message() {
        let outcome: Result<AbortOutcome, ConsoleError> = Ok(AbortOutcome::NotFound(
            ConsoleError::SessionNotFound("run not found or already finished".to_string()),
        ));
        let msg = render_outcome("run-404", &outcome);
        assert!(msg.contains("run-404"), "got: {msg}");
        assert!(msg.contains("not found"), "got: {msg}");
        assert!(msg.contains("C002"), "got: {msg}");
    }

    #[test]
    fn render_outcome_not_found_error_code() {
        let outcome: Result<AbortOutcome, ConsoleError> = Ok(AbortOutcome::NotFound(
            ConsoleError::SessionNotFound("run not found or already finished".to_string()),
        ));
        assert_eq!(error_code_for(&outcome).as_deref(), Some("C002"));
    }

    // ── render_outcome — Unauthorized ────────────────────────────────────────

    #[test]
    fn render_outcome_unauthorized_message() {
        let outcome: Result<AbortOutcome, ConsoleError> =
            Ok(AbortOutcome::Unauthorized(ConsoleError::NotAuthenticated));
        let msg = render_outcome("run-401", &outcome);
        assert!(msg.contains("rejected"), "got: {msg}");
        assert!(msg.contains("X-API-Key"), "got: {msg}");
        assert!(msg.contains("C012"), "got: {msg}");
    }

    #[test]
    fn render_outcome_unauthorized_error_code() {
        let outcome: Result<AbortOutcome, ConsoleError> =
            Ok(AbortOutcome::Unauthorized(ConsoleError::NotAuthenticated));
        assert_eq!(error_code_for(&outcome).as_deref(), Some("C012"));
    }

    // ── render_outcome — Err (engine unreachable) ────────────────────────────

    #[test]
    fn render_outcome_unreachable_points_at_bastion_serve() {
        let outcome: Result<AbortOutcome, ConsoleError> = Err(ConsoleError::Io(
            "connecting to engine abort endpoint at http://localhost:8080/events/run-9/abort: \
             connection refused"
                .to_string(),
        ));
        let msg = render_outcome("run-9", &outcome);
        assert!(msg.contains("bastion serve"), "got: {msg}");
        assert!(msg.contains("C009"), "got: {msg}");
    }

    #[test]
    fn render_outcome_unreachable_error_code() {
        let outcome: Result<AbortOutcome, ConsoleError> =
            Err(ConsoleError::Io("connection refused".to_string()));
        assert_eq!(error_code_for(&outcome).as_deref(), Some("C009"));
    }

    #[test]
    fn render_outcome_missing_engine_key_points_at_bastion_serve() {
        let outcome: Result<AbortOutcome, ConsoleError> = Err(ConsoleError::ConfigError(
            "engine_api_key not configured".to_string(),
        ));
        let msg = render_outcome("run-9", &outcome);
        assert!(msg.contains("bastion serve"), "got: {msg}");
        assert!(msg.contains("C005"), "got: {msg}");
        let code = error_code_for(&outcome);
        assert_eq!(code.as_deref(), Some("C005"));
    }

    // ── render_outcome distinctness — every outcome renders a different message ──

    #[test]
    fn render_outcome_all_four_outcomes_are_distinct_messages() {
        let accepted: Result<AbortOutcome, ConsoleError> = Ok(AbortOutcome::Accepted {
            run_id: "run-x".to_string(),
            status: "aborting".to_string(),
        });
        let not_found: Result<AbortOutcome, ConsoleError> = Ok(AbortOutcome::NotFound(
            ConsoleError::SessionNotFound("run not found or already finished".to_string()),
        ));
        let unauthorized: Result<AbortOutcome, ConsoleError> =
            Ok(AbortOutcome::Unauthorized(ConsoleError::NotAuthenticated));
        let unreachable: Result<AbortOutcome, ConsoleError> =
            Err(ConsoleError::Io("connection refused".to_string()));

        let messages: Vec<String> = vec![
            render_outcome("run-x", &accepted),
            render_outcome("run-x", &not_found),
            render_outcome("run-x", &unauthorized),
            render_outcome("run-x", &unreachable),
        ];
        for (i, a) in messages.iter().enumerate() {
            for (j, b) in messages.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "outcome messages must be distinct: {a:?} vs {b:?}");
                }
            }
        }
    }
}
