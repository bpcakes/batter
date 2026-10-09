# runledger-core agent guide

## Purpose
Shared durable-execution contracts: handler traits, job/workflow enums, runtime types, and workflow enqueue/build validation.

## Key entrypoints
- `src/lib.rs`: crate API surface.
- `src/jobs.rs`: public jobs/workflow exports.
- `src/jobs/handler.rs`: handler traits and registry trait.
- `src/jobs/runtime_types.rs`: `JobContext`, `JobFailure`, `JobProgress`.
- `src/jobs/execution.rs`: borrowed `JobExecution` services and the custom-runtime trait.
- `src/jobs/invocation.rs`: `JobInvocationOwner` and the read-only `JobInvocation`
  exit observation, its waiters and synchronous exit hooks.
- `src/jobs/dead_letter.rs`: `JobDeadLetterInfo`, `JobDeadLetterReason`, and the
  `JobDeadLetterOrigin` that tells hooks whether the worker or the reaper delivered them.
- `src/jobs/status.rs`: persisted status/event enums.
- `src/jobs/workflow_enqueue/*`: workflow enqueue builders, shared step-shape
  validation, and DAG validation.

## Edit here for X
- Shared job/domain enums or constants: `src/jobs/status.rs`.
- Handler trait contracts: `src/jobs/handler.rs`.
- Workflow enqueue shapes/builders: `src/jobs/workflow_enqueue/{types,run_builder,step_builder,dag_builder}.rs`.
- Shared step/DAG validation: `src/jobs/workflow_enqueue/{step_validation,build_validation,dag_validation}.rs`.

## Invariants
- Prefer direct cutovers only for internal code-only refactors within one coordinated deploy. Changes to durable job/workflow contracts, enums, or payloads that can outlive a deploy require backward compatibility or an explicit staged rollout; update dependents in the same change.
- Keep this crate free of storage, transport, and app-specific logic.
- Prefer explicit validation errors over implicit panics.
- Keep this crate free of Tokio. `JobInvocation` observes an invocation's exit
  without authority to end it; only the non-cloneable owner ends it, once, by
  `end` or drop. `JobExecutionServices::invocation` defaults to `None`, never to
  a signal that silently never ends. Contain each waiter-wake and hook panic so
  later waiters and hooks still run; return all contained notification panics.
  The panic report owns disposal: release strings, retain opaque allocations
  without running their destructors, and prefer borrowed `payloads()` diagnostics.
  Clone, wake and destroy wakers outside the invocation state mutex, including
  replacement and removal, because their callbacks may re-enter the signal.

## Common commands
- `cargo check -p runledger-core`
- `cargo test -p runledger-core`
- `cargo clippy -p runledger-core --all-targets -- -D warnings`
