//! Pane-free question path (`BA.21.C` task 1) — pure core.
//!
//! [`PendingQuestion`](super::PendingQuestion) /
//! [`PendingQuestions`](super::PendingQuestions) in the parent module are
//! entirely tmux-pane-shaped: a question is captured from a pane, keyed on
//! the tmux session it came from, and answered by injecting keystrokes back
//! into that same session. A headless engine chain — one suspended by
//! `engine-rs`'s `nodes/suspend.rs` with no pane anywhere in the loop — has
//! none of that: no session name to key on, no pane to inject into. This
//! module is the pane-free equivalent: a registry keyed on the suspended
//! run's id, a question builder that reads the run's own suspend state
//! instead of a captured pane, and a resolver that maps an operator's tap
//! back to the run it was asked for instead of to a `send_keys` target.
//!
//! The registry / builder / resolver above are pure logic: no HTTP call, no
//! tmux call, no task spawn, mirroring the discipline at the top of the
//! parent module. Below that (`BA.21.C` task 3) is the thin async delivery
//! shell over that pure core — scanning suspended runs, sending over the
//! injected `OperatorTransport`, and deciding (never issuing) the resume —
//! modelled on `super::super::notify::stale_run_alarm`'s
//! `deliver_once` / `spawn_alarm_delivery_loop` / `AlarmDeliveryHandle`
//! shape, this repo's established drain-over-an-injected-transport pattern.
//! The actual `ApiClient::resume_run` call and the boot wiring that decides
//! *when* to spawn the loop are task 4's job (`src/serve/mod.rs`), not
//! this module's.
//!
//! This module deliberately does **not** reuse or wrap
//! [`PendingQuestion`](super::PendingQuestion) /
//! [`PendingQuestions`](super::PendingQuestions) — `PendingQuestion.session`
//! stays a tmux session name, unaffected by anything in here. It DOES reuse
//! `engine_core::operator::OperatorPayload` — the pane-bound path never
//! goes near an engine gate, but this path exists *because* a suspended
//! engine run's question is exactly that shape.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use engine_core::operator::{
    OperatorPayload, OperatorPayloadLimits, OperatorResponseOption, OperatorTransport,
};

use crate::serve::notify::{PendingPayloads, ResponseVerdict};

/// Key under which a suspended run's `TaskContext::metadata` may carry a
/// pre-built structured question (an `engine_core::operator::OperatorPayload`
/// serialized as JSON, with `gate_id`/`digest` not yet meaningful — both are
/// unconditionally overwritten by [`question_from_suspended`]). When absent
/// or not deserializable, a question is synthesized from the run's
/// `workflow_type` / `reason` / `resume_at` instead.
pub const HEADLESS_QUESTION_METADATA_KEY: &str = "operator_question";

/// Deterministic prefix for this path's gate-id space. Disjoint by
/// construction from `engine_core::workflows::approve_and_run::gate_id_for`'s
/// `"approve-and-run:"` space and from the notify test route's per-request
/// uuid space — the same disjointness argument
/// `crate::serve::mod::resolve_pending_lookup`'s doc comment already makes
/// for those two, extended here to a third source.
const GATE_ID_PREFIX: &str = "hq-";

/// Deterministic gate id for `run_id` — the SAME id every time this run is
/// asked about, which is what lets [`PendingHeadlessQuestions::register`]
/// dedup a repeated delivery tick against the one entry already pending for
/// this run rather than minting a new id per tick.
#[must_use]
pub fn gate_id_for_run(run_id: uuid::Uuid) -> String {
    format!("{GATE_ID_PREFIX}{run_id}")
}

/// Whether `gate_id` falls in this module's id space. Used by the delivery
/// shell (task 3) to refuse to resolve a foreign gate id as a headless tap.
#[must_use]
pub fn is_headless_gate_id(gate_id: &str) -> bool {
    gate_id.starts_with(GATE_ID_PREFIX)
}

/// One question this process has sent (or is about to send) to the
/// operator on behalf of a suspended run — the pane-free analogue of
/// [`super::PendingQuestion`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadlessQuestion {
    /// The suspended run this question was asked on behalf of. Contrast
    /// [`super::PendingQuestion::session`], which is a tmux session name and
    /// carries no run id at all.
    pub run_id: uuid::Uuid,
    /// The validated-shape payload actually rendered to the operator.
    pub payload: OperatorPayload,
    /// When this question was registered.
    pub asked_at: DateTime<Utc>,
    /// Whether a tap has already been accepted for this question. Set by
    /// [`PendingHeadlessQuestions::mark_answered`]; a second tap after this
    /// is `AlreadyAnswered`, never a second resume.
    pub answered: bool,
}

