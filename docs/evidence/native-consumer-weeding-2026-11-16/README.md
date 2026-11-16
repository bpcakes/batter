# Native consumer pattern repairs, 2026-11-16

Owners: `batter-6p9u`, `batter-bwc7`, `batter-ad1e`, `batter-hixn`, `batter-f1j7`.
See the [ADR-010 assessment](../api-assessments/native-runtime.md#native-consumer-pattern-repairs-2026-11-16)
and [native migration notes](../../../runledger/CHANGELOG.md#native-consumer-pattern-repairs).

## Public API review

Compared baseline `7cc8c82ef90bb6b2babcfab9ee60dda678311064` with this working
tree using pinned Rust 1.100.0 (0ef1bc386 2026-11-09). For both native packages:

```sh
RUSTC_BOOTSTRAP=1 cargo rustdoc -p <package> --lib --all-features --locked -- \
  -Zunstable-options --output-format json
python3 docs/evidence/public-api-diff-2026-10-09/surface.py \
  <before-doc-dir> <after-doc-dir> runledger-postgres runledger-runtime
```

Postgres baseline JSON was captured before its edits; runtime baseline came from
an isolated detached worktree at the same base. Each JSON was copied immediately
after its successful command. The temporary worktree was removed. The JSON
format is unstable; `RUSTC_BOOTSTRAP` is used only for this evidence extraction,
not a build/CI dependency or source feature gate.

The saved [kind/path comparison](surface.txt) reports:

| Package | Before | After | Removed | Added |
| --- | ---: | ---: | ---: | ---: |
| `runledger-postgres` | 3398 | 3376 | 22 | 0 |
| `runledger-runtime` | 1497 | 1514 | 0 | 17 |

All Postgres removals belong to `JobProgressUpdate`, `update_job_progress`,
`update_job_progress_for_lease`, their fields, traits and reexports. The four
retained running/ordinary-progress method signatures and SQL strings were
compared directly and remain unchanged. No schema or persisted format changed.

Runtime additions are `JobsEnvironmentError`, its accessor and traits/reexport,
`RuntimeShutdownSignal::unix_termination`, and `RuntimeError::InvalidEnvironment`
with its conversion. `RuntimeError` was already non-exhaustive. The surface
walker's `JobsEnvironmentError: From` line describes the related
`From<JobsEnvironmentError> for RuntimeError` implementation in raw rustdoc JSON;
it does not mean a public conversion into the opaque error.

The kind/path walker misses same-name signature changes. A separate
[signature comparison](signatures.py) normalizes numeric rustdoc IDs and compares
function signatures, generics and headers through local public reexports:

```sh
python3 docs/evidence/native-consumer-weeding-2026-11-16/signatures.py \
  <before-runtime.json> <after-runtime.json>
```

It found exactly six changed paths: these three methods through both `config`
and `prelude`. Direct source review confirmed return types are the only cuts:

| Method | Before | After |
| --- | --- | --- |
| `JobsConfig::from_env()` | `Self` | `Result<Self, JobsEnvironmentError>` |
| `IntentPromoterConfig::from_env()` | `Self` | `Result<Self, JobsEnvironmentError>` |
| `IntentPromoterConfig::from_env_with_jobs_config_defaults(&JobsConfig)` | `Self` | `Result<Self, JobsEnvironmentError>` |

`Supervisor::builder_from_env` keeps its signature and propagates the new error.
No infallible environment shim or arbitrary-stage shim remains, since either
would restore the invalid state. Every removed/retyped call shape has a native
`Breaking:` example and replacement `Migration:` rustdoc. This inventory is not
a general semantic compatibility proof; facade/native dependency reexports keep
the same identity and expose these native cuts.

## Focused executed evidence

Executed on macOS with Rust 1.100.0:

- Five Runlimit regressions exercise string payloads and opaque panicking Drop
  payloads through both observation helpers and fixed-window/GCRA/attempt memory
  mutations. Restoring old disposal caused all five tests to fail; restoring the
  repair passed. Core/memory all-target/all-feature strict Clippy passed.
- Eight runtime configuration tests cover absence/defaults, malformed/blank/
  non-Unicode/overflow/range errors, independent promoter inheritance and redacted
  diagnostics. Runtime all-target/all-feature strict Clippy passed before the
  final pre-submission signal test and documentation edits.
- Six `native_entrypoints` tests pass, including the inert child entry, actual
  SIGINT/SIGTERM settlement and awaited pool closure, delivery before supervisor
  submission, the Ctrl-C-only SIGTERM failure control, public environment-builder
  rejection in isolated children, and initialization/stopped-startup announcement.
  These use a lazy/closed pool, not a live database health assertion.
- Four live Postgres progress tests and all 45 package doctests passed, including
  17 compile-fail cases. Postgres strict Clippy passed after one final extra
  progress-total assertion. Its isolated live rerun then timed out while its
  Docker fixture remained Created; no SQL assertion executed, and failed-start
  cleanup removed that fixture. Complete final-tree results belong in the owning
  Beads and PR after `scripts/verify.sh`.

Hosted Linux/macOS execution is separate evidence. Fresh-agent downstream
implementation/modification exercises remain unexecuted.
