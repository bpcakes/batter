# Define trusted request metadata and explicit propagation

Owning Bead: `batter-in2`. Implement the reference application boundary that consumes `batter_axum::CorrelationId` without turning diagnostics into authority or request lifetime. Preserve the existing independent `operational_http` / `request_admission` composition and do not duplicate adapter correlation, response-header, observation, or infrastructure-rendering behavior.

## Progress

- [x] Read repository, foundation, Axum, and reference-service guidance; inspect the Bead and verify both blocking dependencies are closed.
- [x] Claim `batter-in2`; confirm the initial working tree is clean.
- [x] Finish current-state/API analysis and freeze the smallest complete application-owned trust model.
- [x] Implement typed request metadata, direct-peer policy, production middleware composition, explicit correlation propagation, and runtime ConnectInfo handoff.
- [x] Add production-seam concurrency, forgery, missing-metadata, nested-operation, authorization, failure, and cancellation tests.
- [x] Update public rustdoc/example, contracts, status, testing, reference README/guide, validation evidence, and the Bead.
- [x] Pass focused checks, both full toolchain matrices, all required HTTP smoke profiles, final Jig gates, and a requirement-by-requirement completion audit.

## Surprises & Discoveries

The prerequisite adapter already replaces inbound and inner response request IDs, emits one retained observation, and supplies `CorrelationId`; the reference already authenticates one configured owner but passes `None` to handler-side infrastructure rendering and has no direct-peer metadata. The native adapter offers an opt-in `ConnectInfo<SocketAddr>` registration path that the reference root does not yet consume. Proxy support and inbound trace-parent handling are optional rather than required; unsupported metadata must be ignored rather than partially trusted.

The accepted socket source port is trustworthy transport data but unstable as a
quota key. `TrustedPeer` therefore retains only the native peer IP. Existing Jig
v9 exhaustive globs already cover the new `src/request.rs` and
`src/http/tests.rs` paths, so no scope or contract edit is needed. Focused library,
doctest, all-target, and warning-denied Clippy checks pass without a dependency
or lockfile change.

## Decision Log

- Use the adapter-owned `CorrelationId` unchanged; add no generator, request-ID type, response-header setter, observer, or infrastructure renderer.
- Keep `OwnerId` as the reference application authority established only by the bearer credential. Request metadata remains a separate type and cannot produce owner authority.
- Support one explicit trusted-peer mode: the native accepted TCP peer supplied by `register_http_with_connect_info_in`. Do not parse `Forwarded`, `X-Forwarded-For`, `X-Real-IP`, `traceparent`, `tracestate`, or client request-ID values. No proxy mode is claimed.
- Do not add task-local or spawned-work helpers because the current reference has no request-owned spawned consumer; explicit extraction and arguments are the contract.
- Exercise middleware without PostgreSQL by factoring the same production business boundary around a test handler; retain actual auth, metadata, admission, operational correlation, rendering, and layer order while substituting no security middleware.
- Retain only the direct peer IP, not its ephemeral source port, in application
  metadata. The full `SocketAddr` remains available at the native trust boundary
  but is not promoted into a stable quota identity.

## Outcomes & Retrospective

The implementation and pre-closure acceptance audit are complete. The reference
root now consumes native direct-peer `ConnectInfo`, the adapter-owned
`CorrelationId`, bearer-selected `OwnerId` and `OperationContext` as separate
types. Handler/domain/authentication bodies and shared infrastructure bodies use
the typed generated correlation while only `operational_http` owns server ID
generation, the response header and HTTP observation.

Requirement-by-requirement evidence:

- Twelve concurrent production-boundary requests carry forged forwarding, trace,
  duplicate request-ID and owner-extension values. Each returns the configured
  owner, expected native peer, unique generated ID and matching nested-operation
  values. The only supported peer mode is direct socket IP; missing native peer
  fails closed and missing optional headers succeeds.
- A paused-time forced-cancellation case holds one request while another
  completes, then proves both IDs remain distinct, the first response/body ID is
  unchanged and only its separate operation context becomes cancelled.
- Production route cases prove generated correlation without the configured
  bearer remains unauthorized and that authenticated domain failure uses the
  same body/header ID before database acquisition.
