# Agent-consumer API experiments: startup, registration and SQLx

Historical planning evidence, executed on Linux on 2026-09-12 with Rust 1.98.1.
These are isolated prototypes, not implemented Batter APIs or a release claim.
Delivery scope and acceptance belong to the linked Beads, not this record.
The resulting epic is `batter-lp2`; inspect it with `br show batter-lp2`. Its four
delivery tasks own constrained registration, startup signals, slot-owned SQLx
pools and actual consumer adoption. None is marked implemented by these trials.

Planning used four sequential architecture/integration reviews and separate
standalone checks. The final consumer-task check initially requested more precise
acceptance; the expanded task passed a fresh recheck. The final graph has A -> B
and A/B/C -> D, with A and C ready. Exact tracker readback and cycle checks passed.

## Question and selected direction

Can an agent initialize native resources and register adapters without owning
signal handoff, rejected pool-finalizer recovery, or the enclosing supervisor?

| Concern | Selected direction | Experimental reason |
| --- | --- | --- |
| Startup authority | Add `Startup::scoped` with `ProtectedStartupScope` and a concrete borrowed `Registration` view | Real HTTP composition worked without exposing cleanup extraction or driver start; negative compilation controls rejected those operations. |
| Unix signals | Opt-in `.with_unix_signals(name)` with synchronous installation at `start()` | An asynchronous-install negative control died from SIGTERM before coordinator polling; synchronous installation covered that interval. |
| SQLx ownership | Synchronous `batter_sqlx::pool_in(slot, options, connection)` | It published native close ownership before returning; a fresh consumer preferred it after trying the asynchronous alternative because its required query already established connectivity. |

The planned names are proposals. The final protected error envelope and internal
signal-name reservation were not implemented by these prototypes. The protected
scope and signal/pool prototypes were evaluated separately, not as one integrated
production implementation.

## Baseline and artifact provenance

The research baseline was Git
`809d5d5b913d26497232f13682b334e1c4aec806`. Concurrent documentation work advanced
HEAD to `39c1b6e3c1c75f808becb5a5e7c33f58001a2ee4`; it was preserved. Experiments
used isolated source copies or path dependencies, with generated Cargo locks.
No production Rust source or dependency graph was changed in this planning pass.

`experiments.tar.gz` preserves prototype sources, manifests, locks, first consumer
sources, compiler diagnostics, retained experiment logs, public handouts and
parent-authored checking scripts. `SHA256SUMS` binds the archive. Build outputs,
local PostgreSQL data and consumer binaries are excluded.

Archive paths retain original scratch-directory names. Some manifests and runners
contain absolute scratch paths. They are historical inputs, not a portable fixture
claimed to have been replayed after packaging. On another machine, extract into a
new directory, rebind path dependencies to the matching archived prototype and
the recorded repository baseline, then record the changed manifest identities.
Do not extract over a working repository or recreate the original absolute paths
by overwriting unrelated files.

## Executed prototype outcomes

The parent inspected sources and reran the following successfully:

- Signal prototype: 14 child-process scenarios, including asserted negative
  behavior for asynchronous installation and unreserved signal names.
- SQLx prototype: 13 tests against an isolated PostgreSQL 18.6 cluster, including
  native queries, missing-role errors, cancellation, LIFO and incomplete close.
- Registration prototype: two actual HTTP runtime tests; native Runledger
  translation and legacy consumers compiled; eight expected compilation failures
  were classified by intended Rust error codes.
- Existing unrestricted startup: a baseline regression subject extracted and
  dropped cleanup, then produced a successful empty cleanup report without
  invoking the finalizer. This demonstrates a documented escape hatch, not a
  contradiction of the existing scope's disclosed limitation.

The signal prototype covered TERM/INT during initialization, an unpolled-owner
interval, handoff, running drain, owner loss, waiter loss and an injected listener
installation error. The injected error was not an actual partial two-listener OS
installation failure. Internal name reservation remains proposed work.

The SQLx cluster used private Unix sockets and local trust authentication. A
missing role exercised a native authentication rejection, not SCRAM/password/TLS
validation. An acknowledged stalled TCP handshake exercised cancellation, not a
successful PostgreSQL connection. Native Runledger runtime settlement was not
executed by the registration prototype.

## Fresh consumer trials

Three fresh agents received public API handouts and behavioral requirements,
without prototype internals, hidden oracle implementations, or prior repair
transcripts. Each implemented an initial application and then a modification.
The pool agent also tried the other candidate afterward. This is three bounded
agent trials, not six or seven independent samples or a reliability percentage.

| Trial | Initial result | Modification and comparison |
| --- | --- | --- |
| Registration | First compile had three errors; second passed real HTTP and cleanup assertions. | First modification passed: second component/resource, real request, joins and LIFO cleanup. |
| Pool | Async candidate passed four initial scenarios on first compile. | Six two-pool scenarios passed; the same agent then passed four initial scenarios with synchronous `pool_in` and preferred it. |
| Signals | Builder consumer compiled first attempt and passed six real-signal scenarios. | Second resource/fallible stage compiled first attempt and passed nine scenarios without new signal-routing code. |

