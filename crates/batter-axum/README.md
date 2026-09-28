# batter-axum

Axum 0.8 integration for the `batter` Tokio foundation. Import its API from
`batter_axum`. This separate dependency provides request readiness/deadline
policy, operation context extensions, sanitized infrastructure failure rendering,
HTTP observations, trusted server correlation, dependency readiness and supervised
native serving. The `browser` module separately provides validated browser-cookie,
mutation-signal and private-response header mechanics.

This crate follows the workspace's [Unix-only platform policy](../../README.md#platform-support).
Windows is unsupported and not planned.

## Browser credential transport

`batter_axum::browser` keeps opaque browser credential transport mechanical and
leaves credential meaning application-owned. `BrowserOrigin::https` accepts one
canonical HTTPS origin; `BrowserOrigin::loopback_http` is the explicit local-only
HTTP path. Both require explicit `scheme://authority` input and reject reverse
solidus or raw non-root paths before URL recovery/normalization. `BrowserCookie`
always emits a host-only `Path=/` cookie, requires an
exact `__Host-` name and Secure under HTTPS, rejects browser-reserved secure
prefixes under loopback HTTP, enforces HttpOnly for `__Host-Http-`, and makes
SameSite, HttpOnly visibility, and lifetime explicit. Setting and removal
append `Set-Cookie` fields instead of replacing siblings, but reject an
existing field with the same case-sensitive cookie name before mutation.

`read_cookie` examines every Cookie field, rejects duplicate target names and
non-UTF8 fields, and returns the unchanged borrowed opaque value. `Strict`
uses Batter's narrower unquoted token/value subset for every pair, so it is
appropriate only when the application controls the whole cookie jar and can
reject otherwise RFC-valid quoted values. `TargetOnly` ignores invalid
unrelated pairs while retaining all target and ambiguity checks. An empty target
remains a present opaque value for the application credential parser to classify.

For an exact-origin JSON surface:

```rust
use batter_axum::browser::{BrowserOrigin, FetchSitePolicy, MutationPolicy};

fn build_policy() -> Result<MutationPolicy, Box<dyn std::error::Error>> {
    Ok(MutationPolicy::exact_origin(BrowserOrigin::https(
        "https://app.example",
    )?)
    .with_fetch_site(FetchSitePolicy::RejectCrossSite)?
    .require_json()?)
}
```

For a custom marker, start from
`MutationPolicy::required_header(RequiredHeader::new(...)?)`. That constructor
automatically requires exactly one `Sec-Fetch-Site: same-origin`; adding a
marker to an exact-origin policy also strengthens any compatible Fetch Metadata
check to that strict mode. The marker can therefore never be the only cross-site
defense. Every configured category is required; checks always run in Origin,
Fetch Metadata, marker, then content-type order. Marker construction rejects
CORS-safelisted and browser-controlled names, plus values that Fetch would
normalize before sending. An accepted name is script-settable and non-safelisted,
but that alone does not prove browser provenance or preflight: user agents can
attach other non-safelisted fields. A marker policy fails closed when Fetch
Metadata is absent, so its mutation URL must be potentially trustworthy and its
supported browsers must emit `Sec-Fetch-Site`. The optional JSON check parses
the complete RFC 9110 media type and parameter byte grammar, not merely an
`application/json` prefix.

