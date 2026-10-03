# ADR-010: Optimize integration for coding-agent consumers

## Status

Accepted guidance, 2026-09-11. Owning Bead: `batter-o4h`.
Strengthened 2026-09-20 by `batter-j6f`. This ADR does not authorize speculative
redesign, but evidence that the canonical path represents a locally preventable
invalid state requires the assessment below rather than another caller warning.

## Context

While implementing an example, the user reported several review/repair rounds
that did not converge. This prompted the agent-only consumption policy: example
repair churn can reveal operational protocols that consumers must reconstruct
instead of invariants enforced by Batter. Follow-up Bead `batter-lxm` codifies
the implementation agent's responsibility to investigate that signal.
This is the user's reported motivation, not a replay or independent audit of
those rounds, a confirmed API diagnosis, or an executed usability benchmark.

## Decision

Batter's only consumers are coding agents generating or modifying applications.
Its public integration path therefore prioritizes convergence on correct native
Rust/Tokio composition. Operational invariants should follow from
library-owned execution, constrained interfaces, validated configuration, and
executable checks. A repeated caller obligation to coordinate cancellation,
joining, registration, finalization, deadline relationships, or error
retention is design feedback and debt, even when accurately documented.

On the canonical path, make invalid operational states unrepresentable whenever
Rust ownership, types, or API shape can express the invariant. Documentation,
examples and semantic tests demonstrate the contract but do not replace API
enforcement. A known-invalid ordering, nesting, paired call, phase transition,
empty input, cleanup sequence or omitted outcome must not remain an ordinary
peer of the supported composition merely because guidance describes the right
choice.

The repository keeps one clear canonical supported path. Examples are consumer
contracts. A lower-level escape hatch may remain for a justified application
boundary, but it must disclose its obligations and must not appear equivalent to
the protected path. Application-specific protocols stay in applications;
upstream-specific lifecycle obligations belong in supported adapters when a
real adapter exists. Agent-only consumption does not justify opaque DSLs,
excessive abstractions, giant instruction files, or claims that types prove
arbitrary remote effects.

Design proposals are assessed with independent failure scenarios and fresh-agent
implementation or modification tasks. Clean review or test volume alone is not
evidence of agent usability. Such evaluations are proposed and unexecuted until
the repository records concrete execution evidence.

## Public API invalid-state review

Every new or materially changed public API must be reviewed before delivery for
invalid states that its canonical consumers can still construct. Do not wait for
a recurring example failure. Ask:

1. Can a caller construct a value, phase or route that execution must later
   reject even though pure construction had enough information to reject it?
2. Must a caller remember ordering, middleware nesting, a paired completion,
   cleanup sequencing, non-emptiness, deadline relationships or exhaustive
   outcome handling for the invariant to hold?
3. Can authority or lifecycle state be represented by a narrower capability,
   opaque validated witness, consuming transition or exhaustive enum instead of
   a broad cloneable handle, raw value, boolean or temporal convention?
4. Can library-owned assembly or execution make the supported composition the
   only ordinary path, rather than documenting which combination of individually
   valid calls is safe?
5. Is the remaining obligation truly application policy, upstream protocol
   behavior, or an unverifiable remote effect? If so, keep that boundary explicit
   and do not imply a stronger local guarantee.

When a common misuse can reach execution, or still compiles where a type-state
transition could prevent it, redesign the canonical path or record the evidence-
backed reason it cannot own the invariant. Typical tools are private constructors,
opaque validated values, nonempty collections, capability tokens, typestate or
consuming transitions, exhaustive results, and library-ordered composition.
These are means, not a requirement to introduce a framework or DSL. Prefer the
smallest native Rust boundary that removes the caller-memory obligation.

A justified low-level escape hatch may expose a weaker contract, but its name,
placement and documentation must distinguish it from the protected path and list
the obligations it leaves with the caller. Its existence does not justify making
the same invalid state representable through the canonical API.

### At-rest rewrap pairing assessment (`batter-t2uq`)

