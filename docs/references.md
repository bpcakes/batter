# Primary references and verification boundaries

These checks record upstream semantics used by current contracts. They do
not establish that Batter tests passed. Each record retains its actual
inspection date and version; re-check `Cargo.lock` and pinned source when
changing an integration. Superseded pin investigations and repair diaries
have been removed; Git retains their history.

[Documentation index](README.md) · [Verification](testing.md#verification)

## Dependency selection and Effect v4 rationale

- [OTLP attempt callback boundary, 2026-11-09](references/runtime.md#otlp-attempt-callback-boundary-rechecked-2026-11-09)
- <a id="dependency-refresh-2026-11-04"></a>[Dependency refresh, 2026-11-04](references/ecosystem.md#dependency-refresh-2026-11-04)
- <a id="macos-refusal-fixture-repair"></a>[macOS refusal fixture repair](references/ecosystem.md#macos-refusal-fixture-repair)
- <a id="facade-feature-and-resolver-semantics-reviewed-2026-09-18-resolver-rechecked-2026-09-30"></a>[Facade feature and resolver semantics: reviewed 2026-09-18; resolver rechecked 2026-09-30](references/ecosystem.md#facade-feature-and-resolver-semantics-reviewed-2026-09-18-resolver-rechecked-2026-09-30)
- <a id="effect-v4"></a>[Effect v4](references/ecosystem.md#effect-v4)
- <a id="boundary-conventions-reviewed-2026-09-08"></a>[Boundary conventions reviewed: 2026-09-08](references/ecosystem.md#boundary-conventions-reviewed-2026-09-08)

## Database fixtures and live probes

- [Native SQLx URL compatibility](references/fixtures.md#native-live-url-compatibility-2026-09-10)
- <a id="fixture-login-pools-2026-10-31"></a>[Fixture login pools (2026-10-31)](references/fixtures.md#fixture-login-pools-2026-10-31)

## Inbound HTTP, browser policy and serving

- <a id="http11-transport-ownership-reviewed-2026-09-10"></a>[HTTP/1.1 transport ownership, reviewed 2026-09-10](references/http.md#http11-transport-ownership-reviewed-2026-09-10)
- <a id="http11-lifetimes-reviewed-2026-09-10"></a>[HTTP/1.1 lifetimes, reviewed 2026-09-10](references/http.md#http11-lifetimes-reviewed-2026-09-10)
- <a id="axum-operational-defaults-exact-native-apis-2026-09-10"></a>[Axum operational defaults: exact native APIs (2026-09-10)](references/http.md#axum-operational-defaults-exact-native-apis-2026-09-10)
- <a id="browser-credential-transport-2026-09-15"></a>[Browser credential transport, 2026-09-15](references/http.md#browser-credential-transport-2026-09-15)
- <a id="private-response-referrer-choice-2026-09-27"></a>[Private-response referrer choice, 2026-09-27](references/http.md#private-response-referrer-choice-2026-09-27)
- <a id="generic-serving-listeners-and-connection-metadata-2026-09-30"></a>[Generic serving listeners and connection metadata, 2026-09-30](references/http.md#generic-serving-listeners-and-connection-metadata-2026-09-30)
- <a id="optional-rendered-fallback-2026-09-30"></a>[Optional rendered fallback (2026-09-30)](references/http.md#optional-rendered-fallback-2026-09-30)
- <a id="sealed-assembly-and-one-time-client-preparation-2026-10-05"></a>[Sealed assembly and one-time client preparation (2026-10-05)](references/http.md#sealed-assembly-and-one-time-client-preparation-2026-10-05)
- <a id="postgresql-session-profile-batching-and-idle-authority-2026-10-06"></a>[PostgreSQL session-profile batching and idle authority (2026-10-06)](references/http.md#postgresql-session-profile-batching-and-idle-authority-2026-10-06)

## Native Runledger and Runlimit integration

- <a id="native-postgresql-docker-dependency-removal-reviewed-2026-11-06"></a>[Native PostgreSQL Docker dependency removal: reviewed 2026-11-06](references/native-runtime.md#native-postgresql-docker-dependency-removal-reviewed-2026-11-06)
- <a id="runledger-claim-write-batching-2026-10-14"></a>[Runledger claim-write batching (2026-10-14)](references/native-runtime.md#runledger-claim-write-batching-2026-10-14)
- <a id="runledger-online-migration-failure-handling-2026-10-25"></a>[Runledger online migration failure handling (2026-10-25)](references/native-runtime.md#runledger-online-migration-failure-handling-2026-10-25)

## Outbound HTTP transport

- <a id="reqwest-consolidation-2026-11-04"></a>[Reqwest consolidation, 2026-11-04](references/outbound-http.md#reqwest-consolidation-2026-11-04)
- <a id="owned-outbound-http1-connections-2026-11-04"></a>[Owned outbound HTTP/1 connections, 2026-11-04](references/outbound-http.md#owned-outbound-http1-connections-2026-11-04)
- <a id="public-https-address-policy-2026-11-05"></a>[Public HTTPS address policy (2026-11-05)](references/outbound-http.md#public-https-address-policy-2026-11-05)
- <a id="outbound-response-body-storage-2026-11-09"></a>[Outbound response-body storage (2026-11-09)](references/outbound-http.md#outbound-response-body-storage-2026-11-09)

## PostgreSQL execution, managed objects and verification

- [PostgreSQL 18 GCRA response timing (2026-11-08)](references/postgresql.md#postgresql-18-gcra-response-timing-2026-11-08)

- <a id="postgresql-18-guarded-index-removal-2026-10-27"></a>[PostgreSQL 18 guarded index removal, 2026-10-27](references/postgresql.md#postgresql-18-guarded-index-removal-2026-10-27)
- <a id="postgresql-18-scoped-managed-apply-2026-10-26"></a>[PostgreSQL 18 scoped managed apply, 2026-10-26](references/postgresql.md#postgresql-18-scoped-managed-apply-2026-10-26)
- <a id="postgresql-18-managed-routine-and-trigger-semantics-2026-10-15"></a>[PostgreSQL 18 managed routine and trigger semantics, 2026-10-15](references/postgresql.md#postgresql-18-managed-routine-and-trigger-semantics-2026-10-15)
- <a id="postgresql-18-constraint-trigger-semantics-2026-10-16"></a>[PostgreSQL 18 constraint trigger semantics, 2026-10-16](references/postgresql.md#postgresql-18-constraint-trigger-semantics-2026-10-16)
- <a id="postgresql-cleanup-prepared-plan-selection-2026-10-12"></a>[PostgreSQL cleanup prepared-plan selection, 2026-10-12](references/postgresql.md#postgresql-cleanup-prepared-plan-selection-2026-10-12)
- <a id="postgresql-column-privileges-for-key-identified-cleanup-2026-10-11"></a>[PostgreSQL column privileges for key-identified cleanup, 2026-10-11](references/postgresql.md#postgresql-column-privileges-for-key-identified-cleanup-2026-10-11)
- <a id="postgresql-18-grant-semantics-for-native-requirement-fragments-2026-10-03"></a>[PostgreSQL 18 grant semantics for native requirement fragments, 2026-10-03](references/postgresql.md#postgresql-18-grant-semantics-for-native-requirement-fragments-2026-10-03)
- <a id="native-query-adapters-reviewed-2026-09-22"></a>[Native query adapters: reviewed 2026-09-22](references/postgresql.md#native-query-adapters-reviewed-2026-09-22)
- <a id="fail-fast-transaction-recovery-reviewed-2026-09-22"></a>[Fail-fast transaction recovery: reviewed 2026-09-22](references/postgresql.md#fail-fast-transaction-recovery-reviewed-2026-09-22)
- <a id="declared-transaction-timeouts-reviewed-2026-09-22"></a>[Declared transaction timeouts: reviewed 2026-09-22](references/postgresql.md#declared-transaction-timeouts-reviewed-2026-09-22)
- <a id="postgresql-18-schema-and-runtime-authority-verification-2026-09-13"></a>[PostgreSQL 18 schema and runtime-authority verification, 2026-09-13](references/postgresql.md#postgresql-18-schema-and-runtime-authority-verification-2026-09-13)
- <a id="owned-execution-and-downstream-policy-correction-2026-09-13"></a>[Owned execution and downstream policy correction, 2026-09-13](references/postgresql.md#owned-execution-and-downstream-policy-correction-2026-09-13)
- <a id="catalog-resolution-and-dependent-type-acl-correction-2026-09-13"></a>[Catalog resolution and dependent type ACL correction, 2026-09-13](references/postgresql.md#catalog-resolution-and-dependent-type-acl-correction-2026-09-13)
- <a id="protected-postgresql-catalog-safeguards-2026-09-14"></a>[Protected PostgreSQL catalog safeguards, 2026-09-14](references/postgresql.md#protected-postgresql-catalog-safeguards-2026-09-14)
- <a id="exact-role-grant-rendering-2026-09-13"></a>[Exact-role grant rendering, 2026-09-13](references/postgresql.md#exact-role-grant-rendering-2026-09-13)
- <a id="managed-object-release-fence--2026-11-05"></a>[Managed-object release fence — 2026-11-05](references/postgresql.md#managed-object-release-fence--2026-11-05)
- <a id="intent-reconciliation-snapshot-and-grants-2026-11-05"></a>[Intent reconciliation snapshot and grants (2026-11-05)](references/postgresql.md#intent-reconciliation-snapshot-and-grants-2026-11-05)
- <a id="shared-profiled-pool-ownership-2026-11-05"></a>[Shared profiled pool ownership (2026-11-05)](references/postgresql.md#shared-profiled-pool-ownership-2026-11-05)

## Runtime, ownership, telemetry and settings

- [VictoriaMetrics v1.156.0: reviewed 2026-11-09](references/runtime.md#victoriametrics-v11560-reviewed-2026-11-09)

- <a id="native-cleanup-acquisition-2026-11-05"></a>[Native cleanup acquisition, 2026-11-05](references/runtime.md#native-cleanup-acquisition-2026-11-05)
- <a id="rust-1981-panic-hooks-for-the-redacting-hook-2026-11-02"></a>[Rust 1.98.1 panic hooks for the redacting hook, 2026-11-02](references/runtime.md#rust-1981-panic-hooks-for-the-redacting-hook-2026-11-02)
- <a id="tokio-1531-blocking-tasks-for-bounded-blocking-work-2026-10-31"></a>[Tokio 1.53.1 blocking tasks for bounded blocking work, 2026-10-31](references/runtime.md#tokio-1531-blocking-tasks-for-bounded-blocking-work-2026-10-31)
- <a id="metrics-facade-0246-reviewed-2026-09-25"></a>[Metrics facade 0.24.6: reviewed 2026-09-25](references/runtime.md#metrics-facade-0246-reviewed-2026-09-25)
- <a id="reference-metrics-export-reviewed-2026-09-26"></a>[Reference metrics export: reviewed 2026-09-26](references/runtime.md#reference-metrics-export-reviewed-2026-09-26)
- <a id="tokio-and-tokio-util"></a>[Tokio and Tokio-util](references/runtime.md#tokio-and-tokio-util)
- <a id="periodic-maintenance-scheduling-semantics-2026-10-08"></a>[Periodic maintenance scheduling semantics, 2026-10-08](references/runtime.md#periodic-maintenance-scheduling-semantics-2026-10-08)
- <a id="runledger-invocation-phases-2026-10-09"></a>[Runledger invocation phases, 2026-10-09](references/runtime.md#runledger-invocation-phases-2026-10-09)
- <a id="retry-attempt-deadline-boundaries-2026-09-14"></a>[Retry attempt deadline boundaries, 2026-09-14](references/runtime.md#retry-attempt-deadline-boundaries-2026-09-14)
- <a id="service-and-finite-command-examples-2026-09-10"></a>[Service and finite-command examples: 2026-09-10](references/runtime.md#service-and-finite-command-examples-2026-09-10)
- <a id="canonical-adapter-future-size-and-depth-2026-09-28"></a>[Canonical adapter future size and depth, 2026-09-28](references/runtime.md#canonical-adapter-future-size-and-depth-2026-09-28)
- <a id="async-startup-initializers-and-boxed-errors-2026-10-11"></a>[Async startup initializers and boxed errors, 2026-10-11](references/runtime.md#async-startup-initializers-and-boxed-errors-2026-10-11)
- <a id="finite-command-signals-and-owned-completion-2026-10-31"></a>[Finite command signals and owned completion (2026-10-31)](references/runtime.md#finite-command-signals-and-owned-completion-2026-10-31)

## Build, verification and repository tooling

- <a id="cargo-test-artifacts-and-profiles-2026-09-24"></a>[Cargo test artifacts and profiles, 2026-09-24](references/tooling.md#cargo-test-artifacts-and-profiles-2026-09-24)
- <a id="rust-minimum-toolchain-reviewed-2026-11-02"></a>[Rust minimum toolchain: reviewed 2026-11-02](references/tooling.md#rust-minimum-toolchain-reviewed-2026-11-02)
- <a id="github-actions-ci-scope-selection-reviewed-2026-11-02"></a>[GitHub Actions CI scope selection: reviewed 2026-11-02](references/tooling.md#github-actions-ci-scope-selection-reviewed-2026-11-02)
- <a id="workspace-packaging-reviewed-2026-09-08"></a>[Workspace packaging: reviewed 2026-09-08](references/tooling.md#workspace-packaging-reviewed-2026-09-08)
- <a id="jig-prebuilt-release-runtime-2026-10-04"></a>[Jig prebuilt release runtime: 2026-10-04](references/tooling.md#jig-prebuilt-release-runtime-2026-10-04)
