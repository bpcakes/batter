# API assessments: HTTP assembly, browser policy and outbound exchange

[ADR-010 policy](../../adr/010-agent-only-consumption.md) · [Assessment index](README.md)

These records apply ADR-010 to specific changes. They retain the evidence
and limitations recorded at delivery; current contracts remain in rustdoc
and [guarantees](../../guarantees.md).

## Explicit HTTP completion (`batter-q1jl`)

The first deadline-telemetry repair at `fc48183` spread one terminal decision
across a private response marker, a captured `Option<Response>`, and an
`OperationError::Interrupted` result. A stacked-admission probe demonstrated the
consequence: an inner timer response leaked its marker and changed the outer
outcome, while a handler-returned interruption had its marker consumed. The user
requested an explicit representation rather than another implicit state protocol.

The replacement is `operation::low_level::ResolvedOutcome<T>`, an exhaustive sum
of successful, failed and interrupted values. `run_resolved_value` delegates to
the existing core boundary; it owns execution, synchronous resolution,
observation and return of the value in that order. No paired completion call,
optional retained-value slot, task, new polling loop or mutable phase flag is
introduced. A completed value cannot be classified as dropped. Debug formatting
does not inspect its payload. The HTTP adapter consumes native response evidence
into `ResolvedOutcome<Response>`; a boundary rendering its own interruption
creates an unmarked response. Reused custom-renderer extensions are cleared of
the adapter's private evidence before the current handoff is constructed.

The next independent panel reproduced stale evidence escaping through early
admission rejection: `RequestPolicy::render_failure` bypassed the interruption
renderer's cleanup. The recurrence assessment located an adapter ownership gap,
not a consumer ordering obligation or a flaw in the typed core outcome. Private
`FailureRendering` centralized request-policy rendering, but the next review
found that browser rejection still returned a raw response inside admission.
Probes and unmatched fallback responses reproduced the same leak when returned
through another admission. These distinct callback families exposed why an
invocation-site cleanup obligation was insufficient.

The final internal contract makes every stored synchronous renderer return an
opaque `PolicyResponse`. Public callback registration wraps the native response
in this value, whose sole constructor removes old interruption evidence. Failure,
browser, liveness, readiness and fallback callbacks therefore cannot return a
raw response to their adapter caller. The public interruption responder alone
attaches the current reason afterward. Public signatures and ordinary handler
responses are unchanged. Recycled response-extension regressions cover all
renderer families, plus rejection, deadline, forced cancellation and returned
handler paths with both old reasons. They retain unrelated metadata and the
existing probe-owned status/severity decisions. This moves the invariant into
the private output type rather than adding consumer instructions.

Native Axum handlers return `Response`, so a private response extension remains
the transport for a handler's interruption reason. This is an upstream response
representation boundary, not a public application phase transition. The library
owns both marker production and consumption; consumers acquire no paired call
or ordering obligation. Returning a replacement native response or clearing all
its extensions discards that response's evidence. The typed terminal outcome
cannot prove application classification truth or remote effects; those remain
explicit obligations of the lower-level resolution seam, as with `run_resolved`.

The existing `run_resolved` result cannot retain a value in its interrupted arm,
which justifies the additive core seam for this adapter. Existing public names
and signatures remain unchanged; no hard cut or deprecated shim is needed.
The trailing-slash option remains idempotent configuration, with no new request
execution phase. Focused regressions cover the original missing event, both
stacked paths and interruption reasons, exact response retention, discarded
renderings, typed outcomes, callback laziness, dispatch destruction and metrics.
Executed verification is recorded in the Bead. A fresh consumer-agent adoption
or mutation exercise remains proposed and unexecuted.

## Private-response referrer assessment (`batter-lto`)

The adapter previously fixed `Referrer-Policy: same-origin`. A consumer needing
`no-referrer` kept separate header middleware whose effect depended on its
placement: inside `private_response`, it was silently overwritten. The
canonical choice is now the exhaustive `Copy` `PrivateResponsePolicy`, which is
itself the Tower layer, so selection and application are one value in one
`layer` call without a paired state and middleware function. Weaker, raw, empty
and comma-listed tokens have no representation (compile-fail rustdoc), and
`no-store`/`nosniff` cannot be deselected. Application keeps an existing
all-`no-referrer` field, so nested layers resolve to the strictest choice in
either order instead of depending on layer order.

The stateless `private_response` remains the compatibility path for the
default. It ignores router state, so pairing it with a policy through
`from_fn_with_state` still applies `same-origin`; its rustdoc directs new
compositions to the typed layer. Intermediate middleware can rewrite the field,
an outer short-circuit bypasses the layer, and page markup or `fetch` options
can change the document's policy; these remain application boundaries.
`NoReferrer` makes same-origin HTML form posts carry `Origin: null`, so
exact-origin form mutation needs `SameOriginReferrer`: a documented browser
protocol consequence, not a local type invariant. Per-group application through
`HttpBoundary` is assessed below (`batter-tc9w.3`). Fresh-agent usability
evaluation is proposed and unexecuted.

## HTTP route group assessment (`batter-tc9w.3`)

`HttpBoundary` previously applied one `RequestPolicy` to every guarded route.
A read-only survey of two downstream consumers found a second request budget
for uploads, about thirty per-handler Origin/CSRF calls, private-response
middleware layered by hand around private route groups, and ordering rules kept
in prose. Each was a caller-memory obligation beside the canonical path: a new
mutating handler compiled without its check, and private-response middleware
placed inside admission would silently miss admission rejections.

Named `RouteGroup`s now carry a `GroupPolicy`, and the boundary installs one
order per group: private-response headers outside admission with the group's
budget, `MutationPolicy` checks on every non-safe method inside it, then
application layers. A mutation check therefore covers new handlers, extension
methods and method fallbacks without per-handler calls. `BrowserPolicy` has no
empty or default value; its two constructors name the mutation choice, and the
application renders the sanitized rejection. Axum retains the last merged
default fallback and panics on two custom ones, so fallback ownership would
otherwise depend on merge order: only the default group may declare a root or
nested fallback, it is merged last, and `with_group` rejects named-group
fallbacks, empty groups and invalid or reused names before routing. Axum's
matcher also gives literals priority over captures, so routes in two groups
could silently shadow each other: assembly decides from the retained patterns
whether two groups can match one request path, in any method, confirms each
shared path through native routing of each pattern alone, and returns an error
naming both groups. A pattern outside the analyzed syntax or a shared path that
request URI construction cannot preserve verbatim fails closed. This keeps
unreachable but conflicting native route literals from panicking during merge;
the existing fallible assembly boundary owns rejection before execution. A bare
`Router` cannot join a group (compile-fail rustdoc). Tests cover these
rejections, per-group deadlines, headers on every rejection, the fallback owner,
and removal of each ordering or overlap rule.