impl HeadlessQuestion {
    #[must_use]
    pub fn new(run_id: uuid::Uuid, payload: OperatorPayload, asked_at: DateTime<Utc>) -> Self {
        Self {
            run_id,
            payload,
            asked_at,
            answered: false,
        }
    }
}

/// The outcome of [`PendingHeadlessQuestions::register`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadlessRegisterOutcome {
    /// A new entry was created under `gate_id`; the caller should send a
    /// question for it.
    Created { gate_id: String },
    /// An unanswered entry for this run already existed; its id is returned
    /// and the caller must send nothing — this is what makes "one question,
    /// one message" true.
    AlreadyPending { gate_id: String },
}

impl HeadlessRegisterOutcome {
    #[must_use]
    pub fn gate_id(&self) -> &str {
        match self {
            HeadlessRegisterOutcome::Created { gate_id }
            | HeadlessRegisterOutcome::AlreadyPending { gate_id } => gate_id,
        }
    }
}

struct PendingHeadlessQuestionsInner {
    by_id: HashMap<String, HeadlessQuestion>,
    order: VecDeque<String>,
}

/// Bounded, FIFO-eviction registry of [`HeadlessQuestion`]s, keyed by
/// `gate_id` — field-for-field modelled on [`super::PendingQuestions`] (same
/// `CAPACITY`, same `Mutex<Inner>` shape, same `by_id`/`order` split, same
/// method set) but NOT built by wrapping it: the two registries must be
/// provably separate allocations, since collapsing them is the failure this
/// block is most likely to cause.
pub struct PendingHeadlessQuestions {
    inner: std::sync::Mutex<PendingHeadlessQuestionsInner>,
}

impl PendingHeadlessQuestions {
    /// Maximum number of pending headless questions held at once, mirroring
    /// [`super::PendingQuestions::CAPACITY`]'s generous headroom above any
    /// plausible in-flight volume.
    pub const CAPACITY: usize = 256;

    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: std::sync::Mutex::new(PendingHeadlessQuestionsInner {
                by_id: HashMap::new(),
                order: VecDeque::new(),
            }),
        }
    }

    fn mutex_lock(&self) -> std::sync::MutexGuard<'_, PendingHeadlessQuestionsInner> {
        self.inner
            .lock()
            .expect("PendingHeadlessQuestions mutex poisoned")
    }

    /// Register a question for `run_id`, deduping against any existing
    /// UNANSWERED entry for the same run id — this is what makes one
    /// question produce one message. Unlike
    /// [`super::PendingQuestions::register`]'s sequential ids, the id here
    /// is [`gate_id_for_run`]'s deterministic id, so the same run always
    /// registers under the same key. A run whose prior question was already
    /// answered gets a fresh `Created` entry (a new suspend-ask cycle for
    /// the same run), reusing the existing FIFO slot rather than growing
    /// `order` with a duplicate.
    pub fn register(
        &self,
        run_id: uuid::Uuid,
        payload: OperatorPayload,
        asked_at: DateTime<Utc>,
    ) -> HeadlessRegisterOutcome {
        let gate_id = gate_id_for_run(run_id);
        let mut inner = self.mutex_lock();

        if let Some(existing) = inner.by_id.get(&gate_id) {
            if !existing.answered {
                return HeadlessRegisterOutcome::AlreadyPending { gate_id };
            }
            inner.by_id.insert(
                gate_id.clone(),
                HeadlessQuestion::new(run_id, payload, asked_at),
            );
            return HeadlessRegisterOutcome::Created { gate_id };
        }

        if inner.by_id.len() >= Self::CAPACITY
            && let Some(oldest) = inner.order.pop_front()
        {
            inner.by_id.remove(&oldest);
        }

        inner.order.push_back(gate_id.clone());
        inner.by_id.insert(
            gate_id.clone(),
            HeadlessQuestion::new(run_id, payload, asked_at),
        );

        HeadlessRegisterOutcome::Created { gate_id }
    }

    /// Look up a question by gate id.
    #[must_use]
    pub fn get(&self, gate_id: &str) -> Option<HeadlessQuestion> {
        self.mutex_lock().by_id.get(gate_id).cloned()
    }

    /// Mark a question answered. No-op if the id is unknown.
    pub fn mark_answered(&self, gate_id: &str) {
        let mut inner = self.mutex_lock();
        if let Some(q) = inner.by_id.get_mut(gate_id) {
            q.answered = true;
        }
    }

    /// Current number of entries held (for tests).
    #[must_use]
    pub fn len(&self) -> usize {
        self.mutex_lock().by_id.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for PendingHeadlessQuestions {
    fn default() -> Self {
        Self::new()
    }
}

/// Build the operator-facing question for a suspended run.
///
/// Prefers a structured question the workflow itself stashed into
/// `entry.snapshot.metadata` under [`HEADLESS_QUESTION_METADATA_KEY`], when
/// present and deserializable as an [`OperatorPayload`]. Otherwise
/// synthesizes one from `entry.workflow_type` / `entry.reason` /
/// `entry.resume_at` with a single `resume` option.
///
/// Either way, `gate_id` is unconditionally overwritten with
/// [`gate_id_for_run`] and `digest` is unconditionally recomputed via
/// [`OperatorPayload::digest_of`] — a workflow-authored payload's own
/// `gate_id`/`digest` (if any) are not this run's gate identity and are not
/// guaranteed self-consistent, so both are always stamped fresh here rather
/// than trusted from the source.
#[must_use]
pub fn question_from_suspended(
    run_id: uuid::Uuid,
    entry: &engine_serve::suspend::SuspendedEntry,
) -> OperatorPayload {
    let mut payload = entry
        .snapshot
        .metadata
        .get(HEADLESS_QUESTION_METADATA_KEY)
        .and_then(|value| serde_json::from_value::<OperatorPayload>(value.clone()).ok())
        .unwrap_or_else(|| {
            let rendered_summary = format!(
                "{} suspended: {} (resume at {})",
                entry.workflow_type, entry.reason, entry.resume_at
            );
            OperatorPayload::new(
                gate_id_for_run(run_id),
                rendered_summary,
                vec![OperatorResponseOption::new("resume", "Resume")],
            )
        });

    payload.gate_id = gate_id_for_run(run_id);
    payload.digest = OperatorPayload::digest_of(&payload.rendered_summary, &payload.options);
    payload
}

/// The outcome of resolving a tap against the [`PendingHeadlessQuestions`]
/// registry — the pane-free analogue of [`super::QuestionVerdict`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadlessVerdict {
    /// The question exists, is unanswered, and `option_key` names one of
    /// its options.
    Accepted {
        run_id: uuid::Uuid,
        option_key: String,
    },
    /// The question exists but was already answered by an earlier tap.
    AlreadyAnswered,
    /// No pending question matches this gate id (never registered,
    /// evicted, or the option key named does not exist on this question).
    UnknownQuestion,
}

