//! Append-only, cross-process durable sink for [`BlockedEdgeRecord`]s.
//!
//! The sink is a JSON-Lines file: one record per line, opened in append mode
//! for every write. That gives us the two properties Task 2's acceptance
//! criteria require without a database:
//!
//! - **Survives a restart of the writing process** — the record lives on disk,
//!   not in an actor field, so a fresh [`BlockedEdgeSink`] pointed at the same
//!   path after a restart sees every record written before it.
//! - **Readable by a separate process** — any process (the `:8090` engine
//!   instance included) that can read the same path can call
//!   [`BlockedEdgeSink::read_all`] with no WebSocket, no direct RPC, and no
//!   coordination with the writer beyond agreeing on the path.
//!
//! Two edges on the same session are two lines — [`BlockedEdgeSink::append`]
//! never overwrites or coalesces, so a session that blocks, is answered, and
//! blocks again produces two distinct, independently timestamped records.

use std::fs::OpenOptions;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::detect::AgentState;

// ── Record ────────────────────────────────────────────────────────────────────

/// One durable Blocked-rising-edge event.
///
/// Carries everything the cross-process consumer (`engine-rs:EN.8.B`, not yet
/// built) needs to re-evaluate its own level predicate: which session, on
/// which host/instance, transitioned from what state into `Blocked`, and when.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockedEdgeRecord {
    /// The tmux session name that crossed into `Blocked`.
    pub session: String,
    /// Host/instance identity of the writing process — distinguishes which
    /// `bastion serve` instance (of potentially several across machines)
    /// observed the edge. Not necessarily a resolvable hostname; any stable
    /// identifier the caller supplies is valid.
    pub host: String,
    /// State observed on the previous poll tick, if any. `None` matches
    /// [`crate::serve::poll::should_emit_needs_input`]'s `prev == None` case
    /// (no prior observation — e.g. the session first appeared already
    /// blocked, post-seed).
    pub from: Option<AgentState>,
    /// State observed this tick — always `AgentState::Blocked` for a record
    /// that satisfied the rising-edge predicate, but not enforced here so the
    /// type stays a plain data carrier or record of the underlying (from, to)
    /// pair.
    pub to: AgentState,
    /// When this edge was observed, in UTC.
    pub observed_at: DateTime<Utc>,
}

impl BlockedEdgeRecord {
    /// Construct a new record. Pure — no I/O, no clock access (the caller
    /// supplies `observed_at` so tests can control it).
    pub fn new(
        session: impl Into<String>,
        host: impl Into<String>,
        from: Option<AgentState>,
        to: AgentState,
        observed_at: DateTime<Utc>,
    ) -> Self {
        Self {
            session: session.into(),
            host: host.into(),
            from,
            to,
            observed_at,
        }
    }
}

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum SinkError {
    #[error("failed to open blocked-edge sink at {path}: {source}")]
    Open {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to write blocked-edge record to {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to serialize blocked-edge record: {0}")]
    Serialize(#[source] serde_json::Error),
    #[error("failed to read blocked-edge sink at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("malformed blocked-edge record on {path} line {line}: {source}")]
    Malformed {
        path: PathBuf,
        line: usize,
        #[source]
        source: serde_json::Error,
    },
}

// ── Sink ──────────────────────────────────────────────────────────────────────

/// Append-only JSONL file sink for [`BlockedEdgeRecord`]s.
///
/// Cheap to construct (just a path) and cheap to clone — every write opens
/// the file fresh in append mode rather than holding a long-lived handle, so
/// there is no actor-field lifetime to worry about and multiple `BlockedEdgeSink`
/// values (e.g. one per poller tick, or one per process) can safely point at
/// the same path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedEdgeSink {
    path: PathBuf,
}

impl BlockedEdgeSink {
    /// Point a sink at `path`. Does not touch the filesystem — the file (and
    /// its parent directory) is created lazily on the first [`Self::append`].
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The path this sink reads and writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one record as a single JSON line. Creates the parent directory
    /// and the file itself if either is missing; never truncates or rewrites
    /// existing content.
    pub fn append(&self, record: &BlockedEdgeRecord) -> Result<(), SinkError> {
        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|source| SinkError::Open {
                path: self.path.clone(),
                source,
            })?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|source| SinkError::Open {
                path: self.path.clone(),
                source,
            })?;

        let line = serde_json::to_string(record).map_err(SinkError::Serialize)?;
        writeln!(file, "{line}").map_err(|source| SinkError::Write {
            path: self.path.clone(),
            source,
        })?;
        Ok(())
    }

    /// Read every record currently in the sink, oldest first.
    ///
    /// A missing file (nothing written yet) is not an error — returns an
    /// empty `Vec`, the same as an empty file. Callable from any process that
    /// can read `path`; no coordination with the writer beyond that.
    pub fn read_all(&self) -> Result<Vec<BlockedEdgeRecord>, SinkError> {
        let file = match std::fs::File::open(&self.path) {
            Ok(file) => file,
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(SinkError::Read {
                    path: self.path.clone(),
                    source,
                });
            }
        };

        let reader = BufReader::new(file);
        let mut records = Vec::new();
        for (idx, line) in reader.lines().enumerate() {
            let line = line.map_err(|source| SinkError::Read {
                path: self.path.clone(),
                source,
            })?;
            if line.trim().is_empty() {
                continue;
            }
            let record: BlockedEdgeRecord =
                serde_json::from_str(&line).map_err(|source| SinkError::Malformed {
                    path: self.path.clone(),
                    line: idx + 1,
                    source,
                })?;
            records.push(record);
        }
        Ok(records)
    }
}

