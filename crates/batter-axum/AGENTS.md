# Axum adapter guide

## Purpose

Translate the foundation's operational contracts into Axum request and probe
behavior. Own validated browser credential transport mechanics while keeping
credential meaning, authentication, authorization, CORS, business logic, and
wire-envelope policy at the composition root.

Follow the root [Unix-only platform policy](../../AGENTS.md#platform-scope).
Windows support and non-Unix fallbacks are out of scope.

## Key entrypoints

- `src/lib.rs` contains the supported policy and value types: `RequestPolicy`,
  `ResponseConstructionBudget`, `HttpObservationLevel`, `HttpFailure` and
  `RequestInterruptionResponder`. It defines no request middleware.
- `src/low_level.rs` is the public `low_level` namespace: the physically
  relocated `observe_http`, `request_admission`, the combined `request_scope`
  compatibility entry point and the status-only `readiness`/`liveness` probe
  handlers, plus re-exports of `operational_http`,
  `operational_http_with_quota`, `dependency_readiness` and the three
  `register_http*` helpers. These eleven items have no crate-root alias; keep
  them reachable only here, and keep the supported types, `AdmittedRequest`,
  `CorrelationId`, `render_infrastructure_failure`, `readiness_status`,
  `default_readiness_level`, the `browser` primitives and `quota_observation`
  at the root, where the canonical path uses them.
- `src/admitted.rs` owns the private admission record, the `AdmittedRequest`
  extractor that guarded handlers use instead of raw extensions and its
  sanitized `AdmittedRequestRejection`.
- `src/boundary.rs` owns the canonical `HttpBoundary`/`AssembledHttp`
  composition, sealed so assembly is consumed only into protected registration
  or `in_process()`, and `src/boundary/client.rs` owns the resulting opaque
  `InProcessClient`, including the one-time router preparation and the
  compile-fail controls that reject recovering, mutating or serving it.
  `src/boundary/guarded.rs` owns the `GuardedRouter` builder. Probe
  routes remain outside admission while the guarded router's default, custom,
  nested, and method fallbacks remain inside it. `src/boundary/fallback.rs`
  provides the alternative metadata-only unmatched-path renderer outside
  admission; assembly rejects combining it with guarded root/nested fallbacks. `src/boundary/assembly.rs`
  owns validation order and group composition; `src/boundary/declared.rs` owns
  `RouteInventory`, the inspection copy of a router admitted through
  `GuardedRouter::from_router` and request dispatch to its declared routes.
  `src/boundary/group.rs` owns `RouteGroup`, `GroupPolicy`, `BrowserPolicy` and
  the per-group layer order; `src/boundary/pattern.rs` decides whether two
  retained route patterns share a request path under the pinned matchit 0.8.4
  rules, and `src/boundary/inventory.rs` confirms probe and group collisions
  through inert routers. `src/boundary/probe.rs` owns `ProbePath` and the
  handlers of application-rendered probes.
- `src/browser.rs` and `src/browser/` own trusted browser-origin validation,
  duplicate-aware named-cookie transport, exact mutation-signal checks, and
  private-response headers with a typed same-origin/no-referrer choice. They do
  not own account/session state, CSRF token protocols, route selection, CORS,
  or application error rendering.
- `src/observation.rs` privately owns response observation and tracing lifetime;
  its single internal composition entry has no admission policy. Its shared
  private state lets nested adapter middleware contribute retained facts to
  the one outer completion event.
- `src/correlation.rs` owns opt-in `operational_http` (published through
  `low_level`), generated `CorrelationId`
  and the standard infrastructure renderer; it composes the existing observer once.
  Its private operational marker carries the generated identity that admission
  records. Its private `renderer_parts` filter removes quota, observation and
  operational ownership state and the admission record at every application
  renderer boundary, retaining public metadata.
- `src/quota_observation.rs` owns bounded facts and a single-take writer for
  `low_level::operational_http_with_quota`; its consuming start/finish states prevent terminal
  facts from being downgraded. Native quota execution belongs in batter-runlimit.
- `src/readiness.rs` translates the foundation's valid readiness decision into
  HTTP status, response extensions and observation severity, and applies them
  after an application probe renderer returns.
- `src/serving.rs` registers any bound `axum::serve::Listener` plus a router with
  the supervisor, including the listener's opt-in direct peer `ConnectInfo`
  through protected authority. Transport policy stays with the listener.
- `../batter/examples/http_service.rs` demonstrates adoption of these public helpers.
  `http_service/config.rs` owns its explicit file/environment settings and
  configured router capacity; example tests run in normal Cargo discovery.
- `tests/http.rs`, `tests/telemetry.rs`, `tests/observation.rs` and
  `tests/scoped_dispatch.rs` cover failures, complete-router observations,
  middleware placement, and future destruction. `tests/operational/` covers forged/concurrent IDs,
  all readiness reasons, native startup/drain and a body surviving wrapper abort;
  `tests/operational/rendered_probes.rs` covers application-rendered probes
  outside admission, every decision through a contradicting renderer and
  application conditions. `tests/operational/admitted_request.rs` covers the
  extractor behind every group, its rejection outside admission and in both
  unsupported lower-level orders, and forged or replaced raw extensions.
- `tests/tls_serving.rs` and `tests/tls_serving/` serve the canonical assembled
  boundary over a test-owned rustls listener with a generated certificate. They
  own the certificate, provider, accept loop and handshake, because those are
  application policy; keep them out of `src/`. They cover acknowledgement before
  readiness, the transport peer, drain with a connection still streaming,
  destruction of an accept parked in its handshake, and release of rejected and
  abandoned listeners.
- `tests/browser.rs` and `tests/browser/` cover the public browser transport
  matrices, sanitized failures, Set-Cookie append/removal, mutation precedence,
  and real Axum private-response layer placement, both referrer choices,
  overwrite and non-weakening nesting.
- `tests/route_groups.rs` and `tests/route_groups/` cover the consumer-shaped
  default/upload/private-browser composition, per-group deadlines, browser
  rejections and headers, rejected overlaps, probes inside groups, invalid
  groups, and one correlated observation, scoped dispatch and context
  cancellation across groups. The pattern unit tests compare the overlap
  analysis with native Axum routing over every short path a witness can use.
  `tests/route_groups/admitted.rs` and `admitted_validation.rs` cover admitted
  routers: declared routes inside their group's policy, unreachable undeclared
  routes and fallbacks, layers reused across served requests, and probe,
  inventory and overlap rejections without polling application code. The
  pattern unit tests also compare the parser with Axum's own route
  registration.
- `tests/http_lifetime.rs` and `tests/http_lifetime/` own real HTTP/1.1 socket,
  handler/body, direct-server and cleanup comparisons. They reuse only private
  workspace `test-support/process/` mechanics, never another package's fixtures
  or self-tests; keep report inspection and later body release separate.
- `tests/http_lifetime_observations.rs` and `tests/http_lifetime_observations/` own real HTTP/1.1
  connection/upload/stream/disconnect milestones through shutdown. Both targets
  use `tests/support/http_process.rs` for PID-bound launch and scenario completion,
  plus the shared Unix watchdog, including under focused Cargo discovery. The
  observation `driver.rs` owns startup/exercise/teardown; both targets use shared
  complete-phase budgets. Keep report and running ownership outside reconciliation
  and timeout the exercise future inside its joined task. Terminal report waits
  and dependent assertions belong to teardown; only the blocked-body abort case
  exposes a deliberate intermediate observer checkpoint. Retain event/wire wait
  diagnostics on cancellation without competing private timers; retain capture
  in the driver and recheck handler-entry counts after terminal shutdown.
- `tests/support/http_graceful.rs` observes the resolved native connection event
  for both lifetime targets. Keep its current-thread requirement and fail on
  missing native evidence; the signal producer is not connection acknowledgement.

## Edit here for X

Change HTTP policy and rendering here. Change operation/lifecycle semantics in
`crates/batter-core`. Keep `RequestPolicy`'s combined readiness and deadline contract unless
a separately approved API change calls for decoupling. Update the root HTTP
contract, source map and implemented status, then record executed checks in the owning Bead.
Keep raw durations outside `RequestPolicy::new`; validation belongs to the opaque
adapter-owned `ResponseConstructionBudget` witness.

## Invariants

The adapter depends on the foundation, never the reverse. Use
`batter_core::telemetry::with_current_dispatch` inside the async request entrypoint to
retain first-poll capture and protect full future destruction. Keep observation
guards and nested spans inside the wrapped future. Do not duplicate its private
pin/drop implementation. Nested Batter observers intentionally emit once; keep
adapter facts in the library-owned shared observation state so wrapper order
cannot discard them. Bound response construction without claiming body
streaming or detached connection-task shutdown. Keep probe routes separate from
guarded business routes. Serving accepts any `axum::serve::Listener`; never move
certificate, provider, protocol-version, ALPN or handshake policy into the
adapter, and never claim that a listener's own spawned work was joined.
Canonical probes accept only validated `ProbePath`
values, and duplicate paths must fail through the boundary's sanitized
configuration error before Axum routing. The canonical guarded builder must
retain route identities through route, merge and typed nesting; do not admit an
opaque nested service whose routes cannot be inspected. Awaited assembly must
reject any guarded route that can match a reserved probe path by querying only
the inert retained inventory or an admitted router's inspection copy, without
polling application code. A router built outside `GuardedRouter` joins only
through `GuardedRouter::from_router` with a `RouteInventory`. Learn its routes
only through an inspection copy whose every route, method fallback and fallback
is replaced by a library reporter; never call the router itself to validate it.
Reject a declared pattern unless a path of it reaches the route registered with
exactly that pattern, and keep the pattern parser accepting exactly the patterns
the pinned Axum router registers. Never merge an admitted router into the
native router: offer it only requests that reach the native router's root
fallback, where no capture has been added, keep the default group's fallbacks
behind that dispatch, serve it, inside its group's policy and its own observer,
only for requests its inspection copy routes to a declared pattern, and keep its
declared patterns disjoint from probes, other groups and its own group's other
routes. Prepare each admitted router, inspection copy and moved fallback router
once at assembly so requests never rebuild their layers, and leave native routes
to Axum's own preparation. Do not
reintroduce raw probe patterns. Route groups keep one fixed order: correlation
and the single observer outermost, then per group the private-response headers,
admission with the group budget, mutation checks on non-safe methods, and the
application's layers. Only the default group owns root or nested fallbacks and
is merged last. A boundary may instead select one rendered fallback outside
admission, reached only after all native and declared routes fail to match.
Reject combining it with a declared guarded fallback. Renderers receive no body
or private ownership state; prefix selection and response headers are application
policy, using PrivateResponsePolicy::apply when appropriate. Matched method
fallbacks remain guarded. Group routes must not share a request path in any method; keep
overlap decisions confirmed by native routing of each pattern alone when its
witness survives request URI construction unchanged; otherwise reject the
overlap before native merging. Update
the pattern analysis and its Axum comparison test whenever the Axum or matchit
pin changes. Apply `low_level::observe_http` after
assembling routes/fallback; use `low_level::request_admission` inside it. If
generated correlation is required, `low_level::operational_http` must be outside
admission, `request_scope`, deadlines, authentication and any other
short-circuiting middleware. Keep the eleven relocated helpers out of the crate
root: a root alias would make a known-invalid composition look equivalent to the
protected path, which ADR-010 forbids. `AssembledHttp` must keep exactly two
outcomes, and `InProcessClient` must keep its router private, implement no Tower
service and add no route, layer or serving conversion. Prepare that router once,
with the same `Router::with_state(())` preparation the pinned Axum
make-service conversions perform, and recheck the pin when it changes. An
in-process response is not serving evidence; keep real socket and TLS tests. The outermost observer owns emission and nested observation
entrypoints perform only their other duties.
Do not log cause contents or untrusted request fields. `operational_http` must
replace inbound header, Tower and adapter identities before observation and must
replace inner response IDs. Keep typed server correlation on completion events
when INFO spans are disabled. Construct test dispatches through private
`test-support/dispatch.rs`; its OFF-filtered inert registration must not enable
macros before the first real dispatcher rebuild or change thread/global selection.
This includes raw subscriber arguments converted implicitly by `with_subscriber`.
Readiness defaults: Starting/Draining INFO; dependency failures while Ready and
Stopped WARN. Existing status-only probes and Problem JSON remain compatible.
Carry the foundation `ReadinessDecision` in response extensions; do not recreate
lifecycle/health classification or accept a broad `HealthStatus` as a failure.
ReadinessPolicy::lifecycle_only uses no dependency monitor; lifecycle and
application conditions still constrain the foundation decision.
Application readiness conditions narrow that foundation decision; never evaluate
them in the adapter. A probe renderer chooses only the body and headers: apply
the readiness status, decision extension and severity, or the liveness 200
and default severity, after it returns so no renderer can alter them, and reserve a rendered probe's
path in the same duplicate and guarded-route checks outside admission.
Keep `readiness_status` and `default_readiness_level` as the canonical reusable
adapter mappings. The unready payload is named `ReadinessUnreadyReason`; do not
introduce a `ReadinessReason` alias or re-export. Pre-cutover extension lookups
must fail loudly rather than compile and miss the new decision extension.
Observation severity overrides are explicit response extensions, independent of
admission. Preserve actual status/outcome and the default WARN for dropped
futures. No application callback belongs in the observation guard's destructor.
Use the shared metadata filter for probes, failure/interruption renderers and
browser rejections. Removing only the quota writer leaves cloned metadata able
to replace the original observer's facts through ordinary middleware redispatch.
Admission records the admitted request only inside the operational wrapper that
owns the request, with the identity from that wrapper's private marker, never
the public `CorrelationId` extension. Keep the record's type private, keep
`AdmittedRequest` without `Clone` or a public constructor, read only the record
in its extractor, never raw extensions, and keep its rejection the sanitized
`HttpFailure::Internal` response. The renderer filter must drop the record so
redispatched metadata is never admitted. Raw `OperationContext` and
`RequestInterruptionResponder` extensions stay for adapters and existing
handlers; do not document them as the handler path.
Keep sanitized HTTP completion fields on the event independently of span filtering.
The all-targets test gate includes the example's live readiness tests and requires
loopback socket permission. Preserve both enabled-event and filtered-event assertions.
Select execution/event context once at first poll, retaining the HTTP span or its
available application parent. Never discover a fallback parent at Drop or record
HTTP fields into the application span. Handler panics propagate; an unwind before
a response is observed as dropped, with no invented status or logged panic payload.
Browser origins come only from validated operator configuration, never Host or
forwarding headers. Scan every Cookie field and reject target ambiguity. Keep
cookie/url dependency types private. Mutation checks are browser signals, not
authentication or complete CSRF protection. Apply one `PrivateResponsePolicy`
layer, or the same-origin compatibility `private_response`, outside application
rejection middleware and inside `low_level::observe_http`; an outer short-circuit cannot
be retroactively decorated. Keep `no-store` and `nosniff` fixed, keep the
referrer choice limited to policies that send nothing cross-origin, and never
weaken an existing all-`no-referrer` field.

## Common commands

Run from the workspace root:

```sh
cargo test -p batter-axum --locked
cargo build -p batter --features axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/test_matrix.py workspace
```