/// Resolve a tap's `(gate_id, option_key)` against `registry`. Pure: exactly
/// one lookup, and on `Accepted` this does **not** itself mark the question
/// answered — callers (task 3) call
/// [`PendingHeadlessQuestions::mark_answered`] only after the resume call
/// has actually been issued.
#[must_use]
pub fn resolve_headless_tap(
    gate_id: &str,
    option_key: &str,
    registry: &PendingHeadlessQuestions,
) -> HeadlessVerdict {
    let Some(question) = registry.get(gate_id) else {
        return HeadlessVerdict::UnknownQuestion;
    };

    if question.answered {
        return HeadlessVerdict::AlreadyAnswered;
    }

    let Some(option) = question
        .payload
        .options
        .iter()
        .find(|opt| opt.key == option_key)
    else {
        return HeadlessVerdict::UnknownQuestion;
    };

    HeadlessVerdict::Accepted {
        run_id: question.run_id,
        option_key: option.key.clone(),
    }
}

// ── Delivery shell (task 3): scan suspended runs, send once, decide resume ──

/// Injectable seam over `engine_serve::suspend::list_suspended`, so every
/// test in this module runs with no real engine process — mirrors
/// `BlockedEdgePoller`'s `CaptureFn` seam and
/// `stale_run_alarm::deliver_once`'s own injected `LiveStateStore` handle.
pub type SuspendedLister =
    Box<dyn Fn() -> Vec<(uuid::Uuid, engine_serve::suspend::SuspendedEntry)> + Send + Sync>;