/// Resolve the default sink path: `$XDG_STATE_HOME/bastion/blocked-edges.jsonl`,
/// falling back to `$HOME/.local/state/bastion/blocked-edges.jsonl`. Returns
/// `None` when neither is set.
///
/// Pure function — reads only the two supplied env values, no I/O — mirroring
/// `config::config_path`'s XDG resolution shape so the two dev-machine
/// `bastion serve` instances (`:8080` console, `:8090` engine) agree on one
/// path without either side hard-coding the other's config.
pub fn default_sink_path(xdg_state_home: Option<String>, home: Option<String>) -> Option<PathBuf> {
    if let Some(xdg) = xdg_state_home {
        Some(
            PathBuf::from(xdg)
                .join("bastion")
                .join("blocked-edges.jsonl"),
        )
    } else {
        home.map(|h| {
            PathBuf::from(h)
                .join(".local")
                .join("state")
                .join("bastion")
                .join("blocked-edges.jsonl")
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "bastion-blocked-edge-test-{}-{}-{}",
            std::process::id(),
            name,
            uuid::Uuid::new_v4()
        ))
    }

    // ── BlockedEdgeRecord::new ──────────────────────────────────────────────

    #[test]
    fn record_new_carries_all_fields() {
        let r = BlockedEdgeRecord::new(
            "sess-a",
            "mini-console",
            Some(AgentState::Working),
            AgentState::Blocked,
            ts(0),
        );
        assert_eq!(r.session, "sess-a");
        assert_eq!(r.host, "mini-console");
        assert_eq!(r.from, Some(AgentState::Working));
        assert_eq!(r.to, AgentState::Blocked);
        assert_eq!(r.observed_at, ts(0));
    }

    #[test]
    fn record_new_accepts_no_prior_observation() {
        let r = BlockedEdgeRecord::new("sess-a", "mini-console", None, AgentState::Blocked, ts(0));
        assert_eq!(r.from, None);
    }

    // ── append / read_all round trip ────────────────────────────────────────

    #[test]
    fn append_then_read_all_round_trips_one_record() {
        let path = temp_path("roundtrip");
        let sink = BlockedEdgeSink::new(&path);
        let record =
            BlockedEdgeRecord::new("sess-a", "mini-console", None, AgentState::Blocked, ts(0));

        sink.append(&record).expect("append should succeed");
        let read = sink.read_all().expect("read_all should succeed");

        assert_eq!(read, vec![record]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_all_on_missing_file_returns_empty_not_error() {
        let path = temp_path("missing");
        let sink = BlockedEdgeSink::new(&path);

        let read = sink.read_all().expect("missing file is not an error");
        assert!(read.is_empty());
    }

    #[test]
    fn append_creates_missing_parent_directory() {
        let path = temp_path("nested")
            .join("sub")
            .join("dir")
            .join("edges.jsonl");
        let sink = BlockedEdgeSink::new(&path);
        let record =
            BlockedEdgeRecord::new("sess-a", "mini-console", None, AgentState::Blocked, ts(0));

        sink.append(&record).expect("append should create parents");
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap().parent().unwrap());
    }

    // ── restart survival ────────────────────────────────────────────────────

    #[test]
    fn record_survives_a_fresh_sink_instance_at_the_same_path() {
        // A process restart means: same path on disk, brand-new in-memory
        // state. Simulate that by dropping the first `BlockedEdgeSink` and
        // constructing a second one pointed at the same path, rather than
        // reusing the same value.
        let path = temp_path("restart");
        {
            let writer = BlockedEdgeSink::new(&path);
            let record =
                BlockedEdgeRecord::new("sess-a", "mini-console", None, AgentState::Blocked, ts(0));
            writer.append(&record).expect("append should succeed");
        } // `writer` dropped here — no in-memory state carries over.

        let after_restart = BlockedEdgeSink::new(&path);
        let read = after_restart
            .read_all()
            .expect("read_all should see the pre-restart record");

        assert_eq!(read.len(), 1);
        assert_eq!(read[0].session, "sess-a");
        let _ = std::fs::remove_file(&path);
    }

    // ── readable from a separate process ────────────────────────────────────

    #[test]
    fn record_is_readable_from_an_independently_constructed_sink() {
        // Nothing here is shared between the writer and reader except the
        // path — the same relationship a second OS process would have.
        let path = temp_path("cross-process");
        let writer = BlockedEdgeSink::new(&path);
        writer
            .append(&BlockedEdgeRecord::new(
                "sess-a",
                "mini-console",
                None,
                AgentState::Blocked,
                ts(0),
            ))
            .expect("append should succeed");

        let reader = BlockedEdgeSink::new(path.clone());
        let read = reader.read_all().expect("read_all should succeed");

        assert_eq!(read.len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    // ── two edges on the same session are distinguished ─────────────────────

    #[test]
    fn two_block_events_on_the_same_session_produce_two_distinct_records() {
        let path = temp_path("re-block");
        let sink = BlockedEdgeSink::new(&path);

        let first =
            BlockedEdgeRecord::new("sess-a", "mini-console", None, AgentState::Blocked, ts(0));
        let second = BlockedEdgeRecord::new(
            "sess-a",
            "mini-console",
            Some(AgentState::Working),
            AgentState::Blocked,
            ts(60),
        );

        sink.append(&first).expect("first append should succeed");
        sink.append(&second).expect("second append should succeed");

        let read = sink.read_all().expect("read_all should succeed");
        assert_eq!(read.len(), 2);
        assert_eq!(read[0], first);
        assert_eq!(read[1], second);
        assert_ne!(read[0].observed_at, read[1].observed_at);
        let _ = std::fs::remove_file(&path);
    }

    // ── malformed line ───────────────────────────────────────────────────────

    #[test]
    fn read_all_reports_malformed_line_with_its_line_number() {
        let path = temp_path("malformed");
        std::fs::write(&path, "not json\n").expect("write should succeed");
        let sink = BlockedEdgeSink::new(&path);

        let err = sink.read_all().expect_err("malformed line should error");
        match err {
            SinkError::Malformed { line, .. } => assert_eq!(line, 1),
            other => panic!("expected Malformed, got {other:?}"),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_all_skips_blank_lines() {
        let path = temp_path("blank-lines");
        let sink = BlockedEdgeSink::new(&path);
        sink.append(&BlockedEdgeRecord::new(
            "sess-a",
            "mini-console",
            None,
            AgentState::Blocked,
            ts(0),
        ))
        .expect("append should succeed");
        // Inject a trailing blank line directly.
        let mut file = OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open for append should succeed");
        writeln!(file).expect("write blank line should succeed");

        let read = sink
            .read_all()
            .expect("read_all should tolerate blank lines");
        assert_eq!(read.len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    // ── serialization shape ──────────────────────────────────────────────────

    #[test]
    fn append_writes_exactly_one_line_per_record() {
        let path = temp_path("one-line");
        let sink = BlockedEdgeSink::new(&path);
        sink.append(&BlockedEdgeRecord::new(
            "sess-a",
            "mini-console",
            None,
            AgentState::Blocked,
            ts(0),
        ))
        .expect("append should succeed");
        sink.append(&BlockedEdgeRecord::new(
            "sess-b",
            "mini-console",
            None,
            AgentState::Blocked,
            ts(1),
        ))
        .expect("append should succeed");

        let contents = std::fs::read_to_string(&path).expect("read_to_string should succeed");
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2);
        for line in lines {
            assert!(serde_json::from_str::<BlockedEdgeRecord>(line).is_ok());
        }
        let _ = std::fs::remove_file(&path);
    }

    // ── path() accessor ──────────────────────────────────────────────────────

    #[test]
    fn path_returns_the_constructed_path() {
        let path = temp_path("accessor");
        let sink = BlockedEdgeSink::new(&path);
        assert_eq!(sink.path(), path.as_path());
    }

    // ── default_sink_path ────────────────────────────────────────────────────

    #[test]
    fn default_sink_path_prefers_xdg_state_home() {
        let path = default_sink_path(Some("/xdg/state".to_string()), Some("/home/u".to_string()));
        assert_eq!(
            path,
            Some(PathBuf::from("/xdg/state/bastion/blocked-edges.jsonl"))
        );
    }

    #[test]
    fn default_sink_path_falls_back_to_home() {
        let path = default_sink_path(None, Some("/home/u".to_string()));
        assert_eq!(
            path,
            Some(PathBuf::from(
                "/home/u/.local/state/bastion/blocked-edges.jsonl"
            ))
        );
    }

    #[test]
    fn default_sink_path_is_none_with_neither_env_var() {
        assert_eq!(default_sink_path(None, None), None);
    }
}
