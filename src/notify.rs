//! macOS desktop notifications via `osascript` (Part B — `monitor::alerts`'s
//! notification sink).
//!
//! Thin I/O shell over a subprocess spawn — not unit-tested itself (Rule 6);
//! manually smoke-tested (confirm a real notification banner appears on a
//! macOS session with notifications enabled, and that `cargo check --target
//! x86_64-unknown-linux-gnu` compiles the `osascript` call out entirely via
//! the `cfg!(target_os = "macos")` guard). See the task spec's Notes for the
//! recorded pass/fail. The pure escaping helper below IS exhaustively unit
//! tested, since it is the injection-safety boundary.

/// Escape `s` for safe interpolation into a double-quoted AppleScript string
/// literal used inside an `osascript -e` command.
///
/// Backslashes are escaped first (`\` → `\\`), then double quotes (`"` →
/// `\"`) — in that order, so escaping the quote doesn't get re-escaped by the
/// backslash step. This is a real injection-safety boundary, not cosmetic:
/// [`send_macos_notification`] shells out to `osascript`, and its `title`/
/// `message` arguments can carry operator-uncontrolled text (a workflow name,
/// a spec slug, a node name) that must not be able to break out of the
/// AppleScript string and inject additional script.
///
/// Pure function — exhaustively unit-tested below.
pub fn escape_for_applescript(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Send a macOS notification banner via `osascript -e 'display notification
/// ...'`.
///
/// No-op (`Ok(())`) on every non-macOS target — this is a compile-time
/// `cfg!(target_os = "macos")` guard, not a runtime `uname` check, since Rust
/// already knows the target triple at compile time. `title`/`message` are
/// escaped via [`escape_for_applescript`] before interpolation.
///
/// Best-effort by contract: every caller (`monitor::events::run_event_loop`,
/// `monitor::watch::run`) must log a returned `Err` via `tracing::warn!` and
/// never propagate it as a fatal error — a desktop notification failing (no
/// `osascript` binary, notifications disabled in System Settings, etc.) must
/// never kill the monitor loop, matching `costs::watch::run`'s posture on
/// transient DB failures.
pub fn send_macos_notification(title: &str, message: &str, sound: &str) -> anyhow::Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }

    let escaped_title = escape_for_applescript(title);
    let escaped_message = escape_for_applescript(message);
    let escaped_sound = escape_for_applescript(sound);

    let script = format!(
        "display notification \"{escaped_message}\" with title \"{escaped_title}\" sound name \"{escaped_sound}\""
    );

    let status = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .status()?;

    if !status.success() {
        anyhow::bail!("osascript exited with status {status}");
    }

    Ok(())
}

/// Fire-and-forget dispatch of [`send_macos_notification`]: spawns the
/// blocking `osascript` subprocess call onto tokio's blocking thread pool via
/// `spawn_blocking`, then detaches the resulting future onto `tokio::spawn`
/// instead of awaiting it.
///
/// This matters because [`send_macos_notification`] is called from inside
/// `monitor::events::run_event_loop`'s single `tokio::select!` task — that
/// task also owns keyboard-event handling and the next redraw. A tick that
/// fires several [`AlertEvent`](crate::monitor::alerts::AlertEvent)s (e.g.
/// many nodes failing at once) must not delay that task by the sum of every
/// `osascript` spawn's wall-clock time, or q/Esc/navigation become
/// unresponsive for the duration. Detaching means the caller returns
/// immediately; a failed spawn or a non-zero `osascript` exit is still
/// logged via `tracing::warn!`, matching every other notification failure's
/// best-effort posture in this crate.
pub fn dispatch_macos_notification(title: String, message: String, sound: &'static str) {
    tokio::spawn(async move {
        match tokio::task::spawn_blocking(move || send_macos_notification(&title, &message, sound))
            .await
        {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!(error = %e, "failed to send desktop notification"),
            Err(e) => tracing::warn!(error = %e, "desktop notification task panicked"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_plain_string_is_unchanged() {
        assert_eq!(escape_for_applescript("hello world"), "hello world");
    }

    #[test]
    fn escape_double_quote() {
        assert_eq!(escape_for_applescript(r#"say "hi""#), r#"say \"hi\""#);
    }

    #[test]
    fn escape_backslash() {
        assert_eq!(escape_for_applescript(r"C:\path"), r"C:\\path");
    }

    #[test]
    fn escape_backslash_then_quote_order_is_correct() {
        // Backslashes must be escaped first so the escaping of a following
        // quote isn't itself re-escaped by a later backslash pass.
        assert_eq!(escape_for_applescript(r#"\""#), r#"\\\""#);
    }

    #[test]
    fn escape_empty_string_is_unchanged() {
        assert_eq!(escape_for_applescript(""), "");
    }

    #[test]
    fn escape_unicode_is_preserved() {
        assert_eq!(escape_for_applescript("spec: ✅ 完了"), "spec: ✅ 完了");
    }

    #[test]
    fn escape_injection_attempt_is_neutralized() {
        // A workflow/node name attempting to break out of the AppleScript
        // string (to append a second `display notification` or `do shell
        // script` clause) must be rendered as inert literal text: every
        // double-quote in the input gets a `\` immediately before it in the
        // output, so it can never terminate the surrounding string literal.
        let malicious = "\" & (do shell script \"touch /tmp/pwned\") & \"";
        let escaped = escape_for_applescript(malicious);
        assert_eq!(
            escaped,
            "\\\" & (do shell script \\\"touch /tmp/pwned\\\") & \\\""
        );
    }

    #[test]
    fn escape_multiple_quotes_and_backslashes_mixed() {
        let input = r#"a"b\c"d\e"#;
        let escaped = escape_for_applescript(input);
        assert_eq!(escaped, r#"a\"b\\c\"d\\e"#);
    }
}
