//! The pure core of `bastion notify send|ask` (`BA.ticket.notify-operator-cli`
//! task 3).
//!
//! This module holds only pure logic and its unit tests — no HTTP, no clap
//! wiring, no lock. Per `CLAUDE.md` standing rule 6, this is where
//! essentially all of this block's coverage lives; the thin I/O shell that
//! calls into these functions is wired up by a later task.
//!
//! **Reuse discipline (non-negotiable for this file):** everything here
//! delegates to the already-tested protocol code in
//! `crate::serve::notify::telegram` and `engine_core::operator`. This file
//! contains no `callback_data` encoder, no `getUpdates` offset arithmetic,
//! and no digest comparison of its own — [`decide_batch`] calls the
//! existing [`resolve_response`](crate::serve::notify::telegram::resolve_response)
//! rather than re-deriving its verdict.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use engine_core::operator::{
    NotifyError, OperatorPayload, OperatorPayloadLimits, OperatorResponse, OperatorResponseOption,
    OperatorTransport, ResponseVerdict, UpdateCursor, ValidatedOperatorPayload, validate,
};
use fs4::{FileExt, TryLockError};

use crate::config::{BotCredentials, ConfigError, bot_env_var_names, configured_bot_slugs};
use crate::serve::notify::telegram::{
    TelegramTransport, check_whatsapp_portability, resolve_response,
};
// `telegram_http`'s reqwest-error/retry-after helpers are `pub(crate)` —
// reachable from here, not re-derived.
use crate::serve::notify::telegram_http::{classify_reqwest_error, retry_after_from_response};

// ── Option parsing (`--option key:Label`) ───────────────────────────────

/// Parse one `--option key:Label` argument into an
/// [`OperatorResponseOption`].
///
/// Splits on the **first** `:` only, so a label may itself contain `:`
/// (e.g. `approve:Approve: ship it`). An empty key or an empty label is
/// rejected with a distinct message so a caller can tell which half was
/// wrong without re-parsing the raw string themselves.
pub fn parse_option(raw: &str) -> Result<OperatorResponseOption, String> {
    let Some((key, label)) = raw.split_once(':') else {
        return Err(format!(
            "--option '{raw}' is missing a ':' separator; expected key:Label"
        ));
    };

    if key.is_empty() {
        return Err(format!("--option '{raw}' has an empty key"));
    }
    if label.is_empty() {
        return Err(format!("--option '{raw}' has an empty label"));
    }

    Ok(OperatorResponseOption::new(key, label))
}

/// Parse every `--option` argument, in order, additionally rejecting a
/// duplicate key — two buttons sharing a key make a tap ambiguous, and
/// `resolve_response` would happily accept either, so this must be caught
/// before any option ever reaches the transport.
pub fn parse_options(raw: &[String]) -> Result<Vec<OperatorResponseOption>, String> {
    let mut options = Vec::with_capacity(raw.len());
    let mut seen_keys: Vec<String> = Vec::with_capacity(raw.len());

    for one in raw {
        let option = parse_option(one)?;
        if seen_keys.iter().any(|k| k == &option.key) {
            return Err(format!("duplicate --option key '{}'", option.key));
        }
        seen_keys.push(option.key.clone());
        options.push(option);
    }

    Ok(options)
}

// ── The ask outcome contract (stdout JSON + exit code) ──────────────────

/// The terminal outcome of `bastion notify ask`. This is the CLI's stdout
/// contract: exactly one of these is printed as one flat JSON object, and
/// every variant maps to a distinct, total exit code via [`Self::exit_code`].
///
/// Exit code 1 is reserved for unconfigured-bot / usage errors and is
/// deliberately not a variant here — those are reported before an
/// `AskOutcome` can even be constructed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskOutcome {
    /// A tap resolved to the expected gate and digest.
    Answered {
        gate_id: String,
        option_key: String,
        decided_at: chrono::DateTime<chrono::Utc>,
    },
    /// No resolving tap arrived before the ask's timeout elapsed.
    Timeout,
    /// A tap resolved to the expected gate but a stale (already
    /// re-rendered) digest — the payload changed after this tap's prompt
    /// was shown, so it must not be treated as an answer.
    StaleDigest {
        gate_id: String,
        option_key: String,
        digest: String,
        decided_at: chrono::DateTime<chrono::Utc>,
    },
    /// A concurrent `notify ask` already holds the per-bot ask lock.
    Busy,
}

impl AskOutcome {
    /// The process exit code for this outcome — a total function over all
    /// four variants: `Answered` 0, `Timeout` 2, `StaleDigest` 3, `Busy` 4.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        match self {
            AskOutcome::Answered { .. } => 0,
            AskOutcome::Timeout => 2,
            AskOutcome::StaleDigest { .. } => 3,
            AskOutcome::Busy => 4,
        }
    }

    /// Render this outcome as the single flat JSON object printed to
    /// stdout. Every field is asserted key-by-key in this module's tests —
    /// this shape is a cross-repo skill's contract, and a rename here must
    /// fail a test in this file rather than silently break a caller
    /// elsewhere.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            AskOutcome::Answered {
                gate_id,
                option_key,
                decided_at,
            } => serde_json::json!({
                "status": "answered",
                "gate_id": gate_id,
                "option_key": option_key,
                "decided_at": decided_at.to_rfc3339(),
            }),
            AskOutcome::Timeout => serde_json::json!({
                "status": "timeout",
            }),
            AskOutcome::StaleDigest {
                gate_id,
                option_key,
                digest,
                decided_at,
            } => serde_json::json!({
                "status": "stale_digest",
                "gate_id": gate_id,
                "option_key": option_key,
                "digest": digest,
                "decided_at": decided_at.to_rfc3339(),
            }),
            AskOutcome::Busy => serde_json::json!({
                "status": "busy",
            }),
        }
    }
}

// ── The per-batch poll decision ─────────────────────────────────────────

/// The result of running one `getUpdates` batch through [`decide_batch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchDecision {
    /// A response in this batch resolved to the expected gate and digest.
    Answered {
        gate_id: String,
        option_key: String,
        decided_at: chrono::DateTime<chrono::Utc>,
    },
    /// A response in this batch resolved to the expected gate but a stale
    /// digest.
    Stale {
        gate_id: String,
        option_key: String,
        digest: String,
        decided_at: chrono::DateTime<chrono::Utc>,
    },
    /// Nothing in this batch resolves the ask — either the batch was
    /// empty, or every response in it answered a different gate. The
    /// caller must keep polling, and (per the cursor rule below) must
    /// still advance its cursor past this batch regardless.
    KeepPolling,
}