The earlier wrapper-only rotation returned a `WrappedKey` that the caller had to
attach to the input descriptor. Attaching it to another descriptor compiled and
failed authentication only when used. The canonical
`Keyring::rewrap_envelope` now authenticates once and returns the complete
`Envelope` with the input descriptor, so ordinary rotation has no separate
attachment step. A complete result cannot be passed directly as a `WrappedKey`
to another envelope's composer. The old wrapper-only signature and split-part
constructors remain for compatibility; their rustdoc marks them as lower-level
paths, and extraction can still reintroduce the mismatch. Cross-descriptor
authentication tests cover that deliberate escape hatch.

This local shape proves only the pairing returned by that call. Decoded and
reconstructed envelopes share the same type and may be unauthenticated; the
method does not read the body. Storage must retain header/body association,
compare-and-swap a complete prior header or covering revision, fence current
key policy, and enforce durable wrapper-encryption budgets. These remote and
application-owned effects cannot be proved by the returned Rust value. Fresh
agent implementation and modification evaluations remain proposed and
unexecuted for this API.

### Operation authority assessment (`batter-tc9w.1`)

A cloneable context previously let a consumer cancel the shared operation or
construct an unrelated root while processing a child callback. That made a
sibling or parent interruption possible through an ordinary execution argument.
The canonical path now passes cloneable `OperationContext` only for observation
and execution. Non-cloneable `OperationOwner` retains explicit cancellation;
child owners derive their deadline and token from the complete parent context.
`RootDeadline` preserves absolute time for an explicitly independent root, and
`OperationAdmission` returns an owner linked to process cancellation only after
readiness. Compile-fail rustdocs reject context cancellation, independent root
construction through a context, and owner cloning. Runtime regressions cover
child isolation, deadline clamping and process cancellation.

The application root still chooses when an independent operation is appropriate;
local types cannot prove that policy. An owner may deliberately give up its
authority with `into_context`, and dropping it does not cancel detached work.
The application must still await any work it owns and keep cleanup separate from
process cancellation. These are explicit ownership limits, not claims of async
drop or remote effect rollback. Fresh-agent usability evaluation is proposed
and unexecuted.

### Bounded metrics setup assessment (`batter-6vn`)

The first metrics API exposed `describe()` beside an application-installed
recorder. Calling it before installation compiled and silently lost every unit
and description, and a recorder built on another `metrics` major version
compiled and silently received nothing. The canonical path is now
`telemetry::metrics::install(recorder)`: it requires a recorder implementing
the re-exported `metrics` 0.24 `Recorder` trait, installs it, and only then
publishes the catalog, so both misuses are rejected at compile time or cannot
be written. A compile-fail rustdoc rejects a non-recorder value, and an
installation test checks descriptions and recording. Recording itself needs no
consumer calls: operations, retries, admission, tasks, cleanup and shutdown
record through library-owned guards with closed label vocabularies.

Installation also owns the catalog target: a local recorder scope cannot redirect
descriptions away from the accepted global recorder. A private forwarding
recorder holds a once-filled shared reference, preserving the upstream `Sync`
bound without adding `Send`. The accepted recorder is retained for the process
lifetime; a rejected recorder is returned intact. Setup fills the reference
before invoking recorder code, and concurrent registrations wait only for that
fill. The cost is one shared slot and a slot read per forwarded recorder call.
This keeps installation on the caller's thread and adds no consumer sequencing
requirement. The installation regression covers local scope dispatch and
rejected-recorder ownership.

An exporter that installs itself globally remains a lower-level escape hatch
that bypasses both checks; the module documentation names that obligation.
On that lower-level recorder path, flushing belongs to the application root
after it awaits supervisor completion; the protected OTLP path below owns it, and the recorder's own buffering, aggregation and faults cannot be
proved by local types. Fresh-agent usability evaluation is proposed and
unexecuted.

### Protected diagnostic completion assessment (`batter-i3ny`)

The reference application duplicated owner/observer monitoring and coordinated
startup failure cleanup, running settlement and final export. Its outer task
could discard an already-known service result if diagnostic finalization panicked.
The supported path now consumes `ScopedStartup` plus an inert prepared diagnostic
adapter through `service::start`. Consumers cannot pass an arbitrary future,
construct diagnostic completion observations, clone the owner, or call a separate
public final flush. Core retains the native result outside the diagnostic task,
then releases the completion observation and joins diagnostics independently.
Unix listeners remain installed before start returns. Read-only observers survive
owner loss; dropped waiters do not initiate shutdown.
The dynamically checked Tokio runtime precondition precedes diagnostic
installation, so an invalid call cannot consume the process-global recorder slot.