Remaining boundaries are explicit. Browser signals are not authentication,
authorization, CORS or complete CSRF protection, and letting a safe method
change state stays application policy. Choosing `without_mutation_checks` for
cookie-authenticated mutations was also left to application policy here; the
browser credential gate assessment below (`batter-52s3`) replaces that with an
enforced or visibly waived decision. Group membership is path-level,
so one path served with two budgets by method must use one group, and unmatched
paths under a named group's prefix receive the default group's policy. The
overlap analysis mirrors the pinned matchit 0.8.4 rules and needs its Axum
comparison test re-run whenever that pin changes. Routers built outside
`GuardedRouter` (`batter-tc9w.8`) and real consumer ports (`batter-tc9w.9`)
were separate tasks; `batter-tc9w.2` has since sealed assembly and moved the
low-level middleware, assessed below.
Fresh-agent usability evaluation is proposed and unexecuted.

## Reference HTTP boundary assessment (`batter-tc9w.4`)

The reference application composed `operational_http`, `request_admission`,
probe handlers and `register_http_with_connect_info_in` by hand, in the order
correlation, trusted peer metadata, bearer authentication, admission. Nothing
recorded a reason for that order. A draining or starting process authenticated
before rejecting, authentication ran outside the request deadline, and unmatched
paths bypassed admission with a bare 404. On 2026-09-28 the repository owner
decided that the canonical path admits before authenticating, as Runlimit's
authenticated assembly does; authentication before admission is a low-level
composition.

The reference now declares its routes on `GuardedRouter` and assembles them
with `HttpBoundary`. Trusted metadata and authentication are route layers inside
admission, the body limit is a guarded layer, and the 404 fallback is guarded.
The boundary owns correlation, the observer, probe placement and admission, so
the application can no longer put its middleware outside admission, merge a
probe inside it or add a second observer. `register_in` still fuses assembly
with native `ConnectInfo` registration, so the root cannot choose plain
registration for a router that needs the peer, and it returns assembly failure
as `HttpRegistrationError` instead of discarding it. Authentication reads the
trusted metadata and fails closed without it, so reversing those two route
layers fails every business request rather than weakening one; ordinary tests
also pin admission first while starting, ready and draining.

The in-process client now consumes the assembly with
`AssembledHttp::in_process` and wraps the adapter's opaque `InProcessClient`;
the reference type keeps both private and unservable. These checks prove local ordering only,
not proxy trust or any remote client behaviour. Fresh-agent usability
evaluation is proposed and unexecuted.

## Admitted router assessment (`batter-tc9w.8`)

Both surveyed downstream consumers build their routes with an OpenAPI router
builder and convert the result with `Router::from`. `GuardedRouter` accepted
only its own route operations, so such a router could not join `HttpBoundary`
without giving up the route identities that probe, overlap and fallback checks
need, and fell back to the low-level middleware and caller memory. Axum 0.8.9
exposes no route enumeration, and learning a router's routes by calling it would
run application handlers, fallbacks or middleware.

`GuardedRouter::from_router` admits such a router only with a `RouteInventory`;
construction rejects an empty inventory and patterns outside the analyzed Axum
syntax. Assembly inspects a copy of the router whose every route, method
fallback and fallback is replaced by a library reporter, so validation runs
Axum's matcher and calls or polls no application handler, fallback or
middleware service. Preparing that copy still runs, once, the constructors of
application layers that Axum applies lazily to handlers, as Axum's preparation
of a served router would, even when assembly is then rejected; an application
layer whose constructor has side effects shows them at assembly. A declared
pattern that no path of it
reaches with exactly that pattern returns `RouteInventoryMismatch`, and any
route of the router matching a probe path, declared or not, returns
`GuardedProbePath`. Completeness of the inventory cannot be proved locally, so
it is enforced by reachability instead: the admitted router is never merged
into the native router, and serving forwards a request to it only when its
inspection copy reaches a declared pattern. An omitted route is therefore
unreachable, visibly answered by the default fallback, rather than exempt from
the probe and overlap checks, and the router's own fallbacks cannot become a
named group's fallback. Declared patterns join the overlap check and must not
overlap other routes of their own group, because admitted routers are consulted
only when native routing finds no route, rather than by Axum's route priority.
Forwarding from a registered outer route was rejected: Axum appends the outer
match to the admitted router's `MatchedPath` and path parameters, and
`Router::route_service` refuses routers. Dispatch is the native router's root
fallback, the one point where a request carries no path captures, so the
default group's fallbacks sit behind it: a nested fallback below a capture that
matched first would add that capture to the forwarded request. Axum prepares
only the router it serves, so assembly prepares each admitted router, its
inspection copy and the moved fallbacks once; otherwise every request would
rebuild their layers and reset state such as a concurrency limit. Native routes
stay in the served router and keep Axum's own preparation. Tests cover these
rejections, unreachable undeclared routes and fallbacks, declared routes inside
their group's policy with native parameters, including below a nested fallback
with a capture, one completion per request, nesting, layers and layer reuse
across requests; they fail when the inspection copy leaves fallbacks in place,
dispatch ignores the inventory, a nested fallback stays in front of the
dispatch, or any copy behind the dispatch is left unprepared.

Remaining boundaries are explicit. The inventory is application input, for
example an OpenAPI document's paths plus any undocumented route, and a route it
omits stays unreachable. Validation proves each declared pattern with one path
filled with `{}`, so a route whose literal spells escaped braces could take that
path and fail the inventory closed. Routes inside an opaque nested service of
the admitted router cannot be declared. A request that no native route matches
gains one library-owned inspection routing for each admitted router whose
declared pattern matches its path; an index built at assembly selects those
routers, so overlap rejection leaves at most one.
Layer state inside an admitted router, and around the default group's
fallbacks when admitted routers exist, is shared by all of their requests, as
with Axum's make-service conversion, even where `axum::serve` would prepare a
native router per connection. No
OpenAPI-builder dependency or convenience is added. Fresh-agent usability
evaluation is proposed and unexecuted; the downstream ports belong to
`batter-tc9w.9`.

## Probe renderer and readiness condition assessment (`batter-tc9w.5`)

`HttpBoundary` mounted only the adapter's empty-body liveness and readiness
handlers. One surveyed consumer documents JSON probe bodies in its OpenAPI
contract, and the other also requires an application condition, held key
leases, before reporting readiness, so both mounted their own probe handlers
beside the boundary. Each such handler chose its path, its placement relative to
admission, its status and severity, and recombined lifecycle and health itself:
a probe merged inside admission would reject while starting, a handler could
answer 200 for an unready decision, and a hand-composed application check could
override the lifecycle.

