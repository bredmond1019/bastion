---
type: Index
title: bastion Decisions Registry
description: Index of atomic, append-only architectural decision records for bastion.
---

# Decisions Registry

Architectural decision records (ADRs) for bastion. Each decision is **one atomic
file**, append-only — never edit a settled decision; supersede it with a new one and link back.

## Decisions

- [D1: Initial OKF Scaffold](./D1-initial-okf.md) — Project initialized on the standard OKF
  documentation structure.
- [D2: Observability Consumer Contract](./D2-observability-consumer-contract.md) — bastion is a
  read-only consumer of orchestrator execution state; the live monitor is gated on orchestrator
  D28 (incremental node-level persistence).
- [D3: Pin the Data Contract](./D3-pin-data-contract.md) — bastion pins v1.0.0 of the
  orchestrator-owned data contract; Hybrid read path; two sources joined by node class name.
  Orchestrator D30 / brain D20.
- [D4: Session Management Surface](./D4-session-management-surface.md) — bastion absorbs tmux
  session management as modules in the binary (the dropped standalone "brain" idea); a second,
  ungated process-control surface alongside workflow observability. Brain D21.
- [D5: Session verbs are synchronous](./D5-sessions-synchronous.md) — the `sessions/` surface is
  plain sync Rust (tmux shell-outs are blocking `std::process::Command`); session verbs are not
  async and add no tokio coupling. Builds on D4.
- [D6: Skip malformed tmux output lines](./D6-malformed-tmux-line-skip.md) — when parsing
  `list-sessions` output, a malformed line is skipped with a stderr warning rather than aborting
  the listing; partial system state beats none. Builds on D4.

<!-- Add a row per decision as they are made. Record new ones with /log-decision-style atomic
     files (D2, D3, …). -->
