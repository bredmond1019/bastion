//! Quick-action command handler for `bastion serve` (BA.11.E).
//!
//! `POST /actions/command` — one-tap remote slash-command trigger, reusing
//! `ask`'s spawn/readiness mechanics (`crate::sessions::ask`).
//!
//! # Routes
//! - `POST /actions/command` — `mode:"inject"` sends `command` into an
//!   existing tmux session; `mode:"spawn"` creates a session, launches
//!   `claude --model <opus|sonnet> --permission-mode bypassPermissions`,
//!   waits for readiness, then sends `command`. Both return the target
//!   session id.
//!
//! # Error mapping
//! - Request validation failure (bad mode/field combination) → 400 + `C006`.
//! - Tmux failures (unknown session, no server, not installed) → delegated
//!   to [`crate::serve::handlers::sessions::tmux_error_to_status`].
//! - Readiness timeout waiting for Claude to launch → 504 + `C007`.
//! - Thread-pool failure (`web::block` panic) → 500 + `C010`.
//!
//! # Pure/I/O split (Rule 6)
//! Request validation ([`CommandRequest::validate`], task 2) and dispatch
//! planning ([`plan_command`], this module) are pure — no I/O, directly
//! unit-tested. The only I/O is the tmux/`ask` execution inside `web::block`.

use actix_web::{HttpResponse, web};

use crate::serve::dto::{
    CommandMode, CommandRequest, CommandResponse, CommandValidationError, ErrorPayload,
};
use crate::serve::handlers::sessions::tmux_error_to_status;
use crate::sessions::ask::{AskError, ensure_session_with_claude};
use crate::sessions::tmux::send_keys;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Model used for `mode:"spawn"` when the request omits `model`.
pub const DEFAULT_COMMAND_MODEL: &str = "sonnet";

// ── Pure dispatch plan ───────────────────────────────────────────────────────

/// A validated [`CommandRequest`] resolved into a concrete dispatch plan.
///
/// Built by [`plan_command`] — pure, no I/O. The handler executes the plan
/// against tmux inside `web::block`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandPlan {
    /// Send `command` into the existing session `session`.
    Inject { session: String, command: String },
    /// Ensure a session named `name` exists with Claude launched via
    /// `launch_cmd`, then send `command`.
    Spawn {
        name: String,
        dir: Option<String>,
        launch_cmd: String,
        command: String,
    },
}

/// Build the exact `claude` launch command string for a spawned session.
///
/// Contract (BA.11.E): `claude --model <model> --permission-mode bypassPermissions`.
pub fn build_launch_cmd(model: &str) -> String {
    format!("claude --model {model} --permission-mode bypassPermissions")
}

/// Resolve the effective spawn model: the request's `model` field, or
/// [`DEFAULT_COMMAND_MODEL`] when omitted.
///
/// Assumes `model`, if present, has already passed [`CommandRequest::validate`]
/// (i.e. it is one of `ALLOWED_COMMAND_MODELS`).
pub fn resolve_model(model: Option<&str>) -> &str {
    model.unwrap_or(DEFAULT_COMMAND_MODEL)
}

/// Turn a **validated** [`CommandRequest`] into a [`CommandPlan`].
///
/// Pure — performs no I/O. Callers must run [`CommandRequest::validate`]
/// first; this function trusts that `session` is present for `inject` and
/// `name` is present for `spawn` (empty-string fallback mirrors `validate`'s
/// own "empty counts as missing" treatment so a malformed plan never panics).
pub fn plan_command(req: &CommandRequest) -> CommandPlan {
    match req.mode {
        CommandMode::Inject => CommandPlan::Inject {
            session: req.session.clone().unwrap_or_default(),
            command: req.command.clone(),
        },
        CommandMode::Spawn => {
            let model = resolve_model(req.model.as_deref());
            CommandPlan::Spawn {
                name: req.name.clone().unwrap_or_default(),
                dir: req.dir.clone(),
                launch_cmd: build_launch_cmd(model),
                command: req.command.clone(),
            }
        }
    }
}

// ── Pure error-mapping helper ────────────────────────────────────────────────

/// Map a [`CommandValidationError`] to a `400 Bad Request` + `C006` payload.
///
/// Pure — no I/O.
pub fn validation_error_response(err: &CommandValidationError) -> HttpResponse {
    HttpResponse::BadRequest().json(ErrorPayload {
        code: "C006".to_owned(),
        message: err.to_string(),
    })
}

