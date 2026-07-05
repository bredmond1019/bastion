//! Vendored C001–C014 error taxonomy for the bastion Console.
//!
//! This module copies the error shape from `claude-sdk-rs/src/core/error.rs`
//! without taking a crate dependency on that library. Later blocks (7C, 9A, 9B,
//! 10A) emit errors through this spine.

use std::fmt;
use thiserror::Error;

/// Numeric error codes for bastion Console operations.
///
/// Mirrors the `ErrorCode` enum from `claude-sdk-rs` (`C001`–`C014`).
/// Display formats as `C{:03}` (e.g. `C001`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    /// `C001`: Claude binary not found.
    BinaryNotFound = 1,
    /// `C002`: Session not found.
    SessionNotFound = 2,
    /// `C003`: Permission denied.
    PermissionDenied = 3,
    /// `C004`: MCP server error.
    McpError = 4,
    /// `C005`: Configuration error.
    ConfigError = 5,
    /// `C006`: Invalid input.
    InvalidInput = 6,
    /// `C007`: Operation timeout.
    Timeout = 7,
    /// `C008`: Serialization error.
    SerializationError = 8,
    /// `C009`: I/O error.
    IoError = 9,
    /// `C010`: Process execution error.
    ProcessError = 10,
    /// `C011`: Stream closed.
    StreamClosed = 11,
    /// `C012`: Not authenticated.
    NotAuthenticated = 12,
    /// `C013`: Rate limit exceeded.
    RateLimitExceeded = 13,
    /// `C014`: UTF-8 conversion error.
    Utf8Error = 14,
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "C{:03}", *self as u16)
    }
}

/// Bastion-side error enum mirroring the `claude-sdk-rs` `Error` taxonomy.
///
/// Each variant carries a `[C0xx]`-prefixed message. The recoverable set
/// (`Timeout`, `RateLimitExceeded`, `StreamClosed`, `Io`, `ProcessError`)
/// is the same as in the source.
#[derive(Error, Debug)]
pub enum ConsoleError {
    /// `[C001]` Claude Code not found in PATH.
    #[error("[{code}] Claude Code not found in PATH", code = ErrorCode::BinaryNotFound)]
    BinaryNotFound,

    /// `[C002]` Session not found.
    #[error("[{code}] Session {0} not found", code = ErrorCode::SessionNotFound)]
    SessionNotFound(String),

    /// `[C003]` Tool permission denied.
    #[error("[{code}] Tool permission denied: {0}", code = ErrorCode::PermissionDenied)]
    PermissionDenied(String),

    /// `[C004]` MCP server error.
    #[error("[{code}] MCP server error: {0}", code = ErrorCode::McpError)]
    McpError(String),

    /// `[C005]` Invalid configuration.
    #[error("[{code}] Invalid configuration: {0}", code = ErrorCode::ConfigError)]
    ConfigError(String),

    /// `[C006]` Invalid input.
    #[error("[{code}] Invalid input: {0}", code = ErrorCode::InvalidInput)]
    InvalidInput(String),

    /// `[C007]` Operation timed out.
    #[error("[{code}] Operation timed out after {0}s", code = ErrorCode::Timeout)]
    Timeout(u64),

    /// `[C008]` Serialization error.
    #[error("[{code}] Serialization error: {0}", code = ErrorCode::SerializationError)]
    SerializationError(String),

    /// `[C009]` I/O error.
    #[error("[{code}] IO error: {0}", code = ErrorCode::IoError)]
    Io(String),

    /// `[C010]` Process execution error.
    #[error("[{code}] Process error: {0}", code = ErrorCode::ProcessError)]
    ProcessError(String),

    /// `[C011]` Stream closed unexpectedly.
    #[error("[{code}] Stream closed unexpectedly", code = ErrorCode::StreamClosed)]
    StreamClosed,

    /// `[C012]` Claude CLI is not authenticated.
    #[error(
        "[{code}] Claude CLI is not authenticated. Run 'claude auth' to authenticate.",
        code = ErrorCode::NotAuthenticated
    )]
    NotAuthenticated,

    /// `[C013]` Rate limit exceeded.
    #[error(
        "[{code}] Rate limit exceeded. Please wait before retrying.",
        code = ErrorCode::RateLimitExceeded
    )]
    RateLimitExceeded,

    /// `[C014]` UTF-8 conversion error.
    #[error("[{code}] UTF-8 conversion error: {0}", code = ErrorCode::Utf8Error)]
    Utf8Error(String),
}

