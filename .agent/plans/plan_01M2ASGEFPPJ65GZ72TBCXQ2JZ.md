# Historical planning session: agent-first startup ownership

User-authorized investigation, isolated API experiments with subagents, planning-
workflow reviews and conversion to delivery Beads. No production Rust or dependency
change, commit or publication was performed. Concurrent batter-fbd documentation
and unrelated tracker updates were preserved.

Outcome: epic batter-lp2 owns final scope. Its four self-contained P1 tasks are
batter-lp2.1 (constrained registration), batter-lp2.2 (startup-owned Unix signals),
batter-lp2.3 (slot-owned native SQLx pools), and batter-lp2.4 (real consumer adoption).
Blocking order is .1 -> .2; .1/.2/.3 -> .4. Only .1 and .3 are ready. Exact saved
description/acceptance/parent/priority readback and zero blocking cycles passed.
This file is historical session evidence, not a duplicate delivery backlog.

Research baseline: 809d5d5b913d26497232f13682b334e1c4aec806. Documentation-only
concurrent HEAD: 39c1b6e3c1c75f808becb5a5e7c33f58001a2ee4. The reproducibility
record and hashed source archive are in
docs/evidence/agent-startup-apis-2026-09-12/README.md. Experiments used Linux,
Rust 1.98.1 and a private PostgreSQL 18.6 cluster, which was stopped after use.
No macOS/minimum-toolchain or complete production live acceptance is inferred.

Four sequential plan review rounds reached stable architecture. Standalone
checks covered every task; D required an expanded scenario/evaluation matrix and
then passed a fresh recheck. Six focused delivery-graph passes and tracker export
readback confirmed concrete outcomes, dependencies, readiness and coverage.

The first Jig work-check run executed passing underlying checks but rejected all
receipts after the tracker export changed during the read-only parallel layer.
Those receipts are not counted as passing gates. Final validation is recorded in
the append-only Jig receipts and the work-finish resolution after tracker writes
settle. Prototype results do not close implementation acceptance.