/// Map an [`AskError`] from the spawn/readiness path to an HTTP response.
///
/// Pure — no I/O (only inspects the error).
///
/// | Source                     | Status | Code |
/// |-----------------------------|--------|------|
/// | [`AskError::Tmux`]          | delegated to [`tmux_error_to_status`] |
/// | [`AskError::Launch`]        | 504    | C007 |
/// | [`AskError::Timeout`]       | 504    | C007 |
/// | [`AskError::UntrustedDir`]  | 400    | C006 |
pub fn ask_error_to_status(err: &AskError) -> HttpResponse {
    match err {
        AskError::Tmux { source, .. } => {
            let (status, payload) = tmux_error_to_status(source);
            HttpResponse::build(status).json(payload)
        }
        AskError::Launch { .. } | AskError::Timeout { .. } => {
            HttpResponse::GatewayTimeout().json(ErrorPayload {
                code: "C007".to_owned(),
                message: err.to_string(),
            })
        }
        AskError::UntrustedDir(_) => HttpResponse::BadRequest().json(ErrorPayload {
            code: "C006".to_owned(),
            message: err.to_string(),
        }),
    }
}

// ── Handler helpers ───────────────────────────────────────────────────────────

/// Build a 500 response from a `BlockingError` (thread panic / runtime shutdown).
fn blocking_error_response(err: actix_web::error::BlockingError) -> HttpResponse {
    HttpResponse::InternalServerError().json(ErrorPayload {
        code: "C010".to_owned(),
        message: format!("blocking thread error: {err}"),
    })
}

// ── Handler ───────────────────────────────────────────────────────────────────

