# Fresh consumer exercise, 2026-09-12

A context-free coding agent implemented `consumer.rs` from public Batter/native
API guidance and the fixed callback interface in `contract.rs`. It did not receive
implementation plans, library tests, previous review findings or the held-out
oracles. It read the root and package READMEs, usage guide and public rustdoc.

The first assignment requested a finite resource command and inert native worker
registration, with the public function signatures fixed. The coordinator wrote
`integration.rs` before delegation. The first implementation compiled without
correction and passed all five cases: concrete work plus cleanup failure, owner
loss with downward cancellation, harmless borrowed-wait cancellation, cleanup
after panic, and native registration/duplicate rejection before process start.

Before giving the second assignment, the coordinator wrote `modification.rs`.
The same agent then added an optional absolute total budget, preserving the first
interfaces. Its first implementation passed the two new checks and all five
original checks: insufficient total is rejected before acquisition, and work
exhaustion leaves time for successful finalization without cancelling the parent.
The agent reported no public-guidance ambiguity. Neither exercise required an
implementation repair. The coordinator corrected its own draft oracle's public
method names before the first compilation; that was not an agent repair.

These files preserve the actual generated consumer, supplied interface and
independent checks. The consumer was first tested outside the repositories and
then this archived package was compiled and executed again. Reproduce from the
Batter root (Cargo fetches the pinned native Git source):

```sh
CARGO_TARGET_DIR=target cargo test --locked --manifest-path docs/evidence/batter-gi4/Cargo.toml
```

This archive is a separate unpublished Cargo workspace, not a root workspace
member or another supported library. The generated lockfile fixes the evaluated
registry graph (including Tokio 1.53.1 and SQLx 0.9.0). The native packages now
use Git revision `d57ec6be61e9f00ccce373b19ca356cafe98f206`, matching the root
workspace. The original exercise used sibling development sources; the pin
follow-up and its rerun are recorded in [validation](../../validation.md).

The executed evidence covers one agent, two bounded assignments and seven
behavioral checks on Linux/Rust 1.98.1. The supplied signatures guided API
selection. This does not establish population-wide agent reliability, independent
handler correctness, or future review-loop convergence. Native callback ownership,
startup failures and faulted PostgreSQL behavior have separate library/live tests.
Logs: `/tmp/batter-gi4-consumer-integration.log`,
`/tmp/batter-gi4-consumer-modification.log`,
`/tmp/batter-gi4-consumer-archive.log`.