/// Decide what one `getUpdates` batch means for an outstanding `ask`.
///
/// Delegates verdict resolution entirely to the existing
/// [`resolve_response`] — this function never compares digests or gate ids
/// itself. Responses are scanned in order; the first response that
/// resolves to [`ResponseVerdict::Accepted`](engine_core::operator::ResponseVerdict::Accepted)
/// wins immediately (an answer always takes precedence over a stale tap
/// seen earlier in the same batch). If nothing in the batch is `Accepted`
/// but at least one response resolves to `StaleDigest`, the first such tap
/// is returned. A response resolving to `UnknownGate` (a different gate's
/// tap) is neither an error nor a match — it is simply skipped while
/// scanning continues.
///
/// **CURSOR RULE** (enforced by the caller, per task 5 — not by this
/// function): the cursor `getUpdates`/`parse_updates` returns must be
/// advanced unconditionally after every batch, including a batch that
/// yields `KeepPolling` because it held only foreign-gate updates. Not
/// advancing past an update this ask chose to ignore replays it forever.
#[must_use]
pub fn decide_batch(
    responses: &[OperatorResponse],
    expected: &ValidatedOperatorPayload,
) -> BatchDecision {
    use engine_core::operator::ResponseVerdict;

    let mut stale: Option<BatchDecision> = None;

    for resp in responses {
        match resolve_response(resp, expected) {
            ResponseVerdict::Accepted {
                gate_id,
                option_key,
                decided_at,
                ..
            } => {
                return BatchDecision::Answered {
                    gate_id,
                    option_key,
                    decided_at,
                };
            }
            ResponseVerdict::StaleDigest {
                gate_id,
                option_key,
                digest,
                decided_at,
            } => {
                if stale.is_none() {
                    stale = Some(BatchDecision::Stale {
                        gate_id,
                        option_key,
                        digest,
                        decided_at,
                    });
                }
            }
            ResponseVerdict::UnknownGate => {}
        }
    }

    stale.unwrap_or(BatchDecision::KeepPolling)
}

// ── The per-bot ask lock ─────────────────────────────────────────────────
//
// `ask` holds an exclusive OS-level advisory lock at
// `<lock_dir>/notify-ask-<slug>.lock` for its whole duration. The hazard
// this guards against is per-TOKEN, not per-process: Telegram gives each
// `getUpdates` call to exactly one consumer per bot token, so two sibling
// `notify ask` invocations against the SAME `--bot` slug steal each other's
// taps at random. Two invocations against DIFFERENT slugs share no token
// and must not contend with each other — hence the lock is keyed by slug,
// not global (operator amendment, 2026-08-23).
//
// `send` never touches any of this — it never reads updates and so cannot
// steal anything (see task 5's I/O shell, which must not acquire this lock
// for `send`).

/// Env var carrying an explicit lock-directory override — second in
/// precedence, after an explicit `--lock-dir` CLI argument and before the
/// `brain.toml` walk-up fallback.
pub const FLEET_LOCK_DIR_ENV: &str = "FLEET_LOCK_DIR";

/// Directory name joined onto a discovered `brain.toml`'s parent (or, when
/// no `brain.toml` is found, onto `cwd`) to form the default lock
/// directory.
const FLEET_LOCK_DIR_NAME: &str = ".fleet-locks";

/// How long to sleep between failed lock-acquisition attempts while
/// bounded-waiting in [`AskLock::acquire`]. `try_lock_exclusive` has no
/// blocking-with-timeout mode of its own, so this polls.
const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Resolve the directory `notify ask` lock files live under.
///
/// Precedence, fleet-standard and NOT reinvented here:
/// 1. `lock_dir_arg` — an explicit `--lock-dir` CLI argument.
/// 2. `fleet_lock_dir_env` — the `FLEET_LOCK_DIR` env var.
/// 3. A `brain.toml` discovered by walking up from `cwd` (via the existing
///    [`crate::config::walk_up_from`] helper — this file does not
///    reimplement a second walk-up), joined with `.fleet-locks`.
/// 4. If no `brain.toml` is found walking up from `cwd`: `cwd` itself
///    joined with `.fleet-locks`.
#[must_use]
pub fn resolve_lock_dir(
    lock_dir_arg: Option<&str>,
    fleet_lock_dir_env: Option<&str>,
    cwd: &Path,
) -> PathBuf {
    if let Some(explicit) = lock_dir_arg {
        return PathBuf::from(explicit);
    }
    if let Some(env_val) = fleet_lock_dir_env {
        return PathBuf::from(env_val);
    }
    let base = crate::config::walk_up_from(cwd, "brain.toml")
        .and_then(|brain_toml| brain_toml.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| cwd.to_path_buf());
    base.join(FLEET_LOCK_DIR_NAME)
}

/// Validate a `--bot` slug is safe to embed directly in a filesystem path.
///
/// A slug comes straight off the command line, so this is a real
/// path-traversal surface, not a hypothetical one: only lowercase ASCII
/// letters, digits, and `-` are accepted. Anything else — in particular a
/// slug containing `/` or `..` — is **rejected**, never silently
/// normalised, so a caller cannot be surprised by a lock landing somewhere
/// other than `lock_dir`.
pub fn validate_lock_slug(slug: &str) -> Result<(), String> {
    if slug.is_empty() {
        return Err("bot slug must not be empty".to_string());
    }
    if !slug
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(format!(
            "bot slug '{slug}' contains characters outside [a-z0-9-]"
        ));
    }
    Ok(())
}

/// The path of the per-bot ask lock file: `<lock_dir>/notify-ask-<slug>.lock`.
///
/// Returns an error (via [`validate_lock_slug`]) rather than a
/// sanitised-but-different path when `slug` is unsafe, so a `/` or `..` in
/// `slug` can never cause the resulting path to escape `lock_dir`.
pub fn lock_path_for(lock_dir: &Path, slug: &str) -> Result<PathBuf, String> {
    validate_lock_slug(slug)?;
    Ok(lock_dir.join(format!("notify-ask-{slug}.lock")))
}

/// Why [`AskLock::acquire`] failed.
#[derive(Debug)]
pub enum AskLockError {
    /// The bounded wait elapsed without acquiring the lock — a sibling
    /// `notify ask` on the same `--bot` slug is holding it. Task 5 maps
    /// this to `AskOutcome::Busy` (exit code 4).
    Busy,
    /// Could not create `lock_dir`, or could not open/lock the lock file,
    /// for a reason other than contention.
    Io(io::Error),
}

