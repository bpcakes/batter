# External consumer sources

Standalone Rust sources compiled by temporary consumer manifests outside this
checkout. They are not workspace members and have no `Cargo.toml` here; they
exist so the manifests those checks generate stay reviewable source in Git.

- `single_facade_consumer.rs` is the acceptance consumer for the
  single-dependency recipe in [reference compatibility](../docs/reference-compatibility.md).
  Its `JobExecutionHandler` runs under `batter::runledger::job_phases`, and an
  independent task per invocation must observe the invocation's exit cancel the
  derived work phase before the run counts as complete.
  Work and recording interruptions have their own timeout-kind diagnostics;
  only application failures report an invalid payload or a closed observer.
  Its paused-time job tests expire work while the native reserve remains and
  cover finalization expiry, cancellation, application failures and success.
  It must keep declaring only `batter` plus ordinary registry crates: adding any
  direct native-workspace dependency or a `[patch]` section would remove exactly
  the property it proves.
- `single_facade_harness.rs` owns the disposable PostgreSQL 18 database through
  Runledger's existing native test support and runs the consumer against it. It
  is built from its own manifest so the consumer keeps proving it needs no test
  support, and it reaches that support through the facade's
  `runledger-test-support` feature, so it declares only `batter` as well. Its
  `test-support` feature selects the existing generic result combiner, not
  database provisioning.

`scripts/check_single_facade_consumer.py` builds both from a Git-free source
copy, asserts the consumer's resolved graph, and requires each executed marker.
It checks formatting through each temporary manifest and denies package-local
compiler warnings in both programs; copied dependencies retain their own policy.
The consumer's private `single_facade_completion.rs` retains the work result
while awaiting checked shutdown. Protected startup owns the pool finalizer, so
failed or uncertain settlement cannot bypass conservative cleanup skipping.
`single_facade_completion_tests.rs` exercises error retention and ordering; the
runner executes it from the same external manifest before the live consumer.
`single_facade_quota.rs` checks the application's expected admission and denial
while retaining every unexpected native outcome in a redacted error. Its tests
cover native backend causes, consumption certainty, interruption progress and
a closed-pool failure through checked shutdown, including cleanup failure.
The harness's private `single_facade_harness_completion.rs` classifies launch
errors and unsuccessful child status, then uses `batter::test_support::finish`
to retain execution and teardown outcomes together. Its redacted report keeps
both causes inspectable and renders exit status without printing error contents.
`single_facade_harness_tests.rs` covers both dual failures, signal status, single
failures and success; the runner executes it before provisioning the live database.
All temporary Cargo builds force `SQLX_OFFLINE=true`, independent of an ambient
`DATABASE_URL`; runtime database selection still belongs to the harness.
`scripts/test_single_facade_consumer.py` covers that script's own assertions.
Runlimit's direct-package consumers stay in [`runlimit/smoke`](../runlimit/smoke);
they prove a different contract and still declare native packages on purpose.
