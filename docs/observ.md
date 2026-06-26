---
type: Reference
title: Observability Module
description: Structured error taxonomy (C001-C014), command event tracing, and logging initialization for bastion.
doc_id: observ
layer: [console]
project: bastion
status: active
keywords: [observability, tracing, errors, ErrorCode, ConsoleError, CommandEvent]
related: [config]
---

# Observability (`src/observ/`)

The `observ` module is the structured observability spine for bastion. It provides:

- **C001-C014 error taxonomy** — vendored from `claude-sdk-rs`, kept as a self-contained copy with no crate dependency.
- **Command event records** — pure builder/serializer for lifecycle events (start, success, error).
- **Thin tracing helpers** — emit structured events via `tracing` macros.
- **Logging initialization** — install the global `tracing-subscriber` once at startup.

All pure logic (record construction, JSON serialization, error Display) is tested exhaustively without I/O. The only thin I/O shells are `emit_start`, `emit_outcome` (call `tracing` macros), and `init_tracing` (installs the global subscriber).

---

## Error taxonomy (`src/observ/errors.rs`)

### `ErrorCode`

Numeric codes `C001`-`C014`. `Display` formats as `C{:03}` (e.g. `C001`).

| Variant | Code | Meaning |
|---|---|---|
| `BinaryNotFound` | C001 | Claude Code binary not found in PATH |
| `SessionNotFound` | C002 | Session not found |
| `PermissionDenied` | C003 | Tool permission denied |
| `McpError` | C004 | MCP server error |
| `ConfigError` | C005 | Invalid configuration |
| `InvalidInput` | C006 | Invalid input |
| `Timeout` | C007 | Operation timeout |
| `SerializationError` | C008 | Serialization failure |
| `IoError` | C009 | I/O error |
| `ProcessError` | C010 | Process execution error |
| `StreamClosed` | C011 | Stream closed unexpectedly |
| `NotAuthenticated` | C012 | Not authenticated |
| `RateLimitExceeded` | C013 | Rate limit exceeded |
| `Utf8Error` | C014 | UTF-8 conversion error |

### `ConsoleError`

`thiserror`-derived enum. Each variant's `Display` includes the `[Cxxx]` prefix so a single `to_string()` gives a fully structured diagnostic line. Variants mirror `ErrorCode` one-to-one.

**Recoverable variants** (same set as `claude-sdk-rs`): `Timeout`, `RateLimitExceeded`, `StreamClosed`, `Io`, `ProcessError`.

### `ErrorContext`

Wraps a `ConsoleError` with an `operation: String` label for call-site context.

```rust
pub struct ErrorContext {
    pub code: ErrorCode,
    pub operation: String,
    pub error: ConsoleError,
}
```

`Display` emits: `[Cxxx] <operation>: <error message>`.

---

## Command events (`src/observ/mod.rs`)

### `EventPhase`

```rust
pub enum EventPhase { Start, Success, Error }
```

Serializes to lowercase JSON strings (`"start"`, `"success"`, `"error"`).

### `CommandEvent`

Pure record for a single command lifecycle event. Construction and JSON serialization involve no I/O.

```rust
pub struct CommandEvent {
    pub command: String,
    pub phase: EventPhase,
    pub duration_ms: Option<u64>,  // None for Start events
    pub error_code: Option<String>, // Some("C0xx") for Error events only
}
```

**Constructors** (all pure):

| Method | Phase | `duration_ms` | `error_code` |
|---|---|---|---|
| `CommandEvent::start(command)` | Start | None | None |
| `CommandEvent::success(command, duration_ms)` | Success | Some | None |
| `CommandEvent::error(command, duration_ms, code)` | Error | Some | Some |

**`to_json(&self) -> String`** — serialize to a JSON line (pure; uses `serde_json`).

### `emit_start(command: &str) -> CommandEvent`

Thin shell: builds a `Start` event and emits `tracing::info!`. Returns the record.

### `emit_outcome(command, duration_ms, error_code: Option<&str>) -> CommandEvent`

Thin shell: builds a `Success` or `Error` event, emits `tracing::info!` or `tracing::error!`, and returns the record. Pass `None` for success, `Some("C0xx")` for an error.

### `init_tracing(verbose: bool, json_logs: bool)`

Installs the process-global `tracing-subscriber`. Call exactly once at startup (panics on repeated calls).

- `verbose = true` → `DEBUG` level; `false` → `INFO` level.
- `json_logs = true` → JSON lines on stderr; `false` → human-readable text.
- Honours `RUST_LOG` when set (via `EnvFilter`).

---

## Dispatch instrumentation (`src/main.rs`)

Every subcommand dispatch is wrapped with start/outcome events:

1. `emit_start(cmd_name)` is called before the subcommand runs.
2. On success, `emit_outcome(cmd_name, elapsed_ms, None)` is called.
3. On error, `classify_error(&err)` maps the `anyhow::Error` to a `C0xx` code, and `emit_outcome(cmd_name, elapsed_ms, Some(code))` is called.

### `classify_error(err: &anyhow::Error) -> String`

Pure function. Resolution order:

1. Downcast to `ConsoleError` → use its `ErrorCode`.
2. Downcast to `std::io::Error` → `C009` (`IoError`).
3. Keyword scan of the error message string:
   - `"timeout"` → `C007`
   - `"permission"` → `C003`
   - `"not found"` / `"no such"` → `C002`
   - `"utf"` / `"utf-8"` → `C014`
4. Default → `C006` (`InvalidInput`).