The lower-level `Diagnostics` trait allows custom adapter implementations. Its
synchronous install, phase ordering, I/O bounds and termination obligations are
explicit; custom trait forwarding can violate them and is not equivalent to the
supported OTLP path. An opaque witness describes retained reports, including
incomplete coverage, not cessation of unsupervised or remote activity. Application
exit policy, configuration sources and deployment restrictions stay at the root.
The catalog's kinds, units, descriptions and validation now have one core owner;
the opt-in adapter guards complete-key capacity before native bridge allocation.

Independent source review found a public timing hazard: nanosecond intervals could
make iterative missed-tick catch-up block finalization. Catch-up now uses constant-
time arithmetic with a saturating count. Behavioral regressions cover phase
panics, retained startup/cleanup errors, early owner loss and signal installation;
SDK/collector tests and a separate worker composition exercise adapter extraction.
Fresh-agent consumer implementation/modification evaluation remains unexecuted.

### Private-response referrer assessment (`batter-lto`)

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

### HTTP route group assessment (`batter-tc9w.3`)

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
authorization, CORS or complete CSRF protection; choosing
`without_mutation_checks` for cookie-authenticated mutations, or letting a safe
method change state, stays application policy. Group membership is path-level,
so one path served with two budgets by method must use one group, and unmatched
paths under a named group's prefix receive the default group's policy. The
overlap analysis mirrors the pinned matchit 0.8.4 rules and needs its Axum
comparison test re-run whenever that pin changes. Routers built outside
`GuardedRouter` (`batter-tc9w.8`), real consumer ports (`batter-tc9w.9`) and
sealing the low-level middleware (`batter-tc9w.2`) remain separate tasks.
Fresh-agent usability evaluation is proposed and unexecuted.

### Reference HTTP boundary assessment (`batter-tc9w.4`)

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

The in-process client still takes the router through `AssembledHttp::into_router`
until `batter-tc9w.2` supplies an opaque request-only client; the reference
type keeps it private and unservable. These checks prove local ordering only,
not proxy trust or any remote client behaviour. Fresh-agent usability
evaluation is proposed and unexecuted.

### Admitted router assessment (`batter-tc9w.8`)

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
the admitted router cannot be declared. Each admitted router adds one
library-owned inspection routing to each request that no native route matches.
Layer state inside an admitted router, and around the default group's
fallbacks when admitted routers exist, is shared by all of their requests, as
with Axum's make-service conversion, even where `axum::serve` would prepare a
native router per connection. No
OpenAPI-builder dependency or convenience is added. Fresh-agent usability
evaluation is proposed and unexecuted; the downstream ports belong to
`batter-tc9w.9`.

### Probe renderer and readiness condition assessment (`batter-tc9w.5`)

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

### Admitted request extractor assessment (`batter-tc9w.6`)

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
canonical path every guarded route and fallback is admitted, which leaves routes
added to the router taken from `AssembledHttp::into_router` and low-level
compositions as the ways to reach the rejection. `batter-tc9w.2` owns replacing
`into_router`. The facade HTTP example and the reference service now extract
the admitted request. Tests cover every group's own context and generated
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

### Native package reachability assessment (`batter-0jsf`)

The facade exposed `batter::runledger` and `batter::runlimit` but not the native
packages they translate. `batter-runledger` re-exported selected
`runledger_postgres` items and held `runledger-core` only as a dev-dependency, so
worker preparation, `JobsConfig`, `JobCatalog`, durable intents and the migrator
were unreachable; `batter-runlimit` re-exported `runlimit-core` and, behind
`postgres`, `runlimit-postgres`, but not `runlimit-memory`, `runlimit-http` or
`runlimit-axum`. Both surveyed consumers therefore declared the same workspace
revision two to eight times, and the cost was structural: every new capability
added another declaration to keep in step, and one consumer fell 139 commits
behind. Repeated declarations a consumer must keep consistent are the same signal
this ADR names for repeated coordination instructions.

