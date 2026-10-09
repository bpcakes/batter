# Whole-application fresh-agent baseline, 2026-10-09

Owning task: `batter-k64t`. One general-purpose coding agent with no session
context built a complete service on the Batter facade from public documentation
only. The question was whether the documentation resolves the choices between
near-duplicate paths, not whether the library works.

## Setup

- Source: a Git-free `git archive` copy of master at `71e37ea`, so the agent
  could not read history, the tracker, or files outside the allowlist.
- Allowed reading: `README.md`, `CHANGELOG.md`, `docs/*.md` except
  `docs/evidence`, crate READMEs, rustdoc comments under `crates/*/src`, the
  facade examples, `runledger/README.md`, `runledger/llms.txt`,
  `runlimit/README.md`, and rustdoc comments under the native packages.
  Excluded: every `AGENTS.md`, every `tests` directory, `consumers/`, the two
  example packages, `docs/evidence`, `scripts/`, the tracker, and the web. The
  agent also read the facade and workspace manifests, which the packet did not
  list; it recorded that itself.
- Dependency rule: only `batter` by path, with features the agent chose, plus
  registry crates; no other workspace package and no `[patch]`.
- Task: typed settings; a startup-owned PostgreSQL pool; schema setup; `POST
  /records` inserting a row and enqueuing a durable `records.notify` job in the
  same transaction; `GET /records/{id}`; liveness and readiness; a worker whose
  handler runs a bounded provider step and a final-state write that survives a
  provider timeout; SIGTERM and SIGINT drain with a non-zero exit on a failed
  shutdown report. `cargo build`, `cargo clippy --all-targets -- -D warnings`
  and `cargo fmt --check` had to pass with `SQLX_OFFLINE=true`; unit tests
  without a database where feasible; no live database.
- `oracle.md` was frozen before launch and not given to the agent. `LOG.md` is
  the agent's own process log, verbatim.

## Result

| Decision | Agent's choice | Against the oracle |
| --- | --- | --- |
| Service root | `Startup::scoped`, `with_unix_signals`, `start`, `wait_checked` | canonical |
| Operation budgets | `OperationOwner::new` roots; `AdmittedRequest::context` per request; `PgQueryHandle::within` for single queries; `context.run` around `run_atomic` | canonical |
| Pool ownership | `reserve_cleanup`, then `RunledgerDatabase::connect_lazy` with a `PgSessionProfile` and a hand-registered close | documented alternative; `pool_in` cannot produce a `RunledgerDatabase` (`batter-nhu5`) |
| Row plus job in one transaction | `batter::runledger::run_atomic` with `record_required_job_enqueue_intent` | canonical |
| Worker registration | native `Supervisor::builder(..).with_catalog(..).prepare()` then `batter::runledger::register_in` | canonical |
| Job handler budget | `JobExecutionHandler` with `job_phases(execution, 500 ms)`, provider step under `work().child`, final write under `finalization()` | canonical |
| HTTP assembly | `GuardedRouter`, `HttpBoundary` with both probes, `assemble`, `register_in`; `in_process` in tests | canonical |
| Handler inputs | `AdmittedRequest` plus `Result<Json<_>, _>` and `Result<Path<_>, _>` extractors | canonical |
| Health and readiness | `HealthMonitor::register_in` and `ReadinessPolicy::new` | canonical |
| Settings | `SettingsSource::from_pairs` and `select`, `milliseconds`, `SecretString`, `JobsConfig::validate` | canonical |
| Facade features | `axum`, `runledger` | canonical |

No decision point was wrong.

| Measure | Value |
| --- | --- |
| Documents read before the first build | 27 |
| Rust written | 1,446 lines in six modules |
| Builds to first clean `cargo build` | 2 |
| `clippy -D warnings` runs to first clean | 4, all repairs in test code |
| Tests | 26 passed, no database |
| Problems surviving three rounds | none |

The first build failed on two things: `batter::runledger` did not re-export
`PgScopeError`, the error that `scope.application` returns, and a helper took
`CorrelationId` by value where the extractor returns a reference. The clippy
rounds were trait bounds and `'static` on test fakes, `type_complexity` on a
test fixture, and a dropped `#[must_use]` value from `JobInvocationOwner::end`.

Two smoke runs without a database behaved as the contracts state: missing
settings failed before any resource with `configuration failed: DATABASE_URL:
missing value`, and an unreachable database failed the `schema.records` stage
after the acquire timeout with the pool finalizer observed and exit status 1.

## Reported gaps and disposition

| Gap | Disposition |
| --- | --- |
| The protected startup scope exposes no operation context for the bounded query after `pool_in` | Sentence added to `docs/usage.md`: clone the startup context before `Startup::scoped`. |
| `pool_in` yields a plain `PgPool`; Runledger APIs need a `RunledgerDatabase` with no constructor from a pool | Evidence added to `batter-nhu5`. |
| `PgScopeError` not re-exported from `batter::runledger` | Re-exported in this change. |
| No budget-aware `run_atomic` on the Runledger side | Sentence added to the adapter README: wrap the call in `context.run`. |
| `AdmittedRequest::correlation_id` return type undocumented | Sentence added to its rustdoc. |
| `PgNativeQuery` rejects the runtime `sqlx::query_as` result | Sentence added to the SQLx adapter README. |
| No test facility for a `JobExecutionHandler` without a worker | Filed as `batter-3upk`. |
| `JobInvocationOwner::end` is `#[must_use]` and the services example never ends the invocation | Sentence added to the `JobExecutionServices::invocation` rustdoc. |
| Definition sync before `register_in` is not named on the Batter side | Sentence added to the adapter README. |
| Shutdown budget versus job timeouts has no guidance | Application policy; not changed. |
| `PgSessionProfile::with_timeouts` versus `new` as the consumer default | Not changed; needs a maintainer decision. |

## Reproduction

The archived `consumer/` crate is the agent's source with one change: the
`batter` dependency path is relative to this repository instead of the scratch
copy, and a `[workspace]` table keeps it out of the root workspace. It was
rebuilt against this tree after archiving:

```sh
cd docs/evidence/whole-app-baseline-2026-10-09/consumer
SQLX_OFFLINE=true cargo test
```

Result: 26 passed. The agent's original tarball hash, over `Cargo.toml` and
`src` before the path edit, was
`91f5b898f6fb98386ebfcc3fc13c7e1d1db0e3b2f0e4894b5a15d5ebce6df12e`.

## Limits

One agent, one run, on the same model family that maintains the repository.
No live PostgreSQL execution, so schema creation, Runledger migration, intent
promotion, worker execution and signal drain were not exercised. The
measurements are the agent's self-reported log, checked against the archived
source and the rebuild. This record does not establish population reliability
or compare against an alternative documentation set.