/// One delivery tick: for each currently suspended run, build its question,
/// register it (skipping any run whose question is already pending — this
/// is what makes one question produce one message), validate the payload
/// against `limits`, insert it into `pending` so the existing
/// [`crate::serve::notify::PendingLookup`] chain can resolve a later tap,
/// and hand it to `transport`.
///
/// A validation error or a `send` error for one run is `warn!`-logged and
/// SKIPPED, never propagated — same skip-on-render-failure contract
/// [`super::super::notify::stale_run_alarm::deliver_once`] follows: one bad
/// run never blocks delivery for the rest of the scan.
///
/// Returns the number of questions actually sent (a successful `send`),
/// which may be less than the number of suspended runs scanned.
pub async fn deliver_once(
    lister: &SuspendedLister,
    registry: &PendingHeadlessQuestions,
    transport: &Arc<dyn OperatorTransport>,
    pending: &PendingPayloads,
    limits: &OperatorPayloadLimits,
    now: DateTime<Utc>,
) -> usize {
    let mut delivered = 0;

    for (run_id, entry) in lister() {
        let payload = question_from_suspended(run_id, &entry);

        let outcome = registry.register(run_id, payload.clone(), now);
        let gate_id = match outcome {
            HeadlessRegisterOutcome::AlreadyPending { .. } => {
                // A question is already outstanding for this run — sending
                // again would violate one-question-one-message.
                continue;
            }
            HeadlessRegisterOutcome::Created { gate_id } => gate_id,
        };

        let validated = match engine_core::operator::validate(payload, limits) {
            Ok(validated) => validated,
            Err(err) => {
                tracing::warn!(
                    target: "bastion::serve",
                    error = %err,
                    run_id = %run_id,
                    gate_id = %gate_id,
                    "headless question payload failed re-validation; skipping"
                );
                continue;
            }
        };

        pending.insert(validated.clone());

        match transport.send(&validated).await {
            Ok(_) => delivered += 1,
            Err(err) => {
                tracing::warn!(
                    target: "bastion::serve",
                    error = %err,
                    run_id = %run_id,
                    gate_id = %gate_id,
                    "headless question delivery failed; skipping"
                );
            }
        }
    }

    delivered
}

/// A handle to the background headless-question delivery loop
/// [`spawn_headless_question_loop`] spawned. Mirrors
/// `stale_run_alarm::AlarmDeliveryHandle`'s "hold or drop" shape exactly:
/// the caller may hold this to [`abort`](Self::abort) the loop, or drop it
/// — dropping does **not** stop the loop.
pub struct HeadlessQuestionHandle {
    task: actix_web::rt::task::JoinHandle<()>,
}

impl HeadlessQuestionHandle {
    /// Stop the background delivery loop.
    pub fn abort(&self) {
        self.task.abort();
    }
}

/// Spawn the background delivery loop: an `actix_web::rt::spawn`ed
/// `tokio::time::interval(interval)` loop whose body is one
/// [`deliver_once`] call per tick, evaluated at `chrono::Utc::now()`.
///
/// `transport` is captured by clone; `registry`/`pending` are captured by
/// `Arc` clone, so this and any other caller can keep sharing the same
/// underlying registries.
#[must_use]
pub fn spawn_headless_question_loop(
    lister: SuspendedLister,
    registry: Arc<PendingHeadlessQuestions>,
    transport: Arc<dyn OperatorTransport>,
    pending: Arc<PendingPayloads>,
    interval: Duration,
) -> HeadlessQuestionHandle {
    let task = actix_web::rt::spawn(async move {
        let limits = OperatorPayloadLimits::default();
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            let now = Utc::now();
            let _ = deliver_once(&lister, &registry, &transport, &pending, &limits, now).await;
        }
    });

    HeadlessQuestionHandle { task }
}

/// Given an already-resolved [`ResponseVerdict`], decide whether it maps to
/// a headless run that should now be resumed. PURE: does not itself mark
/// the entry answered (callers call [`mark_headless_answered`] only after
/// the resume call has actually been issued) and never resumes anything
/// itself.
///
/// Returns `Some(run_id)` only for an `Accepted` verdict whose `gate_id`
/// falls in this module's `hq-` id space AND resolves to an unanswered
/// headless entry via [`resolve_headless_tap`]. Every other case —
/// `StaleDigest`, `UnknownGate`, a gate id outside the `hq-` space, an
/// already-answered entry, or an option key the payload does not offer —
/// returns `None`.
#[must_use]
pub fn headless_resume_for(
    verdict: &ResponseVerdict,
    registry: &PendingHeadlessQuestions,
) -> Option<uuid::Uuid> {
    let ResponseVerdict::Accepted {
        gate_id,
        option_key,
        ..
    } = verdict
    else {
        return None;
    };

    if !is_headless_gate_id(gate_id) {
        return None;
    }

    match resolve_headless_tap(gate_id, option_key, registry) {
        HeadlessVerdict::Accepted { run_id, .. } => Some(run_id),
        HeadlessVerdict::AlreadyAnswered | HeadlessVerdict::UnknownQuestion => None,
    }
}

/// Mark the headless question at `gate_id` answered. Thin wrapper over
/// [`PendingHeadlessQuestions::mark_answered`] so the boot sink (task 4)
/// marks a question answered only AFTER the resume call has actually been
/// issued — never before, and never as a side effect of
/// [`headless_resume_for`] itself.
pub fn mark_headless_answered(registry: &PendingHeadlessQuestions, gate_id: &str) {
    registry.mark_answered(gate_id);
}