impl ConsoleError {
    /// Return the `ErrorCode` for this variant.
    pub fn code(&self) -> ErrorCode {
        match self {
            ConsoleError::BinaryNotFound => ErrorCode::BinaryNotFound,
            ConsoleError::SessionNotFound(_) => ErrorCode::SessionNotFound,
            ConsoleError::PermissionDenied(_) => ErrorCode::PermissionDenied,
            ConsoleError::McpError(_) => ErrorCode::McpError,
            ConsoleError::ConfigError(_) => ErrorCode::ConfigError,
            ConsoleError::InvalidInput(_) => ErrorCode::InvalidInput,
            ConsoleError::Timeout(_) => ErrorCode::Timeout,
            ConsoleError::SerializationError(_) => ErrorCode::SerializationError,
            ConsoleError::Io(_) => ErrorCode::IoError,
            ConsoleError::ProcessError(_) => ErrorCode::ProcessError,
            ConsoleError::StreamClosed => ErrorCode::StreamClosed,
            ConsoleError::NotAuthenticated => ErrorCode::NotAuthenticated,
            ConsoleError::RateLimitExceeded => ErrorCode::RateLimitExceeded,
            ConsoleError::Utf8Error(_) => ErrorCode::Utf8Error,
        }
    }

    /// Return `true` for errors that are potentially recoverable by retrying.
    ///
    /// Recoverable: `Timeout`, `RateLimitExceeded`, `StreamClosed`, `Io`, `ProcessError`.
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            ConsoleError::Timeout(_)
                | ConsoleError::RateLimitExceeded
                | ConsoleError::StreamClosed
                | ConsoleError::Io(_)
                | ConsoleError::ProcessError(_)
        )
    }
}

/// Pairs a `ConsoleError` with the originating command/operation name.
///
/// This is the top-level error wrapper used at the dispatch layer so an error
/// report always identifies *which* command failed and with which `C0xx` code.
#[derive(Debug)]
pub struct ErrorContext {
    /// The command or operation that failed (e.g. `"inspect"`, `"monitor"`).
    pub operation: String,
    /// The underlying structured error.
    pub error: ConsoleError,
}

impl ErrorContext {
    /// Construct an `ErrorContext` from an operation name and a `ConsoleError`.
    pub fn new(operation: impl Into<String>, error: ConsoleError) -> Self {
        Self {
            operation: operation.into(),
            error,
        }
    }

    /// Return the `ErrorCode` of the underlying error.
    pub fn code(&self) -> ErrorCode {
        self.error.code()
    }

    /// Return `true` when the underlying error is recoverable.
    pub fn is_recoverable(&self) -> bool {
        self.error.is_recoverable()
    }
}