Registration's first errors were a missing module path in the handout, a concrete
startup-error conversion into an unsuitable boxed initializer error, and an
incorrect assumption that `check_shutdown` returned a report rather than unit.
The plan therefore requires complete import/signature examples and concrete error
handling. It does not redesign unrelated existing report APIs.

The parent independently sent 14 real OS signals across the two signal consumers,
checking externally acknowledged phases, absence of premature cleanup/readiness,
one finalization, LIFO for the added dependency, bounded completion and clean
diagnostics. It also reran the synchronous pool consumer's four scenarios, the
async consumer's six-scenario modification, and both registration consumers.

The first parent registration rerun reused a shared Cargo output name and printed
the modified consumer's marker for the original variant. That run was rejected as
original-variant evidence. The parent rebuilt each manifest with a distinct Rust
metadata value and directly executed it, then observed the correct per-variant
markers and passing assertions. Future evaluations must bind executable identity
to each source variant, using distinct target directories or equivalent evidence.

The synchronous pool candidate did not receive the fresh two-pool modification;
that remains final delivery acceptance. Prototype synchronous two-pool tests did
pass. Small unused-field warnings in consumer builds are retained, not reported
as denied-warning or full verification passes.

## Reproduction commands used before packaging

These paths identify this run. Rebind them deliberately for another environment.
`TMPDIR` points to home cache because the separate `/tmp` filesystem filled during
experimentation. Initial disk failures and repaired compiler failures are retained.

```sh
RUSTUP_TOOLCHAIN=1.98.1 TMPDIR=/home/aa/.cache/batter-signal-api.t2SEh7 cargo run --manifest-path /home/aa/.cache/batter-signal-api.t2SEh7/experiment/Cargo.toml --offline --locked
RUSTUP_TOOLCHAIN=1.98.1 TMPDIR=/home/aa/.cache CARGO_TARGET_DIR=/home/aa/.cache/batter-pool-api.XDZEQM/target cargo test --manifest-path /home/aa/.cache/batter-pool-api.XDZEQM/Cargo.toml --offline --locked
bash /home/aa/.cache/batter-agent-api-plan.cxZy2c/rerun-registration.sh
python3 /home/aa/.cache/batter-agent-api-plan.cxZy2c/check-consumer-signals.py
RUSTUP_TOOLCHAIN=1.98.1 TMPDIR=/home/aa/.cache CARGO_TARGET_DIR=/home/aa/Documents/batter/target cargo rustc --locked --offline --manifest-path /home/aa/.cache/batter-fresh-registration-80waV4/original/Cargo.toml --bin fresh-registration-consumer -- -C metadata=parent_original_20260912
/home/aa/Documents/batter/target/debug/fresh-registration-consumer
RUSTUP_TOOLCHAIN=1.98.1 TMPDIR=/home/aa/.cache CARGO_TARGET_DIR=/home/aa/Documents/batter/target cargo rustc --locked --offline --manifest-path /home/aa/.cache/batter-fresh-registration-80waV4/modified/Cargo.toml --bin fresh-registration-consumer -- -C metadata=parent_modified_20260912
/home/aa/Documents/batter/target/debug/fresh-registration-consumer
```

Database fixtures must be externally authorized and disposable. The experimental
cluster is not part of the supported adapter API. Its credentials and fixed socket
coordinates describe this local fixture only; never substitute a production URL.
The private experimental server was stopped with `pg_ctl stop -m fast` after the
consumer reruns. No unrelated cluster was used or stopped; its data was retained
outside the repository and excluded from the archive.

## Source-grounded limits

Tokio 1.53.1's installed `src/signal/unix.rs` and
[official signal documentation](https://docs.rs/tokio/latest/tokio/signal/unix/fn.signal.html)
establish process-global permanent signal handlers and coalescing. Synchronous
installation does not restore handlers on drop, cover time before `start()`, or
make kernel arrival an atomic readiness fence.

SQLx 0.9.0's installed pool `options.rs`, `inner.rs` and `mod.rs`, with its
[pool options](https://docs.rs/sqlx/latest/sqlx/pool/struct.PoolOptions.html) and
[pool documentation](https://docs.rs/sqlx/latest/sqlx/struct.Pool.html), establish
that lazy creation can start maintenance work. Registration before return is not
a claim that another runtime worker cannot act during creation. Lazy creation
plus one acquisition is not equivalent to all `connect_with` warmup semantics.

`is_closed()` can become true before `close().await` finishes; the held-checkout
experiment demonstrated timeout with that flag already true. Pool close does not
certify termination of detached server sessions or resolve commit ambiguity.
Native options are passed through, not sanitized or made environment-independent;
even `new_without_pgpass` retains other native environment behavior.

No Rust 1.94, macOS, hosted CI, production workload, complete workspace matrix,
complete reference live inventory or population-level agent evaluation was run
for these prototypes. Those requirements remain attached to implementation tasks.
