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

## Context

A provider call can fail in a way that leaves the caller unsure whether the
provider received the request. Every decision above the call depends first on
one transport question: could this attempt have reached the peer? Those
decisions include replaying the request, releasing a dispatch fence, refunding
a budget and reconciling later. With reqwest, the answer depends on how the
client was built and on which error predicates the caller checks. Its defaults
follow redirects, replay some requests and read bodies without a bound. Batter
had no outbound HTTP API. The [Effect reconciliation](../effect-v4-reconciliation.md)
recorded that as the current boundary, not as a decision. `batter-3s1a` asks
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

### Upstream behaviour

These facts were read from the registry sources that this workspace's
`Cargo.lock` resolves: reqwest 0.13.5, hyper-util 0.1.20, tower-http 0.6.11,
url 2.5.8, http 1.5.0, rustls 0.23.46, rustls-platform-verifier 0.7.1 and
rustls-native-certs 0.8.4. Each path is relative to its package's source root.

- **Error text.** reqwest's `Error` Display appends the full URL, including
  its query (`reqwest-0.13.5/src/error.rs:301`). The type's own rustdoc warns
  about API keys in a query (`src/error.rs:17-19`).
- **Body size.** reqwest has no response size limit.
  - `bytes()` collects the whole body (`src/async_impl/response.rs:290-292`,
    `430-437`), and `json()` calls it (`269-273`).
  - `chunk()` returns one frame at a time and is the only bounded hook
    (`310-324`).
- **Redirects.** The default policy is `limited(10)` (`src/redirect.rs:160-165`).
  - On 307 and 308, tower-http keeps the method and the body
    (`tower-http-0.6.11/src/follow_redirect/mod.rs:274-291`).
  - A hop to another host strips only `Authorization`, `Cookie`, `Cookie2`,
    `Proxy-Authorization` and `WWW-Authenticate` (`src/redirect.rs:239-252`).
    The body and any custom credential header travel to the new host.
  - `Referer` is on by default (`src/async_impl/client.rs:313`) and keeps the
    previous URL's query (`src/redirect.rs:294-304`).
- **Retries.** The default policy retries protocol NACKs, up to two extra
  attempts per request and without a budget (`src/retry.rs:195-203`).
  - When the `http2` feature is compiled, it sends the request again after a
    remote GOAWAY(NO_ERROR) or REFUSED_STREAM. It does not look at the method
    (`src/retry.rs:273-318`).
  - `http2` is a reqwest default feature (`Cargo.toml:113-118`). Any crate in a
    build can enable it through feature unification.
- **Proxies.** Discovery from `HTTP_PROXY`, `HTTPS_PROXY` and `ALL_PROXY` is on
  by default (`src/async_impl/client.rs:310`, `417-419`;
  `hyper-util-0.1.20/src/client/proxy/matcher.rs:99-111`, `228-236`).
- **TLS.** reqwest's `rustls` feature compiles rustls's aws-lc-rs provider
  (`Cargo.toml:86-91`, `169-173`); `rustls-no-provider` compiles none (`174-177`).
  - When rustls 0.23.46 has both *ring* and aws-lc-rs compiled,
    `ClientConfig::builder()` panics unless the process installed a default
    provider first (`rustls-0.23.46/src/crypto/mod.rs:243-286`). When reqwest
    builds its own rustls configuration without a compiled provider feature,
    it panics too unless a default is installed
    (`src/async_impl/client.rs:719-723`, `2498-2511`).
  - `tls_backend_preconfigured` hands a `rustls::ClientConfig` to the connector
    as given: reqwest applies none of its own ALPN, version or verifier settings
    to it (`src/async_impl/client.rs:2209-2235`, `643-687`;
    `src/connect.rs:383-390`).
  - On Linux, rustls-platform-verifier loads the host certificate store when the
    verifier is built and fails when the store is empty
    (`rustls-platform-verifier-0.7.1/src/verification/others.rs:85-111`).
    rustls-native-certs reads only `SSL_CERT_FILE` or `SSL_CERT_DIR` when either
    is set (`rustls-native-certs-0.8.4/src/lib.rs:48-57`, `118-124`). On macOS,
    verification calls Security.framework synchronously during the handshake
    (`rustls-platform-verifier-0.7.1/src/verification/apple.rs:209`, `247`).