/// `POST /actions/command` — quick-action inject/spawn dispatch.
///
/// Validates the request, plans the dispatch (both pure), then executes the
/// tmux/`ask` I/O inside `web::block`. Returns 200 + [`CommandResponse`] on
/// success; 400 on a validation failure; 404/503/504/500 on execution
/// failures (see module docs for the mapping).
pub async fn command(body: web::Json<CommandRequest>) -> HttpResponse {
    let req = body.into_inner();

    if let Err(err) = req.validate() {
        return validation_error_response(&err);
    }

    let plan = plan_command(&req);

    match plan {
        CommandPlan::Inject { session, command } => {
            let result = web::block({
                let session = session.clone();
                move || send_keys(&session, &command)
            })
            .await;

            match result {
                Ok(Ok(())) => HttpResponse::Ok().json(CommandResponse { session }),
                Ok(Err(err)) => {
                    let (status, payload) = tmux_error_to_status(&err);
                    HttpResponse::build(status).json(payload)
                }
                Err(err) => blocking_error_response(err),
            }
        }
        CommandPlan::Spawn {
            name,
            dir,
            launch_cmd,
            command,
        } => {
            let result = web::block({
                let name = name.clone();
                move || -> Result<(), AskError> {
                    ensure_session_with_claude(&name, dir.as_deref(), &launch_cmd)?;
                    send_keys(&name, &command).map_err(|e| AskError::Tmux {
                        op: "send-keys (quick-action command)".to_string(),
                        source: e,
                    })
                }
            })
            .await;

            match result {
                Ok(Ok(())) => HttpResponse::Ok().json(CommandResponse { session: name }),
                Ok(Err(err)) => ask_error_to_status(&err),
                Err(err) => blocking_error_response(err),
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::tmux::TmuxError;

    // ── build_launch_cmd ─────────────────────────────────────────────────────

    #[test]
    fn build_launch_cmd_opus() {
        assert_eq!(
            build_launch_cmd("opus"),
            "claude --model opus --permission-mode bypassPermissions"
        );
    }

    #[test]
    fn build_launch_cmd_sonnet() {
        assert_eq!(
            build_launch_cmd("sonnet"),
            "claude --model sonnet --permission-mode bypassPermissions"
        );
    }

    // ── resolve_model ────────────────────────────────────────────────────────

    #[test]
    fn resolve_model_uses_default_when_absent() {
        assert_eq!(resolve_model(None), DEFAULT_COMMAND_MODEL);
    }

    #[test]
    fn resolve_model_uses_provided_value() {
        assert_eq!(resolve_model(Some("opus")), "opus");
    }

    // ── plan_command — inject ────────────────────────────────────────────────

    fn inject_req(session: &str, command: &str) -> CommandRequest {
        CommandRequest {
            mode: CommandMode::Inject,
            session: Some(session.to_owned()),
            name: None,
            dir: None,
            model: None,
            command: command.to_owned(),
        }
    }

    fn spawn_req(
        name: &str,
        dir: Option<&str>,
        model: Option<&str>,
        command: &str,
    ) -> CommandRequest {
        CommandRequest {
            mode: CommandMode::Spawn,
            session: None,
            name: Some(name.to_owned()),
            dir: dir.map(str::to_owned),
            model: model.map(str::to_owned),
            command: command.to_owned(),
        }
    }

    #[test]
    fn plan_command_inject_targets_named_session() {
        let plan = plan_command(&inject_req("main", "/status"));
        assert_eq!(
            plan,
            CommandPlan::Inject {
                session: "main".to_owned(),
                command: "/status".to_owned(),
            }
        );
    }

    // ── plan_command — spawn ─────────────────────────────────────────────────

    #[test]
    fn plan_command_spawn_builds_exact_launch_cmd_with_model() {
        let plan = plan_command(&spawn_req("work", Some("/repo"), Some("opus"), "/status"));
        assert_eq!(
            plan,
            CommandPlan::Spawn {
                name: "work".to_owned(),
                dir: Some("/repo".to_owned()),
                launch_cmd: "claude --model opus --permission-mode bypassPermissions".to_owned(),
                command: "/status".to_owned(),
            }
        );
    }

    #[test]
    fn plan_command_spawn_defaults_model_when_absent() {
        let plan = plan_command(&spawn_req("work", None, None, "/status"));
        assert_eq!(
            plan,
            CommandPlan::Spawn {
                name: "work".to_owned(),
                dir: None,
                launch_cmd: "claude --model sonnet --permission-mode bypassPermissions".to_owned(),
                command: "/status".to_owned(),
            }
        );
    }

    #[test]
    fn plan_command_spawn_sonnet_model_element_level() {
        let plan = plan_command(&spawn_req("work", None, Some("sonnet"), "/ping"));
        match plan {
            CommandPlan::Spawn {
                name,
                dir,
                launch_cmd,
                command,
            } => {
                assert_eq!(name, "work");
                assert_eq!(dir, None);
                assert_eq!(
                    launch_cmd,
                    "claude --model sonnet --permission-mode bypassPermissions"
                );
                assert_eq!(command, "/ping");
            }
            other => panic!("expected Spawn plan, got {other:?}"),
        }
    }

    // ── validation_error_response ────────────────────────────────────────────

    #[test]
    fn validation_error_response_is_400_c006() {
        let resp = validation_error_response(&CommandValidationError::InjectMissingSession);
        assert_eq!(resp.status(), 400);
    }

    // ── ask_error_to_status ──────────────────────────────────────────────────

    #[test]
    fn ask_error_tmux_delegates_to_tmux_error_to_status() {
        let err = AskError::Tmux {
            op: "new-session".to_string(),
            source: anyhow::Error::new(TmuxError::NoServer),
        };
        let resp = ask_error_to_status(&err);
        assert_eq!(resp.status(), 503);
    }

    #[test]
    fn ask_error_tmux_unknown_session_maps_to_404() {
        let err = AskError::Tmux {
            op: "send-keys".to_string(),
            source: anyhow::Error::new(TmuxError::ExitError {
                code: 1,
                stderr: "can't find session: work".to_string(),
            }),
        };
        let resp = ask_error_to_status(&err);
        assert_eq!(resp.status(), 404);
    }

    #[test]
    fn ask_error_launch_maps_to_504_gateway_timeout() {
        let err = AskError::Launch {
            session: "work".to_string(),
            timeout_secs: 30,
        };
        let resp = ask_error_to_status(&err);
        assert_eq!(resp.status(), 504);
    }

    #[test]
    fn ask_error_timeout_maps_to_504_gateway_timeout() {
        let err = AskError::Timeout {
            timeout_secs: 30,
            out: "/tmp/out.json".to_string(),
            pane_output: "(pane)".to_string(),
        };
        let resp = ask_error_to_status(&err);
        assert_eq!(resp.status(), 504);
    }

    #[test]
    fn ask_error_untrusted_dir_maps_to_400() {
        let err = AskError::UntrustedDir("/repo".to_string());
        let resp = ask_error_to_status(&err);
        assert_eq!(resp.status(), 400);
    }
}
