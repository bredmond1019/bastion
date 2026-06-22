use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "bastion",
    version,
    about = "Control panel for the agentic engineering stack",
    long_about = "\
bastion is a personal Rust CLI that serves as the unified control panel for the agentic \
engineering stack. It exposes two surfaces:\n\n\
  Workflow observability — live and post-mortem views of workflow execution backed by the \
Python orchestrator's PostgreSQL database (`monitor`, `inspect`, `costs`, `run`, `status`).\n\n\
  Process / session control — tmux session management without any database dependency \
(`sessions`, `attach`, `new`, `kill`, `send`, `capture`, `ask`).\n\n\
Configuration is read from env vars (highest precedence), then from \
~/.config/bastion/config.toml (or $XDG_CONFIG_HOME/bastion/config.toml), then from \
built-in defaults.",
    after_help = "\
Examples:\n  \
bastion sessions                          # list tmux sessions\n  \
bastion monitor                           # live workflow graph (all active runs)\n  \
bastion monitor --workflow-id abc123      # live graph for one run\n  \
bastion costs --last 7d                   # LLM spend for the last 7 days\n  \
bastion validate ./docs                   # validate markdown/MDX content\n  \
bastion run my-workflow --args '{\"k\":1}'  # trigger a workflow via FastAPI\n  \
bastion man                               # print the roff man page to stdout\n  \
bastion man --out /tmp/man               # write bastion.1 to a directory"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Launch the interactive session dashboard (default when no subcommand is given)
    Tui,
    /// Live TUI graph monitor for workflow execution (reads orchestrator PostgreSQL)
    Monitor {
        /// Filter to a specific workflow ID (shows all active runs if omitted)
        #[arg(short, long)]
        workflow_id: Option<String>,
    },
    /// Static post-mortem graph view for a completed run
    Inspect {
        /// Run ID to inspect
        run_id: String,
    },
    /// Validate markdown/MDX content files for front-matter, link integrity, and lint rules
    Validate {
        /// Path to content directory (defaults to current dir)
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Summarise LLM token spend aggregated from the orchestrator database
    Costs {
        /// Time window — "7d", "30d", or "all"
        #[arg(long, default_value = "7d")]
        last: String,
    },
    /// Trigger a workflow run via the FastAPI orchestrator API
    Run {
        /// Workflow name to trigger
        workflow: String,
        /// JSON args to pass to the workflow (e.g. '{"key": "value"}')
        #[arg(long)]
        args: Option<String>,
        /// Drop into `bastion monitor` after triggering
        #[arg(long)]
        monitor: bool,
    },
    /// Quick stack health check — prints orchestrator API + DB reachability (non-TUI)
    Status,
    /// List all tmux sessions with their last line of pane output
    Sessions,
    /// Attach your terminal to an existing tmux session
    Attach {
        /// Name of the session to attach to
        session: String,
    },
    /// Create a new detached tmux session
    New {
        /// Name of the session to create
        session: String,
        /// Working directory for the new session
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Kill (remove) a tmux session
    Kill {
        /// Name of the session to kill
        session: String,
    },
    /// Send a command to a tmux session without attaching
    Send {
        /// Name of the target session
        session: String,
        /// Command to send (multi-word; no quoting needed)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        cmd: Vec<String>,
    },
    /// Print recent pane output for a tmux session without attaching
    Capture {
        /// Name of the session to capture output from
        session: String,
        /// Maximum number of lines to print (prints all if omitted)
        #[arg(long)]
        lines: Option<usize>,
    },
    /// Run a single Claude Code turn against an interactive tmux session and collect its output
    Ask {
        /// tmux session name to use (created if absent)
        #[arg(long)]
        session: String,
        /// Path to a file holding the full prompt + output-format instructions
        #[arg(long)]
        prompt_file: PathBuf,
        /// Path Claude should write the answer to; bastion waits for it
        #[arg(long)]
        out: PathBuf,
        /// Working dir if the session must be created (must be Claude-trusted)
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Seconds to wait for completion
        #[arg(long, default_value = "180")]
        timeout: u64,
        /// Command used to start Claude if the session is cold
        #[arg(long, default_value = "claude --permission-mode bypassPermissions")]
        launch_cmd: String,
    },
    /// Generate a roff man page for bastion
    #[command(hide = true)]
    Man {
        /// Write bastion.1 (and one page per subcommand) into this directory instead of
        /// printing to stdout
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[test]
    fn clap_debug_assert_passes() {
        Cli::command().debug_assert();
    }

    #[test]
    fn long_help_contains_examples_block() {
        let help = Cli::command().render_long_help().to_string();
        assert!(
            help.contains("bastion sessions"),
            "long help should include sessions example"
        );
        assert!(
            help.contains("bastion monitor"),
            "long help should include monitor example"
        );
        assert!(
            help.contains("bastion costs"),
            "long help should include costs example"
        );
    }

    #[test]
    fn long_help_mentions_both_surfaces() {
        let help = Cli::command().render_long_help().to_string();
        assert!(
            help.contains("observability") || help.contains("workflow"),
            "long help should mention workflow observability surface"
        );
        assert!(
            help.contains("session") || help.contains("tmux"),
            "long help should mention session control surface"
        );
    }

    #[test]
    fn man_subcommand_parses() {
        let cli = Cli::try_parse_from(["bastion", "man"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Man { out: None })));
    }

    #[test]
    fn man_out_flag_parses() {
        let cli = Cli::try_parse_from(["bastion", "man", "--out", "/tmp/man"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Man { out: Some(_) })));
        if let Some(Commands::Man { out: Some(p) }) = cli.command {
            assert_eq!(p, PathBuf::from("/tmp/man"));
        }
    }

    #[test]
    fn bare_bastion_parses_to_none() {
        let cli = Cli::try_parse_from(["bastion"]).unwrap();
        assert!(cli.command.is_none());
    }

    #[test]
    fn tui_subcommand_parses() {
        let cli = Cli::try_parse_from(["bastion", "tui"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Tui)));
    }

    #[test]
    fn existing_verb_still_parses() {
        let cli = Cli::try_parse_from(["bastion", "sessions"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Sessions)));
    }

    #[test]
    fn ask_required_flags_parse() {
        let cli = Cli::try_parse_from([
            "bastion",
            "ask",
            "--session",
            "my-session",
            "--prompt-file",
            "/tmp/prompt.txt",
            "--out",
            "/tmp/answer.txt",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Ask {
                session,
                prompt_file,
                out,
                dir,
                timeout,
                launch_cmd,
            }) => {
                assert_eq!(session, "my-session");
                assert_eq!(prompt_file, std::path::PathBuf::from("/tmp/prompt.txt"));
                assert_eq!(out, std::path::PathBuf::from("/tmp/answer.txt"));
                assert!(dir.is_none());
                assert_eq!(timeout, 180, "default timeout should be 180");
                assert_eq!(
                    launch_cmd, "claude --permission-mode bypassPermissions",
                    "default launch-cmd mismatch"
                );
            }
            other => panic!("expected Ask, got {other:?}"),
        }
    }

    #[test]
    fn ask_all_flags_parse() {
        let cli = Cli::try_parse_from([
            "bastion",
            "ask",
            "--session",
            "work",
            "--prompt-file",
            "/home/user/p.txt",
            "--out",
            "/home/user/out.json",
            "--dir",
            "/home/user/project",
            "--timeout",
            "60",
            "--launch-cmd",
            "claude --debug",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Ask {
                session,
                prompt_file,
                out,
                dir,
                timeout,
                launch_cmd,
            }) => {
                assert_eq!(session, "work");
                assert_eq!(prompt_file, std::path::PathBuf::from("/home/user/p.txt"));
                assert_eq!(out, std::path::PathBuf::from("/home/user/out.json"));
                assert_eq!(dir, Some(std::path::PathBuf::from("/home/user/project")));
                assert_eq!(timeout, 60);
                assert_eq!(launch_cmd, "claude --debug");
            }
            other => panic!("expected Ask, got {other:?}"),
        }
    }

    #[test]
    fn ask_missing_required_flags_fails() {
        // --session missing → parse should fail
        assert!(
            Cli::try_parse_from(["bastion", "ask", "--prompt-file", "/p", "--out", "/o"]).is_err()
        );
        // --prompt-file missing → parse should fail
        assert!(Cli::try_parse_from(["bastion", "ask", "--session", "s", "--out", "/o"]).is_err());
        // --out missing → parse should fail
        assert!(
            Cli::try_parse_from(["bastion", "ask", "--session", "s", "--prompt-file", "/p"])
                .is_err()
        );
    }
}
