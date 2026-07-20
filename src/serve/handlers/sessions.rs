//! Session REST handlers for `bastion serve`.
//!
//! All handlers run synchronous tmux calls inside `web::block` to avoid
//! blocking the actix runtime thread.  Routing is done in
//! [`crate::serve::mod`] and all routes inherit the `BearerAuthMiddleware`
//! from the `/api` scope.
//!
//! # Routes
//! - `GET  /api/sessions`                 — list all sessions
//! - `GET  /api/sessions/{name}/pane`     — capture pane (`?lines=N` optional)
//! - `POST /api/sessions/{name}/send`     — send literal keystrokes + Enter
//! - `POST /api/sessions/{name}/key`      — send a named tmux key (Escape, arrows, C-c, …)
//! - `POST /api/sessions`                 — create a new session
//! - `DELETE /api/sessions/{name}`        — kill a session
//!
//! # Error mapping
//! Tmux failures are classified by [`tmux_error_to_status`] (a pure helper):
//! - [`TmuxError::NotInstalled`] / [`TmuxError::NoServer`] → 503 + `C001`
//! - [`TmuxError::ExitError`] with an "unknown session" stderr → 404 + `C002`
//! - Other [`TmuxError::ExitError`] → 500 + `C010`
//! - Non-tmux errors (e.g. thread panic) → 500 + `C010`

use actix_web::http::StatusCode;
use actix_web::{HttpResponse, web};
use serde::Deserialize;

use crate::serve::dto::{ErrorPayload, KeyBody, NewSessionBody, PaneDto, SendBody, SessionDto};
use crate::sessions::model::{Pane, parse_sessions};
use crate::sessions::tmux::{
    TmuxError, capture_pane_raw, kill_session, list_sessions_raw, new_session, send_keys,
    send_named_key,
};

// ── Query params ──────────────────────────────────────────────────────────────

/// Query parameters for `GET /api/sessions/{name}/pane`.
#[derive(Debug, Deserialize)]
pub struct PaneQuery {
    /// Maximum number of trailing lines to return.  `None` → all non-blank lines.
    pub lines: Option<usize>,
}

// ── Pure error-mapping helper ─────────────────────────────────────────────────

/// Map a tmux [`anyhow::Error`] to an HTTP [`StatusCode`] and [`ErrorPayload`].
///
/// This is a **pure** function (no I/O) — it only inspects the error chain.
///
/// | Source                                   | Status | Code  |
/// |------------------------------------------|--------|-------|
/// | [`TmuxError::NotInstalled`]               | 503    | C001  |
/// | [`TmuxError::NoServer`]                   | 503    | C001  |
/// | [`TmuxError::ExitError`] — unknown session| 404    | C002  |
/// | [`TmuxError::ExitError`] — other          | 500    | C010  |
/// | Any other error                           | 500    | C010  |
pub fn tmux_error_to_status(err: &anyhow::Error) -> (StatusCode, ErrorPayload) {
    if let Some(tmux_err) = err.downcast_ref::<TmuxError>() {
        match tmux_err {
            TmuxError::NotInstalled => (
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorPayload {
                    code: "C001".to_owned(),
                    message: "tmux is not installed".to_owned(),
                },
            ),
            TmuxError::NoServer => (
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorPayload {
                    code: "C001".to_owned(),
                    message: "no tmux server running".to_owned(),
                },
            ),
            TmuxError::ExitError { stderr, .. } if is_unknown_session(stderr) => (
                StatusCode::NOT_FOUND,
                ErrorPayload {
                    code: "C002".to_owned(),
                    message: format!("session not found: {stderr}"),
                },
            ),
            TmuxError::ExitError { code, stderr } => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorPayload {
                    code: "C010".to_owned(),
                    message: format!("tmux error (exit {code}): {stderr}"),
                },
            ),
        }
    } else {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorPayload {
                code: "C010".to_owned(),
                message: err.to_string(),
            },
        )
    }
}

/// True when a tmux `ExitError` stderr indicates an unknown/missing session target.
///
/// Matches the message patterns tmux emits when the `-t <name>` target does not
/// resolve to a live session:
/// - `"can't find session: <name>"` — `kill-session`, `has-session`
/// - `"can't find pane: <name>"` — `send-keys`, `capture-pane` resolve the
///   target as a pane, so an unknown *session* surfaces as a missing pane
///   (observed on tmux 3.6b). Without this, an inject/pane read against an
///   unknown session would fall through to the generic 500/C010 branch instead
///   of the documented 404/C002 (serve-api §12.3 / §10).
/// - `"session not found: <name>"` — some tmux builds
fn is_unknown_session(stderr: &str) -> bool {
    stderr.contains("can't find session")
        || stderr.contains("can't find pane")
        || stderr.contains("session not found")
}