impl std::fmt::Display for AskLockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AskLockError::Busy => write!(f, "ask lock busy: timed out waiting to acquire"),
            AskLockError::Io(e) => write!(f, "ask lock I/O error: {e}"),
        }
    }
}

impl std::error::Error for AskLockError {}

/// An exclusive, OS-level advisory lock on one bot's `notify ask` lock
/// file, held for the guard's lifetime and released — by the kernel,
/// unconditionally, including if this process is killed — the instant the
/// guard is dropped and its underlying file descriptor is closed. This is
/// deliberately not a bare "does the lockfile exist" check: that primitive
/// would wedge the channel permanently behind any killed holder, which is
/// exactly the failure this exists to rule out.
#[derive(Debug)]
pub struct AskLock {
    _file: File,
}

impl AskLock {
    /// Try to acquire the exclusive lock at `path`, polling up to
    /// `timeout` before giving up. Never blocks unboundedly — the caller's
    /// own `--timeout-secs` is expected to be threaded straight into
    /// `timeout`, so a busy lock never leaves `ask` polling `getUpdates`
    /// unlocked past its own deadline.
    ///
    /// Creates `path`'s parent directory (and the lock file itself) if
    /// they do not already exist.
    pub fn acquire(path: &Path, timeout: Duration) -> Result<Self, AskLockError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(AskLockError::Io)?;
        }

        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .map_err(AskLockError::Io)?;

        let deadline = Instant::now() + timeout;
        loop {
            match FileExt::try_lock(&file) {
                Ok(()) => return Ok(AskLock { _file: file }),
                Err(TryLockError::WouldBlock) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(AskLockError::Busy);
                    }
                    std::thread::sleep(LOCK_POLL_INTERVAL.min(deadline - now));
                }
                Err(TryLockError::Error(e)) => return Err(AskLockError::Io(e)),
            }
        }
    }
}

// ── The I/O shell: `bastion notify send|ask` (task 5) ───────────────────
//
// Everything above this point is pure logic (option parsing, the outcome
// contract, the per-batch decision, the lock). Everything below is the
// thin shell wiring it to real env vars, a real `TelegramTransport`, and
// real stdin/stdout/stderr — per `CLAUDE.md` standing rule 6 this is the
// I/O boundary and is manually smoke-tested, not unit-tested, except for
// the small pure helpers factored out below it.

/// Bot slugs `bastion serve` polls in the background
/// (`NotifyPollLoop::run` for `telegram`, `SessionQaBridge::run_outbound`
/// for `codesessions`). `ask` against one of these warns on stderr that it
/// competes with that loop for the same `getUpdates` stream; `send` never
/// warns (it never reads updates and so cannot steal anything).
pub const POLLED_BOT_SLUGS: &[&str] = &["telegram", "codesessions"];

/// The `-` stdin marker `--text`/`--summary` accept.
const STDIN_MARKER: &str = "-";

/// Resolve a `--text`/`--summary` CLI argument: [`STDIN_MARKER`] means
/// "read from stdin" (via the injected `stdin` closure, so this is
/// unit-testable without a real stdin); anything else is returned as-is.
///
/// Pure w.r.t. the injected closure — exhaustively unit-tested below.
pub fn resolve_text_arg(
    arg: &str,
    stdin: impl FnOnce() -> io::Result<String>,
) -> io::Result<String> {
    if arg == STDIN_MARKER {
        stdin()
    } else {
        Ok(arg.to_string())
    }
}

/// Read all of stdin to a `String`, trimming a single trailing `\n` (or
/// `\r\n`) — not all trailing whitespace — so `echo "text" | bastion notify
/// send --text -` doesn't carry an extra blank line onto the sent message.
fn read_stdin_text() -> io::Result<String> {
    use std::io::Read;
    let mut buf = String::new();
    io::stdin().read_to_string(&mut buf)?;
    if buf.ends_with('\n') {
        buf.pop();
        if buf.ends_with('\r') {
            buf.pop();
        }
    }
    Ok(buf)
}

/// Build the plain `sendMessage` request body for `bastion notify send`:
/// text only, no `reply_markup`, no buttons — deliberately NOT
/// [`crate::serve::notify::telegram::sendmessage_body`], which always
/// attaches an inline keyboard built from a payload's options.
///
/// Pure — unit-tested below.
#[must_use]
pub fn build_send_body(text: &str, chat_id: &str) -> serde_json::Value {
    serde_json::json!({
        "chat_id": chat_id,
        "text": text,
    })
}

/// The error message for an unconfigured (or unknown/typo'd) `--bot`
/// slug: names BOTH derived env vars for `slug` and lists the slugs that
/// DO have a complete pair — never a token or chat id value. An unknown
/// slug and a genuinely-unconfigured known slug are indistinguishable at
/// this layer (both resolve to "no env pair set"), so they share this one
/// message.
///
/// Pure — unit-tested below.
#[must_use]
pub fn unconfigured_bot_message(slug: &str, configured: &[String]) -> String {
    let (token_var, chat_var) = bot_env_var_names(slug);
    if configured.is_empty() {
        format!(
            "bot '{slug}' is not configured — set both {token_var} and {chat_var} \
             (no bots are currently configured)"
        )
    } else {
        format!(
            "bot '{slug}' is not configured — set both {token_var} and {chat_var} \
             (configured bots: {})",
            configured.join(", ")
        )
    }
}

/// Whether `--bot <slug>` warrants [`competing_poller_warning`].
///
/// Pure — unit-tested below.
#[must_use]
pub fn is_polled_bot_slug(slug: &str) -> bool {
    POLLED_BOT_SLUGS.contains(&slug)
}

/// The stderr warning printed when `ask --bot <slug>` targets a bot
/// `bastion serve` also polls in the background. `None` when `slug` is not
/// one of [`POLLED_BOT_SLUGS`].
///
/// Pure — unit-tested below.
#[must_use]
pub fn competing_poller_warning(slug: &str) -> Option<String> {
    if is_polled_bot_slug(slug) {
        Some(format!(
            "warning: --bot {slug} is also polled by `bastion serve`'s background loop \
             (NotifyPollLoop/SessionQaBridge) — this `ask` and that loop will compete for \
             the same Telegram update stream while both run"
        ))
    } else {
        None
    }
}

