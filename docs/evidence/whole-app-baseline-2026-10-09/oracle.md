# Frozen oracle: whole-application fresh-agent baseline, 2026-10-09

Written before the fresh agent was launched. The agent does not receive this file.

Source given to the agent: Git-free `git archive HEAD` copy of batter at
71e37ea (master, 2026-10-09), extracted to `baseline/src`.

Reading allowlist given to the agent: README.md, CHANGELOG.md, docs/*.md except
docs/evidence, crates/*/README.md, rustdoc comments under crates/*/src,
crates/batter/examples, runledger/README.md, runledger/llms.txt,
runlimit/README.md, rustdoc comments under runledger/*/src and runlimit/*/src.
Excluded: AGENTS.md and crate AGENTS.md, every tests directory, consumers/,
examples/reference-service, examples/postgres-lifecycle, docs/evidence, scripts,
.agent, the tracker, Git history.

## Decision points and the canonical answer at this revision

| Decision | Canonical | Accepted alternatives | Wrong |
| --- | --- | --- | --- |
| Service root | `Startup::scoped(...).with_unix_signals(..).start()` then `wait_checked()` | `service::start(startup, NoDiagnostics)` | `Supervisor::start` driven directly; `Startup::new`; `.wait().await?` without the checked form |
| Operation budgets | `OperationOwner::new(..).into_context()`; children via `context.child(..)` | `OperationOwner::at(RootDeadline)` | any attempt at `OperationContext::new` |
| Pool ownership | `scope.reserve_cleanup("postgres.pool")` then `batter::sqlx::pool_in` | `RunledgerDatabase::connect_lazy` with a `PgSessionProfile` when Runledger needs the profiled owner | pool created outside startup with manual close; `PgPoolOptions::connect` with no registered finalizer |
| Write a row and enqueue a job atomically | `batter::runledger::run_atomic` scope: `.application(..)`, `.record_required_job_enqueue_intent(..)` or `.queue().enqueue_job(..)` | `batter::sqlx::run_atomic_in` for the row plus a separate native intent is wrong, so there is no accepted alternative | two transactions; native `record_job_enqueue_intent_tx` on a raw `sqlx::Transaction`; `low_level::PgAtomicTransaction` |
| Worker registration | `batter::runledger::register_in(scope, "worker", context, prepared)` with `native::runtime::Supervisor::builder(..).prepare()` | none | native `Supervisor::run_until_shutdown_report`; a bare `tokio::spawn` |
| Job handler budget | `JobExecutionHandler` + `job_phases(execution, reserve)`, provider step under `work()`, final write under `finalization()` | none | fresh root from `remaining_work_budget`; legacy `JobHandler` with `JobContext` and no phases; final write under `work()` |
| HTTP assembly | `GuardedRouter` + `HttpBoundary::new(policy).with_liveness(..).with_readiness(..).assemble(..)` then `AssembledHttp::register_in` | `register_with_connect_info_in` | `low_level::register_http_in` with a raw `Router`; probes mounted as ordinary routes |
| Handler inputs | `AdmittedRequest` extractor | none | `Extension<OperationContext>` |
| Health and readiness | `HealthMonitor::new(policy, probe).register_in(registration, name)` + `ReadinessPolicy::new(status, reader)` | `with_condition` for application conditions | `HealthMonitor::run`; a readiness handler that performs the database probe inline |
| Settings | `SettingsSource` (`from_pairs`/`read_file`), `bounded_u64`, `milliseconds`, `SecretString` for the URL | hand parsing with equivalent bounds | `std::env::var` with `unwrap` or `expect` |
| Facade features | `axum`, `sqlx`, `runledger` (which implies `sqlx`) | plus `runledger-test-support` in dev-dependencies | declaring `batter-axum`, `batter-sqlx`, `runledger-*` directly; a `[patch]` section |

## Measurements to record

- Documents read, in order, and which one answered each decision.
- Cargo invocations and the first error of each failing one; count compile
  rounds to first clean `cargo build` and to first clean `clippy -D warnings`.
- Lines of Rust written.
- Gaps the agent reports, with the file it expected them in.
- For each decision point: canonical, accepted alternative, or wrong.