// ── Handler helpers ───────────────────────────────────────────────────────────

/// Build a 500 response from a `BlockingError` (thread panic / runtime shutdown).
fn blocking_error_response(err: actix_web::error::BlockingError) -> HttpResponse {
    HttpResponse::InternalServerError().json(ErrorPayload {
        code: "C010".to_owned(),
        message: format!("blocking thread error: {err}"),
    })
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// `GET /api/sessions` — list all tmux sessions as JSON.
///
/// Returns 200 with a JSON array of [`SessionDto`].  Returns 503 when tmux is
/// unavailable, 500 on other failures.
pub async fn list_sessions() -> HttpResponse {
    match web::block(list_sessions_raw).await {
        Ok(Ok(raw)) => {
            let dtos: Vec<SessionDto> = parse_sessions(&raw).iter().map(SessionDto::from).collect();
            HttpResponse::Ok().json(dtos)
        }
        Ok(Err(err)) => {
            let (status, payload) = tmux_error_to_status(&err);
            HttpResponse::build(status).json(payload)
        }
        Err(err) => blocking_error_response(err),
    }
}

/// `GET /api/sessions/{name}/pane?lines=N` — capture the last N lines of a session pane.
///
/// Returns 200 with a [`PaneDto`].  Returns 404 when the session does not exist,
/// 503 when tmux is unavailable, 500 on other failures.
pub async fn get_pane(name: web::Path<String>, query: web::Query<PaneQuery>) -> HttpResponse {
    let session_name = name.into_inner();
    let n = query.into_inner().lines;

    let result = web::block({
        let sname = session_name.clone();
        move || capture_pane_raw(&sname)
    })
    .await;

    match result {
        Ok(Ok(raw)) => {
            let pane = Pane::new(session_name, raw);
            HttpResponse::Ok().json(PaneDto::from_pane(&pane, n))
        }
        Ok(Err(err)) => {
            let (status, payload) = tmux_error_to_status(&err);
            HttpResponse::build(status).json(payload)
        }
        Err(err) => blocking_error_response(err),
    }
}

/// `POST /api/sessions/{name}/send` — send literal keystrokes + Enter to a session.
///
/// Returns 204 No Content on success.  Returns 404 when the session does not
/// exist, 503 when tmux is unavailable, 500 on other failures.
pub async fn send(name: web::Path<String>, body: web::Json<SendBody>) -> HttpResponse {
    let session_name = name.into_inner();
    let keys = body.into_inner().keys;

    match web::block(move || send_keys(&session_name, &keys)).await {
        Ok(Ok(())) => HttpResponse::NoContent().finish(),
        Ok(Err(err)) => {
            let (status, payload) = tmux_error_to_status(&err);
            HttpResponse::build(status).json(payload)
        }
        Err(err) => blocking_error_response(err),
    }
}

/// `POST /api/sessions/{name}/key` — send a named tmux key to a session.
///
/// `KeyBody.key` is a symbolic tmux key name (`Escape`, `Enter`, `Up`, `Down`,
/// `Left`, `Right`, `C-c`, etc.) — sent **without** `-l` so tmux resolves it.
///
/// Returns 204 No Content on success.  Returns 404 when the session does not
/// exist, 503 when tmux is unavailable, 500 on other failures.
pub async fn send_key(name: web::Path<String>, body: web::Json<KeyBody>) -> HttpResponse {
    let session_name = name.into_inner();
    let key = body.into_inner().key;

    match web::block(move || send_named_key(&session_name, &key)).await {
        Ok(Ok(())) => HttpResponse::NoContent().finish(),
        Ok(Err(err)) => {
            let (status, payload) = tmux_error_to_status(&err);
            HttpResponse::build(status).json(payload)
        }
        Err(err) => blocking_error_response(err),
    }
}

/// `POST /api/sessions` — create a new detached tmux session.
///
/// Returns 201 Created on success.  Returns 503 when tmux is unavailable,
/// 500 on other failures.
pub async fn create_session(body: web::Json<NewSessionBody>) -> HttpResponse {
    let b = body.into_inner();
    let name = b.name;
    let dir = b.dir;

    match web::block(move || new_session(&name, dir.as_deref())).await {
        Ok(Ok(())) => HttpResponse::Created().finish(),
        Ok(Err(err)) => {
            let (status, payload) = tmux_error_to_status(&err);
            HttpResponse::build(status).json(payload)
        }
        Err(err) => blocking_error_response(err),
    }
}

/// `DELETE /api/sessions/{name}` — kill a tmux session.
///
/// Returns 204 No Content on success.  Returns 404 when the session does not
/// exist, 503 when tmux is unavailable, 500 on other failures.
pub async fn delete_session(name: web::Path<String>) -> HttpResponse {
    let session_name = name.into_inner();

    match web::block(move || kill_session(&session_name)).await {
        Ok(Ok(())) => HttpResponse::NoContent().finish(),
        Ok(Err(err)) => {
            let (status, payload) = tmux_error_to_status(&err);
            HttpResponse::build(status).json(payload)
        }
        Err(err) => blocking_error_response(err),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::tmux::TmuxError;

    fn make_tmux_err(e: TmuxError) -> anyhow::Error {
        anyhow::Error::new(e)
    }

    // ── tmux_error_to_status ──────────────────────────────────────────────────

    #[test]
    fn not_installed_maps_to_503_c001() {
        let err = make_tmux_err(TmuxError::NotInstalled);
        let (status, payload) = tmux_error_to_status(&err);
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(payload.code, "C001");
        assert!(!payload.message.is_empty());
    }

    #[test]
    fn no_server_maps_to_503_c001() {
        let err = make_tmux_err(TmuxError::NoServer);
        let (status, payload) = tmux_error_to_status(&err);
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(payload.code, "C001");
        assert!(payload.message.contains("server"));
    }

    #[test]
    fn exit_error_cant_find_session_maps_to_404_c002() {
        let err = make_tmux_err(TmuxError::ExitError {
            code: 1,
            stderr: "can't find session: work".to_owned(),
        });
        let (status, payload) = tmux_error_to_status(&err);
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(payload.code, "C002");
        assert!(payload.message.contains("work"));
    }

    #[test]
    fn exit_error_session_not_found_maps_to_404_c002() {
        let err = make_tmux_err(TmuxError::ExitError {
            code: 1,
            stderr: "session not found: mysession".to_owned(),
        });
        let (status, payload) = tmux_error_to_status(&err);
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(payload.code, "C002");
    }

    #[test]
    fn exit_error_cant_find_pane_maps_to_404_c002() {
        // tmux's send-keys/capture-pane report an unknown session target as a
        // missing pane (observed on tmux 3.6b) — still a 404/C002, not 500.
        let err = make_tmux_err(TmuxError::ExitError {
            code: 1,
            stderr: "can't find pane: no-such-session".to_owned(),
        });
        let (status, payload) = tmux_error_to_status(&err);
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(payload.code, "C002");
    }

    #[test]
    fn exit_error_other_maps_to_500_c010() {
        let err = make_tmux_err(TmuxError::ExitError {
            code: 2,
            stderr: "unexpected tmux error".to_owned(),
        });
        let (status, payload) = tmux_error_to_status(&err);
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(payload.code, "C010");
    }

    #[test]
    fn exit_error_empty_stderr_maps_to_500_c010() {
        let err = make_tmux_err(TmuxError::ExitError {
            code: 1,
            stderr: String::new(),
        });
        let (status, payload) = tmux_error_to_status(&err);
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(payload.code, "C010");
    }

    #[test]
    fn non_tmux_error_maps_to_500_c010() {
        let err = anyhow::anyhow!("some completely unrelated error");
        let (status, payload) = tmux_error_to_status(&err);
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(payload.code, "C010");
        assert!(!payload.message.is_empty());
    }

    // ── is_unknown_session ────────────────────────────────────────────────────

    #[test]
    fn is_unknown_session_matches_cant_find() {
        assert!(is_unknown_session("can't find session: work"));
        assert!(is_unknown_session("can't find session: my-special-name"));
    }

    #[test]
    fn is_unknown_session_matches_session_not_found() {
        assert!(is_unknown_session("session not found: xyz"));
    }

    #[test]
    fn is_unknown_session_rejects_unrelated_stderr() {
        assert!(!is_unknown_session("duplicate session: work"));
        assert!(!is_unknown_session("some other tmux error"));
        assert!(!is_unknown_session(""));
    }

    #[test]
    fn is_unknown_session_rejects_no_server_message() {
        // no-server errors are 503, not 404 — must not match unknown-session
        assert!(!is_unknown_session(
            "no server running on /tmp/tmux-501/default"
        ));
        assert!(!is_unknown_session(
            "error connecting to /tmp/tmux-501/default"
        ));
    }
}