Each native library package is now reachable through a feature-gated namespace
that is the native package itself, so facade and direct paths cannot diverge into
two identities. The invalid state this removes is a consumer holding two source
identities for one workspace; it is now unrepresentable through the documented
recipe because there is nothing left to declare separately.

The namespaces are deliberately weaker than the protected path, and the review
question is whether they make a known-invalid composition look equivalent to it.
They do not, and the naming carries that:

- `runledger::native::{core, postgres, runtime}` is reached through a segment
  named `native`, and its module documentation states the obligations a caller
  takes on by building a live native supervisor instead of handing inert
  preparation to `register_in`: observing initialization, requesting shutdown
  within a budget, classifying the settlement and ordering dependency cleanup.
  `register_in` still accepts only a `PreparedSupervisor`, and the existing
  compile-fail rustdocs still reject a live supervisor and a closure hiding one.
  Reachability adds no way to pass a live supervisor through the protected
  boundary.
- `runlimit::native_transport::{http, axum}` is named apart from
  `runlimit::http`, which remains the protected quota-before-body assembly. The
  native layer's documentation states that subject derivation, rejection mapping,
  status and body selection stay with the caller and that it acquires no operation
  deadline, admission ordering or telemetry from the foundation. Its features
  select neither `batter-axum` nor `runlimit::http`, so the two never appear as
  interchangeable spellings of one capability.
- `runledger::native::test_support` is documented as test-only and must not be
  selected in a deployed graph. This is application policy: Cargo cannot express
  "development graphs only" for a feature, and the facade cannot detect the
  caller's profile. The recipe requires resolver 2 or 3 for dev-dependency
  feature separation and an explicit resolver at a virtual workspace root;
  resolver 1 also enables these features in ordinary dependency builds.

What types cannot express here is deliberate. These are the native packages'
own APIs; Batter does not narrow them, and narrowing them would create the second
identity the task exists to remove. The remaining boundary is that a consumer can
reach a low-level native path where a protected one exists. That is the standing
escape-hatch position of this ADR, not a new weakening, and the documented
canonical path is unchanged. Fresh-agent usability evaluation of the recipe is
proposed and unexecuted.

### Generic serving listener assessment (`batter-tc9w.7`)

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
Batter installs no crypto provider and takes no TLS dependency outside test
material. A listener that spawns handshakes onto the runtime instead of polling
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

### Native grant fragment assessment (`batter-qjxi`)

Before this change, an application that wanted least-authority roles for the
native Runledger and Runlimit stores had to hand-maintain their relation, column
and privilege lists beside its own declarations. Nothing in the API could reject a
stale or over-broad list, and two audited consumer shapes had already drifted
apart. `GrantFragment` moves that knowledge to the library that owns the SQL.

Construction answers the first two questions. `grant_fragment` takes a non-empty
operation selection and rejects an empty one before any declaration exists, so a
selection that could never authorize work is not representable as a compiled
value. A fragment's column group requires that fragment to already declare the
parent relation, so an application never attaches a parent declaration for a
native relation and discovery defaults never silently choose that relation's
ownership, row-type or PUBLIC behavior. Relation and column names are validated
inside `FragmentObjectPolicy` and always qualified by its schema, so a fragment
cannot reach an object outside the schema the application named.

Partial state is unrepresentable rather than documented. Every fragment builder
and `ExactRoleManifest::with_fragment` consume their value and return
`Result<Self, _>`, so a rejected declaration or a declaration-bound overflow
yields no partially extended fragment and no manifest to compile. There is no
`&mut` append that could leave an executable partial grant set, and no second
compiler or renderer: a fragment reaches the one existing
`ExactRoleManifest::compile` and `GrantPlan::render`, which already normalize
duplicate and reordered declarations and reject contradictory purposes, PUBLIC
choices, row-type options and object/privilege pairs.

Authority is named by behavior, not by role. `RunledgerOperation` and
`RunlimitOperation` are non-exhaustive enums of operation groups, with no
administrator preset and no application role name, so a caller cannot ask for
"all privileges" and cannot accidentally select the privileged full-schema
snapshot, durable promotion, catalog synchronization or scheduled dispatch while
asking for intent submission. Grant options are never declared and every
declaration is required and provisioned, so a fragment cannot smuggle in an
allowance the application did not ask for.

