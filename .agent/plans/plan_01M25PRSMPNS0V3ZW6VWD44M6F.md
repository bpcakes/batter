# Reduce test infrastructure failure surface

Owning Bead: batter-27t; tracing repair: batter-gg4. Git baseline: 486e0b0f9f4c4439077418715843b30042205f7e. Preserve existing uncommitted HTTP work and tracker history. No commit or publication requested.

## Progress
- [x] Research pinned upstream tracing and HTTP semantics; compare both fixtures and existing Unix harness.
- [x] Consolidate process evidence and event storage; add adverse controls.
- [x] Repair test dispatcher construction with deterministic upstream regression.
- [x] Make mutation reproduction portable; consolidate contracts and record local parallel load.
- [x] Pass both full compiler matrices, ten rebuilt HTTP smokes and fresh Jig gates.

## Surprises & Discoveries
tracing-core 0.1.36 single-dispatch optimization uses the thread default when first registering a callsite. An unsubscribed thread can cache Never for the only registered dispatcher. A standalone reproduction fails without an inert dispatcher and passes with one; upstream tokio-rs/tracing#2874 is open. HTTP observation Python launch independently omitted launch identity and success evidence already present in workspace test-support/process. Event assertions exposed MutexGuard lifetime to panic.

## Decision Log
Keep two HTTP scenario implementations: they observe different transport/resource boundaries. Share the existing native process protocol, plus narrow synchronous EventLog storage. Require scenario-specific completion after exercise and teardown. No public production API changes. Test dispatcher helper registers an inert sentinel before constructing real subscribers; never install a global default. Keep real dispatch lifetime local instead of retaining every capture forever. Hosted CI/macOS remain unverified; stress local unchanged timing bounds. Portable mutation script resolves Axum from locked cargo metadata and patches a private copy.

## Outcomes & Retrospective
Implemented. Both full compiler matrices pass 680 Rust executions each, all ten rebuilt HTTP smokes pass, 1,000 core binary repeats and 200 cooperative HTTP repeats pass under four concurrent processes. Bare-dispatch mutation fails the deterministic regression as expected. Portable HTTP mutation baseline and both semantic negative variants pass their required outcomes. Final Jig gates passed with fresh receipts for all five targets; docs/validation.md records exact evidence and unverified external scope.

## Context and work
Sources: test-support/process, new private test-support/events.rs and dispatch.rs, both crates/batter-axum/tests/http_lifetime* targets, foundation test subscriber constructors. The subprocess protocol uses exact child_fixture argv and PID-bound stdin with independent emergency and parent-death limits. Generic mechanics must know no HTTP scenarios. Attach control modules explicitly from HTTP wrappers.

## Concrete steps and validation
Use cargo fmt; cargo test -p batter-axum --test http_lifetime --test http_lifetime_observations --locked; deterministic callsite control in an isolated process; repeated core binaries on both toolchains. Run python3 scripts/check_http_graceful_mutation.py, preserving positive baseline and two semantic negative variants. Then bash scripts/verify.sh and RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh; build http_service on each and run scripts/smoke_http.py default, SIGINT, deadline, warn-filter, warn-filter+deadline. Update docs/validation.md, guarantees/status/references/ADRs/testing and Beads. Finish with scripts/jig work evidence, work gates, work check and work finish. Inspect every exit status; never count build errors as mutation success. External PostgreSQL probes remain opt-in and documented ignored.

## Recovery and interfaces
Only test infrastructure/config/docs change. Cargo.lock stays generated and unchanged in root; mutation uses its own workspace and target directory. Repeated commands are read-only except append-only Jig evidence and Beads export. Preserve prior plan histories and unrelated .epicd files. No migration, production interface, subscriber default or panic hook is introduced.

Implementation note: initial repeated-binary setup assumed feature profiles always
produced distinct standalone artifact names. It stopped before testing. Corrected
the all-feature selection to the workspace command and snapshotted each executable
before later builds; 1,200 actual repetitions then passed. The shared EventLog
leaves async notifications local; no generic runtime/event framework was added.

Final fresh api:test receipt: receipt_01M25QM4T8TB9S5SV2S5RYYB4F. Evidence: /tmp/batter-hardening-gates-final.json. Root lock and production runtime behavior unchanged. Both Beads closed. No commit or publication performed.
