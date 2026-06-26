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
    /// Raise log verbosity to DEBUG level (default: INFO).
    ///
    /// Repeated use (-v -v) is accepted but has the same effect as a single -v;
    /// the underlying tracing filter moves from INFO to DEBUG on first use.
    #[arg(short = 'v', long, global = true)]
    pub verbose: bool,

    /// Emit structured JSON log lines to stderr instead of human-readable text.
    ///
    /// Useful for machine consumers, log aggregators, or piping bastion output
    /// into a JSON processor (e.g. `bastion --json-logs status | jq '.'`).
    #[arg(long, global = true)]
    pub json_logs: bool,

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
    /// Query the OKF brain knowledge graph for structural relationships
    ///
    /// Builds a directed graph from the [[link]] corpus under --root (or the workspace
    /// resolved via --workspace / config default) and answers structural questions:
    /// which nodes depend on a given node (--dependents), what is transitively affected
    /// if it changes (--blast-radius), or what does it transitively reference (--lineage).
    ///
    /// Output is one greppable line per result: `<relation>: <id>\t<path>`.
    ///
    /// Exactly one of --dependents, --blast-radius, or --lineage must be supplied.
    #[command(group(
        clap::ArgGroup::new("query-mode")
            .required(true)
            .args(["dependents", "blast_radius", "lineage"])
    ))]
    Brain {
        /// Show nodes that directly reference <NODE_ID> via [[link]] (incoming edges)
        #[arg(long, value_name = "NODE_ID")]
        dependents: Option<String>,
        /// Show all nodes transitively affected by a change to <NODE_ID>
        #[arg(long, value_name = "NODE_ID")]
        blast_radius: Option<String>,
        /// Show all nodes that <NODE_ID> transitively references (forward reachability)
        #[arg(long, value_name = "NODE_ID")]
        lineage: Option<String>,
        /// Root directory of the OKF corpus to scan (explicit override; takes precedence
        /// over --workspace and the config default)
        #[arg(long)]
        root: Option<PathBuf>,
        /// Named workspace from the [workspaces] registry in the bastion config file
        /// (~/.config/bastion/config.toml).  Alias: --knowledge-dir.
        #[arg(long, visible_alias = "knowledge-dir", value_name = "NAME")]
        workspace: Option<String>,
    },

    /// Start the HTTP+WebSocket network face (Tailscale-reachable)
    ///
    /// Binds an actix-web server on the configured address (default 0.0.0.0:4317) and
    /// enforces mandatory bearer-token auth on protected routes.  Exposes:
    ///   GET  /health   — public liveness probe (no auth required)
    ///   GET  /ws       — WebSocket upgrade endpoint (bearer token required)
    ///
    /// The bearer token must be supplied via BASTION_SERVE_TOKEN (or --token).
    /// The bind address can be overridden via BASTION_SERVE_ADDR (or --addr).
    Serve {
        /// Bind address (overrides BASTION_SERVE_ADDR; default 0.0.0.0:4317)
        #[arg(long)]
        addr: Option<String>,
        /// Bearer token for protected routes (overrides BASTION_SERVE_TOKEN; mandatory)
        #[arg(long)]
        token: Option<String>,
    },

    /// Query the code-as-graph surface for symbol definitions, references, and dependents
    ///
    /// Builds a directed symbol graph from `.rs` source files under --root (or the workspace
    /// resolved via --workspace / config default) using tree-sitter extraction (deterministic,
    /// no LLM) and answers structural questions:
    /// which file defines a symbol (--def), what are its call/import sites (--refs),
    /// or which symbols directly call it (--dependents).
    ///
    /// Output is one greppable line per result:
    ///   def:       `def: <name>\t<path>:<line>`
    ///   refs:      `ref: <name>\t<path>:<line>`
    ///   dependents: `dependent: <name>\t<path>`
    ///
    /// Exactly one of --def, --refs, or --dependents must be supplied.
    /// Coverage: Rust (.rs) files only; other languages are skipped.
    #[command(group(
        clap::ArgGroup::new("code-query-mode")
            .required(true)
            .args(["def", "refs", "dependents"])
    ))]
    Code {
        /// Find the definition(s) of <SYMBOL> (file + line)
        #[arg(long, value_name = "SYMBOL")]
        def: Option<String>,
        /// Find all call sites and use imports of <SYMBOL>
        #[arg(long, value_name = "SYMBOL")]
        refs: Option<String>,
        /// Find symbols that directly call <SYMBOL> (direct predecessors in the code graph)
        #[arg(long, value_name = "SYMBOL")]
        dependents: Option<String>,
        /// Root directory of the Rust source tree to scan (explicit override; takes precedence
        /// over --workspace and the config default)
        #[arg(long)]
        root: Option<PathBuf>,
        /// Named workspace from the [workspaces] registry in the bastion config file.
        /// Alias: --knowledge-dir.
        #[arg(long, visible_alias = "knowledge-dir", value_name = "NAME")]
        workspace: Option<String>,
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

    // ── Brain subcommand ──────────────────────────────────────────────────────

    #[test]
    fn brain_dependents_parses() {
        let cli = Cli::try_parse_from(["bastion", "brain", "--dependents", "d20"]).unwrap();
        match cli.command {
            Some(Commands::Brain {
                dependents,
                blast_radius,
                lineage,
                root,
                workspace,
            }) => {
                assert_eq!(dependents, Some("d20".to_string()));
                assert!(blast_radius.is_none());
                assert!(lineage.is_none());
                // --root is now Option<PathBuf>; unset when not supplied
                assert!(root.is_none());
                assert!(workspace.is_none());
            }
            other => panic!("expected Brain, got {other:?}"),
        }
    }

    #[test]
    fn brain_blast_radius_parses() {
        let cli = Cli::try_parse_from(["bastion", "brain", "--blast-radius", "d20"]).unwrap();
        match cli.command {
            Some(Commands::Brain {
                dependents,
                blast_radius,
                lineage,
                root,
                workspace,
            }) => {
                assert!(dependents.is_none());
                assert_eq!(blast_radius, Some("d20".to_string()));
                assert!(lineage.is_none());
                assert!(root.is_none());
                assert!(workspace.is_none());
            }
            other => panic!("expected Brain, got {other:?}"),
        }
    }

    #[test]
    fn brain_lineage_parses() {
        let cli = Cli::try_parse_from(["bastion", "brain", "--lineage", "d3"]).unwrap();
        match cli.command {
            Some(Commands::Brain {
                dependents,
                blast_radius,
                lineage,
                root,
                workspace,
            }) => {
                assert!(dependents.is_none());
                assert!(blast_radius.is_none());
                assert_eq!(lineage, Some("d3".to_string()));
                assert!(root.is_none());
                assert!(workspace.is_none());
            }
            other => panic!("expected Brain, got {other:?}"),
        }
    }

    #[test]
    fn brain_root_flag_sets_some() {
        let cli = Cli::try_parse_from([
            "bastion",
            "brain",
            "--dependents",
            "d20",
            "--root",
            "/path/to/brain",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Brain {
                root, workspace, ..
            }) => {
                assert_eq!(root, Some(PathBuf::from("/path/to/brain")));
                assert!(workspace.is_none());
            }
            other => panic!("expected Brain, got {other:?}"),
        }
    }

    #[test]
    fn brain_workspace_flag_parses() {
        let cli = Cli::try_parse_from([
            "bastion",
            "brain",
            "--dependents",
            "d20",
            "--workspace",
            "client-a",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Brain {
                root, workspace, ..
            }) => {
                assert!(root.is_none());
                assert_eq!(workspace, Some("client-a".to_string()));
            }
            other => panic!("expected Brain, got {other:?}"),
        }
    }

    #[test]
    fn brain_knowledge_dir_alias_parses() {
        // --knowledge-dir is a documented alias for --workspace
        let cli = Cli::try_parse_from([
            "bastion",
            "brain",
            "--dependents",
            "d20",
            "--knowledge-dir",
            "my-notes",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Brain { workspace, .. }) => {
                assert_eq!(workspace, Some("my-notes".to_string()));
            }
            other => panic!("expected Brain, got {other:?}"),
        }
    }

    #[test]
    fn brain_root_and_workspace_both_accepted() {
        // clap allows both; resolver gives --root precedence
        let cli = Cli::try_parse_from([
            "bastion",
            "brain",
            "--dependents",
            "d20",
            "--root",
            "/explicit",
            "--workspace",
            "brain",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Brain {
                root, workspace, ..
            }) => {
                assert_eq!(root, Some(PathBuf::from("/explicit")));
                assert_eq!(workspace, Some("brain".to_string()));
            }
            other => panic!("expected Brain, got {other:?}"),
        }
    }

    #[test]
    fn brain_no_query_flag_fails() {
        // None of the mutually-exclusive query flags → parse should fail.
        assert!(Cli::try_parse_from(["bastion", "brain"]).is_err());
    }

    #[test]
    fn brain_two_query_flags_fails() {
        // Two mutually-exclusive flags → parse should fail.
        assert!(
            Cli::try_parse_from([
                "bastion",
                "brain",
                "--dependents",
                "d20",
                "--lineage",
                "d20"
            ])
            .is_err()
        );
    }

    // ── Code subcommand ───────────────────────────────────────────────────────

    #[test]
    fn code_def_parses() {
        let cli = Cli::try_parse_from(["bastion", "code", "--def", "alpha"]).unwrap();
        match cli.command {
            Some(Commands::Code {
                def,
                refs,
                dependents,
                root,
                workspace,
            }) => {
                assert_eq!(def, Some("alpha".to_string()));
                assert!(refs.is_none());
                assert!(dependents.is_none());
                assert!(root.is_none());
                assert!(workspace.is_none());
            }
            other => panic!("expected Code, got {other:?}"),
        }
    }

    #[test]
    fn code_refs_parses() {
        let cli = Cli::try_parse_from(["bastion", "code", "--refs", "alpha"]).unwrap();
        match cli.command {
            Some(Commands::Code {
                def,
                refs,
                dependents,
                ..
            }) => {
                assert!(def.is_none());
                assert_eq!(refs, Some("alpha".to_string()));
                assert!(dependents.is_none());
            }
            other => panic!("expected Code, got {other:?}"),
        }
    }

    #[test]
    fn code_dependents_parses() {
        let cli = Cli::try_parse_from(["bastion", "code", "--dependents", "render"]).unwrap();
        match cli.command {
            Some(Commands::Code {
                def,
                refs,
                dependents,
                ..
            }) => {
                assert!(def.is_none());
                assert!(refs.is_none());
                assert_eq!(dependents, Some("render".to_string()));
            }
            other => panic!("expected Code, got {other:?}"),
        }
    }

    #[test]
    fn code_root_flag_sets_some() {
        let cli =
            Cli::try_parse_from(["bastion", "code", "--def", "alpha", "--root", "/src"]).unwrap();
        match cli.command {
            Some(Commands::Code {
                root, workspace, ..
            }) => {
                assert_eq!(root, Some(PathBuf::from("/src")));
                assert!(workspace.is_none());
            }
            other => panic!("expected Code, got {other:?}"),
        }
    }

    #[test]
    fn code_workspace_flag_parses() {
        let cli = Cli::try_parse_from([
            "bastion",
            "code",
            "--def",
            "alpha",
            "--workspace",
            "bastion-src",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Code {
                root, workspace, ..
            }) => {
                assert!(root.is_none());
                assert_eq!(workspace, Some("bastion-src".to_string()));
            }
            other => panic!("expected Code, got {other:?}"),
        }
    }

    #[test]
    fn code_knowledge_dir_alias_parses() {
        let cli = Cli::try_parse_from([
            "bastion",
            "code",
            "--def",
            "alpha",
            "--knowledge-dir",
            "my-src",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Code { workspace, .. }) => {
                assert_eq!(workspace, Some("my-src".to_string()));
            }
            other => panic!("expected Code, got {other:?}"),
        }
    }

    #[test]
    fn code_no_query_flag_fails() {
        // ArgGroup requires exactly one of --def / --refs / --dependents.
        assert!(Cli::try_parse_from(["bastion", "code"]).is_err());
    }

    #[test]
    fn code_two_query_flags_fails() {
        assert!(
            Cli::try_parse_from(["bastion", "code", "--def", "alpha", "--refs", "alpha"]).is_err()
        );
    }

    // ── Global --verbose / --json-logs flags ─────────────────────────────────

    #[test]
    fn verbose_short_flag_sets_true() {
        let cli = Cli::try_parse_from(["bastion", "-v", "status"]).unwrap();
        assert!(cli.verbose, "--v should set verbose = true");
        assert!(!cli.json_logs, "json_logs should default to false");
    }

    #[test]
    fn verbose_long_flag_sets_true() {
        let cli = Cli::try_parse_from(["bastion", "--verbose", "status"]).unwrap();
        assert!(cli.verbose, "--verbose should set verbose = true");
    }

    #[test]
    fn json_logs_flag_sets_true() {
        let cli = Cli::try_parse_from(["bastion", "--json-logs", "status"]).unwrap();
        assert!(cli.json_logs, "--json-logs should set json_logs = true");
        assert!(!cli.verbose, "verbose should default to false");
    }

    #[test]
    fn verbose_and_json_logs_together() {
        let cli = Cli::try_parse_from(["bastion", "--verbose", "--json-logs", "sessions"]).unwrap();
        assert!(cli.verbose);
        assert!(cli.json_logs);
        assert!(matches!(cli.command, Some(Commands::Sessions)));
    }

    #[test]
    fn global_flags_default_to_false() {
        let cli = Cli::try_parse_from(["bastion", "status"]).unwrap();
        assert!(!cli.verbose, "verbose should default to false");
        assert!(!cli.json_logs, "json_logs should default to false");
    }

    #[test]
    fn global_flags_can_precede_subcommand() {
        // Flags before subcommand
        let cli = Cli::try_parse_from(["bastion", "--verbose", "--json-logs", "status"]).unwrap();
        assert!(cli.verbose);
        assert!(cli.json_logs);
        assert!(matches!(cli.command, Some(Commands::Status)));
    }

    #[test]
    fn global_flags_can_follow_subcommand() {
        // clap global flags can appear after the subcommand too
        let cli = Cli::try_parse_from(["bastion", "status", "--verbose"]).unwrap();
        assert!(cli.verbose);
        assert!(!cli.json_logs);
        assert!(matches!(cli.command, Some(Commands::Status)));
    }

    #[test]
    fn short_v_flag_no_subcommand() {
        // -v alone (no subcommand) should parse fine (command = None)
        let cli = Cli::try_parse_from(["bastion", "-v"]).unwrap();
        assert!(cli.verbose);
        assert!(cli.command.is_none());
    }

    // ── Serve subcommand ─────────────────────────────────────────────────────

    #[test]
    fn serve_parses_with_no_flags() {
        // Flags are optional at the CLI level (config can supply them).
        let cli = Cli::try_parse_from(["bastion", "serve"]).unwrap();
        match cli.command {
            Some(Commands::Serve { addr, token }) => {
                assert!(addr.is_none(), "addr should default to None");
                assert!(token.is_none(), "token should default to None");
            }
            other => panic!("expected Serve, got {other:?}"),
        }
    }

    #[test]
    fn serve_addr_flag_parses() {
        let cli = Cli::try_parse_from(["bastion", "serve", "--addr", "127.0.0.1:9999"]).unwrap();
        match cli.command {
            Some(Commands::Serve { addr, token }) => {
                assert_eq!(addr, Some("127.0.0.1:9999".to_string()));
                assert!(token.is_none());
            }
            other => panic!("expected Serve, got {other:?}"),
        }
    }

    #[test]
    fn serve_token_flag_parses() {
        let cli = Cli::try_parse_from(["bastion", "serve", "--token", "my-secret"]).unwrap();
        match cli.command {
            Some(Commands::Serve { addr, token }) => {
                assert!(addr.is_none());
                assert_eq!(token, Some("my-secret".to_string()));
            }
            other => panic!("expected Serve, got {other:?}"),
        }
    }

    #[test]
    fn serve_both_flags_parse() {
        let cli = Cli::try_parse_from([
            "bastion",
            "serve",
            "--addr",
            "0.0.0.0:4317",
            "--token",
            "secret-token",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Serve { addr, token }) => {
                assert_eq!(addr, Some("0.0.0.0:4317".to_string()));
                assert_eq!(token, Some("secret-token".to_string()));
            }
            other => panic!("expected Serve, got {other:?}"),
        }
    }
}