- Source inspection confirms the application uses only `operational_http` plus
  `request_admission`, has no `observe_http`/`request_scope` nesting, no
  request-owned spawn helper and no server-ID generator/header setter. The
  existing delivery UUID generator is unrelated domain identity.
- Public rustdoc, guarantees, integration/testing/usage/status/reference
  compatibility contracts, package README/guide, primary-source note and exact
  validation evidence are current. Durable correlation, proxy trust, inbound
  trace retention, task-local/spawn propagation and Runlimit remain explicitly
  downstream rather than partially implemented here.

Focused library/doctest/all-target/Clippy checks, both complete Rust matrices and
all ten rebuilt HTTP smoke profiles passed. The initial Jig verify profile passed
all five targets with fresh evidence. Two independent repository explorations
agreed that no adapter API change or quota/trace placeholder belongs in this
task; a focused Rust security-boundary review found no new defect. Tracker
closure and the post-export Jig refresh passed. `batter-in2` is closed with its
scope limits recorded. Final target validation
`receipt_01M2NBPGR420PRVV8NJR1SB0YK` reused the unchanged successful API
Clippy/format/test receipts and refreshed repository contract/file-budget
receipts `receipt_01M2NBPG3NEZMF30FBFRZEM8MH` and
`receipt_01M2NBPG533JSXMRY7C7ETHGPN`. Final evidence is fresh and the required
verify gate has no unresolved item.

## Context and orientation

`examples/reference-service/src/http.rs` owns business routes and middleware. `src/auth.rs` maps an opaque bearer token to `OwnerId`. `src/config.rs` owns prepared HTTP inputs. `src/runtime.rs` owns native listener registration. `crates/batter-axum/src/correlation.rs` owns `CorrelationId`, `operational_http`, and `render_infrastructure_failure`; it must remain unchanged unless a demonstrated adapter defect appears.

## Plan of work and interfaces

Add an application-owned request-metadata module with an opaque trusted direct peer and metadata containing the shared correlation value. A direct-peer policy constructs metadata only from native `ConnectInfo`; it ignores all forwarding/trace/client-ID headers. Carry that policy separately from `BearerAuthenticator` in `PreparedHttp`. Install metadata outside authentication and admission but inside `operational_http`. Authenticate by replacing any preexisting owner extension. Extract metadata, authority, and `OperationContext` separately in handlers; pass metadata explicitly into response/error helpers so bodies agree with the outer generated header and shared infrastructure rendering receives the typed ID. Switch the production runtime to the ConnectInfo serving helper.

Put substantial boundary tests in a dedicated source test module or integration target covered by existing exhaustive Jig globs. Use the exact production boundary around a deterministic handler to prove forged/missing optional headers, direct peers, authority replacement, concurrent isolation, nested operation propagation, cancellation, and body/header identity. Keep actual route tests for authorization and domain failure. No live database is required; live provider/durable persistence remains owned by downstream Beads.

## Validation and acceptance

During iteration run focused reference package tests/doctests/check/clippy. Then run `bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`; build `cargo build -p batter-axum --example http_service --locked` and run `python3 scripts/smoke_http.py --binary target/debug/examples/http_service` in default, `--signal SIGINT`, `--deadline`, `--warn-filter`, and `--warn-filter --deadline` profiles. Inspect Jig evidence/gates and finish with `scripts/jig work check --plan-id <id>` plus fresh evidence. Record exact platform, toolchains, commands, counts, outcomes, lock identity, and unverified proxy/live/hosted boundaries in `docs/validation.md`. Close and sync the Bead only after the final audit.

## Idempotence and recovery

All focused request tests use in-process routers or test-owned loopback resources and bounded synchronization. They require no external database. Preserve unrelated open Jig plans and never reset the worktree. If a check fails, retain its evidence, repair the concrete defect, and rerun the affected scope. Tracker export and final documentation can stale Jig receipts; refresh the inexpensive or required gates after final mutations.

## Dependencies

Use existing workspace Axum, Batter, tower, Tokio, tracing, and UUID versions. Add no dependency unless current source proves it is necessary. Existing Jig input scopes already include all `examples/*/src/**`, `examples/*/tests/**`, docs, and tracker files; update `.jig.toml` and `.agent/jig-contract.json` together only if a new source root escapes those exhaustive scopes.