`with_rendered_liveness` and `with_rendered_readiness` keep the path, the
placement outside admission and every collision check on the boundary, exactly
as for the empty-body probes. The renderer receives a `Copy` decision and the
request metadata and returns a response; the boundary then sets the status from
`readiness_status` and replaces the decision and severity extensions, or for
liveness sets 200 and removes any severity override, so no renderer, including
one returning a contradicting status or forged extensions, changes what an
orchestrator or the completion event sees.
Application conditions narrow the foundation decision rather than wrap it. A
check returns only whether its condition holds (a check returning a decision
does not compile), is asked only after the dependency sample establishes
readiness and before the final lifecycle read, and can only turn Ready into the
new `ReadinessUnreadyReason::Condition`, which carries a validated
`ReadinessCondition` name (raw strings do not compile) and defaults to WARN.
Conditions accumulate, so adding one cannot silently drop another, and the
empty-body `dependency_readiness` reports them too because it reads the same
decision. A condition may still be asked while the process is starting,
whenever the dependency is healthy, because lifecycle is read last; its answer
is then discarded. Tests send every decision through a contradicting renderer,
probe both condition answers in every unready lifecycle and dependency phase,
narrow Ready through the condition, keep a liveness completion at INFO under
INFO filtering, deny both renderers the quota observer's writer, keep both
probes outside admission and reject every duplicate and guarded probe path.
They fail when a renderer's status or severity survives, when the quota writer
stays on renderer metadata, when a condition replaces a dependency reason, or
when a condition's answer decides the decision for a starting lifecycle.

The metadata isolation review found an indirect capability after direct quota
writer removal: cloned `Parts` still carried shared observation state, allowing
redispatch through quota middleware to replace the original probe's quota facts.
The same incomplete filter existed in failure/interruption and browser rejection
rendering. This was an adapter design gap, not an application protocol obligation.
All renderer crossings now use one private filter removing the quota writer,
shared observation state and operational ownership marker. Ordinary nested
middleware keeps those states; a new request made from renderer metadata gets
independent correlation and completion under an operational wrapper. Public
correlation and application metadata remain available to renderers. Merely adding
another probe-only removal would leave sibling paths exposed; a new public
metadata type would break signatures without being needed to remove these
library-owned capabilities. Redispatch regressions cover both probes, admission
failure, captured interruption rendering and browser rejection with ordinary and
quota wrappers, checking original facts and independent child completion.

Remaining boundaries are explicit. Condition checks and renderers run
synchronously inside the probe request, and probes carry no
response-construction deadline because they sit outside admission; a check or
renderer that blocks, performs I/O or panics affects that probe like any
handler, which the evaluator cannot detect. Whether a check reflects key leases
or any other remote fact is application state that Batter cannot verify. Checks
added under one name report the same condition, deliberately allowing several
checks of one requirement; distinct requirements need distinct names for a
renderer to tell them apart. Adding `Condition` breaks exhaustive matches on
`ReadinessUnreadyReason`, a change taken under the exhaustive-enum policy with
a changelog migration note. `readiness_status` fixes the unready status at 503,
and a hand-mounted handler built from `ReadinessPolicy::decision` remains a
weaker low-level composition. Fresh-agent usability evaluation is proposed and
unexecuted; the downstream ports belong to `batter-tc9w.9`.

## Admitted request extractor assessment (`batter-tc9w.6`)

Admission inserted the request's `OperationContext` and
`RequestInterruptionResponder` as native extensions, and `operational_http` a
public `CorrelationId`. One surveyed consumer takes `Extension<OperationContext>`
in 55 handlers, plus the responder and correlation in scope handlers; the
other reads the context extension on every handler. Each handler therefore
reassembled the admitted request from three independent extension reads. A
handler outside admission compiled and answered Axum's missing-extension 500,
whose text names the internal type. Any application layer could insert or
replace those values, for example a context with another deadline, and the
handler could not tell. Admission placed outside correlation, an unsupported
order, still gave handlers all three values while its own rejections and
deadline responses lacked the generated identity.

Handlers now extract one `AdmittedRequest`. Admission records it privately,
and only inside the operational wrapper that generated the request's
correlation, taking the identity from that wrapper's private marker rather than
the public extension. The public value has a private field and is not `Clone`,
so it can be neither constructed nor inserted into request extensions
(compile-fail rustdocs). The extractor reads only admission's private record,
so raw extensions can neither supply nor alter it. Where nothing was recorded,
it answers `AdmittedRequestRejection`, the fixed 500 Problem JSON of
`HttpFailure::Internal` at the default WARN severity. That includes a
lower-level `request_admission` or `request_scope` without `operational_http`
outside it, so the known-invalid order fails every extracting request instead
of looking equivalent to the protected path. The shared renderer filter also
removes the record, so a request redispatched from renderer metadata is not
admitted. Axum's handler traits do not expose which extractors a handler
uses, so assembly cannot reject an extracting handler outside admission. On the
canonical path every guarded route and fallback is admitted. Appending a route
to an assembled boundary is no longer expressible at all since `batter-tc9w.2`
sealed assembly, so a low-level composition, or a router with no Batter layer,
is the way to reach the rejection. The facade HTTP example and the reference
service now extract the admitted request. Tests cover every group's own context and generated
identity, the rejection outside admission and in both unsupported lower-level
orders, forged and replaced raw extensions, and redispatch from every renderer.
They fail when the renderer filter keeps the record or when admission takes the
public identity.

Remaining boundaries are explicit. The raw extensions are still inserted for
Batter's adapters, whose Runlimit boundary reads the context and responder, and
for existing handlers; removing them would break both, so they remain documented
as ordinary replaceable values. Copying a whole extension map from an admitted
request into another request copies the record with every other value. The
rejection cannot use the application's envelope because no request policy is
known outside admission; an application that needs its envelope extracts
`Result<AdmittedRequest, AdmittedRequestRejection>`. Fresh-agent usability
evaluation is proposed and unexecuted; the downstream ports belong to
`batter-tc9w.9`.

## Generic serving listener assessment (`batter-tc9w.7`)

Owned serving accepted only `tokio::net::TcpListener`. A read-only survey of two
downstream consumers found one of them rebuilding TLS serving as its own managed
component of about two hundred lines. It owned a rustls listener and its
handshakes, which is application policy, but it also repeated Batter's
registration, startup acknowledgement, graceful-drain signal and stopped-proof
assembly, which is not. Those repeated lines are the caller-memory obligation:
copied lifecycle wiring can acknowledge startup at the wrong moment, omit the
graceful-shutdown signal, or discard the `ComponentExit` proof, and nothing in
the adapter's shape prevented it.

`register_http`, `register_http_in`, `register_http_with_connect_info_in` and
both `AssembledHttp` registration methods now accept any `axum::serve::Listener`.
The canonical assembled boundary therefore serves TLS through exactly the path
that already owns listener transfer, acknowledgement, drain and conservative
cleanup, and the application keeps only the part it actually owns. Existing
`TcpListener` call sites keep their signatures and behaviour, so this removes an
obligation without adding a choice.
The public listener parameters use argument-position `impl Listener` with
associated-address bounds. Adding a named listener type parameter would break
existing explicit registration-target arguments; compilation coverage checks
those original call forms for the raw helpers and both assembled methods. This
signature choice preserves the same ownership transfer and address constraints;
it introduces no additional registration phase or caller obligation.