- **Connect evidence.** reqwest documents `Error::is_connect` only as "related
  to connect" (`src/error.rs:157`).
  - It looks for a hyper-util legacy error of kind `Connect` in the source
    chain (`src/error.rs:158-172`).
  - hyper-util 0.1.20 assigns that kind to connector failures
    (`src/client/legacy/client.rs:517`) and to pool-checkout failures (`377-378`,
    `464`, `473`).
  - Both happen inside `connection_for`, which completes before the request is
    handed to a connection: `279-284` precede `324`.
  - reqwest's connect timeout wraps the whole connector, including DNS, TCP and
    TLS (`src/connect.rs:904-917`, `950-953`).
- **Timeout evidence.** `Error::is_timeout` is true for any
  `io::ErrorKind::TimedOut` in the source chain (`src/error.rs:118-149`). That
  includes a kernel connect timeout or an expired `TCP_USER_TIMEOUT`, which
  reqwest sets to 30 seconds on Linux (`src/async_impl/client.rs:307-308`). It
  does not mean that the caller's deadline passed.
- **Repeated sends below the client.** hyper-util's `retry_canceled_requests` defaults
  to true (`src/client/legacy/client.rs:1034`), and reqwest never sets it.
  - When a reused pooled connection hands back a request that it never wrote,
    hyper-util sends the request again on another connection (`241-272`).
  - A handshake or readiness failure after the connector becomes `SendRequest`,
    not `Connect` (`547`, `559`, `570`, `1663-1665`).
- **Paths.** `url`'s `PathSegmentsMut::extend` silently drops `.` and `..`
  segments (`url-2.5.8/src/path_segments.rs:247-249`). It percent-encodes `/` and
  `%`, as well as the path set, which includes `?` and `#` (`src/parser.rs:23`,
  `38`). It leaves `;` unencoded (`38`, `42`). Parsing deletes tab, line feed and
  carriage return before it applies the dot-segment rules (`76-80`, `1240-1250`,
  `1312-1342`), so `.`, tab, `.` would become `..`.
- **URI length.** reqwest converts the URL to an `http::Uri` only when the
  request is executed, and a builder error is returned if that conversion
  fails (`reqwest-0.13.5/src/async_impl/client.rs:2639-2642`). `http::Uri`
  refuses more than 65,534 bytes (`http-1.5.0/src/uri/mod.rs:145`, `296-298`).

These facts are the standards context:

- [RFC 9110 section 9.2.2](https://www.rfc-editor.org/rfc/rfc9110.html#section-9.2.2)
  says a client should not automatically retry a non-idempotent request unless
  it has some means to know that the request semantics are idempotent
  regardless of the method, or some means to detect that the original request
  was never applied. `proves_unsent()` is evidence of the second kind, for one
  attempt.
- [RFC 9113 section 8.7](https://www.rfc-editor.org/rfc/rfc9113.html#section-8.7)
  lets an HTTP/2 peer assert that a request was not processed, through
  REFUSED_STREAM or the last stream identifier of a GOAWAY.

This package does not support HTTP/2.

## Decision

Batter adds `batter-outbound-http`. It owns one question for one HTTP request,
whether this attempt could have reached the peer, and nothing above it. The
following stay with applications or provider crates:

- what a status or body means;
- whether a request may be replayed, and idempotency keys;
- Retry-After;
- dispatch fences and reconciliation;
- token acquisition and request signing;
- webhook ingress;
- HTTP/2 and operator-supplied destinations.

### Placement

The package is a standalone leaf, following the `batter-at-rest` precedent. By
default it depends on neither `batter-core` nor the facade, so a provider crate
that does not use Batter's runtime can still use it.

- The opt-in `operation` feature adds
  `PreparedRequest::send_in(&OperationContext, ResponseLimit)` and depends on
  `batter-core`.
- The facade re-exports the package as `batter::outbound_http` behind its opt-in
  `outbound-http` feature, which enables `operation`.
- reqwest 0.13.5 is built without default features and with
  `rustls-no-provider`, so it compiles no crypto provider of its own. The
  package builds its own rustls 0.23 `ClientConfig` and hands it to reqwest
  through `tls_backend_preconfigured`, which uses it as given. The
  configuration:
  - takes the *ring* provider explicitly (`builder_with_provider`), the provider
    this workspace already uses, so the package never installs or reads a
    process-default provider;
  - offers TLS 1.3 and 1.2 only (`with_protocol_versions`);
  - verifies HTTPS origins with the operating system's verifier
    (`rustls-platform-verifier`), and gives `Origin::loopback` clients an
    empty root store, so a plain-HTTP client never reads the host certificate
    store;
  - sets ALPN to `http/1.1` itself.

  The workspace lockfile therefore contains no aws-lc-rs. Feature unification
  cannot switch the backend either: reqwest selects rustls for a
  preconfigured rustls configuration even when another crate compiles its
  native-tls support (source-inspected).
- The public API uses `http` 1.x types, re-exported as `batter_outbound_http::http`,
  and `Bytes`. No reqwest type is public.

### Client profile

`Client::new(Origin, ConnectTimeout)` is the only constructor, and a built
client cannot be reconfigured. Every client:

- follows no redirects. A 3xx is returned as a response, so a connect failure
  can never belong to a later hop of a request whose first hop was delivered;
- offers only `http/1.1` in ALPN, through the package's own TLS configuration,
  even when another crate compiles reqwest's HTTP/2. The ClientHello test
  checks this with `http2` compiled into the test build;
- calls `no_proxy()`, so proxy environment variables are ignored. A test
  covers `HTTP_PROXY` and `http_proxy` for a loopback origin.
  `HTTPS_PROXY` and `ALL_PROXY` for an HTTPS origin go through the same
  disabled lookup and are source-inspected;
- turns off gzip, brotli, zstd and deflate, so the body limit counts body bytes
  as received: chunked framing is removed and content coding is not. gzip is
  tested with its feature compiled. brotli, zstd and deflate are not compiled
  in this workspace and are source-inspected;
- turns off hickory DNS, so feature unification cannot change the resolver.
  The feature is not compiled here, so this is source-inspected;
- requires a `ConnectTimeout` above zero and at most 60 seconds;
- never reuses a connection idle for more than four seconds, below the
  five-second keep-alive timeout that many servers use by default
  (unverified). hyper-util may keep an idle connection open for up to about
  eight seconds before it evicts it (source-inspected). The
  `idle_connections_are_closed_before_common_server_keep_alive_timeouts` test
  shows that a request after a 4.5-second pause opens a new connection. This
  makes the stale keep-alive race less likely; that race is reported `Unknown`;
- uses the *ring* TLS configuration described under Placement. On Linux, an
  HTTPS client reads the host certificate store when it is built; a loopback
  client never reads it;
- has no client-wide total or read timeout, because each send's deadline
  bounds its exchange.

Also set as defence in depth: `http1_only()`, `reqwest::retry::never()`,
`referer(false)` and `https_only` for an HTTPS origin. None of them is
observable through the profile:

- With ALPN fixed to `http/1.1`, removing `http1_only()` fails no test.
- reqwest's retries act only on HTTP/2 protocol NACKs, and HTTP/2 is never
  negotiated.
- reqwest sets a `Referer` only on a followed redirect.
- Every request URL comes from the validated HTTPS origin.

Cloning shares the connection pool. `Debug` shows no configuration.

### Preparation

- **Origins.** `Origin::https` accepts only `https://host` or
  `https://host:port`, with an optional trailing slash. `Origin::loopback`
  accepts plain HTTP only to a literal loopback socket address. Userinfo, a
  path, a query, a fragment and port zero are refused, and so is a host that
  `http::Uri` rejects (`OriginError::Unparseable`). `Origin`'s `Debug` shows
  only the scheme kind.
- **Paths.** `client.request(method, segments)` builds a path below the origin
  from separate segments. Each segment is percent-encoded, so `/`, `?`, `#`
  and `%` stay inside it: `["v1", "effects"]` targets `/v1/effects`, while
  `["v1/effects"]` is one encoded segment. These segments are refused:
  - an empty, `.` or `..` segment, which `url` would drop silently;
  - a segment whose part before the first `;` is empty, `.` or `..`, such as
    `;x` or `..;x=1`, because servers that strip `;` path parameters would
    treat it the same way;
  - a segment containing a tab, line feed or carriage return
    (`ControlCharacter`), because parsing deletes those characters before its
    dot-segment rules.
- **Headers.** Every value passed to `header()` is marked sensitive; the
  content type set with a body is not. A unit test checks the prepared
  request's headers. These headers are reserved and refused: `Host`,
  `Content-Length`, `Transfer-Encoding`, `Connection`, `Upgrade`, `TE`,
  `Trailer`, `Expect`, `Keep-Alive`, `Proxy-Connection`, `Proxy-Authorization`
  and `Content-Type`.
- **Body.** A request has one body, set with `json` or with `body` and its
  content type.
- **Errors.** `CONNECT` is refused. `prepare()` reports the first problem it
  finds as a value-free `PrepareError`: `EmptySegment`, `DotSegment`,
  `ControlCharacter`, `InvalidUri`, `ReservedHeader`, `BodyAlreadySet`, `Json`,
  `ConnectMethod` or `Build`.
  - `InvalidUri` refuses a URL that `http::Uri` rejects, for example one longer
    than 65,534 bytes. reqwest would refuse it only when the request is
    executed.
  - Builder failures that `prepare()` detects become `Build` before an
    exchange exists. `Build` also covers a check that every segment survives
    URL normalization as exactly one segment. That check is defence in depth:
    with url 2.5.8 the refusals above leave no input that reaches it.
  - The `Debug` output of the draft and of the prepared request shows nothing
    about the request.
- **Sending once.** `PreparedRequest` is not `Clone`, and `send` consumes it;
  `compile_fail` doctests keep both. Every further attempt is a new
  preparation in application code.

### Outcomes

`PreparedRequest::send(self, deadline, limit)` returns one `Exchange`. It is
exhaustive and `#[must_use]`.

| Outcome | Carries | What this attempt established |
| --- | --- | --- |
| `NotStarted` | `NotStarted`, whose `reason()` is a `StartRefusal`: `DeadlineTooClose` or `Cancelled` | The request was never handed to the HTTP client. |
| `NotDispatched` | `NotDispatched`, whose `failure()` is a `ConnectFailure`: `TimedOut` or `Failed` | reqwest's `is_connect` reported a failed connection, so this attempt wrote no request bytes. |
| `Unknown` | `UnknownReason`: `Deadline`, `Cancelled` or `Transport` | The request may have reached the peer, and no response head that the HTTP client accepted arrived. The request may also still reach the peer after this outcome is returned. |
| `Responded` | `Response`: the status, the headers and a `Body` | A response head arrived, possibly from an intermediary. The status is kept when the body then fails. |

- **Body.** The body is one of three values.
  - `Body::Complete` holds the body as received within the `ResponseLimit`,
    which is one byte to 64 MiB, after chunked framing is removed. A response
    with neither `Content-Length` nor chunked framing ends when the peer closes
    the connection, so its truncation cannot be detected
    (`an_undelimited_body_ends_at_connection_close`).
  - `Body::TooLarge` reports a declared `Content-Length` above the limit,
    without reading. It also reports a streamed body, which stops at the first
    chunk that would exceed the limit.
  - `Body::Failed` carries `Deadline`, `Cancelled` or `Transport`.
  - The buffer grows only up to the limit and releases its spare capacity
    before `Body::Complete`, so its memory stays within the limit. This is
    source-level and not separately tested. Preallocation from a declared
    length is capped at 1 MiB, which no test exercises.
- **Headers.** Every received header value is marked sensitive.
- **Evidence and labels.** `proves_unsent()` is true only for `NotStarted` and
  `NotDispatched`. It covers this attempt only. `label()` returns a stable,
  value-free label, from `not_started.deadline` to `responded.body_transport`.
- **Unforgeable unsent evidence.** Only the package produces the two outcomes
  that prove nothing was sent.
  - Their payloads, `NotStarted` and `NotDispatched`, have no public
    constructor and are not `Clone`, and a `compile_fail,E0451` doctest keeps
    the constructor private. `reason()` and `failure()` return plain `Copy`
    enums for matching.
  - Code that wraps an exchange therefore cannot turn an interruption into
    proof that nothing was sent. A wrapper that loses an exchange can build
    only `Exchange::Unknown(..)`, which stays constructible because it claims
    nothing.
- **No escape from the match.** Every outcome enum is exhaustive. A `match`
  therefore needs no wildcard arm that could absorb a later variant, and
  adding a variant is a public hard cut. A wildcard arm, `if let` or
  `matches!` still compiles. Only a consumer-side lint can forbid one, as the
  [ADR-010 assessment](010-agent-only-consumption.md#outbound-exchange-assessment-batter-3s1a)
  records. No `From` conversion produces an outcome. The package adds no
  instrumentation of its own; its dependencies' log events, which can name the
  destination host, socket addresses and certificate names, are listed in the
  [references](../references.md#reqwest-0135-and-hyper-util-0120-for-the-outbound-exchange-2026-11-02) and are
  filtered by the application's logger.

### Deadlines

`RESERVE` and `MINIMUM_ATTEMPT` are public constants, both 50 ms.

1. The exchange deadline is the caller's deadline minus `RESERVE`. The send
   returns `NotStarted(DeadlineTooClose)` in two cases: the subtraction
   underflows, or less than `MINIMUM_ATTEMPT` remains before the exchange
   deadline. The check and the first poll of reqwest's request future run in
   the same task poll, with no await between them. A request is therefore
   never first polled inside the last `RESERVE + MINIMUM_ATTEMPT` of the
   caller's budget, apart from thread preemption between the check and that
   poll.
2. `send_in` returns `NotStarted(Cancelled)` for a context that is already
   cancelled. It checks this before the deadline, matching the precedence of
   cancellation over deadline in `OperationContext::check`. Another thread can
   cancel the context after that check and before the first poll. The outcome
   is then the conservative `Unknown(Cancelled)`, not `NotStarted`
   (source-inspected).
3. One timer at the exchange deadline bounds both the response head and the
   body read. reqwest's total and read timeouts are not set; its connect
   timeout is the `ConnectTimeout`. Deadline reasons come from this timer,
   never from `is_timeout`.
4. The exchange therefore returns its classification at least `RESERVE` before
   the caller's deadline, apart from scheduling delay and the macOS limit
   below, so an outer timer at the same deadline does not discard it.
   `RESERVE` is not a recording budget:
   - Send in the work phase of `OperationContext::reserve_finalization` and
     record a durable outcome in its finalization phase.
   - A cancelled context runs no recording step, including one whose parent
     was cancelled. The dispatch fence that the application wrote before
     sending is then what remains held.

Sending must run inside a Tokio runtime with its time driver enabled.

On macOS, certificate verification calls the operating system synchronously
inside the poll that drives the TLS handshake, and that call may fetch over the
network. Neither the connect timeout nor the exchange timer can interrupt it,
so the `RESERVE` bound does not hold while it blocks. This is source-inspected;
the local macOS checks in `batter-8e8r` do not exercise certificate verification.

`MINIMUM_ATTEMPT` exists because a request written just before the deadline
could rarely be answered. A refusal that proves nothing was sent would become
an `Unknown`. Both values are heuristics, not measurements (unverified), and
scheduling delay can exceed `RESERVE`.

### Ready-result precedence

Inside the exchange, a biased select polls the response future first, then
cancellation, then the timer. The body loop does the same for each chunk. When
a head or a chunk is ready at the same time as the timer or cancellation, the
ready result is kept, and a cancellation ready with an expired timer is
reported as the cancellation. This preserves evidence: a head that arrived is
not reported as `Unknown` because the timer fired in the same poll.

Loopback tests leave the exchange unpolled until both results are ready:

- `head_ready_with_the_expired_timer_is_kept` and
  `head_ready_with_cancellation_is_kept` keep a 204 head and its empty body's
  end against an expired timer and against cancellation, so they pin the order
  in both selects;
- `cancellation_ready_with_the_expired_timer_is_reported` and
  `cancellation_ready_with_the_expired_timer_during_the_body_is_reported`
  report the cancellation before a head and during the body.

A ready non-empty body chunk tying with the timer or cancellation is untested
and source-inspected.

It is sound only because of the client profile. Redirects and reqwest's
replays are disabled, so polling a request that was already handed over cannot
make the peer receive a second copy. Further polling can deliver what the peer
already sent. It can also let hyper-util place a request that was never written
on another connection; if the deadline wins then, the outcome is `Unknown`
(source-inspected).

This deliberately differs from `OperationContext::run`. Its
[documented precedence](../guarantees.md) is cancellation, then deadline, then
completion, accepted under `batter-4c4`. `run` wraps arbitrary futures, and
polling an arbitrary future after its deadline can start new effects. The
exchange owns one specific future, and polling it further cannot begin a new
request at the peer.

`run`'s precedence is unchanged, and so is its consequence for composition. An
exchange wrapped in `run`, `retry`, `select!` or `timeout` is dropped when the
outer interruption wins, and a dropped exchange returns no outcome. Because
`send_in` returns `RESERVE` before its context's deadline, a wrapping `run` on
the same context should normally see the exchange complete first (unverified).
Cancellation still wins over a ready exchange there, and `retry`'s attempt
boundary has the same order: it discards a result returned in the same poll as
a cancellation. Because the unsent outcomes cannot be built outside the
package, such a wrapper can report a lost exchange only as `Unknown`.

### Transport evidence, not a provider interface

ADR-014 rejected provider adapters because the roots' failure contracts did not
match: retry owners, key-retention assumptions and ambiguity handling differed.
It rejected a generic provider interface on the same grounds, and that
rejection stands.

This package makes none of those decisions. It has:

- no provider trait and no status-to-meaning helper;
- no retry or replay, and no idempotency key;
- no fence or reconciliation;
- no Retry-After classification.

It returns local evidence about one attempt. The three roots already agree on
that evidence, even though they act on it differently, and those actions stay
in each root:

- A retries in process under deduplicated replay.
- B never replays an unknown outcome and reconciles instead.
- C refunds budgets or restores fences and retries through durable jobs.

The differences that ADR-014 found therefore do not touch what this package
owns.

### Rejected alternatives

- **An adapter that requires `batter-core`.** It would force Batter's runtime
  into provider crates that have none today, such as C's client crates. The
  bridge needs only a deadline and a cancellation signal, so `operation` stays
  opt-in.
- **reqwest's `rustls` feature, with aws-lc-rs.** The first implementation used
  it. It compiles a second rustls provider beside the *ring* provider this
  workspace already uses. With both compiled, every
  `rustls::ClientConfig::builder()` or `ServerConfig::builder()` call made
  without an installed process default panics. One path in this repository is
  bollard's TLS connection to Docker, which testcontainers uses when Docker is
  reached over TLS. The workspace had already chosen *ring* so that no
  aws-lc-rs toolchain enters the graph
  ([references](../references.md#generic-serving-listeners-and-connection-metadata-2026-09-30)).
  - Documenting an `install_default` call for every application root instead
    would add a caller instruction, which ADR-010 counts as design debt.
  - Installing a provider from the library would make a process-wide choice
    that belongs to the application root.
  - The package therefore passes *ring* explicitly in its own configuration.
- **Changing `run`'s precedence, or a completion-first option for arbitrary
  futures.** Polling arbitrary work after its deadline can start new effects.
  It would also reverse the precedence accepted under `batter-4c4`. The
  exchange keeps its own precedence for its own future instead.
- **A permit parameter.** Only C holds an admission permit through the body.
  Because the exchange reads the body itself, a `Bulkhead` permit held across
  the awaited send already covers the whole exchange. A parameter would add a
  pairing without adding an invariant.
- **Retry or replay integration.** [ADR-003](003-replay-and-budgets.md)
  requires the caller to authorize replay and to choose one retry owner. The
  roots choose differently: A retries in process, and B and C retry through
  durable jobs. A second attempt is a new preparation in application code.
  Retry only after a `ConnectFailure`, with a backoff and a fresh budget check.
  A `NotStarted` stands for the deadline or context it was given and returns
  without yielding. A loop that resends after every `proves_unsent()` outcome
  under the same budget therefore spins without yielding.
- **Publicly constructible unsent outcomes.** The first implementation let
  callers build `NotStarted` and `NotDispatched` for test fixtures. A wrapper
  around `run`, `retry`, `timeout` or `select!` could then map an interruption
  to proof that nothing was sent, and an application releasing its fence on
  `proves_unsent()` would trust it. Fixture convenience is not a reason that
  ADR-010 accepts. The payloads are now opaque, and the facade example's tests
  use real outcomes.
- **A response-header allowlist.** Which headers matter is provider policy.
  Every value is marked sensitive instead, so a header map's `Debug` output
  hides it.
- **Zeroizing sinks.** Only B zeroizes response buffers, and B cannot adopt. A
  zeroizing final buffer would not cover the copies held inside hyper and the
  TLS stack (unverified).
- **Per-exchange telemetry.** Only B emits it, and a dropped future cannot
  report an outcome. `label()` gives a value-free label for the caller to
  record under its own boundary, without a second event beside
  `OperationContext::run`'s.
- **HTTP/2.** No root hand-writes HTTP/2 fault handling. Its peer-asserted
  non-processing signals would need another outcome and their own fault tests.
- **Operator-supplied destinations.** They need an address policy applied at
  resolution time, which this package does not have. One validated origin per
  client is the supported shape. Egress admission stays on the watch list.
- **Retry-After parsing.** It stays with the deferred `batter-fq8`, because
  what a 429 or a 503 means is provider policy. The exchange returns the
  headers.

## Consequences and limits

- **Version line.** The fault tests ran against the versions that this
  repository's lockfile resolves: reqwest 0.13.5, hyper 1.11.1, hyper-util
  0.1.20 and rustls 0.23.46. The manifest requires reqwest `^0.13.5` and does
  not constrain hyper or hyper-util. reqwest 0.13.5 itself admits hyper from
  1.1 and hyper-util from 0.1.12 (`Cargo.toml:327-328`, `347-348`).
  - `NotDispatched` rests on implementation behaviour, not on a documented
    upstream contract. An application's lockfile may resolve releases, older
    or newer, that these tests never ran against. Keep the tested versions, or
    re-run the fault tests after upgrading them.
  - hyper-util 0.1.21 is already published. Its `src/client/legacy/client.rs`
    changes imports and capture syntax, moves one call, and adds three HTTP/2
    builder setters. The release also changes `pool.rs`, `connect/http.rs` and
    other files. The `Connect`, `Canceled` and `SendRequest` assignments in
    `client.rs` are unchanged (source-inspected). The fault tests have not run
    against it.
  - `tls_backend_preconfigured` recognises the package's configuration only
    while the package and reqwest use the same rustls release. Both require
    rustls 0.23, which Cargo resolves to one release in a graph. A reqwest
    release on another rustls line would make `Client::new` return
    `ClientError::Build`, and reqwest documents no semver stability for that
    method's internals (source-inspected).
- **Feature set under test.** Every build of the package's own tests compiles
  reqwest's `http2` and `gzip` features through the package's
  dev-dependencies, as an unrelated dependency could through feature
  unification.
  - The ClientHello test shows the `http/1.1`-only ALPN offer, and the body and
    wire tests show undecoded gzip bytes and no `Accept-Encoding`.
  - The shipped feature set, `rustls-no-provider` alone, is compile-checked by
    verification and executed by the facade example's tests in the
    `outbound-shipped-features` matrix batch. The facade feature runner
    requires reqwest's features in that build to equal those of the package's
    normal graph, so a dev-dependency that adds a reqwest feature fails
    verification instead of silently changing what the batch executes.
  - brotli, zstd, deflate and hickory are not compiled here, so those overrides
    are source-inspected.
- **Platform.** Linux test evidence is supplemented by local macOS checks in
  `batter-8e8r`. The CI
  definition's macOS job compiles the package and runs none of its tests.
  macOS certificate verification remains unverified.
- **Crypto providers in the application's graph.** The package neither
  installs nor needs a process-default provider, but two hazards remain in an
  application's own graph:
  - If the graph compiles both *ring* and aws-lc-rs into rustls, for example
    because another crate enables reqwest's `rustls` feature, crates that call
    `rustls::ClientConfig::builder()` without a provider need the application
    root to call `rustls::crypto::CryptoProvider::install_default` first.
  - A plain `reqwest::Client::new()` built elsewhere panics without an
    installed provider when the graph's reqwest 0.13 has only
    `rustls-no-provider`. The proxy test's control installs *ring* in its
    child process for that reason.
- **Trust store.** On Linux, an HTTPS client loads the host certificate store
  when it is built. That store follows `SSL_CERT_FILE` and `SSL_CERT_DIR`, and
  an empty one makes `Client::new` return `ClientError::Build`, while a
  loopback client still builds. The Linux-only
  `an_empty_trust_store_refuses_https_clients_but_not_loopback_ones` test shows
  both. On macOS the verifier loads nothing when it is built and asks the
  operating system during each handshake (source-inspected).
- **Conservative false negatives.** Some requests that the peer never
  processed are reported `Unknown`, which never claims more than the evidence
  supports.
  - A loopback test covers a deadline that expires during the TLS handshake. A
    deadline or connect timeout that expires during DNS or TCP setup is not
    exercised.
  - These cases are source-inspected only: a keep-alive connection that the
    server closes just as the request is written, a handshake or readiness
    failure after the connector, and a request that hyper hands back unwritten
    on a fresh connection.
  - A URL longer than 65,534 bytes no longer belongs here. `prepare()` refuses
    it with `InvalidUri`, which
    `urls_that_are_not_http_uris_are_refused_before_an_exchange_exists` tests.
- **Dropped futures.** Dropping the send future after hand-off yields no
  outcome; treat it as `Unknown`. An outer `select!`, `timeout`, `run`, retry
  attempt cap or shutdown can drop it. That loss is conservative. No test drops
  an exchange.
- **Hidden repeated sends.** hyper-util's second send of a never-written
  request, after a reused connection turns out to be closed, cannot be turned
  off through reqwest. It runs below the replay owner. It is not a replay, but
  it makes wire attempts unobservable. This is source-inspected; no test reuses
  a closed pooled connection.
- **What `Responded` means.** It is a status line from the peer, possibly from
  an intermediary such as a load balancer, a web application firewall or a
  proxy. It is never proof that the provider processed, accepted or refused the
  request.
- **What stays visible.** Query strings still reach the provider and
  intermediaries; the package only keeps them out of its own values.
- **What the package cannot cover.**
  - A hand-built reqwest client can bypass the package, and only a
    consumer-side lint can forbid that.
  - In a multi-request operation, "an earlier request was sent" stays
    application evidence.
- **Adoption.**
  - Consumer B cannot adopt while its provider crates forbid batter-named
    dependencies.
  - The billing library would need reqwest 0.13.
  - batter-otlp and the reference service keep their private copies on
    reqwest 0.12.28 at this decision. The later `batter-7oob` refresh migrates
    those private transports to 0.13.5 with their own bundled-root TLS profile;
    they still do not adopt the outbound exchange API.
- **Promotion.** Stable promotion still needs two adopting application roots
  under the [integration contract](../integrations.md). It also needs exercised
  partial-startup and shutdown failures and a documented external version
  contract. Until then the package is experimental. Its outcome enums stay
  exhaustive, so every new variant is a hard cut with a `Breaking:` entry.
- **Consumer contract.** The facade's runnable `outbound_exchange` example
  shows the fence around one send:
  - it writes its in-flight fence in an explicit `example.fence` work-phase
    step before `send_in`;
  - it settles that application-owned fence from each outcome;
  - it records the result in a reserved finalization phase.

  A cancelled parent cancels finalization too. The example then records
  nothing, and the fence written before the send stays held;
  `parent_cancellation_in_flight_holds_the_fence_and_records_nothing` tests
  that case.
- **Invalid states.** The
  [ADR-010 assessment](010-agent-only-consumption.md#outbound-exchange-assessment-batter-3s1a)
  lists what the canonical path makes unrepresentable and what remains.
  Fresh-agent usability evaluation is proposed and unexecuted.

### Watch list

Each item below lacks the evidence that a Batter-owned version would need. A
later survey that meets an item's trigger files a concrete Bead citing this
record. These items are not backlog.

| Candidate | Current evidence | Trigger |
| --- | --- | --- |
| HTTP/2 exchanges | No root hand-writes HTTP/2 fault handling. One of C's clients negotiates HTTP/2 only because of feature unification. | An adopting root that must use HTTP/2, with fault tests for REFUSED_STREAM and GOAWAY. |
| An explicit proxy, trust roots or crypto provider | B selects its own crypto provider, and B cannot adopt. | An adopting root that needs an explicit proxy, its own trust roots or another crypto provider. |
| Zeroizing response buffers | Only B zeroizes. | Two adopting roots whose data policy requires it. |
| Evidence for a dropped exchange | Only B records an event when a provider future is dropped. | Two adopting roots that need drop evidence through Batter. |
| A shared loopback fault peer | All three roots hand-write HTTP/1.1 fault fixtures, with differing expectations. This package keeps its own peer in its tests. | A second adopting root that needs the same fault modes through Batter. |
| Egress admission for operator-supplied destinations | A and C need it, with different address policies and refusal semantics. | Two in-process roots with the same address policy at resolution time. |
