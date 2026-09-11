# CLAUDE.md — bastion

@AGENTS.md

The file above carries everything that is true for any agent working in this repo: what bastion is,
the orientation pointers, the standing rules, build/test/run, the environment, the directory map,
the SDLC pipeline notes, the response style and the stopping rule. **Read it as part of these
instructions** — Claude Code loads it automatically through the `@` import.

Only Claude-specific content belongs below.

## Fleet & Core Skills

The harness carries specialized skills in `.claude/skills/` (and `.agents/skills/`). Always consult
the corresponding skill before executing high-stakes fleet operations:

| Skill | Primary Focus | When to consult |
|---|---|---|
| **`brain-graph`** | `bastion brain` / `bastion code` verb surface — flags, exactly-one-of rule, output grammar | BEFORE running either verb by hand or scripting them |
| **`check-blast-radius`** | Which instrument (`bastion brain`, `bastion code`, `mev`'s `related:` graph) answers "what breaks" | BEFORE renaming/deleting a doc, doc_id, or public Rust symbol; before saying "nothing references this" |
| **`commit-in-this-fleet`** | Safe git operations across multi-repo & vault symlinks | BEFORE any `git add`, `commit`, `stash`, `reset`, or `mv` |
| **`derive-state-safely`** | Authored vs derived state and writer execution | BEFORE running `mev emit-state --write`, `set-block-status`, or other state writers |
| **`edit-state-json`** | Canonical `planning/state.json` schema & graph edges | BEFORE hand-editing `state.json` or authoring `depends_on`/`carryover` |
| **`fleet-push-discipline`** | Dependency-ordered fleet push & CI-red triage | BEFORE `scripts/sync/git_push.sh`, and never `git push` this repo directly |
| **`notify-operator`** | Operator alerting discipline via `bastion notify` | BEFORE sending notifications or deciding a lane is blocked |
| **`pick-the-next-block`** | The three `mev` query verbs and their three different meanings of "ready" | When choosing the next block to work, or claiming one is startable |
| **`ping-agent`** | Cross-lane messaging envelopes & registry protocol | BEFORE sending or triaging cross-lane messages |
| **`record-a-bail`** | Classifying a block/task bail for `bails[]` — artifact vs. check, self vs. foreign | Whenever a block or task bails and a `bails[]` entry must be written |
| **`report-to-the-operator`** | Concise operator reporting ceiling & format | When drafting chat replies, turn outputs, and run reports |
| **`run-the-gates`** | Fleet validation suite & gate diagnostics | BEFORE running `validate-brain` or `harness.json` checks |
| **`stamp-workflow-run-id`** | Recording a `Workflow` tool run id into an SDLC engine's state file | Immediately after every `Workflow({name:'sdlc-task'\|'sdlc-flow', ...})` call |
| **`stop-or-continue`** | Session restart vs continuation correctness criteria | When an underlying binary/engine changes; never restart for token budget |
| **`write-carryover-entry`** | Whether a finding belongs in `carryover[]` at all, and how to write a `clears_when` that fires | BEFORE adding any `carryover[]` entry, at every `/handoff`/`/wrap-up`/`/log-work` |
| **`write-okf-markdown`** | OKF YAML frontmatter & index.md row maintenance | BEFORE creating or editing any `.md` under `docs/` or `planning/` |
| **`write-repo-doc`** | Reader-first internal documentation standards | BEFORE writing or restructuring docs under `docs/` or guides |