What remains with the application is explicit and deliberately not implied to be
stronger. A fragment cannot carry the current-database declaration, discovery
scope or defaults, role-attribute ceilings or the current-database ownership
guard, and `FragmentObjectPolicy` makes the application state its PUBLIC
delivery, row-type and ownership choices rather than inheriting a library
default. Which operations a deployment runs is application policy; selecting one
it never invokes provisions authority it does not need, and no local type can
detect that. The requirements describe this source version's statements: they are
not evidence that the schema is installed, that a remote effect occurred, or that
an application's authorization model is correct, and rendering grants cannot
remove privileges a provisioned role already holds. Required `UPDATE` and
`DELETE` privileges, including row-locking `UPDATE` columns, remain arbitrary SQL
authority over those rows, so no payload-secrecy, tenant-isolation, immutable
identity or history-protection guarantee follows from a narrower column list.
Installed trigger ownership, routine bodies and application-added trigger
dependencies stay explicit prerequisites. Fresh agent implementation and
modification evaluations remain proposed and unexecuted for this API.

## Recurring example review defects

The implementation agent must initiate an assessment when the same confirmed
invariant fails again after repair, or when a repair introduces a confirmed
failure in a coupled lifecycle phase, such as cleanup racing unfinished work.
Do not wait for the user to notice. An existing review-round limit is a backstop,
not a reason to postpone assessment; reviewer comment counts alone are not a
trigger or proof of a design gap. "It is only example code" cannot dismiss the
signal: examples are consumer contracts for the library's intended agents.

The proactive review above applies even without a failure. When recurrence or a
coupled-phase failure supplies stronger evidence, complete these steps before
another dependent repair:

1. Identify the recurring defects with source, failure-scenario, and repair
   evidence. Separate confirmed defects from duplicate or unsupported reviewer
   concerns and changed requirements; label missing execution evidence.
2. Trace each invariant to what the public API enforces and what the caller must
   remember. Identify the consumer, foundation, adapter, or upstream owner.
3. Classify the cause using the distinctions below. State whether another local
   patch removes the cause or merely covers another case of the same protocol.
4. Record the concern, evidence, alternatives, and recommended owner/remedy in
   the owning Bead; create a linked design Bead when separate scope is needed.
   Explicitly report the assessment to the user before continuing dependent
   repairs. Beads owns any resulting delivery scope and dependencies.

| Cause | Response |
| --- | --- |
| Implementation mistake | The supported API already enforces the invariant when used correctly. Fix the consumer and clarify misleading examples or guidance. |
| Library or adapter design gap | The consumer repeatedly reconstructs ownership, cleanup, timing, registration, or error-retention protocols. Assess moving the shared obligation into library-owned execution or a justified supported adapter. Another caller instruction alone does not close the design concern. |
| Application or upstream protocol complexity | The behavior belongs to application policy or the upstream dependency. Reassess whether the example needs it and whether reusable upstream lifecycle mechanics justify an adapter; preserve the upstream's responsibilities. |

Causes can coexist; keep uncertain classifications explicit. Assess reducing
optional example behavior without weakening required consumer contracts or
semantic failure tests to make a review pass. Clean review is not proof of
agent usability.

Implement a bounded remedy when evidence supports it and it is already within
the user's authorized scope. If the remedy expands scope or changes a public
contract beyond that authorization, present the concrete problem, alternatives,
and recommended scope change for a user decision before dependent changes.
Continue independent authorized work. Do not silently redesign the library or
continue an indefinite sequence of patches around the same caller obligation.
Validate an accepted remedy against the recurring failure scenarios and retain
the distinction between executed checks and proposed fresh-agent evaluations.

## Consequences and limits

This guidance preserves native errors, optional adapters, Unix scope, dependency
direction, and the existing honest limits: no asynchronous `Drop`, runtime-death
survival, or detached-task termination guarantee. It does not make unimplemented
adapters available, redesign APIs, or turn every application root into a
foundation protocol.
