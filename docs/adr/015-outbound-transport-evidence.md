# ADR-015: Outbound transport evidence

## Status

Accepted, 2026-11-02. Owning Bead: `batter-3s1a`. The repository owner approved
the assessment before this record was written and asked for its
implementation. It adds the standalone package `batter-outbound-http` and the
facade's opt-in `outbound-http` feature. This is a focused experimental extraction
under [ADR-014](014-reuse-after-two-consumers.md#stable-adapter-promotion)
track 1, not a stable promotion. It does not reopen the provider adapters or
the generic provider interface that ADR-014 rejected.

Naming amendment, 2026-11-03 (`batter-r2kc`): before any release, the package
was renamed from `batter-outbound`, the facade feature from `outbound` and the
facade module from `batter::outbound`, so the name states that it is HTTP and
does not suggest an outbox or a retrying delivery layer. No released name
changed, so no `Breaking:` entry, `Migration:` note or deprecation shim applies.

Public-destination amendment, 2026-11-05 (`batter-321j`): add opt-in
`Origin::public_https`. Address filtering belongs at the actual dial boundary;
a separate caller DNS check cannot constrain a later lookup. Existing origin
constructors and the exchange evidence vocabulary remain compatible.

## Context

A provider call can fail in a way that leaves the caller unsure whether the
provider received the request. Every decision above the call depends first on
one transport question: could this attempt have reached the peer? Those
decisions include replaying the request, releasing a dispatch fence, refunding
a budget and reconciling later. With reqwest, the answer depends on how the
client was built and on which error predicates the caller checks. Its defaults
follow redirects, replay some requests and read bodies without a bound. Batter
had no outbound HTTP API when this decision was proposed. `batter-3s1a` asks
whether the transport part, below provider meaning, is shared enough for
Batter to own.

### Evidence

On 2026-11-02 the three applications of ADR-014 were surveyed again,
read-only. As in ADR-014, their identities, paths and commits stay outside this
repository. The survey read source and commit history and modified no
consumer, and its counts were taken at the inspected consumer commits, not in
this repository.

- **One rule in every root.** Consumers A, B and C each hand-write the same
  transport rule. A connector failure proves that nothing was sent, any later
  failure leaves the outcome unknown, and a received status line is a
  response. The same rule also appears in three other places:
  - an owner-maintained billing library, which is a library, not an
    application root;
  - batter-otlp's private `BoundedClient::exchange` in
    `crates/batter-otlp/src/pipeline/transport.rs`;
  - the reference service's provider client in
    `examples/reference-service/src/provider.rs`.
- **The other parts come in pairs.** Each remaining part is shared by a
  different pair of roots, never by all three:

  | Part | Roots that hand-write it |
  | --- | --- |
  | The transport rule above | A, B and C |
  | A client that follows no redirects, never replays and ignores proxies | A and B |
  | A capped body read under the response deadline | B and C |
  | A deadline gate before the first poll | A and C |

  The bundle is therefore justified by coupling under
  [ADR-010](010-agent-only-consumption.md), not by two-root evidence for the
  whole. A connector failure proves that nothing was sent only when the same
  code also disables redirects and replays. The body bound must sit inside the
  deadline that bounds the response head.
- **Repairs.**
  - C needed about twelve dispatch-evidence repairs within three days. It
    hand-copied URL stripping across 22 endpoints until a lint banned direct
    sends, and it retrofitted start-before-deadline gates into 27 gateway
    methods.
  - B lost a received status when the body stalled under one shared timer, and
    the fix was copied between provider crates.
  - A needed four configurations to nest its transport timer inside the retry
    attempt cap.
  - The billing library repaired its status-to-certainty mapping repeatedly.
- **Drift.** One of C's clients disables redirects but leaves reqwest's HTTP/2
  protocol-NACK replay live. An unrelated dependency enables HTTP/2 for the
  whole build through Cargo feature unification. The same client honours proxy
  environment variables, sets no connect timeout and leaves a credential header
  unmarked. A reads response bodies without a bound.
- **Adoption.** B's provider crates forbid any batter-named, Git or path
  dependency, dev-dependencies included. B is therefore evidence of need, not
  an adopter. The realistic first adopters are A's direct client and C's
  clients. The billing library would have to move from reqwest 0.12 to 0.13.
- **Size.** The most expensive repairs concern status-to-meaning mappings,
  fences and persistence. Those are provider and application policy that this
  package deliberately does not own. The transport part is roughly 40 to 100
  lines per copy. The package removes a class of silent mistakes, not most of
  the work.

### Transport ownership correction, 2026-11-04

The original implementation used reqwest's pool. Its first-poll start gate
could not see the later synchronous handoff to hyper's connection, or prevent
a spawned connection driver writing after the exchange task stalled. No replay
or redirect restriction closed that same-request timing gap. This is a library
design gap under ADR-010, corrected by `batter-u387` at the user's request.
The corrected ownership model's upstream evidence is recorded in
[references](../references.md#owned-outbound-http1-connections-2026-11-04).

## Decision

Keep the standalone send-once package and its existing public request/outcome
API. Replace the pooled transport with one fresh TCP/TLS connection and hyper's
low-level HTTP/1 driver per exchange. The exchange future drives that connection
itself; no driver or writer is spawned. hyper-util supplies only Tokio I/O
adaptation. reqwest is dev-only for proxy and feature-unification controls.

The client still fixes one validated origin, an explicit ring rustls
configuration, TLS 1.3/1.2, ALPN http/1.1, the operating system verifier for
HTTPS and empty roots for loopback HTTP. There is no connection reuse, pool,
resend, redirect, proxy lookup, content decoder or HTTP/2. Each exchange pays
fresh connection/TLS setup; clones share only configuration.

### Phases and gates

Before handoff, cancellation and deadline win over connection progress. The
connect timeout covers DNS, TCP, TLS and HTTP/1 setup under the exchange's
single deadline, the caller's deadline minus RESERVE. Entry still requires
MINIMUM_ATTEMPT inside that deadline. The final gate immediately precedes the
synchronous send_request call, with no await between them.

- NotStarted: no handoff because the entry budget was too short or the exchange
  expired/was cancelled during setup or at the final gate.
- NotDispatched: setup failed or its connect timeout elapsed before handoff.
- Unknown: handoff occurred, but no accepted response head is available.
- Responded: a response head is retained, including when the body fails.

The first two outcomes carry opaque library-created unsent proof. They follow
owned phases, rather than reqwest's is_connect or hyper-util's private error
kinds. No exact hyper-util implementation or pool-resend behavior determines
the classification.

After handoff, available replies win over cancellation and deadline. The
connection may read a reply after stopping, but every write, vectored write,
flush and shutdown checks the gate before polling its underlying transport.
Both the plaintext TLS stream and the TCP stream beneath it are guarded,
because TLS reads may flush encrypted records. This preserves ready-response
precedence without allowing a stalled writer to resume after interruption.
The bounded body read uses the same driver and deadline; status survives all
body failures. Dropping the exchange drops the connection and yields no outcome.

### Ownership and compatibility

The default leaf has no batter-core dependency. Opt-in operation adds send_in
over OperationContext; the facade selects it with outbound-http. Preparation,
reserved headers, path/query encoding, sensitive values, immutable client
configuration, consuming non-Clone requests and exhaustive must-use outcomes
remain. The facade example remains the consumer contract. Public names,
signatures, variants and labels are unchanged, so no compile-time hard cut or
deprecated shim is required. No pooled compatibility path is retained.

Provider meaning, replay authorization, idempotency keys, Retry-After, dispatch
fences and reconciliation remain with applications. No provider trait, retry
owner, telemetry layer, connection pool or application framework is introduced.

## Alternatives and limits

A consumer poll_before wrapper can close late handoff only for its own polls;
it cannot control a spawned writer and can discard a ready response. Keeping
the pool with more caller instructions therefore fails ADR-010. Changing
batter-core's arbitrary-future precedence would permit new application effects
after expiry and is outside this transport boundary.

Deadlines remain cooperative: already-entered polls, synchronous macOS platform
verification and thread preemption between check and I/O cannot be interrupted.
Kernel-accepted bytes may reach the provider later through normal transport or
retransmission. Unknown remains real remote uncertainty. A post-handoff failure
may also be Unknown for an unwritten request; no inferred unsent proof is added.
An outer run/retry/timeout can discard the Exchange under its own precedence.
Reserve finalization time separately from RESERVE for durable recording.

The package remains an experimental ADR-014 track-1 extraction. Earlier local
macOS evidence (`batter-8e8r`) predates this transport replacement. New platform
and verification claims require executed evidence in `batter-u387`.
[Guarantees](../guarantees.md#outbound-provider-exchange) and
[references](../references.md#owned-outbound-http1-connections-2026-11-04) name
current tests and inspected upstream versions. Fresh-agent adoption remains
proposed and unexecuted; two real application roots are still required for
stable promotion.