Peer metadata stays library-owned. Pinned Axum 0.8.9 implements `Connected` for a
bare listener only for `TcpListener`, and generically only for
`ListenerExt::tap_io`, so a consumer reaching connect info for its own listener
had to know that and wrap it. `register_http_with_connect_info_in` now applies
that empty tap itself, and `ConnectInfo<L::Addr>` follows from the registration
choice rather than from an upstream implementation detail. The address bounds in
the signature are Axum's own `Connected` and `Debug` requirements, so a listener
whose address cannot supply connect info fails to compile at registration instead
of answering Axum's missing-extension 500 on every request.

What remains with the application is explicit and could not be narrowed locally.
`Listener::accept` cannot return an error, so retry and backoff after a failed
accept, certificate and key selection, protocol versions, ALPN,
client-certificate rules and handshake concurrency are the listener's own policy;
Batter installs no process-default crypto provider and takes no serving TLS
dependency outside test material. Outside test material, the only TLS
dependencies of its libraries are client-side, in `batter-outbound-http` and in
`batter-otlp`'s exporter transport, and they configure no listener. A listener
that spawns handshakes onto the runtime instead of polling
them inside `accept` creates detached descendants, exactly like Axum's own
connection tasks: registration proves nothing about their termination, and a
wrapper abort or panic still conservatively skips cleanup. Rust cannot express
"this trait implementation detaches nothing", so that stays a disclosed caller
obligation rather than an invented guarantee. Drain destroys the accept in
progress without awaiting it, which the suite asserts directly.

The pre-existing mismatch between a handler extracting `ConnectInfo` and a
registration that installs none is unchanged: `Router` carries no connect-info
type, so the two registration methods remain the only place that choice is
visible. `batter-tc9w.9` owns porting the surveyed consumers. Tests serve an
assembled boundary over a real generated-certificate rustls listener and cover
startup acknowledgement before readiness, a probe outside admission, the
transport peer against a client-owned oracle, drain while a response is still
streaming on an open connection, release of an accept parked in its handshake,
and release of rejected and abandoned listeners. Fresh-agent usability evaluation
is proposed and unexecuted.

## Sealed HTTP assembly assessment (`batter-tc9w.2`)

`AssembledHttp::into_router` handed the complete boundary back as a bare
`Router`. Every supported composition was therefore one method call away from a
known-invalid one: a route appended afterwards bypassed correlation, the
observer and every group's admission; a layer added afterwards sat outside the
single observer; a second observer added that way silently observed nothing;
and the value could be handed to `axum::serve` or any other serving path with
no listener contract. The crate root published the eleven deliberately
caller-ordered helpers beside the canonical types, so `observe_http`,
`request_admission`, `request_scope`, `operational_http`,
`operational_http_with_quota`, `readiness`, `liveness`,
`dependency_readiness`, `register_http`, `register_http_in` and
`register_http_with_connect_info_in` looked equivalent to `HttpBoundary` at the
import site. Both are exactly the shape this ADR names as design debt: a public
API offering a supported composition and a known-invalid ordering or nesting
through the same surface, documented rather than prevented.

Assembly now has exactly two outcomes. `register_in` and
`register_with_connect_info_in` consume it into protected serving, and the new
consuming `in_process()` returns an opaque cloneable `InProcessClient` whose
only operation is `request(&self, Request<Body>) -> Response`. The router is
unreachable: no conversion or accessor, no `Deref`/`AsRef`, no public field, no
route or layer method, no Tower `Service` or `MakeService` implementation and no
serving conversion. Each of those is a separate compile-fail control, as is the
removed `into_router`: a returning escape cannot hide behind another that is
still absent, so the `into_router` control does not chain a route call, and
`Service<Request>` has its own control because `axum::serve` exercises only the
make-service conversion. The conversion and accessor traits are controlled by
bound rather than by use, because an identity-return control rejects only an
identity return: `Into<Router>`, `AsRef<Router>` and `Deref` each have their
own unsatisfied-bound control on both types, and reading the private router
field has its own control beside them. The eleven helpers moved to `low_level`, physically
relocated where they were defined at the root so none can stay accidentally
public there, with one compile-fail control per helper name rather than one per
group, for the same reason. These controls assert non-compilation; rustdoc does
not enforce the annotated error codes, so the codes document the expected cause
and are not themselves the assertion.
Choosing the weaker contract is now a visible, deliberate import, which is what
this ADR asks of a low-level escape hatch.

The remedy also had to keep the sealed path honest about what it proves. The
client prepares its router once, with the same `Router::with_state(())`
preparation pinned Axum performs in `into_make_service` and
`into_make_service_with_connect_info`, so the layer-construction-count
regression still holds while make-service stays unexposed. A request is polled,
and its future destroyed, inside the caller's task under the boundary's
existing dispatch and observation ownership, with nothing spawned, so the
aborted-request destruction tests continue to observe exactly one completion.
An in-process response establishes response construction only, so the real
socket and TLS serving tests are retained unchanged and the facade example's
phase test now serves the actual router through protected registration in a
separately owned harness supervisor rather than taking its router.

What remains with the caller is explicit. A request may carry explicitly
inserted synthetic extensions, including a chosen `ConnectInfo`; the client
constrains direct API conversions, not an application's own transport wrapper,
and no local type can make a synthetic peer evidence of a remote client. The
reference service therefore keeps its own wrapper, which replaces
`ConnectInfo<SocketAddr>` on every request and exposes neither the adapter
client nor a router. Runlimit's protected assembly stays the one production
composition that orders this middleware itself, because authenticated quota
checking has to sit between admission and body extraction, which `HttpBoundary`
does not express; it now imports from `low_level` and is named as that seam in
the adapter guide. One consequence is deliberate: because the boundary's own
observer is outermost and the plain observer allocates no quota record, a
boundary request carries no quota writer at all, so a composition that needs an
outer quota wrapper belongs entirely in `low_level`. The renderer-isolation
coverage follows that reachability rather than keeping a scenario the seal
removed. This is a public hard cut with no deprecated aliases and no
`into_router` shim, so an invalid API path cannot return as a rollback
mechanism; consumers can stay on their prior immutable revision until migrated.
Fresh agent implementation and modification evaluations remain proposed and
unexecuted for this API.

## Probe response policy assessment (`batter-probe-response-policy-2wz4`)

A consumer with a private administrative surface requires `no-store`, `nosniff`
and a chosen referrer policy on every response it serves. The canonical path
could not express that. `BrowserPolicy` belongs to a route group, and probes are
mounted outside every group precisely so no admission gate can reach them, so a
probe response was never covered. A renderer could apply the policy for GET and
HEAD, but an unsupported method on a probe path is answered by the probe's own
method fallback, which runs no renderer, so those `405` responses carried no
policy headers at all. The only remedy was a post-assembly
`PrivateResponsePolicy` layer over a router taken from the boundary — the escape
`batter-tc9w.2` removes. That is the shape this ADR names as design debt from the
other direction: the canonical path could not express a stated consumer
invariant, so the supported composition and the escape hatch were not
interchangeable, and sealing first would have left the consumer with no path at
all.

