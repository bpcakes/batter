# Protected quota failure rendering and wire contract

Owning Bead: `batter-7ib`. Git baseline is captured by `scripts/jig work start`. The current worktree contains earlier uncommitted Runlimit work; preserve its index and every unrelated change.

## Progress

- [x] Review and classify four reported findings against source.
- [x] Record cause and alternatives in the owning Bead.
- [x] Implement a request-scoped failure responder owned by `request_admission`; use it for nested quota interruption.
- [x] Remove the quota writer before public probe handlers, without changing their `not_checked` observation.
- [x] Type quota-owned wire rejections; document and test the exact status/code/no-store contract.
- [x] Test every backend consumption projection through protected HTTP.
- [x] Run focused tests, both supported-toolchain verifies, HTTP smokes, Jig gates; record exact outcomes.

## Surprises & Discoveries

- `OperationContext::run` has a biased select but a nested cancellation can still finish the result branch in the same poll; a scratch nested-operation program returned `Ok` after the inner operation reported interruption. The outer `RequestPolicy` renderer therefore cannot be relied on to catch an inner interruption.
- The root quota observer intentionally covers probes, but their route subtree bypasses the boundary that claims its writer.
- A custom admission failure renderer could reconstruct a request from original parts and claim the quota writer before the protected boundary ran. Renderer metadata now excludes that private capability on both immediate and retained interruption paths.
- Axum's `route_layer` panics for an empty router, although `with_public_probes` accepts one. Applying the probe writer middleware with `layer` accepts empty and fallback-only routers while still wrapping declared probe routes.

## Decision Log

- Keep interruption rendering with `RequestPolicy`, which owns the application-selected failure envelope and original request parts. Use one opaque request-scoped capability rather than asking Runlimit to copy failure rendering or request-part capture.
- Keep native quota decisions and storage upstream. Use a closed internal HTTP rejection type for fixed quota wire codes; interruption is not a fixed quota rejection.
- Claim and drop the writer on the explicit public-probe subtree. This preserves `NotChecked` and prevents application probe handlers from fabricating quota facts.
- Keep ordinary request metadata available to failure renderers while removing the private quota observation extension from their snapshots. A renderer has no reason to publish native quota truth.
- Pin `Retry-After` presence and native whole-second values at the HTTP wire boundary, including its absence on 401.

## Outcomes & Retrospective

Implemented and validated locally. The comprehensive source review controller
converged at `.git/jig/review-fix/b2a4b86f-e145-43fb-a003-4908c8890025/`
with two complete terminal reviews and no actionable findings. Its final source
fingerprint was `112dc4a9a803ab46dba111863887024fb9b64dde8526f3f989f9259829cc6a84`.
`cargo test -p batter-runlimit --all-features --locked` passed one internal
response-code test, 28 protected HTTP tests, 14 native quota tests, 3 positive
and 5 compile-fail doctests. `bash scripts/verify.sh` and
`RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` passed, as did all five HTTP
smoke modes and the quota example on each toolchain. Jig work check passed
`api:test` with fresh receipt `receipt_01M2T1HBG88SSBCQ90V25PY337`; Clippy,
formatting, contract and file-budget targets also passed and the `verify`
gate is fresh. The local checks do not establish live PostgreSQL, Linux or
hosted CI behavior. No commit or publication was performed.

## Context and orientation

`crates/batter-axum/src/lib.rs` owns `RequestPolicy` and `request_admission_inner`. `crates/batter-axum/src/quota_observation.rs` owns the single-use quota writer. `crates/batter-runlimit/src/http.rs` composes protected and public routes. `crates/batter-runlimit/tests/http.rs` and `tests/http/` own protected HTTP regressions. `docs/integrations.md`, `docs/status.md`, and `docs/validation.md` record the consumer contract and evidence.

## Plan of work

First add a responder whose constructor is private to Axum admission and whose render method takes `HttpFailure`. It captures exactly the same original request parts as the current failure-renderer path, so nested adapters cannot accidentally render from mutated request metadata. Admission uses it for its own failure, and inserts a clone into the admitted request for the Runlimit boundary. The boundary maps `Interruption` to `HttpFailure` and invokes that responder; a missing responder is an internal integration fault, not a separate quota code. Keep the public integration path opaque.

Then make fixed quota rejections take a closed enum rather than arbitrary strings. Keep the current reachable status/code mapping except for interruption. Wrap explicit public probes in a middleware that claims and discards the writer before application code. Add a dedicated `tests/http/response_contract.rs` module to avoid overgrowing `tests/http.rs`.

## Concrete steps

1. Edit Axum admission and rustdoc with a small example of obtaining and using the responder inside a nested adapter.
2. Edit Runlimit assembly and rejection mapping; preserve `Retry-After`, retained backend/auth errors, telemetry and allowed behavior.
3. Add deterministic tests for inner cancellation and deadline, configured JSON renderer, probe writer inaccessibility, exact quota wire codes/no-store, and all backend consumption statuses. Use generic scenario names.
4. Update integration/status/validation contracts and this plan as evidence becomes available. Update Jig exhaustive input scopes only if new source roots are introduced; files under existing globbed test roots need no scope edit.
5. Run controller validation, Jig work checks, inspect final diff, and close the Bead after acceptance.

## Validation and acceptance

Required: `cargo test -p batter-runlimit --all-features --locked`; `bash scripts/verify.sh`; `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`; both-toolchain HTTP smoke profiles and quota example; `scripts/jig work check --plan-id <id>`, inspect evidence/gates, and require current `api:test`. The final inventory has 28 protected HTTP tests, 14 quota tests and 8 doctests. No live PostgreSQL/Linux/hosted CI claim.

## Idempotence and recovery

The review-fix controller applies only its journaled patch and preserves the index. If a candidate fails validation, use the persisted assignment and controller recovery, not a manual partial overlay. The Bead and Jig plan remain in progress until final checks pass; do not commit or publish.

## Interfaces and dependencies

No new dependency is expected. `batter-runlimit` may call a public, opaque Axum responder method; Axum must not depend on Runlimit. Existing native `runlimit-core` revision and Unix-only scope remain fixed.