On the canonical path, a route group's `BrowserPolicy` installs the private
headers and mutation checks; see [route groups](#http-boundary-and-route-groups).
In a manual composition, wrap assembled private routes and their fallback with one
`.layer(browser::PrivateResponsePolicy::...)`, choosing the referrer policy
deliberately for that route group. Every policy overwrites
`Cache-Control: no-store` and `X-Content-Type-Options: nosniff`; only the two
referrer policies that never send referrer information cross-origin are
representable. `SameOriginReferrer`, the default, keeps the serialized `Origin`
on same-origin HTML form posts, as `MutationPolicy::exact_origin` requires; a
non-CORS form post to a different origin carries `Origin: null`. `NoReferrer`
also withholds same-origin referrers, so every HTML form post, even a
same-origin one, carries `Origin: null` and fails an exact-origin check; use
CORS-mode `fetch` mutations or a custom-marker policy on those pages.
`Sec-Fetch-Site` is unaffected. The field only sets the document's initial
policy; page markup and `fetch` options can still change it. Applying either
policy keeps an existing all-`no-referrer` field, so an outer layer cannot
weaken an inner `NoReferrer` choice. The compatibility
`middleware::from_fn(browser::private_response)` applies `SameOriginReferrer`.
Put mutation rejection inside the layer so rejection responses receive the
headers, and put `observe_http` outside it to observe the final
application-selected status once. An outer short-circuit that does not call the
private layer cannot receive its headers.

These helpers do not authenticate a caller, distinguish a browser from a
non-browser client, select routes, configure CORS or
proxies, create or compare CSRF tokens, parse application credentials, revoke
server state, or define response bodies. SameSite and Fetch Metadata remain
defense-in-depth signals; `Cache-Control: no-store` is not a complete privacy
guarantee.

## HTTP boundary and route groups

Construct each configured route with `ProbePath::new`, then use
`HttpBoundary::new(RequestPolicy)` with `with_liveness`, `with_readiness` and
build application routes with `GuardedRouter`, and call `assemble(guarded)`.
`ProbePath` rejects captures, wildcards and other
non-literal route syntax before Axum can mount it. The fallible probe-registration
methods reject a repeated path across all probe kinds before Axum routing can
panic. Awaited assembly reserves each complete probe route identity and returns
a sanitized error if any guarded literal, capture or wildcard route can match
it. `GuardedRouter` retains identities through route, merge and typed nesting;
opaque `nest_service` input is unavailable because it cannot disclose its
routes. Assembly queries only a library-owned inert inventory, so it does not
poll application handlers, fallbacks or middleware. The boundary mounts probes outside admission,
applies admission to every guarded route plus its default, custom, nested and
method fallbacks, and installs server correlation with the single HTTP observer
outermost; the order cannot be changed by the caller. Register the result with
`AssembledHttp::register_in`.
Probes receive HTTP status/outcome/latency events without acquiring an execution
policy. Guarded fallbacks and rejected requests are observed too, but the
fallback remains inside admission and is rejected while the process is not
accepting work.

`HttpBoundary::new` also accepts a `GroupPolicy`, and `with_group` adds named
`RouteGroup`s. The routes passed to `assemble` are the group named `default`.
Each group has its own `RequestPolicy`, for example a longer upload budget, and
an optional `BrowserPolicy`. From the outside in, the boundary installs server
correlation with the single HTTP observer, then for each group its
`PrivateResponsePolicy` headers when it has a browser policy, lifecycle
admission with the group's response-construction deadline, `MutationPolicy`
checks on every method other than GET, HEAD, OPTIONS and TRACE when configured,
and finally the application's own layers and handlers. The private headers
therefore cover the group's admission, deadline, method and mutation
rejections, and the mutation renderer receives the admitted request's
metadata. `BrowserPolicy::with_mutation_checks` takes the application's
renderer for the sanitized `MutationRejection`;
`BrowserPolicy::without_mutation_checks` states that the group's mutating
routes rely on other authentication. There is no empty browser policy.

Only the default group may declare a root or nested fallback, so every
unmatched path reaches the default fallback inside the default group's
admission, even under a named group's route prefix. A named group needs at
least one route and a unique name of 1–96 ASCII alphanumeric, `.`, `_` or `-`
bytes; `with_group` rejects other groups before routing. Assembly rejects a
probe path that any group's route can match, and two groups whose routes can
match the same request path, even with different methods. The sanitized
`BoundaryAssemblyError::OverlappingGroupPaths` names both groups. Overlap is
decided from the retained route patterns and confirmed by routing the shared
path through each pattern alone; no application code runs.

```rust
use axum::{response::IntoResponse, routing::{get, put}};
use batter_axum::{
    BrowserPolicy, GroupPolicy, GuardedRouter, HttpBoundary, RequestPolicy, RouteGroup,
    browser::{BrowserOrigin, MutationPolicy, PrivateResponsePolicy},
};

async fn assemble(
    ordinary: RequestPolicy,
    uploads: RequestPolicy,
    account: RequestPolicy,
) -> Result<batter_axum::AssembledHttp, Box<dyn std::error::Error>> {
    let browser = BrowserPolicy::with_mutation_checks(
        PrivateResponsePolicy::SameOriginReferrer,
        MutationPolicy::exact_origin(BrowserOrigin::https("https://app.example")?),
        |rejection, _parts| (rejection.status(), rejection.code()).into_response(),
    );
    let upload_routes = GuardedRouter::new().route("/uploads/{name}", put(|| async { "stored" }));
    let account_routes = GuardedRouter::new().route("/account", get(|| async { "profile" }));
    Ok(HttpBoundary::new(ordinary)
        .with_group(RouteGroup::new("uploads", uploads, upload_routes))?
        .with_group(RouteGroup::new(
            "account",
            GroupPolicy::browser(account, browser),
            account_routes,
        ))?
        .assemble(GuardedRouter::new().route("/work", get(|| async { "ok" })))
        .await?)
}
```

### Migrating manual compositions to route groups

Existing `HttpBoundary::new(RequestPolicy)` and `assemble(guarded)` calls keep
their meaning; route groups are additive.

- Replace per-handler Origin, Fetch Metadata or custom-marker checks with one
  `BrowserPolicy::with_mutation_checks` on the group that owns those routes, and
  move the rejection envelope into its renderer. Handlers no longer call
  `MutationPolicy::check`.
- Replace hand-written private-response middleware, or `private_response`
  layered around a route group, with `GroupPolicy::browser` and the chosen
  `PrivateResponsePolicy`. The boundary places the headers outside the group's
  admission, so admission and deadline rejections receive them too.
- Replace separately layered `request_admission` routers that differ only in
  their budget with one `RouteGroup` per `RequestPolicy`.
- Drop prose ordering rules such as "add admission before merging probes" or
  "keep exactly one HTTP observer": the boundary installs that order.
- Move a fallback from a named group into the default group, and split routes
  that two groups would share, including one path served with different
  methods, into a single group.

## Low-level composition and observation

For compositions the boundary cannot express, `request_admission`, `observe_http`
and `operational_http` remain available. `observe_http` requires no lifecycle
state and adds no deadline or operation context. `RequestPolicy` keeps readiness
and deadlines combined. The existing `request_scope` remains the combined
observation/admission compatibility entry point. Only the outermost observer
emits a completion event, so stacking observers cannot duplicate HTTP events.
Nested Batter middleware publishes retained adapter facts, including quota
facts, into that observer's shared private state in either operational/quota
wrapper order. A plain observer may sit outside `operational_http`, but
`request_scope`, admission, deadlines and other rejecting middleware must sit
inside it when all outcomes require generated correlation.

HTTP completion events default to WARN for 5xx responses and INFO otherwise.
Applications can select a different level by returning
`(axum::Extension(HttpObservationLevel(tracing::Level::INFO)), response)` or
inserting that value into `response.extensions_mut()`. Both observers honor the
response extension without changing status, outcome, fields or event count.
Request extensions and client headers do not configure severity. A dropped future
with no response retains WARN. The span remains INFO; subscriber filtering still
controls delivery of events at all levels.

Each completion event carries its own normalized method, route template, optional
status, outcome and latency. WARN/ERROR events retain these fields when INFO spans
are disabled. Application correlation carried by spans still requires those spans
to be enabled. The observer retains its HTTP span or the available application
span at first poll through execution and destruction. Later ambient request spans
cannot replace the completion event's parent. HTTP fields are only recorded into
the HTTP span, never the inherited application span. Per-layer filters can still
hide context in an individual sink. Formatters that show both span and event
fields may display the HTTP fields twice on the same completion line; there is
still one event.

Set overrides in handlers, failure renderers or middleware inside observation.
Middleware changing a response must retain, replace or remove the override to
match its own policy. Status-only probes retain default severity.
`ReadinessPolicy` selects INFO for Starting/Draining and WARN for Stopped and
unhealthy dependencies while Ready. Their status remains 503 and outcome remains `server_error`, so
status/outcome alerts still need application-owned probe filtering.

Axum's `Router::layer` runs after routing and covers only routes/fallback already
assembled when applied. Routes appended later bypass it. A service wrapper
outside routing records `<unmatched>` because the matched route template is not
yet available. The [runnable example](../batter/examples/http_service.rs) uses
`HttpBoundary`; the [`observe_http` rustdoc](src/lib.rs) demonstrates the manual
composition.

`with_failure_renderer` accepts application-controlled response mapping with a
snapshot of request parts. Establish trusted metadata outside the boundary.
Handlers should use the application's renderer for their own failures.

The deadline ends at response construction. Streaming bodies, WebSockets, client
disconnect lifetime, authentication, proxy trust, and request body limits need
application policy. A joined server wrapper does not prove detached connection
tasks have stopped. Automatic observations omit raw error/request contents;
the application installs its tracing subscriber. A polled response future dropped
before completion emits `dropped` without a status, under its first-poll subscriber.
A never-polled future emits nothing. Dropping a body after response construction
does not produce another HTTP event.

Handler panics propagate through the middleware. An unwind before a response
emits one WARN `dropped` event without a status and cancels admitted operation
context. Batter does not convert the panic to a 500 or log its payload. Rust's
default panic hook remains separate and may print the payload.

From the workspace root:

```sh
cargo test -p batter-axum --locked
cargo run -p batter --features axum --example http_service --locked
```

The facade-owned example binds `127.0.0.1:3000` by default and provides `/live`, `/ready`,
`/work`, and `/fail`. `BATTER_BIND`, `BATTER_REQUEST_TIMEOUT_MS` and
`BATTER_BULKHEAD_CAPACITY` configure its listener, deadline and concurrency.
`BATTER_ENV_FILE` may name one literal dotenv file; there is no default `.env`
search. SIGINT/SIGTERM initiate shutdown on Unix.

Version 0.0.1; Rust 1.94 minimum; the manifest targets crates.io. MIT licensed.

The HTTP composition example combines lifecycle readiness with a cached reader
from the foundation health monitor. Its simulated probe runs independently of
HTTP traffic; application probe and timing policy remain explicit.

## Opt-in operational composition

Use `operational_http` instead of outer `observe_http` to generate a UUID, replace
incoming x-request-id and Tower/adapter identity extensions, observe once, and
replace the response ID header. Extract `Extension<CorrelationId>` and propagate
its string explicitly to nested application metadata. It carries no authority.
The HTTP completion event has its own `request_id` even with all INFO spans
disabled, including on drop under another dispatch. Native nested tracing still
requires enabled spans; no tenant/principal or inbound trace context is inferred.

`RequestPolicy::with_infrastructure_json()` explicitly selects application/json
with `{code, message, request_id}`, fixed sanitized messages and no-store. The
request_id is a generated string or null if no typed correlation was installed;
headers are never fallback identity. Handlers may call
`render_infrastructure_failure(failure, Some(&id))` for the same infrastructure
envelope. Default Problem JSON and `with_failure_renderer` remain available;
the last renderer selection wins. Domain error mappings remain application-owned.

Mount `dependency_readiness::<E>` with `ReadinessPolicy::new(handle.status(), health)`
outside admission. Its response has an empty body and 200 only when lifecycle is
Ready and the latest read-only health snapshot is healthy. The response extension
carries the foundation's `ReadinessDecision`: either Ready or Unready with a
Starting/Draining/Stopped or typed dependency-unready reason. Healthy has no
dependency-unready representation. `readiness_status` exposes the adapter's
200/503 mapping and `default_readiness_level` exposes its INFO/WARN mapping;
`with_level` receives the valid decision and can delegate unmatched cases to that
default. Match `ReadinessUnreadyReason` from `batter_core::readiness` only inside an
unready decision. The old `ReadinessReason` name is deliberately absent from
both crates so stale response-extension lookups fail to compile even after an
import change. Decisions do no probe I/O and are point-in-time observations, not
atomic with future drain.

Inside protected `Startup::scoped` composition, bind a native `TcpListener`, assemble the
guarded `GuardedRouter` through `HttpBoundary`, then call
`AssembledHttp::register_in(scope, "http", listener)`; `register_http_in(scope,
"http", listener, router)` accepts a plain router for compositions outside the
boundary. It acknowledges when
the registered task runs and delegates graceful drain to Axum. Registration
failure and abandoned startup release the listener. Native accept errors are
retried by Axum. Streaming bodies can outlive response deadlines and direct
wrapper abortion; dependent cleanup remains conservatively skipped on forced
abort. `register_http(&mut Supervisor, ...)` remains source-compatible for the
lower-level path. The [operational tests](tests/operational.rs) exercise that limit with a
real socket and explicitly release the outstanding body afterward.

When authentication admission needs the direct TCP peer, replace the registration
call with `register_http_with_connect_info_in(scope, "http", listener, router)`.
Middleware and handlers can then extract native `ConnectInfo<SocketAddr>`, including
the peer port, through Axum's make-service conversion. Behind a proxy this is the
proxy's socket address; forwarding headers do not select it. Authentication and
proxy trust remain application policy. The listener ownership and shutdown
contract is identical to `register_http_in`; existing plain registrations keep
their behavior. See the helper's rustdoc for a complete registration example.
