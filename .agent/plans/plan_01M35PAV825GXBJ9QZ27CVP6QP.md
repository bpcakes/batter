# Native SQLx query helpers — batter-ywd2

Deliver requested item4 as a separate PR stacked on #14 (2785f27). Add native
query helpers without a query language, ORM, migrations or connection extraction.
SQLx remains0.9.0, Rust1.94 minimum, Unix-only.

## Progress

- Inspected SQLx0.9 Query/Map/QueryScalar implementations and current lease,
  policy scope, operation budget and retained completion paths.
- Rust1.94 scratch compilation proves dispatch for all four operations and a
  lending scope closure. The repository implementation compiles; five focused live cases, two budget
  unit controls and the native policy consumer pass on Rust1.98.1. Final gate results are recorded below.
- Independent contract review found no blockers in sealed dispatch, selected scope
  modes, native phase ownership, retained pooled outcomes or same-lease disposition.
- Scoped all-target Clippy passes after splitting macro-shape test assertions into
  focused helpers without weakening them. Starting final two-toolchain verification.

## Surprises & Discoveries

Map fetches must use its native mapper; generic Execute fetch would erase it.
Native Execute accepts Map and QueryScalar, but execute deliberately discards rows
and mappers. Generic default dispatch required an unnecessarily long query borrow;
concrete implementations allow native SQLx to shorten it. The prototype in
/tmp/batter-native-query-full.rs compiles on1.94 with all three native types.
Runtime constructors share macro return types, so macro-only origin is not provable.
Current .cargo/config.toml selects SQLX_OFFLINE=true. New generated metadata must
be covered by both Jig input contracts. Pool-return cleanup awaits after a query
result; use existing retain_with_fallback to preserve that observed result across
cleanup cancellation, before telemetry. No observed result means interruption.

Macro Send controls exposed Rust's higher-ranked opaque-future limitation
(error points to rust-lang/rust#100013). The final sealed trait has no public
query-lifetime parameter, dispatches through a crate-owned connection borrow,
and uses one boxed Send future per helper call. Query output/error types remain
concrete. A crate-private consuming PgLease::finish_query shares the existing
normalization/disposition path, avoiding nested lending callbacks. The native
consumer's require_send check and pooled Send check now compile. Macro metadata
was generated against PostgreSQL18.6; no hand-authored query metadata or lockfile.

The first full Rust1.94.0 run stopped in the existing native worker example:
its disposable container exposed no 5432/tcp port after ten discovery attempts.
The container was already removed at inspection. An unchanged focused rerun
passed; the underlying Docker cause is unconfirmed. Preserve the failed log and
repeat the full minimum-toolchain gate without changing assertions or timeouts.

The unchanged full minimum-toolchain rerun passed tests, doctests and Clippy,
but rustdoc failed reading its existing target/doc search index (missing on-disk
column). Preserved that generated directory outside the repository and reran the
exact rustdoc command from verify.sh. No tracked source or assertions changed;
retain the passed earlier checks instead of re-executing them for this artifact failure.

## Decision Log

PgNativeQuery is sealed and accepts native SQLx query values. Its associated
Output preserves native rows/mapped structs/scalars. PgPolicyScope helpers return
Result<Output,P::Error> through scope.sql and ordinary From<sqlx::Error>, preserving
runner-selected fail-fast/recoverable behavior. PgAtomicScope helpers preserve
PgScopeError<sqlx::Error>. Native policy intent/queue phases forward the same four
methods without changing their consuming phase transition.

PgQueryHandle::within takes an existing pool, parent context, positive maximum,
static operation name and error mapper Fn(OperationError<sqlx::Error>)->E. It owns
one child deadline clamped to the parent across calls, never resets it, and acquires
one lease per query inside that boundary. PgLease owns cleanup/disposition; observed
native results are retained before cleanup and resolved before telemetry. No retry,
server-cancellation/rollback promise, arbitrary-session reset or provisioning.

## Outcomes & Retrospective

Implementation and independent contract review complete. Final evidence on macOS
arm64, PostgreSQL 18.6:

- Rust 1.98.1: full verify.sh passed, live inventory 118/118, HTTP smokes 5/5.
- Rust 1.94.0: full workspace tests, doctests, both Clippy commands and rustdoc
  passed; live inventory 118/118, HTTP smokes 5/5. See the two retries above.
- Jig required fmt, clippy, contract and file-budget gates passed. Final api:test
  receipt receipt_01M35V6V32QTMWZFC5X0ZA6XHA belongs to successful run
  run_01M35TP3WDWZVTQ5Q9Y1TESVCJ. Evidence/gates report fresh passing results.
- The first Jig test command failed; its diagnostic fell beyond the truncated
  progress preview. A direct no-run compile and directory scan then stalled at
  target/debug/deps (a process sample showed readdir). Stopped only these owned
  diagnostics and reran the unchanged gate with a fresh task-owned target directory,
  /tmp/batter-ywd2-jig-target. It passed. The initial failure's cause is unconfirmed.
  Successful lint/fmt receipts were reused; target-directory selection changes
  artifact storage, not source, test selection, toolchain or database prerequisites.

Logs: /tmp/batter-ywd2-verify-1981-final.log;
/tmp/batter-ywd2-verify-1940-initial-failed.log;
/tmp/batter-ywd2-verify-1940-doc-index-failed.log;
/tmp/batter-ywd2-worker-1940-rerun.log;
/tmp/batter-ywd2-rustdoc-1940-rebuild.log;
/tmp/batter-ywd2-live-1981.log and -live-1940-final.log;
/tmp/batter-ywd2-http-1981.log and -http-1940.log;
/tmp/batter-ywd2-jig.log and -jig-rerun.json.

Prepared as a separate PR stacked on feat/sqlx-fail-fast. No merge or publication.

## Implementation and validation

Add native_query.rs (+scope submodule if needed), pooled_query.rs and public
exports under crates/batter-sqlx. Forward native policy methods in
runledger/runledger-postgres/src/atomic/policy.rs. Add actual query!, query_as! and
query_scalar! test cases with Cargo-generated .sqlx metadata; enable SQLx macros
in the adapter devdependency. Add crates/*/.sqlx/** to each Rust action input list
in .jig.toml and .agent/jig-contract.json together. Keep Cargo.lock Cargo-generated.

Tests: three macro types across one/optional/all/execute, bind inference, missing
rows, mapper/decode errors, affected rows, scope-mode statement counts and terminal
refusal; native phase use. Pooled tests cover preflight/unpolled acquisition,
parent-clamped shared budget, acquisition/query cancellation, failed lease retirement,
successful reuse and observed output retained through cleanup interruption. Existing
fixtures provision only the owned external disposable PostgreSQL18.6 cluster.
No semantic test relaxation. Update exact live inventory and contract/status/docs.

ADR010 review: no downstream query implementations, raw owner extraction or query
reuse; fixed policy/error selection; no helper runs after poisoned/rolled-back
scope; phases remain consuming. Runtime SQL meaning/macro origin and effects are
upstream/application limits. Use an independent fresh contract review before delivery.

Run focused compile/live/lint checks, then both full verify.sh commands, exact live
SQLx suites and rebuilt five-mode HTTP smokes per toolchain, plus required Jig gates.
Inspect evidence/gates; do not edit tracked inputs during work check. Final tracker
and documentary edits refresh cheap policy gates while reusing unchanged Rust passes.
Commit, push and open separate stacked PR; never merge or publish crates.

## Recovery

Additive source-only change. Remove helper modules/exports and tests to revert;
existing closure APIs remain. Query failures/cancellation retire the exact acquired
connection; retained query success says nothing beyond native observed result.
