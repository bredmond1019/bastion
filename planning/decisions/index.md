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

<!-- Add a row per decision as they are made. Record new ones with /log-decision-style atomic
     files (D2, D3, …). -->