impl fmt::Display for ErrorContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.code(), self.operation, self.error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- ErrorCode Display ---

    #[test]
    fn error_code_display_all_variants() {
        assert_eq!(ErrorCode::BinaryNotFound.to_string(), "C001");
        assert_eq!(ErrorCode::SessionNotFound.to_string(), "C002");
        assert_eq!(ErrorCode::PermissionDenied.to_string(), "C003");
        assert_eq!(ErrorCode::McpError.to_string(), "C004");
        assert_eq!(ErrorCode::ConfigError.to_string(), "C005");
        assert_eq!(ErrorCode::InvalidInput.to_string(), "C006");
        assert_eq!(ErrorCode::Timeout.to_string(), "C007");
        assert_eq!(ErrorCode::SerializationError.to_string(), "C008");
        assert_eq!(ErrorCode::IoError.to_string(), "C009");
        assert_eq!(ErrorCode::ProcessError.to_string(), "C010");
        assert_eq!(ErrorCode::StreamClosed.to_string(), "C011");
        assert_eq!(ErrorCode::NotAuthenticated.to_string(), "C012");
        assert_eq!(ErrorCode::RateLimitExceeded.to_string(), "C013");
        assert_eq!(ErrorCode::Utf8Error.to_string(), "C014");
    }

    #[test]
    fn error_code_numeric_values_sequential() {
        let codes: &[(ErrorCode, u16)] = &[
            (ErrorCode::BinaryNotFound, 1),
            (ErrorCode::SessionNotFound, 2),
            (ErrorCode::PermissionDenied, 3),
            (ErrorCode::McpError, 4),
            (ErrorCode::ConfigError, 5),
            (ErrorCode::InvalidInput, 6),
            (ErrorCode::Timeout, 7),
            (ErrorCode::SerializationError, 8),
            (ErrorCode::IoError, 9),
            (ErrorCode::ProcessError, 10),
            (ErrorCode::StreamClosed, 11),
            (ErrorCode::NotAuthenticated, 12),
            (ErrorCode::RateLimitExceeded, 13),
            (ErrorCode::Utf8Error, 14),
        ];
        for (code, expected) in codes {
            assert_eq!(*code as u16, *expected, "wrong numeric value for {code:?}");
        }
    }

    // --- ConsoleError::code() ---

    #[test]
    fn console_error_code_all_variants() {
        let cases: &[(ConsoleError, ErrorCode)] = &[
            (ConsoleError::BinaryNotFound, ErrorCode::BinaryNotFound),
            (
                ConsoleError::SessionNotFound("s".into()),
                ErrorCode::SessionNotFound,
            ),
            (
                ConsoleError::PermissionDenied("p".into()),
                ErrorCode::PermissionDenied,
            ),
            (ConsoleError::McpError("m".into()), ErrorCode::McpError),
            (
                ConsoleError::ConfigError("c".into()),
                ErrorCode::ConfigError,
            ),
            (
                ConsoleError::InvalidInput("i".into()),
                ErrorCode::InvalidInput,
            ),
            (ConsoleError::Timeout(30), ErrorCode::Timeout),
            (
                ConsoleError::SerializationError("json".into()),
                ErrorCode::SerializationError,
            ),
            (ConsoleError::Io("io".into()), ErrorCode::IoError),
            (
                ConsoleError::ProcessError("proc".into()),
                ErrorCode::ProcessError,
            ),
            (ConsoleError::StreamClosed, ErrorCode::StreamClosed),
            (ConsoleError::NotAuthenticated, ErrorCode::NotAuthenticated),
            (
                ConsoleError::RateLimitExceeded,
                ErrorCode::RateLimitExceeded,
            ),
            (ConsoleError::Utf8Error("utf8".into()), ErrorCode::Utf8Error),
        ];
        for (err, expected_code) in cases {
            assert_eq!(err.code(), *expected_code, "wrong code for {err:?}");
        }
    }

    // --- ConsoleError::is_recoverable() ---

    #[test]
    fn is_recoverable_true_for_recoverable_variants() {
        assert!(ConsoleError::Timeout(5).is_recoverable());
        assert!(ConsoleError::RateLimitExceeded.is_recoverable());
        assert!(ConsoleError::StreamClosed.is_recoverable());
        assert!(ConsoleError::Io("disk".into()).is_recoverable());
        assert!(ConsoleError::ProcessError("temp".into()).is_recoverable());
    }

    #[test]
    fn is_recoverable_false_for_non_recoverable_variants() {
        assert!(!ConsoleError::BinaryNotFound.is_recoverable());
        assert!(!ConsoleError::SessionNotFound("x".into()).is_recoverable());
        assert!(!ConsoleError::PermissionDenied("tool".into()).is_recoverable());
        assert!(!ConsoleError::McpError("mcp".into()).is_recoverable());
        assert!(!ConsoleError::ConfigError("cfg".into()).is_recoverable());
        assert!(!ConsoleError::InvalidInput("bad".into()).is_recoverable());
        assert!(!ConsoleError::SerializationError("json".into()).is_recoverable());
        assert!(!ConsoleError::NotAuthenticated.is_recoverable());
        assert!(!ConsoleError::Utf8Error("bad bytes".into()).is_recoverable());
    }

    // --- ConsoleError Display (messages include [Cxxx] prefix) ---

    #[test]
    fn console_error_display_includes_code() {
        assert!(ConsoleError::BinaryNotFound.to_string().contains("[C001]"));
        assert!(
            ConsoleError::SessionNotFound("abc".into())
                .to_string()
                .contains("[C002]")
        );
        assert!(ConsoleError::Timeout(10).to_string().contains("[C007]"));
        assert!(ConsoleError::Timeout(10).to_string().contains("10s"));
        assert!(
            ConsoleError::NotAuthenticated
                .to_string()
                .contains("[C012]")
        );
        assert!(
            ConsoleError::Utf8Error("x".into())
                .to_string()
                .contains("[C014]")
        );
    }

    // --- ErrorContext construction + accessor round-trip ---

    #[test]
    fn error_context_construction_and_accessors() {
        let ctx = ErrorContext::new("inspect", ConsoleError::Timeout(30));
        assert_eq!(ctx.operation, "inspect");
        assert_eq!(ctx.code(), ErrorCode::Timeout);
        assert!(ctx.is_recoverable());
    }

    #[test]
    fn error_context_non_recoverable() {
        let ctx = ErrorContext::new("brain", ConsoleError::BinaryNotFound);
        assert_eq!(ctx.operation, "brain");
        assert_eq!(ctx.code(), ErrorCode::BinaryNotFound);
        assert!(!ctx.is_recoverable());
    }

    #[test]
    fn error_context_display_includes_code_and_operation() {
        let ctx = ErrorContext::new("monitor", ConsoleError::ConfigError("bad".into()));
        let s = ctx.to_string();
        assert!(s.contains("C005"), "expected C005 in: {s}");
        assert!(s.contains("monitor"), "expected operation name in: {s}");
    }
}