`with_probe_response_policy` keeps the selection typed and library-installed.
The caller supplies an existing `PrivateResponsePolicy` value, and assembly
layers it around the probe routers before anything is merged into them, so it
reaches exactly the probes. Pinned Axum 0.8.9 `MethodRouter::layer` wraps the
method fallback as well as each method handler and carries `allow_header`
through unchanged, which is why the implicit `405` gains the headers while
keeping its status and `Allow`. Nothing is added after assembly, so the sealed
`AssembledHttp` still reaches registration, and the consumer's own filed
reproduction passes with no router escape.

The invariant the selection does not claim is as important as the one it does.
It changes response headers only: it cannot alter probe status, the readiness
decision, correlation, the single observation, or the rule that probes stay
outside application admission, and a test drives GET, HEAD, POST and OPTIONS
against both probes in the starting, ready and draining phases, with and without
renderers, asserting each of those alongside exactly one completion event per
request. It is deliberately opt-in, because a consumer that wants no private
probe policy must keep its current responses, and deliberately scoped to probes,
because unmatched paths belong to the guarded or rendered fallback and guarded
routes to their group's policy; a test asserts the selection reaches neither.
Selecting twice keeps the last policy, as `with_failure_renderer` does, since a
replaced choice is not an invalid state. Fresh agent implementation and
modification evaluations remain proposed and unexecuted for this API.

## Browser credential gate assessment (`batter-52s3`)

`BrowserPolicy::without_mutation_checks(response)` was guarded only by its
rustdoc, which asked callers to choose it only when the group's mutating routes
accept no ambient browser credentials or authenticate them otherwise.
`browser::read_cookie` read any header map, so a cookie session on an unchecked
group compiled and ran exactly like the protected composition. One downstream
service ran every browser group without checks while issuing cookie sessions;
its forgery protection rested on about 129 state-changing handlers each calling
one of four hand-written origin helpers, one of which read only the first
`Origin` value, so a duplicated field passed. Its repair was the
`with_mutation_checks(.., MutationPolicy::exact_origin(..), ..)` Batter already
shipped. Under the review questions above this is design debt: a known-invalid
composition looked equivalent to the protected path (question 4), and the
invariant lived in each handler's memory (question 2).

Gating cookie issuance was considered and rejected: `BrowserCookie` emits a
host-only `Path=/` cookie, so a browser attaches a cookie minted on a checked
group to every request to the host, including an unchecked group's. The
invariant belongs where a credential is accepted, which is where a handler
reads it. Each browser group therefore installs a gate inside its admission,
in the position the mutation check already held, that records privately
whether the request may present cookies, decided from the method it arrived
with: safe methods always, other methods after the group's checks pass, under
`MutationCheckWaiver::ApplicationChecksMutations`, and never under
`MutationCheckWaiver::NoAmbientCredentials`. `without_mutation_checks` now
requires that waiver, so an unchecked browser group states its reason at the
call site (compile-fail rustdoc for the one-argument form). The canonical
reader is the `BrowserCredentials` extractor, which reads only the gate's
record. Where the gate refused, or where no browser group recorded anything,
including a plain group, the default group without a browser policy, a
lower-level composition and a bare router, extraction answers a typed
`BrowserCredentialsRejection` that renders as the sanitized 500 of
`HttpFailure::Internal`, as `AdmittedRequestRejection` does for the same class
of composition error. The decision ignores whether a Cookie field is present,
so a misplaced handler fails on its first request of that method rather than
only when a browser sends a cookie. The value has no constructor and is not
`Clone` (compile-fail rustdocs), and `browser::read_cookie` no longer resolves
(compile-fail rustdoc).

Batter cannot make the misplacement a compile error. Axum's handler traits do
not expose which extractors a handler uses, and routes are type-erased before a
group receives them, so the decision is library-owned execution, not a type
state. `ApplicationChecksMutations` stays a supported choice because some
mutations legitimately present ambient credentials without a check
`MutationPolicy` can express, such as a cross-site form post verified against
the application's own state value or a synchronizer-token protocol; it is now
named, searchable, and documented as an application obligation Batter neither
performs nor observes. `low_level::read_unchecked_cookie` keeps the old reader
for compositions that own the check themselves. It has a new name so that the
compiler's import suggestion for the removed `browser::read_cookie` cannot
silently move a canonical consumer onto it. A handler that parses the raw
`Cookie` field bypasses the gate, and copying a whole extension map carries the
record, as it carries the admission record. Tests cover every row of the
decision, refusal with and without a cookie on four standard methods and an
extension method, the response shape inside the group's private headers, an
application layer rewriting the method in both directions, an admitted router,
the default group and two lower-level compositions; negative controls that
accept the waived refusal, decide from the rewritten method, accept a missing
record, or skip recording after a passed check each fail a test. The shared
renderer filter also removes the record; no current renderer runs after a gate
records one, so that removal is unexercised defense against a future layer
order. Fresh-agent usability evaluation and the downstream port are proposed
and unexecuted.

## In-process routing state reuse assessment (`batter-oj37`)

`InProcessClient::request` forwarded the caller's request straight into the
prepared router. A request built from a whole extension map cloned inside an
earlier dispatch, for example `request.extensions().clone()` in a handler or
fallback, therefore reached pinned Axum 0.8.9 carrying that dispatch's routing
records, which Axum treats as an outer router's state. Through a real assembled
boundary, a map from `/source/{source_id}` reused for `/view/{id}` made
`Path<u64>` answer 500 because it received two parameters (three from a nested
source); an inherited invalid UTF-8 record made a valid path answer 400;
`MatchedPath`, and with it the completion event's `route`, became the earlier
pattern followed by the destination's; and the inherited Batter observation and
operational state suppressed the destination's request span and completion
event entirely. The protected client accepted a known-invalid input and executed
it into wrong captures and missing telemetry, which is the shape ADR-010 names
as design debt.