/// Snapshot the process environment into a `HashMap`, for
/// [`configured_bot_slugs`] — real I/O, not unit-tested (Rule 6); the pure
/// logic it feeds is.
fn env_snapshot() -> HashMap<String, String> {
    std::env::vars().collect()
}

/// Resolve `--bot <slug>` credentials or fail with
/// [`unconfigured_bot_message`]. UNCONFIGURED (or unknown) IS A HARD
/// ERROR — never a silent fallback to another bot (operator amendment,
/// 2026-08-23) and never a value in the error text.
fn load_bot_or_error(slug: &str) -> anyhow::Result<BotCredentials> {
    match crate::config::load_named_bot_config(slug) {
        Ok(Some(creds)) => Ok(creds),
        Ok(None) => {
            let configured = configured_bot_slugs(&env_snapshot());
            anyhow::bail!(unconfigured_bot_message(slug, &configured))
        }
        Err(ConfigError::IncompleteNamedBotConfig(missing)) => {
            anyhow::bail!(
                "bot '{slug}' config is incomplete: {missing} must also be set (both \
                 BOT_TOKEN and CHAT_ID or neither)"
            )
        }
        Err(other) => anyhow::bail!("failed to load bot '{slug}' config: {other}"),
    }
}

/// How long to sleep before retrying a retryable [`NotifyError`] — the
/// transport's own `retry_after_secs` hint for `RateLimited`, a short
/// fixed backoff otherwise. Never called for a permanent error;
/// `NotifyError::is_retryable()` draws that line, not this function.
async fn sleep_before_retry(err: &NotifyError) {
    let dur = match err {
        NotifyError::RateLimited { retry_after_secs } => Duration::from_secs(*retry_after_secs),
        _ => Duration::from_millis(500),
    };
    tokio::time::sleep(dur).await;
}

/// Scan `responses` for the one this outstanding ask should acknowledge,
/// alongside its resolved [`ResponseVerdict`] — same "first `Accepted`,
/// else first `StaleDigest`" precedence as [`decide_batch`], and built on
/// the exact same [`resolve_response`] primitive, so the two never
/// disagree about which response (if any) answers this batch. Kept
/// separate from `decide_batch` only because `acknowledge` needs the
/// original [`OperatorResponse`] (its `ack`/`message` handles), which
/// `BatchDecision` does not carry.
fn find_ack_target<'a>(
    responses: &'a [OperatorResponse],
    expected: &ValidatedOperatorPayload,
) -> Option<(&'a OperatorResponse, ResponseVerdict)> {
    let mut stale: Option<(&OperatorResponse, ResponseVerdict)> = None;
    for resp in responses {
        let verdict = resolve_response(resp, expected);
        match &verdict {
            ResponseVerdict::Accepted { .. } => return Some((resp, verdict)),
            ResponseVerdict::StaleDigest { .. } => {
                if stale.is_none() {
                    stale = Some((resp, verdict));
                }
            }
            ResponseVerdict::UnknownGate => {}
        }
    }
    stale
}

/// Print `outcome`'s JSON contract to stdout and terminate the process
/// with its exit code — never returns. This is the one place `notify ask`
/// exits with anything other than 1 (config/validation/transport errors
/// bubble up through `anyhow::Result` instead, per `main.rs`'s existing
/// error-to-exit-1 handling), so every terminal `AskOutcome` funnels
/// through here.
fn print_outcome_and_exit(outcome: &AskOutcome) -> ! {
    println!("{}", outcome.to_json());
    std::process::exit(outcome.exit_code());
}

/// `bastion notify send --text <msg|-> [--bot <slug>]` — fire-and-forget
/// `sendMessage`: no buttons, no digest, no poll, no lock. Prints nothing
/// on success and returns `Ok(())` (main.rs exits 0); any failure —
/// unconfigured bot, transport error — is an `Err` (main.rs exits 1).
pub async fn run_send(bot_slug: &str, text_arg: &str) -> anyhow::Result<()> {
    let text = resolve_text_arg(text_arg, read_stdin_text)?;
    let creds = load_bot_or_error(bot_slug)?;

    let body = build_send_body(&text, &creds.chat_id);
    let method = "sendMessage";
    tracing::debug!(method, "calling Telegram Bot API");

    let url = crate::serve::notify::telegram_http::api_url(&creds.bot_token, method);
    let client = reqwest::Client::new();
    let resp =
        client.post(url).json(&body).send().await.map_err(|e| {
            anyhow::anyhow!("{method} request failed: {}", classify_reqwest_error(&e))
        })?;

    let status = resp.status().as_u16();
    if let Some(err) = crate::serve::notify::telegram_http::classify_http_status(
        status,
        retry_after_from_response(&resp),
    ) {
        anyhow::bail!("{method} failed: {err}");
    }

    Ok(())
}

/// `bastion notify ask --gate-id <id> --summary <text|-> --option
/// <key:Label> [...] [--timeout-secs N] [--bot <slug>] [--lock-dir <dir>]`
/// — builds and validates an `OperatorPayload`, holds the per-bot ask
/// lock, sends it, and long-polls until a resolving tap arrives or the
/// timeout elapses. Terminates the process directly via
/// [`print_outcome_and_exit`] for every `AskOutcome` (exit 0/2/3/4);
/// returns `Err` (main.rs exits 1) for config/validation/permanent-
/// transport failures.
#[allow(clippy::too_many_arguments)]
pub async fn run_ask(
    bot_slug: &str,
    gate_id: String,
    summary_arg: &str,
    options_raw: &[String],
    timeout_secs: u64,
    lock_dir_arg: Option<&Path>,
) -> anyhow::Result<()> {
    let summary = resolve_text_arg(summary_arg, read_stdin_text)?;
    let options = parse_options(options_raw).map_err(|e| anyhow::anyhow!(e))?;

    // Both validation gates run BEFORE any HTTP request or lock acquisition,
    // so an over-limit payload is rejected with no partial send.
    let payload = OperatorPayload::new(gate_id, summary, options);
    let limits = OperatorPayloadLimits::default();
    let validated = validate(payload, &limits).map_err(|e| anyhow::anyhow!("{e}"))?;
    check_whatsapp_portability(&validated, &limits).map_err(|e| anyhow::anyhow!("{e}"))?;

    let creds = load_bot_or_error(bot_slug)?;

    if let Some(warning) = competing_poller_warning(bot_slug) {
        eprintln!("{warning}");
    }

    let cwd = std::env::current_dir()?;
    let lock_dir = resolve_lock_dir(
        lock_dir_arg.and_then(Path::to_str),
        std::env::var(FLEET_LOCK_DIR_ENV).ok().as_deref(),
        &cwd,
    );
    let lock_path = lock_path_for(&lock_dir, bot_slug).map_err(|e| anyhow::anyhow!(e))?;
    let timeout_dur = Duration::from_secs(timeout_secs);

    let _lock = match AskLock::acquire(&lock_path, timeout_dur) {
        Ok(lock) => lock,
        Err(AskLockError::Busy) => print_outcome_and_exit(&AskOutcome::Busy),
        Err(AskLockError::Io(e)) => anyhow::bail!("failed to acquire ask lock: {e}"),
    };

    let transport = TelegramTransport::with_credentials(creds.bot_token, creds.chat_id);
    let deadline = Instant::now() + timeout_dur;

    // Initial send: retry on a retryable NotifyError within the deadline,
    // stop (exit 1, reason on stderr) on a permanent one.
    loop {
        match transport.send(&validated).await {
            Ok(_) => break,
            Err(e) if e.is_retryable() => {
                if Instant::now() >= deadline {
                    print_outcome_and_exit(&AskOutcome::Timeout);
                }
                sleep_before_retry(&e).await;
            }
            Err(e) => anyhow::bail!("send failed: {e}"),
        }
    }

    // Poll loop: the cursor is carried forward across every batch,
    // including one that yields BatchDecision::KeepPolling because it held
    // only foreign-gate updates — never reset, never skipped.
    let mut cursor: Option<UpdateCursor> = None;
    loop {
        if Instant::now() >= deadline {
            print_outcome_and_exit(&AskOutcome::Timeout);
        }

        match transport.poll_responses(cursor.clone()).await {
            Ok((responses, next_cursor)) => {
                if next_cursor.is_some() {
                    cursor = next_cursor;
                }

                match decide_batch(&responses, &validated) {
                    BatchDecision::KeepPolling => {}
                    BatchDecision::Answered { .. } | BatchDecision::Stale { .. } => {
                        if let Some((resp, verdict)) = find_ack_target(&responses, &validated) {
                            // Acknowledgement is what answers the callback query
                            // and strips the inline keyboard; a failure here is
                            // logged, never fatal — the decision itself already
                            // resolved.
                            if let Err(e) = transport.acknowledge(resp, &verdict).await {
                                tracing::warn!(error = %e, "failed to acknowledge operator response");
                            }
                            match verdict {
                                ResponseVerdict::Accepted {
                                    gate_id,
                                    option_key,
                                    decided_at,
                                    ..
                                } => print_outcome_and_exit(&AskOutcome::Answered {
                                    gate_id,
                                    option_key,
                                    decided_at,
                                }),
                                ResponseVerdict::StaleDigest {
                                    gate_id,
                                    option_key,
                                    digest,
                                    decided_at,
                                } => print_outcome_and_exit(&AskOutcome::StaleDigest {
                                    gate_id,
                                    option_key,
                                    digest,
                                    decided_at,
                                }),
                                ResponseVerdict::UnknownGate => unreachable!(
                                    "find_ack_target never returns an UnknownGate verdict"
                                ),
                            }
                        }
                    }
                }
            }
            Err(e) if e.is_retryable() => {
                if Instant::now() >= deadline {
                    print_outcome_and_exit(&AskOutcome::Timeout);
                }
                sleep_before_retry(&e).await;
            }
            Err(e) => anyhow::bail!("poll failed: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::operator::{AckHandle, MessageHandle, OperatorPayload, OperatorPayloadLimits};

    // ── parse_option / parse_options ─────────────────────────────────

    #[test]
    fn parse_option_splits_on_first_colon_only() {
        let opt = parse_option("approve:Approve: ship it").expect("should parse");
        assert_eq!(opt.key, "approve");
        assert_eq!(opt.label, "Approve: ship it");
    }

    #[test]
    fn parse_option_rejects_missing_separator() {
        let err = parse_option("approve-Approve").unwrap_err();
        assert!(err.contains("':'"), "unexpected message: {err}");
    }

    #[test]
    fn parse_option_rejects_empty_key() {
        let err = parse_option(":Approve").unwrap_err();
        assert!(err.contains("empty key"), "unexpected message: {err}");
    }

    #[test]
    fn parse_option_rejects_empty_label() {
        let err = parse_option("approve:").unwrap_err();
        assert!(err.contains("empty label"), "unexpected message: {err}");
    }

    #[test]
    fn parse_option_empty_key_and_empty_label_messages_are_distinct() {
        let empty_key = parse_option(":Approve").unwrap_err();
        let empty_label = parse_option("approve:").unwrap_err();
        assert_ne!(empty_key, empty_label);
    }

    #[test]
    fn parse_options_rejects_duplicate_keys() {
        let raw = vec![
            "approve:Approve".to_string(),
            "approve:Also approve".to_string(),
        ];
        let err = parse_options(&raw).unwrap_err();
        assert!(err.contains("duplicate"), "unexpected message: {err}");
        assert!(err.contains("approve"), "unexpected message: {err}");
    }

    #[test]
    fn parse_options_accepts_distinct_keys_in_order() {
        let raw = vec!["approve:Approve".to_string(), "reject:Reject".to_string()];
        let opts = parse_options(&raw).expect("should parse");
        assert_eq!(opts.len(), 2);
        assert_eq!(opts[0].key, "approve");
        assert_eq!(opts[1].key, "reject");
    }

    // ── AskOutcome::exit_code ─────────────────────────────────────────

    #[test]
    fn exit_code_is_total_over_all_four_variants() {
        let now = chrono::Utc::now();
        assert_eq!(
            AskOutcome::Answered {
                gate_id: "g".to_string(),
                option_key: "approve".to_string(),
                decided_at: now,
            }
            .exit_code(),
            0
        );
        assert_eq!(AskOutcome::Timeout.exit_code(), 2);
        assert_eq!(
            AskOutcome::StaleDigest {
                gate_id: "g".to_string(),
                option_key: "approve".to_string(),
                digest: "deadbeef".to_string(),
                decided_at: now,
            }
            .exit_code(),
            3
        );
        assert_eq!(AskOutcome::Busy.exit_code(), 4);
    }

    // ── AskOutcome::to_json, key by key ─────────────────────────────────

    #[test]
    fn to_json_answered_carries_expected_keys() {
        let now = chrono::Utc::now();
        let json = AskOutcome::Answered {
            gate_id: "gate-1".to_string(),
            option_key: "approve".to_string(),
            decided_at: now,
        }
        .to_json();
        assert_eq!(json["status"], "answered");
        assert_eq!(json["gate_id"], "gate-1");
        assert_eq!(json["option_key"], "approve");
        assert_eq!(json["decided_at"], now.to_rfc3339());
        assert_eq!(
            json.as_object().expect("object").len(),
            4,
            "unexpected extra/missing key in answered shape"
        );
    }

    #[test]
    fn to_json_timeout_carries_only_status() {
        let json = AskOutcome::Timeout.to_json();
        assert_eq!(json["status"], "timeout");
        assert_eq!(json.as_object().expect("object").len(), 1);
    }

    #[test]
    fn to_json_stale_digest_carries_expected_keys() {
        let now = chrono::Utc::now();
        let json = AskOutcome::StaleDigest {
            gate_id: "gate-1".to_string(),
            option_key: "approve".to_string(),
            digest: "deadbeef".to_string(),
            decided_at: now,
        }
        .to_json();
        assert_eq!(json["status"], "stale_digest");
        assert_eq!(json["gate_id"], "gate-1");
        assert_eq!(json["option_key"], "approve");
        assert_eq!(json["digest"], "deadbeef");
        assert_eq!(json["decided_at"], now.to_rfc3339());
        assert_eq!(json.as_object().expect("object").len(), 5);
    }

    #[test]
    fn to_json_busy_carries_only_status() {
        let json = AskOutcome::Busy.to_json();
        assert_eq!(json["status"], "busy");
        assert_eq!(json.as_object().expect("object").len(), 1);
    }

    // ── decide_batch ──────────────────────────────────────────────────

    fn approve_reject() -> Vec<OperatorResponseOption> {
        vec![
            OperatorResponseOption::new("approve", "Approve"),
            OperatorResponseOption::new("reject", "Reject"),
        ]
    }

    fn validated(gate_id: &str, summary: &str) -> ValidatedOperatorPayload {
        let payload = OperatorPayload::new(gate_id, summary, approve_reject());
        engine_core::operator::validate(payload, &OperatorPayloadLimits::default())
            .expect("payload validates")
    }

    fn response_for(gate_id: &str, digest: &str, option_key: &str) -> OperatorResponse {
        OperatorResponse {
            gate_id: gate_id.to_string(),
            digest: digest.to_string(),
            option_key: option_key.to_string(),
            received_at: chrono::Utc::now(),
            ack: Some(AckHandle("ack-1".to_string())),
            message: Some(MessageHandle {
                chat_id: "chat-1".to_string(),
                message_id: 1,
            }),
        }
    }

    /// The truncated digest prefix `resolve_response` compares against,
    /// derived from `expected` the same way the transport does — see
    /// `crate::serve::notify::telegram::CALLBACK_DIGEST_PREFIX_LEN`.
    fn matching_prefix(expected: &ValidatedOperatorPayload) -> String {
        expected
            .payload()
            .digest
            .chars()
            .take(crate::serve::notify::telegram::CALLBACK_DIGEST_PREFIX_LEN)
            .collect()
    }

    #[test]
    fn decide_batch_empty_batch_keeps_polling() {
        let expected = validated("gate-1", "diff summary");
        assert_eq!(decide_batch(&[], &expected), BatchDecision::KeepPolling);
    }

    #[test]
    fn decide_batch_one_matching_tap_answers() {
        let expected = validated("gate-1", "diff summary");
        let prefix = matching_prefix(&expected);
        let responses = vec![response_for("gate-1", &prefix, "approve")];

        match decide_batch(&responses, &expected) {
            BatchDecision::Answered {
                gate_id,
                option_key,
                ..
            } => {
                assert_eq!(gate_id, "gate-1");
                assert_eq!(option_key, "approve");
            }
            other => panic!("expected Answered, got {other:?}"),
        }
    }

    #[test]
    fn decide_batch_foreign_gate_tap_keeps_polling_not_an_error() {
        let expected = validated("gate-1", "diff summary");
        let prefix = matching_prefix(&expected);
        let responses = vec![response_for("gate-other", &prefix, "approve")];

        assert_eq!(
            decide_batch(&responses, &expected),
            BatchDecision::KeepPolling
        );
    }

    #[test]
    fn decide_batch_stale_digest_tap_is_stale_never_answered() {
        let expected = validated("gate-1", "diff summary");
        let responses = vec![response_for("gate-1", "stale-prefix", "approve")];

        match decide_batch(&responses, &expected) {
            BatchDecision::Stale {
                gate_id,
                option_key,
                digest,
                ..
            } => {
                assert_eq!(gate_id, "gate-1");
                assert_eq!(option_key, "approve");
                assert_eq!(digest, "stale-prefix");
            }
            other => panic!("expected Stale, got {other:?}"),
        }
    }

    #[test]
    fn decide_batch_two_responses_only_second_matches_answers_from_second() {
        let expected = validated("gate-1", "diff summary");
        let prefix = matching_prefix(&expected);
        let responses = vec![
            response_for("gate-other", "irrelevant", "reject"),
            response_for("gate-1", &prefix, "approve"),
        ];

        match decide_batch(&responses, &expected) {
            BatchDecision::Answered {
                gate_id,
                option_key,
                ..
            } => {
                assert_eq!(gate_id, "gate-1");
                assert_eq!(option_key, "approve");
            }
            other => panic!("expected Answered, got {other:?}"),
        }
    }

    #[test]
    fn decide_batch_foreign_gate_tap_precedes_matching_tap_still_answers() {
        let expected = validated("gate-1", "diff summary");
        let prefix = matching_prefix(&expected);
        let responses = vec![
            response_for("gate-foreign", "whatever", "reject"),
            response_for("gate-1", &prefix, "approve"),
        ];

        match decide_batch(&responses, &expected) {
            BatchDecision::Answered { gate_id, .. } => assert_eq!(gate_id, "gate-1"),
            other => panic!("expected Answered, got {other:?}"),
        }
    }

    // ── resolve_lock_dir precedence (pure) ─────────────────────────────

    #[test]
    fn resolve_lock_dir_prefers_explicit_arg_over_everything() {
        let cwd = std::env::temp_dir();
        let resolved = resolve_lock_dir(Some("/explicit/dir"), Some("/env/dir"), &cwd);
        assert_eq!(resolved, PathBuf::from("/explicit/dir"));
    }

    #[test]
    fn resolve_lock_dir_falls_back_to_env_when_no_arg() {
        let cwd = std::env::temp_dir();
        let resolved = resolve_lock_dir(None, Some("/env/dir"), &cwd);
        assert_eq!(resolved, PathBuf::from("/env/dir"));
    }

    #[test]
    fn resolve_lock_dir_walks_up_to_brain_toml_when_no_arg_or_env() {
        let dir = tempfile::tempdir().expect("tempdir");
        let brain_toml = dir.path().join("brain.toml");
        fs::write(&brain_toml, "").expect("write brain.toml");
        let nested = dir.path().join("a/b/c");
        fs::create_dir_all(&nested).expect("create nested");

        let resolved = resolve_lock_dir(None, None, &nested);
        assert_eq!(resolved, dir.path().join(FLEET_LOCK_DIR_NAME));
    }

    #[test]
    fn resolve_lock_dir_falls_back_to_cwd_when_no_brain_toml_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        // No brain.toml anywhere under dir — walk-up must exhaust and this
        // must fall back to `cwd`, not panic or wander past the tempdir.
        let resolved = resolve_lock_dir(None, None, dir.path());
        assert_eq!(resolved, dir.path().join(FLEET_LOCK_DIR_NAME));
    }

    // ── validate_lock_slug / lock_path_for ──────────────────────────────

    #[test]
    fn validate_lock_slug_accepts_lowercase_alnum_and_hyphen() {
        assert!(validate_lock_slug("lane").is_ok());
        assert!(validate_lock_slug("lane-2").is_ok());
        assert!(validate_lock_slug("a1-b2-c3").is_ok());
    }

    #[test]
    fn validate_lock_slug_rejects_empty() {
        assert!(validate_lock_slug("").is_err());
    }

    #[test]
    fn validate_lock_slug_rejects_path_separator() {
        let err = validate_lock_slug("lane/evil").unwrap_err();
        assert!(err.contains("lane/evil"), "unexpected message: {err}");
    }

    #[test]
    fn validate_lock_slug_rejects_dotdot() {
        assert!(validate_lock_slug("..").is_err());
        assert!(validate_lock_slug("../etc").is_err());
    }

    #[test]
    fn validate_lock_slug_rejects_uppercase_and_other_symbols() {
        assert!(validate_lock_slug("Lane").is_err());
        assert!(validate_lock_slug("lane_2").is_err());
        assert!(validate_lock_slug("lane.2").is_err());
    }

    #[test]
    fn lock_path_for_matches_expected_shape() {
        let lock_dir = PathBuf::from("/fleet/.fleet-locks");
        let path = lock_path_for(&lock_dir, "lane").expect("valid slug");
        assert_eq!(
            path,
            PathBuf::from("/fleet/.fleet-locks/notify-ask-lane.lock")
        );
    }

    #[test]
    fn lock_path_for_two_different_slugs_yield_different_paths() {
        let lock_dir = PathBuf::from("/fleet/.fleet-locks");
        let a = lock_path_for(&lock_dir, "lane").expect("valid slug");
        let b = lock_path_for(&lock_dir, "second-bot").expect("valid slug");
        assert_ne!(a, b);
    }

    #[test]
    fn lock_path_for_same_slug_yields_same_path() {
        let lock_dir = PathBuf::from("/fleet/.fleet-locks");
        let a = lock_path_for(&lock_dir, "lane").expect("valid slug");
        let b = lock_path_for(&lock_dir, "lane").expect("valid slug");
        assert_eq!(a, b);
    }

    #[test]
    fn lock_path_for_traversal_slug_is_rejected_not_escaped() {
        let lock_dir = PathBuf::from("/fleet/.fleet-locks");
        // If this ever silently normalised instead of rejecting, the escape
        // check below is what would catch a `../..`-style slug landing
        // outside `lock_dir` — but the real contract is that it errors
        // before a path is even constructed.
        assert!(lock_path_for(&lock_dir, "../../etc/passwd").is_err());
        assert!(lock_path_for(&lock_dir, "lane/../../etc").is_err());
    }

    // ── AskLock::acquire ──────────────────────────────────────────────

    #[test]
    fn ask_lock_acquires_when_free_and_releases_on_drop() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("notify-ask-lane.lock");

        let guard = AskLock::acquire(&path, Duration::from_secs(1)).expect("should acquire");
        drop(guard);

        // Released on drop: a fresh acquire must succeed immediately.
        let guard2 =
            AskLock::acquire(&path, Duration::from_millis(200)).expect("should re-acquire");
        drop(guard2);
    }

    #[test]
    fn ask_lock_creates_parent_dir_if_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir
            .path()
            .join("nested/does/not/exist/notify-ask-lane.lock");

        let guard = AskLock::acquire(&path, Duration::from_secs(1)).expect("should acquire");
        assert!(path.exists());
        drop(guard);
    }

    #[test]
    fn ask_lock_contended_case_times_out_to_busy() {
        // Contended case, covered in-process: two `AskLock`s (i.e. two file
        // handles opened separately, mirroring two OS processes) against
        // the SAME path. `flock`(2) advisory locks are per-open-file-
        // description, so opening the path twice from one process
        // faithfully reproduces the two-process contention this guards
        // against — no second process required.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("notify-ask-lane.lock");

        let _first = AskLock::acquire(&path, Duration::from_secs(1)).expect("first should acquire");

        let started = Instant::now();
        let second = AskLock::acquire(&path, Duration::from_millis(300));
        let elapsed = started.elapsed();

        match second {
            Err(AskLockError::Busy) => {}
            other => panic!("expected Busy, got {other:?}"),
        }
        assert!(
            elapsed >= Duration::from_millis(250),
            "expected the bounded wait to actually wait, elapsed={elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "bounded wait must not run away past its timeout, elapsed={elapsed:?}"
        );
    }

    #[test]
    fn ask_lock_released_holder_lets_a_second_acquire_succeed_before_timeout() {
        // A killed/dropped holder must not wedge the channel: once the
        // first guard is dropped, a second `acquire` — even one already
        // mid-poll — must succeed well before its own timeout elapses.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("notify-ask-lane.lock");

        let first = AskLock::acquire(&path, Duration::from_secs(1)).expect("first should acquire");

        let path_clone = path.clone();
        let handle =
            std::thread::spawn(move || AskLock::acquire(&path_clone, Duration::from_secs(5)));

        std::thread::sleep(Duration::from_millis(150));
        drop(first);

        let second = handle.join().expect("thread should not panic");
        assert!(
            second.is_ok(),
            "expected the second acquire to succeed after the holder dropped"
        );
    }

    // ── resolve_text_arg / read_stdin_text ────────────────────────────

    #[test]
    fn resolve_text_arg_non_dash_is_returned_as_is() {
        let result = resolve_text_arg("hello", || panic!("stdin must not be called"));
        assert_eq!(result.unwrap(), "hello");
    }

    #[test]
    fn resolve_text_arg_dash_reads_from_injected_stdin() {
        let result = resolve_text_arg("-", || Ok("from stdin".to_string()));
        assert_eq!(result.unwrap(), "from stdin");
    }

    #[test]
    fn resolve_text_arg_dash_propagates_stdin_error() {
        let result = resolve_text_arg("-", || Err(io::Error::other("stdin broke")));
        assert!(result.is_err());
    }

    // ── build_send_body ─────────────────────────────────────────────

    #[test]
    fn build_send_body_has_no_reply_markup() {
        let body = build_send_body("hello", "chat-1");
        assert_eq!(body["chat_id"], "chat-1");
        assert_eq!(body["text"], "hello");
        assert!(
            body.get("reply_markup").is_none(),
            "send must never attach an inline keyboard"
        );
    }

    // ── unconfigured_bot_message ────────────────────────────────────

    #[test]
    fn unconfigured_bot_message_names_both_derived_vars() {
        let msg = unconfigured_bot_message("lane", &[]);
        assert!(msg.contains("BASTION_LANE_BOT_TOKEN"));
        assert!(msg.contains("BASTION_LANE_CHAT_ID"));
    }

    #[test]
    fn unconfigured_bot_message_lists_configured_slugs() {
        let msg = unconfigured_bot_message("typo", &["lane".to_string(), "telegram".to_string()]);
        assert!(msg.contains("BASTION_TYPO_BOT_TOKEN"));
        assert!(msg.contains("BASTION_TYPO_CHAT_ID"));
        assert!(msg.contains("lane"));
        assert!(msg.contains("telegram"));
    }

    #[test]
    fn unconfigured_bot_message_never_contains_a_credential_shaped_value() {
        // Regression guard: the message must be built only from slug/env-var
        // names, never from a token or chat id value.
        let msg = unconfigured_bot_message("lane", &["codesessions".to_string()]);
        assert!(!msg.to_lowercase().contains("secret"));
        assert!(!msg.contains("123:ABC"));
    }

    #[test]
    fn unconfigured_bot_message_empty_configured_list_says_none_configured() {
        let msg = unconfigured_bot_message("lane", &[]);
        assert!(msg.contains("no bots are currently configured"));
    }

    // ── is_polled_bot_slug / competing_poller_warning ───────────────

    #[test]
    fn is_polled_bot_slug_true_for_telegram_and_codesessions() {
        assert!(is_polled_bot_slug("telegram"));
        assert!(is_polled_bot_slug("codesessions"));
    }

    #[test]
    fn is_polled_bot_slug_false_for_lane_and_unknown() {
        assert!(!is_polled_bot_slug("lane"));
        assert!(!is_polled_bot_slug("some-other-bot"));
    }

    #[test]
    fn competing_poller_warning_present_for_polled_slugs() {
        assert!(competing_poller_warning("telegram").is_some());
        assert!(competing_poller_warning("codesessions").is_some());
    }

    #[test]
    fn competing_poller_warning_none_for_lane() {
        assert!(competing_poller_warning("lane").is_none());
    }

    // ── find_ack_target ──────────────────────────────────────────────

    fn make_response(gate_id: &str, digest_prefix: &str, option_key: &str) -> OperatorResponse {
        OperatorResponse {
            gate_id: gate_id.to_string(),
            digest: digest_prefix.to_string(),
            option_key: option_key.to_string(),
            received_at: chrono::Utc::now(),
            ack: Some(AckHandle("ack-1".to_string())),
            message: Some(MessageHandle {
                chat_id: "chat-1".to_string(),
                message_id: 42,
            }),
        }
    }

    fn make_expected() -> ValidatedOperatorPayload {
        let payload = OperatorPayload::new(
            "gate-1",
            "diff summary",
            vec![
                OperatorResponseOption::new("approve", "Approve"),
                OperatorResponseOption::new("reject", "Reject"),
            ],
        );
        validate(payload, &OperatorPayloadLimits::default()).unwrap()
    }

    #[test]
    fn find_ack_target_returns_accepted_match() {
        let expected = make_expected();
        let digest_prefix = &expected.payload().digest
            [..crate::serve::notify::telegram::CALLBACK_DIGEST_PREFIX_LEN];
        let responses = vec![make_response("gate-1", digest_prefix, "approve")];

        let (resp, verdict) =
            find_ack_target(&responses, &expected).expect("should find a matching response");
        assert_eq!(resp.gate_id, "gate-1");
        assert!(matches!(verdict, ResponseVerdict::Accepted { .. }));
    }

    #[test]
    fn find_ack_target_skips_foreign_gate_and_returns_none() {
        let expected = make_expected();
        let responses = vec![make_response("some-other-gate", "deadbeef0000", "approve")];

        assert!(find_ack_target(&responses, &expected).is_none());
    }

    #[test]
    fn find_ack_target_prefers_accepted_over_earlier_stale() {
        let expected = make_expected();
        let digest_prefix = &expected.payload().digest
            [..crate::serve::notify::telegram::CALLBACK_DIGEST_PREFIX_LEN];
        let responses = vec![
            make_response("gate-1", "0000deadbeef", "approve"), // stale digest
            make_response("gate-1", digest_prefix, "reject"),   // accepted
        ];

        let (_, verdict) = find_ack_target(&responses, &expected).unwrap();
        match verdict {
            ResponseVerdict::Accepted { option_key, .. } => assert_eq!(option_key, "reject"),
            other => panic!("expected Accepted, got {other:?}"),
        }
    }
}

// ── The reuse discipline itself ─────────────────────────────────────────
//
// Mechanical guard, kept OUTSIDE this file so `include_str!` on
// `notify_cli.rs` cannot trivially match its own guard strings: this file
// must never grow its own callback_data encoder, offset-advance
// arithmetic, or digest comparison — all three already exist, tested, in
// `src/serve/notify/telegram.rs`. This module's own reliance on
// `resolve_response` (rather than a local digest comparison) is the
// positive half of that guarantee and is exercised by `decide_batch`'s
// tests above; a later task's binary-level contract test carries the
// mechanical grep-style enforcement over the file itself.