The invariant is that the client accepts only an arrival-shaped request: one
whose extension map carries no Axum routing record. The four detectable records
are the path-parameter record, read through the public `RawPathParams`
extractor, `MatchedPath`, `NestedPath` and `OriginalUri`. Pinned Axum writes
`OriginalUri` on every `Router` dispatch, writes the path-parameter record
whenever it routes to a route, a path fallback or a `nest_service` service, and
writes its private nested matched-path record only in the same routing step as
the path-parameter record, so a cloned map from any dispatch carries at least
one of them ([upstream facts](../../references/http.md#in-process-routing-state-reuse-2026-11-13)).
The client enforces it at its library-owned entry, before the router, any layer
or any application code is polled: any one record refuses the whole request,
and nothing is stripped or repaired. Any `RawPathParams` result except
`MissingPathParams` counts as present, so a rejection Axum adds later fails
closed. The refusal is the existing sanitized `HttpFailure::Internal`
infrastructure response with no correlation identifier, carrying the new
`ReusedRoutingState` response extension, which has no public constructor or
field, so a caller or test can tell the refusal from a 500 the boundary built;
one ERROR event with a fixed message and no request-derived field records it.
The marker is `Copy`, and a handler can return an inner client's refused
response or attach a copy, so the client removes the marker from every response
it dispatched and keeps the other response extensions: only the refusal path
leaves `request` carrying it.
The request never entered the boundary, so it has no completion event, request
span or `x-request-id` header. Caller-inserted application extensions, such as
`ConnectInfo` and the application's own types, stay accepted and reach handlers
untouched. The supported migration copies each application extension by type
into a fresh request.

The invariant is enforced at run time rather than by type. The extension map is
an opaque upstream type map, so no Rust type can say which values a given map
holds. A `Result`-returning `request` or a typed fresh-request builder would be
a hard cut across roughly 150 call sites in 34 files for a rare composition, and
a builder that accepted caller extensions would still have to filter the
cloneable public records by name at run time, which is this check behind a
larger surface. The signature, `request(&self, Request<Body>) -> Response`, is
unchanged, so this is a behavior change with no hard cut and no deprecated shim.

Rejected alternatives: resetting each record individually is impossible,
because `UrlParams` and `MatchedNestedPath` are private Axum types and `http`
1.5.0 `Extensions` has no iteration or retain, and stripping only the nameable
records would leave a mixed contract that silently repairs some maps and refuses
others; dropping the whole map with `clear` would delete application metadata
the caller chose; an upstream reset facility does not exist at the pinned
versions. Wrappers that hold an `InProcessClient`, such as the reference
service's request client, inherit the refusal by delegating to it.
`batter_runlimit::http::TestClient` does not wrap `InProcessClient`: it
dispatches its own prepared router, so it does not inherit this check
(follow-up `batter-gp93`, since resolved by the
[Runlimit in-process assessment](#runlimit-in-process-routing-state-assessment-batter-gp93)).

Evidence: route-group tests capture maps from a captured route, a nested route,
an invalid UTF-8 capture, a path fallback, the catch-all fallback and a native
`nest_service` service, insert each record alone into a fresh request, and
assert the refusal, its marker and body, and that no application layer or
destination handler ran; a separate test asserts the single static ERROR event,
no completion event and no path or parameter contents in the captured output.
The accepted path covers application extensions and `ConnectInfo`, a
boundary-built 500 without the marker, the copy-by-type migration with the
destination's own captures, `MatchedPath`, one completion event for its pattern
and a fresh request identifier, and captures accumulating through nested routes
and an admitted router inside one dispatch. It also covers an invalid UTF-8
record left alone after the nameable records are removed, and a handler that
attaches a copied marker or returns an inner client's refusal, whose dispatched
response has no marker and keeps its other extension. Negative controls were
run once by hand and are not part of the suite: with the entry check disabled
every refusal test fails; with marker removal disabled the copied-marker test
fails; with only an `Ok` extractor result counted as a record the isolated
invalid UTF-8 case fails.

Residual at delivery, closed by the
[entry-check follow-up](#in-process-entry-check-follow-up-assessment-batter-nff2-batter-28kb):
a map could carry Batter's private observation, operational or admission
records with no detected Axum record. One source is a `low_level` wrapper
around a service that is not a `Router`. Another is the map of a dispatch no
pattern matched, such as `OPTIONS *`: its only Axum record is the public
`OriginalUri`, which an application can remove.
Fresh-agent usability evaluation of the refusal and migration remains proposed
and unexecuted.

## Runlimit in-process routing state assessment (`batter-gp93`)

`batter_runlimit::http::PreparedHttp::in_process` returned a `TestClient` that
held its own prepared router and dispatched `router.clone().oneshot(request)`.
It did not wrap `InProcessClient`, so the `batter-oj37` entry check above did
not apply to it. Reproduced through `TestClient` itself on pinned Axum 0.8.9,
with the shared check disabled by hand: a whole extension map captured in a
protected `/source/{source_id}` handler and reused for `/view/{id}` passed
lifecycle admission, was authenticated (authentication calls 1 to 2) and
charged one native batch check (1 to 2), then `Path<u64>` answered 500 with
"Expected 1 but got 2", and the inherited observation state suppressed the
completion event entirely. A known-invalid input therefore reached the
protected assembly's authentication and consumed quota before failing, which is
the shape ADR-010 names as design debt.

The contract is the one `batter-oj37` decided: arrival-shaped requests only,
refused whole at entry, nothing stripped, with the sanitized
`HttpFailure::Internal` response carrying `ReusedRoutingState`. The check, the
refusal and the dispatched-response marker removal moved, unchanged, into one
lower-level type, `batter_axum::low_level::RouterClient`, which also owns the
one-time `Router::with_state(())` preparation. `InProcessClient` holds one in
its private field and delegates to it; `TestClient` holds one, inserts the
caller's synthetic peer and delegates. The refusal therefore precedes lifecycle
admission, authentication, the native quota check, public probes and handlers.
`ReusedRoutingState` gains only a crate-private constructor whose one caller is
that refusal, so it still has no public constructor (field and constructor
compile-fail controls).

Wrapping `InProcessClient` was rejected. That type is the proof of a sealed
`HttpBoundary` assembly, with correlation and the single observer outermost,
probes outside admission and every route group's policy, and its only
constructor is `AssembledHttp::in_process`. Runlimit's assembly orders the
`low_level` middleware itself, putting the quota check between admission and
body extraction, which `HttpBoundary` cannot express. A public constructor
from an arbitrary router would let any caller-composed router claim that proof,
making a known-invalid composition look equivalent to the protected path.
Duplicating the check in Runlimit was rejected: two copies of a pin-sensitive
check would each need rechecking whenever the Axum or `http` pin changes, and
because the marker has no public constructor Runlimit would need a second marker
for the same refusal. A bare shared check function was rejected because it
would leave three paired caller duties: call it before dispatch, strip the
marker from every dispatched response and prepare the router once. The type
owns all three.

Invalid-state review of `RouterClient`:

1. `RouterClient::new` accepts any `Router<()>`. Nothing about a router's
   composition can be validated at construction, and execution never has to
   reject a constructed client; this is the low-level weakness, and its rustdoc
   states that it establishes nothing about correlation, observation,
   admission, probe placement, route-group policy or authentication.
2. Preparation, the entry check and marker removal are library-owned. The
   caller keeps the ordering obligations that each `low_level` helper its
   router uses already documents.
3. It is not a Tower service and has no router accessor, public field or
   serving conversion, and it has no conversion into `InProcessClient`. Each
   has a compile-fail control, as `InProcessClient` has: `axum::serve`,
   `tower::Service<Request>`, reading the private field, an `into_router`
   method, `Into<Router>`, `AsRef<Router>`, `Deref<Target = Router>` and
   `Into<InProcessClient>`. `TestClient` gains a control against being
   returned as an `InProcessClient` and keeps its `axum::serve` control.
4. The canonical path is unchanged: `AssembledHttp::in_process` still returns
   the protected client. `RouterClient` is reachable only under `low_level`,
   with a crate-root import control.
5. At this delivery the `batter-oj37` residual was unchanged and cost more
   here; the
   [entry-check follow-up](#in-process-entry-check-follow-up-assessment-batter-nff2-batter-28kb)
   now refuses it. Batter's private records with no detected Axum record were
   neither refused nor cleared. Reproduced through `TestClient`, first by hand
   and then by a characterization test that the follow-up inverted: a map
   captured in a
   protected custom root fallback for `OPTIONS *` carries `OriginalUri` as its
   only detected record, and is refused whole. With `OriginalUri` removed by
   the caller it was accepted, authenticated (1 to 2) and charged one native
   check (1 to 2), and answered 200 with no `x-request-id`, while the handler
   saw the client's unreplaced inbound `x-request-id` and no completion event
   was emitted, because the inherited operational and observation records
   stayed in place. The residual took two deliberate caller steps and was stated on
   `RouterClient::request`, `TestClient::request` and in the contracts at
   this delivery. The characterization test failed once the entry check
   refused the inherited records, and closing `batter-28kb` inverted it
   together with those statements.
   `RouterClient` lives in `batter-axum` and could detect those private
   records itself; that widened the `batter-oj37` contract and was done in
   follow-up `batter-28kb`. Making Runlimit's quota stage expressible by
   `HttpBoundary`, so its client could be a real `InProcessClient`, is
   follow-up `batter-uskw`.

Signatures are unchanged and no name is removed, so there is no hard cut and no
deprecated shim. A rustdoc JSON comparison with
[`surface.py`](../public-api-diff-2026-10-09/surface.py) against the
`batter-oj37` commit `f3224582` (nightly rustdoc, all features) found no
removed entry in `batter-axum` or `batter-runlimit`. The additions are
`RouterClient` with `new`, `request`, `Clone` and its `low_level` re-export,
and `TestClient`'s re-export from its new private module, at the unchanged
`batter_runlimit::http::TestClient` path. `batter-runlimit` no longer declares `tower` as a normal
dependency because its library no longer calls `oneshot`; the dependency
graph is unchanged because Axum still depends on it.

Evidence: [Runlimit reused-routing tests](../../../crates/batter-runlimit/tests/http/reused_routing.rs)
reuse a map captured in a protected captured route and assert the 500 refusal,
its marker, the exact sanitized body, `Cache-Control: no-store`, no
`x-request-id`, unchanged authentication, quota-check and handler counts, one
static ERROR event, no completion event, and no path, credential, principal or
label text in the captured output. The accepted case copies an application
extension by type into a fresh request that also carries a caller-chosen
`ConnectInfo`, and asserts the destination's own capture, `MatchedPath`,
principal, label and the transport's synthetic peer, one completion event for
`/view/{id}` and one more authentication, check and handler call each. A third
test has a handler attach a copied marker and asserts the dispatched response
drops it and keeps its own extension. Two more pin the documented order: a
reused map sent to the public `/live` probe is refused without running the
probe, which then answers a fresh request; and after drain is requested a
fresh request receives admission's 503 while a reused map still receives the
marked 500 refusal, with no authentication, check, handler or probe call. The existing `batter-axum` reused-routing
route-group tests now exercise the delegated implementation through
`InProcessClient`, and the `RouterClient` rustdoc example exercises it over a
bare router. Negative controls were run once by hand and are not part of the
suite: with the entry check disabled, the refusal, probe, lifecycle and
copied-marker tests fail while the accepted test passes; with marker removal
disabled, the copied-marker test fails. Compile-fail error codes were checked with nightly rustdoc, because
stable rustdoc does not compare them. Fresh-agent usability evaluation remains
proposed and unexecuted.

## In-process entry-check follow-up assessment (`batter-nff2`, `batter-28kb`)

A post-merge review of `batter-oj37` and the `batter-gp93` reproduction showed
the entry check left a known-invalid input reaching execution: a request whose
extensions carry Batter's own request records and no Axum routing record. The
map of a dispatch no pattern matched, such as `OPTIONS *`, with its public
`OriginalUri` removed, was accepted: the destination answered 200 with the
earlier request's correlation, no `x-request-id` and no completion event, and
through Runlimit's `TestClient` it was authenticated and charged.

The invariant is that an in-process request is arrival-shaped. The adapter can
express more of it than `batter-oj37` enforced, because the records in question
are its own private types. `low_level::RouterClient::request`, which
`InProcessClient::request` and Runlimit's `TestClient` delegate to, now also
refuses a request carrying any of the five private request records that
`renderer_parts` removes: the quota record, the observation state, the
operational wrapper's marker, the admission record and the browser credential
record. No other crate can name or build those types, so one is present only
in a map copied from a request a Batter layer already processed. The predicate
`correlation::carries_private_record` sits beside the renderer filter so the
two lists stay together. The refusal, its marker and its single static ERROR
event are unchanged, nothing is stripped, and no signature changed. This is
refusal, not the clearing `batter-oj37` listed as a non-goal: clearing would
accept a request the caller built from another request's map.

Decisions recorded with it:

- A live request that an outer Axum router dispatched and a route, fallback or
  `nest_service` closure forwards into the client is refused on every request.
  That is kept deliberately. Such a request carries the outer router's records,
  and mounting the client behind a router would serve the boundary outside
  protected registration, which sealed assembly exists to prevent. The
  supported shape rebuilds a fresh request from the arrival's method, target,
  headers and body and copies application extensions by type; a test pins both
  the refusal and that shape. The earlier contract sentence that the client
  does not constrain an application's own transport wrapper is corrected.
- A hand-inserted `OriginalUri` on a never-dispatched request stays refused.
  Axum keeps an inherited one, and the client cannot tell the two apart.
- Public values a caller copies by type, such as `CorrelationId`, are not
  refused. A test shows the boundary generates its own identity.
- The refusal stays a sanitized internal-failure response identified only by
  its marker. A `Result`-returning signature remains rejected for the reasons
  recorded under `batter-oj37`; the documents now say that status and body do
  not identify a refusal.
- The marker keeps its name, `ReusedRoutingState`, although it now also marks
  Batter's own records: renaming a public item that Runlimit's transport and
  its tests already use would be a hard cut for no behavioral gain. Its
  rustdoc states what it covers.

Evidence: in-crate tests insert the quota, operational, admission and
credential records alone, and the observation state alone, and assert the
predicate, the refusal before the router runs, and that the renderer filter
removes each. Route-group tests refuse the `OPTIONS *` map with `OriginalUri`
removed and maps captured behind `low_level::observe_http` and
`low_level::operational_http` around a service that is not a `Router`, refuse
a request an outer router forwards and serve the rebuilt one, and accept a
copied public `CorrelationId`. Runlimit's test refuses the same map with no
authentication, quota check, handler or probe run. The reference service's
request client has a test for the inherited refusal. A hand-run negative
control, not part of the suite: with the private-record check disabled, the
in-crate refusal test and the two route-group tests for Batter-only maps fail.

Unverified: detection of Axum's routing records is checked against Axum 0.8.9
only, while the dependency requirement admits later 0.8 releases. Fresh-agent
usability evaluation of the refusal and both migrations remains proposed and
unexecuted.

## Public-origin invalid-state review (`batter-321j`)

The new constructor stores an immutable policy in the opaque origin; cloning
or preparing requests cannot drop it. The dialer filters the exact addresses
it receives before any TCP attempt, including every fallback candidate. There
is no resolver override or public setter that can remove the policy. A caller
can deliberately construct an ordinary `Origin::https` for private services;
choosing which hostnames and service classes are authorized remains application
policy. Address classification cannot establish OS routing or remote trust.
The public policy also rejects the globally reachable AS112 and AMT
special-purpose blocks. One shared classifier enforces the literal and resolved
address rules; endpoint/neighbor tests cover its range boundaries, and dialer
tests reject these blocks before any connection attempt. This corrects the
classification data without adding a caller obligation or a new API state.

## Response-body storage invalid-state review (`batter-53nf`)

`Body::Complete(Bytes)` keeps its signature, but what owns its allocation
changed, so the review applies. Callers cannot construct the storage or reach
its allocation: the buffer is private, has no reserve or shrink, and
`Bytes::from_owner` transfers it so that the last handle's release wipes it. No
ordering, paired call or cleanup step exists for a caller to forget on the
supported path. What remains is application policy the type cannot own. `Bytes`
derefs to `[u8]`, so a caller can always copy the plaintext; an opaque body type
could withhold that, but parsing or persisting a body needs the plaintext, so it
would burden normal use without removing the obligation. The upstream and host
boundaries stay explicit: hyper, rustls and kernel buffers, destructor-skipping
exits, swap and core dumps.

## Outbound exchange assessment (`batter-3s1a`)

Amended 2026-11-04, `batter-u387`. The original extraction removed repeated
client-profile, preparation, body-limit, deadline and redaction obligations.
Consumer feedback then identified a remaining library design gap: a start gate
before reqwest's first poll did not cover later handoff to a connection, and a
spawned pool connection task could write while the exchange was stalled.
Consumer poll-order wrappers covered only part of that gap and could discard a
ready reply. No redirect/replay rule prevented late progress of the same request.

The protected path remains `Client::new(Origin, ConnectTimeout)`, a draft,
`prepare()`, then one consuming `send(deadline, limit)` or optional
`send_in(context, limit)`. The facade example places the dispatch fence before
send, uses a work-phase context, and records in a reserved finalization phase.

Against ADR-010's proactive review:

1. Construction rejects invalid origins, limits, path normalization hazards,
   framing/hop headers, repeated bodies, serialization failure and CONNECT
   before an exchange exists. The library builds Host, origin-form URI and
   body length. Path-shaped opaque identifiers remain application policy:
   `["v1/effects"]` is one encoded segment, not two path components.
2. Consumers no longer coordinate polling with dispatch or a background writer.
   The exchange owns fresh TCP/TLS setup and the low-level HTTP/1 driver. The
   immediate handoff gate has no intervening await; write/flush gates also
   cover the raw TCP stream below TLS, including read-triggered TLS writes.
   A stalled exchange cannot start a background write, and reading an available
   reply after stopping cannot advance its writer. Consumers still choose the
   parent budget, finalization reserve and provider-specific response limit.
3. PreparedRequest remains non-Clone and is consumed by send. Unsent evidence
   remains opaque, non-Clone and library-created; Exchange remains exhaustive
   and must-use. No caller-supplied transport or public lower-level escape hatch
   can bypass ownership. Private setup/handoff phases determine NotStarted and
   NotDispatched; upstream error-kind guessing is removed.
4. Library-owned execution replaces consumer poll_before patches. Before
   handoff, stopping wins over connection progress; afterward available replies
   win, with gated writes. A connection setup error/timeout proves unsent;
   expiry/cancellation before handoff also proves unsent. No pool, reuse or
   silent resend exists. Each call pays fresh connection/TLS setup.
5. Provider processing, intermediary status meaning, replay, deduplication,
   dispatch fences and durable reconciliation remain application/remote policy.
   Bytes already accepted by the kernel can arrive later. Cooperative polls
   cannot interrupt synchronous macOS certificate verification or preemption
   between a check and socket operation. A dropped future produces no Exchange;
   it drops the connection, and the application must conservatively retain its
   pre-send fence. An outer run/timeout can still discard a ready Exchange.

Remaining consumer obligations are not type proofs (`batter-4p4a`):

- **Outcome interpretation:** Rust permits wildcard arms, `if let`, `matches!`
  and an or-pattern combining `Unknown` with an unsent outcome. Exhaustiveness
  and `must_use` do not prevent those omissions. Distinguishing provider results
  and retaining uncertainty is application policy; the package README documents
  the optional consumer lint against wildcard enum arms.
- **Replay and budgets:** resending on every `proves_unsent()` with the same
  expired deadline or cancelled context spins without yielding. Only a
  `ConnectFailure` justifies an application-owned retry with backoff and a fresh
  budget check. `ReplaySafety` has no value for a non-idempotent request that
  may be replayed only after unsent proof; see the
  [retry adoption contract](../../integrations.md#outbound-provider-exchange-experimental-transport-evidence).
- **Operation phase:** `send_in` accepts any `OperationContext`, including a
  parent or finalization context, so it cannot establish that recording time
  was reserved. The application chooses the work-phase context; accepting
  `OperationPhases` alone would exclude retry attempt contexts. A caller-chosen
  budget that outlives its true parent is also outside the exchange's proof.
- **Provider and destination policy:** endpoint response limits, query
  credentials visible to intermediaries, multi-request history, fences before
  sending, and operator-supplied destinations remain application decisions.
  `Origin::https` validates a URL shape, not an egress-address policy or remote
  processing. Calls through another client bypass this package entirely.

The tests use independent failure scenarios, not additional caller instructions:
controlled I/O and a paused Tokio clock stop a ready connection before handoff,
then stop queued and partially written requests after handoff for both expiry
and cancellation. They verify no background writer while the exchange is
unpolled, no resumed writes after stopping, and appropriate evidence. Loopback
head/body tests preserve ready-response precedence and status retention; TLS
setup, framing, response limits, redaction, fresh connections and facade
feature/type identity remain exercised. `batter-4p4a` adds bounded address
fallback, preserves the prior default Accept header, exercises the raw guard
with real TLS buffered-request/error-read controls and tests setup completion
after its connect timeout. See [guarantees](../../guarantees.md#outbound-provider-exchange).

No public name, signature, variant or label is removed or added. A deprecated
shim is unnecessary because existing callers keep their call shape; a pooled
compatibility path would reintroduce the invalid timing state. Current
execution outcomes belong to `batter-u387` and `batter-4p4a`; earlier macOS evidence predates
this implementation. Fresh-agent mutation/adoption tasks remain proposed and
unexecuted: implement or migrate a provider mutation using a durable fence,
keep it for Unknown/unusable replies, and record only labels. Clean reviews or
test count alone do not establish agent usability.
