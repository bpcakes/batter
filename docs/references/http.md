# References: Inbound HTTP, browser policy and serving

[Reference index](../references.md) · [Documentation index](../README.md)

These source checks support current contracts. Each record names the
version and date actually inspected; it is not a new dependency audit.
Re-check the resolved dependency before changing an integration.

## HTTP/1.1 transport ownership, reviewed 2026-09-10

The follow-up harness review checked the [Cargo environment reference](https://doc.rust-lang.org/cargo/reference/environment-variables.html)
and [Rust module reference](https://doc.rust-lang.org/reference/items/modules.html).
`CARGO_MANIFEST_DIR` identifies the consuming package, not the source file's
original package. Source inclusion also compiles nested test modules in that
consumer. Therefore generic process mechanics now live in workspace-private
`test-support/process/`, with fixture assets, scenario policy and self-tests
attached only by their owning foundation suite.

For `batter-u0m`, rechecked Cargo.lock and the downloaded primary Cargo registry
sources: Axum **0.8.9**, Hyper **1.11.1**, hyper-util **0.1.20**, Tokio **1.53.1**,
and http-body **1.1.0**. The test adds a direct development dependency on the
already resolved http-body and enables Tokio io-util; no versions changed.
The browser could not retrieve the pinned docs.rs pages; source inspection used
the exact locally resolved registry packages, not a different online version.

- [Axum serving source](https://docs.rs/axum/0.8.9/src/axum/serve/mod.rs.html):
  `WithGracefulShutdown::run` spawns the graceful-signal future, spawns each
  connection in `handle_connection`, stops accepting on the signal and waits on
  `close_tx.closed()`. Each connection owns a receiver until its future ends.
  The serving wrapper does not retain connection JoinHandles. Aborting it drops
  its wait; it does not join connections. `IntoFuture` returns Ok after run;
  connection errors are observed separately inside the spawned connection task.
- [Hyper HTTP/1 builder](https://docs.rs/hyper/1.11.1/hyper/server/conn/http1/struct.Builder.html):
  the inspected builder defaults keep-alive to true and half-close support to
  false. `half_close(true)` would permit continued response work after a read
  EOF during a request. The fixture uses Axum's default transport and explicitly
  closes both client socket directions; it makes no write-half-only claim.
- [http-body contract](https://docs.rs/http-body/1.1.0/http_body/trait.Body.html):
  body frame polling is separate from constructing a Response. The controlled
  test body acknowledges a pending poll, releases data explicitly, records None
  and records destruction. Native client framing and EOF are checked separately.

These implementation facts motivate the tests; only the executed loopback
evidence establishes the observed behavior in [ADR-008](../adr/008-http-transport-ownership.md).

## HTTP/1.1 lifetimes, reviewed 2026-09-10

For `batter-u0m`, rechecked the actual Cargo.lock and locally cached upstream
registry source: **Axum 0.8.9**, **Hyper 1.11.1**, **hyper-util 0.1.20** and
**Tokio 1.53.1**. Versioned docs.rs retrieval failed in this session; these
semantics were verified directly in the exact resolved upstream source rather
than inferred from another version's documentation.

- [Axum serving source](https://docs.rs/axum/0.8.9/src/axum/serve/mod.rs.html):
  `WithGracefulShutdown::run` spawns the signal task, stops accepting after the
  signal, drops its listener, and awaits `close_tx.closed()`. `handle_connection`
  spawns native connection tasks, passes graceful shutdown to their futures and
  drops a completion receiver after they finish. Connection errors do not become
  the outer serving result. Aborting the wrapper is not a join of these tasks.
- [Hyper HTTP/1 builder](https://docs.rs/hyper/1.11.1/hyper/server/conn/http1/struct.Builder.html#method.half_close)
  and resolved `src/server/conn/http1.rs`: the native default is `half_close = false`.
  Supporting a client that closes its write side while awaiting a response is a
  distinct configuration. `src/proto/h1/dispatch.rs` polls reads/keep-alive while
  managing pending response futures/bodies; actual disconnect timing is tested,
  not inferred as a universal property from the builder option.
- [hyper-util connection shutdown](https://docs.rs/hyper-util/0.1.20/hyper_util/server/conn/auto/struct.UpgradeableConnection.html#method.graceful_shutdown)
  and resolved `src/server/conn/auto/mod.rs`: graceful shutdown delegates to the
  selected native protocol connection. The connection future must continue to
  be polled. Axum's connection task, rather than Batter's request context, owns it.
- [Tokio TCP stream](https://docs.rs/tokio/1.53.1/tokio/net/struct.TcpStream.html)
  and resolved `src/net/tcp/stream.rs`: `AsyncWrite::poll_shutdown` shuts down the
  write direction. The fixture separately uses native `Shutdown::Both` and drops
  its client socket for a full local close, retaining reads for the half-close
  control. Local close is not proof of peer receipt or body-message completion.

The fixture reports terminal chunk/Content-Length framing, body destruction,
socket destruction/EOF and direct wrapper outcome separately. See
[ADR-009](../adr/009-http-lifetime-observations.md) for the measured ownership decisions.
No dependency or Cargo.lock change was needed.

## Axum operational defaults: exact native APIs (2026-09-10)

Before implementation of batter-7r3.4, inspected the downloaded primary crate
sources for **Axum 0.8.9** (`src/serve/mod.rs`, `src/routing/mod.rs`) and
**tower-http 0.6.11** (`src/request_id.rs`, `Cargo.toml`). Cargo subsequently
resolved tower-http 0.6.11 with only its request-id feature, UUID 1.26.0 and Axum
0.8.9 in the generated lockfile; it reported tower-http 0.7.1 as available.
This task selects the audited 0.6 API, not a general dependency refresh. No
foundation dependency on HTTP libraries was introduced. `http-body` 1.0.1 is an
adapter test dependency for a controlled native streaming body.

The [Tower HTTP request-ID source](https://docs.rs/crate/tower-http/0.6.11/source/src/request_id.rs)
shows SetRequestId retaining inbound headers and potentially an existing RequestId
extension. MakeRequestUuid calls native UUID v4 generation. The opt-in boundary
therefore uses that generator directly and replaces header/Tower/adapter values;
it does not deploy the preserving setter at an untrusted boundary.

The [Axum serve source](https://docs.rs/crate/axum/0.8.9/source/src/serve/mod.rs)
spawns native connection tasks and the graceful-signal listener. Graceful return
waits on native connection completion; aborting the direct serve future does not
join those tasks. Native accept errors are retried. This grounds the deliberately
limited register_http helper and the executed streaming-abort regression.

The [Axum routing source](https://docs.rs/crate/axum/0.8.9/source/src/routing/mod.rs)
applies layers to already assembled route/fallback services. That determines
`low_level::operational_http` placement, retained MatchedPath templates and 405/fallback
coverage. Versioned docs.rs web requests failed in this environment; exact local
Cargo source inspection, compilation and runtime tests supplied API evidence.

## Browser credential transport, 2026-09-15

- [Cookies: HTTP State Management Mechanism,
  draft-ietf-httpbis-rfc6265bis-22](https://datatracker.ietf.org/doc/draft-ietf-httpbis-rfc6265bis/)
  remains an active Internet-Draft in the RFC Editor queue. Its server syntax,
  cookie-name prefix, Secure, HttpOnly, SameSite, Domain, Path, Max-Age, and
  Expires sections inform the fixed host-only response contract. Its server
  profile prohibits more than one Set-Cookie field with the same cookie-name in
  one response; Batter checks that invariant before appending. The draft is
  cited as work in progress, not a final RFC.
- The living [WHATWG URL Standard](https://url.spec.whatwg.org/) defines URL
  origins and tuple-origin serialization. Batter delegates parsing and canonical
  ASCII origin serialization to `url` 2.5.8, then applies its narrower HTTPS or
  loopback-HTTP configuration policy.
- [Fetch Metadata Request Headers, W3C Working Draft 1 April
  2025](https://www.w3.org/TR/fetch-metadata/) defines `Sec-Fetch-Site` as a
  Structured Field token, names `cross-site`, `same-origin`, `same-site`, and
  `none`, and advises forward-compatible handling of unknown values. Batter's
  compatible mode ignores syntactically valid unknown tokens but rejects an
  explicit `cross-site`; its strict mode requires exact `same-origin`, and that
  mode is automatic for every custom-marker policy.
- [Upgrade Insecure Requests, W3C Candidate Recommendation 8 October
  2015](https://www.w3.org/TR/upgrade-insecure-requests/#preference) permits user
  agents to append `Upgrade-Insecure-Requests: 1` to navigation requests. This
  is the concrete counterexample to inferring script provenance or a completed
  CORS preflight from an otherwise non-safelisted, non-forbidden field name.
- [RFC 9651](https://www.rfc-editor.org/rfc/rfc9651.html) supplies the Structured
  Field token grammar used to distinguish a future Fetch Metadata token from a
  malformed, comma-list, or whitespace-containing field.
- [RFC 9111 section 5.2.2.5](https://datatracker.ietf.org/doc/html/rfc9111#section-5.2.2.5)
  defines the `no-store` response directive for private and shared caches and
  explicitly says it is not a sufficient privacy mechanism against malicious or
  compromised caches.
- The living [Referrer Policy specification](https://w3c.github.io/webappsec-referrer-policy/#referrer-policy-same-origin)
  defines `same-origin` as sending referrer information only for same-origin
  requests. The living [Fetch Standard Origin-header
  algorithm](https://fetch.spec.whatwg.org/#origin-header) retains the serialized
  origin for a same-origin non-CORS mutation under that policy and uses `null`
  when the current URL is cross-origin. Batter selects this policy so private
  pages can submit same-origin HTML forms to an exact-origin mutation surface
  without leaking referrers cross-origin.
- The living [Fetch Standard](https://fetch.spec.whatwg.org/#x-content-type-options-header)
  defines the `X-Content-Type-Options: nosniff` response behavior. Batter emits
  that fixed field but makes no broader CSP or content-validation claim.
- [`cookie` 0.18.2](https://docs.rs/cookie/0.18.2/cookie/) reports Rust 1.56 and
  MIT OR Apache-2.0. Batter uses it with default features disabled for response
  serialization; signing, private jars, key expansion, and percent encoding are
  not enabled or exposed.
- [`url` 2.5.8](https://docs.rs/url/2.5.8/url/) reports Rust 1.63 and MIT OR
  Apache-2.0. Batter enables only its `std` feature and does not expose `Url` in
  public signatures.
- [`axum` 0.8.9](https://docs.rs/axum/0.8.9/axum/) reports Rust 1.80 and MIT.
  Public Router integration tests pin the actual middleware/layer behavior used
  by private responses and observation. The workspace minimum is Rust 1.98.1.

## Private-response referrer choice, 2026-09-27

Rechecked for `batter-lto` against the living specifications.

- The [Referrer Policy editor's draft](https://w3c.github.io/webappsec-referrer-policy/#referrer-policy-no-referrer)
  defines `no-referrer` as sending no referrer information to any origin,
  including same-origin navigations, and
  [`same-origin`](https://w3c.github.io/webappsec-referrer-policy/#referrer-policy-same-origin)
  as sending the full referrer URL only on same-origin requests. Its
  [delivery section](https://w3c.github.io/webappsec-referrer-policy/#referrer-policy-delivery)
  also lists the `meta` element, `referrerpolicy` attributes, the `noreferrer`
  link relation and inheritance, so a response field cannot constrain page
  content. [Header parsing](https://w3c.github.io/webappsec-referrer-policy/#parse-referrer-policy-from-header)
  keeps the last recognized token across fields, and a redirect response
  [replaces](https://w3c.github.io/webappsec-referrer-policy/#set-requests-referrer-policy-on-redirect)
  the next request's policy. Batter emits one field with one recognized token.
- The HTML Standard [creates a document or worker policy
  container](https://html.spec.whatwg.org/multipage/browsers.html#creating-a-policy-container-from-a-fetch-response)
  whose referrer policy comes from the response's `Referrer-Policy` field.
- The Fetch Standard [Origin-header algorithm](https://fetch.spec.whatwg.org/#append-a-request-origin-header)
  consults the referrer policy only when response tainting is not `cors`, the
  method is neither `GET` nor `HEAD`, and the
  [request mode](https://fetch.spec.whatwg.org/#concept-request-mode) is not
  `cors`. There `no-referrer` always serializes `null`, and `same-origin`
  serializes `null` only for a cross-origin URL. Navigations, including HTML
  form submissions, use `navigate` mode, while a
  [`Request`](https://fetch.spec.whatwg.org/#dom-request) built from a URL
  string falls back to `cors` mode. `NoReferrer` therefore breaks exact-origin
  HTML form mutations but not CORS-mode script mutations.
- The [Fetch Metadata editor's draft](https://w3c.github.io/webappsec-fetch-metadata/#abstract-opdef-set-site)
  derives `Sec-Fetch-Site` from the request origin, its URL list and direct user
  navigation, not from the referrer policy, so strict custom-marker policies are
  unaffected by `NoReferrer`.

## Generic serving listeners and connection metadata, 2026-09-30

Rechecked for `batter-tc9w.7` against the resolved Axum 0.8.9 sources in the
locked registry, not the `latest` documentation.

- [`axum::serve::Listener`](https://docs.rs/axum/0.8.9/axum/serve/trait.Listener.html)
  (`src/serve/listener.rs`) requires `Send + 'static`, an
  `Io: AsyncRead + AsyncWrite + Unpin + Send + 'static`, an `Addr: Send`, and
  `accept(&mut self) -> impl Future<Output = (Io, Addr)> + Send`. Accept returns
  no error, so the trait's own text requires an implementation to log and retry
  instead; the pinned `TcpListener` and `UnixListener` implementations loop over
  `handle_accept_error`, which returns immediately for connection errors and
  otherwise logs and sleeps one second. A listener therefore cannot report an
  accept failure to Batter's component exit.
- `serve(listener, make_service)` (`src/serve/mod.rs`) accepts any such listener,
  and `Router<()>` implements `Service<IncomingStream<'_, L>>` for every
  `L: Listener` (`src/routing/mod.rs`). `WithGracefulShutdown` additionally
  requires `L::Addr: Debug`, which the adapter's public bounds pass through.
- `WithGracefulShutdown::run` selects between `listener.accept()` and the signal.
  When the signal wins it breaks the loop, so the accept future in progress —
  including a handshake awaited inside it — is dropped without being awaited,
  then the listener itself is dropped, and only afterwards does it wait for the
  spawned connection tasks to finish. Batter's registered task therefore releases
  a pending accept and the listener before it awaits connection completion, and
  still cannot claim that connection tasks or a listener's own spawned work
  terminated.
- [`Connected`](https://docs.rs/axum/0.8.9/axum/extract/connect_info/trait.Connected.html)
  (`src/extract/connect_info.rs`) is implemented for `IncomingStream<'_, L>` in
  exactly two shapes: `SocketAddr` for the concrete `TcpListener`, and `L::Addr`
  for `TapIo<L, F>` where `L: Listener`, `L::Addr: Clone + Sync + 'static` and
  `F: FnMut(&mut L::Io) + Send + 'static`. There is no blanket implementation for
  a bare custom listener, so `ListenerExt::tap_io` is the pinned route to connect
  info for one. `TapIo` keeps `Io` and `Addr` and runs its closure on each
  accepted `Io` after `accept` returns; an empty closure changes nothing
  observable, and the two `Connected` implementations produce the same
  `SocketAddr` for a `TcpListener`.
- Test-only transport material resolved with Cargo at the same date:
  `rcgen` 0.14.10, whose `generate_simple_self_signed` returns a
  `CertifiedKey { cert, signing_key }` and whose default features select `ring`,
  `rustls` 0.23.44 and `tokio-rustls` 0.26.5, all taken with `default-features =
  false` and the `ring` provider so no `aws-lc-rs` toolchain enters the graph.
  `rustls` 0.23.44 was already resolved for the workspace through SQLx's
  `tls-rustls-ring`. The suite selects the provider explicitly with
  `builder_with_provider`, so feature unification cannot make provider selection
  ambiguous. These crates are dev-dependencies of `batter-axum` only; no
  published package gains a TLS dependency.


### Optional rendered fallback (2026-09-30)

The lockfile still resolves Axum 0.8.9 and matchit 0.8.4. Rechecked the pinned
[Router source](https://docs.rs/axum/0.8.9/src/axum/routing/mod.rs.html) and
[path router](https://docs.rs/axum/0.8.9/src/axum/routing/path_router.rs.html):
`reset_fallback` resets root, nested and catch-all fallback routing while keeping
explicit routes, including their method routers. `fallback_service` installs a
new root and catch-all fallback. Layers affect routes present when added.
The optional boundary renderer therefore uses the existing root dispatch after
native routing; admitted inventories still select declared routes before the
renderer. The renderer's own router carries correlation/observation because the
dispatch service is installed after the native route layer. Root/nested guarded
fallback declarations are rejected with this mode. Local tests cover native and
declared matched-method dispatch, unmatched paths and probe/group validation.


### Sealed assembly and one-time client preparation (2026-10-05)

The lockfile still resolves Axum 0.8.9. Rechecked the pinned
[Router source](https://docs.rs/axum/0.8.9/src/axum/routing/mod.rs.html) for the
preparation an `AssembledHttp` consumed into `InProcessClient` must reproduce.
`Router::into_make_service` and `Router::into_make_service_with_connect_info`
both call `Router::with_state(())` before wrapping, with the comment that this
turns everything into a `Route` eagerly "rather than doing that per request";
the `Service<IncomingStream<'_, L>>` implementation that `axum::serve` uses
clones the router and calls `with_state(())` per accepted connection for the
same reason. `PathRouter::with_state` converts each `Endpoint::MethodRouter`
into a state-applied method router and leaves an `Endpoint::Route` alone, so a
layer that Axum applies lazily to a handler is constructed during preparation
rather than on every request. `Router<()>`'s `Service<Request<B>>`
implementation is always ready and dispatches through `call_with_state(req, ())`.

Rechecked at the same date, for the probe response policy, the pinned
[method routing source](https://docs.rs/axum/0.8.9/src/axum/routing/method_routing.rs.html):
`MethodRouter::layer` maps the layer over every method handler *and* over
`fallback`, and carries `allow_header` through unchanged, while
`MethodRouter::route_layer` deliberately leaves the fallback alone. A probe is
registered as `get(...)`, so the implicit `405` for an unsupported method comes
from that fallback. `HttpBoundary::with_probe_response_policy` therefore layers
the probes with `layer`, not `route_layer`, which is what lets the rejection
gain the policy headers while keeping its status and `Allow` header. Recheck
this when the Axum pin changes.

`InProcessClient::new` therefore calls `with_state(())` once and clones that
prepared router per request, which reproduces the served router's layer
construction count exactly; the layer-count regression in
[route group tests](../../crates/batter-axum/tests/route_groups/admitted.rs)
asserts it through the client instead of the previously exposed make-service.
Make-service stays unexposed, so no caller can serve the client. Recheck these
semantics whenever the Axum pin changes. No dependency or `Cargo.lock` change
was required.

### PostgreSQL session-profile batching and idle authority (2026-10-06)

PostgreSQL 18 distinguishes the authenticated session identity from the active
execution identity: [SET SESSION AUTHORIZATION](https://www.postgresql.org/docs/18/sql-set-session-authorization.html)
sets both, whereas `SET ROLE` changes the current user. Changing the session
identity to another user requires an initially authenticated superuser. A direct
restricted-login regression therefore declares the effective role as the wrong
login, rather than relying on a session-authorization statement to succeed. Both
profile constructors must reject that declaration before protected work even
when role, schema and setting permissions are valid.

The lockfile still resolves SQLx 0.9.0; live regression execution uses PostgreSQL
18.6. PostgreSQL 18's [role inquiry](https://www.postgresql.org/docs/18/functions-info.html#FUNCTIONS-INFO-ACCESS-TABLE)
defines `pg_has_role(user, role, 'SET')` as permission to assume the role, unlike
plain membership or immediately inherited privileges. [REVOKE](https://www.postgresql.org/docs/18/sql-revoke.html)
can clear only SET OPTION while retaining membership. The
[role GUC check](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/commands/variable.c)
uses `member_can_set_role`; reading the current role name does not repeat it.
Profile validation therefore checks SET permission in its existing statement,
including ordinary idle acquisition. This is a point-in-time check, not fencing
against subsequent changes or a new check on native fast acquisition.

PostgreSQL 18's [function expression initialization](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/executor/execExpr.c)
checks function EXECUTE using the current user before evaluating any rows.
Setup therefore uses native `SET ROLE` before the batched `set_config` query,
preserving profiles whose login can assume the effective role without inheriting
its function grants. A separate live target revokes PUBLIC execution of
`set_config`, grants only the effective role, and covers both profile constructors
through direct setup and ordinary pool reuse.

PostgreSQL 18's [multi-statement simple-query contract](https://www.postgresql.org/docs/18/protocol-flow.html#PROTOCOL-FLOW-MULTI-STATEMENT)
and SQLx 0.9.0's local `sqlx-core/src/raw_sql.rs` agree that one raw SQL batch
executes in an implicit transaction. A later fixture-drop error therefore rolls
back an earlier function-grant restoration in the same batch. The live fixture
acknowledges restoration separately before its drop batch and retains both
errors. A test-owned function and an intentionally failing nonempty-schema drop
exercise this boundary without injecting failure into the built-in function ACL.

PostgreSQL's [SELECT evaluation contract](https://www.postgresql.org/docs/18/sql-select.html#SQL-SELECT-LIST)
places output-expression evaluation after sorting when those expressions are not
sort/group/distinct keys. Profile setup orders array rows by ordinality before
its volatile `set_config` output, preserving declaration order after the native
role switch. The name/value arrays are
built from one assignment sequence; fixed query width and two binds avoid the
former target-list ceiling without imposing a new profile-count limit. Settings
remain subject to native resource and value limits.

The [PostgreSQL 18 GUC implementation](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/misc/guc.c)
uses `ShowGUCOption` and `convert_int_from_base_unit` to display positive integer
timeouts in a divisible unit (`d`, `h`, `min`, `s`, `ms`), while zero has no suffix.
[Interval EPOCH extraction](https://www.postgresql.org/docs/18/functions-datetime.html#FUNCTIONS-DATETIME-EXTRACT)
returns total seconds as numeric; multiplying by 1,000 recovers milliseconds.
Native tests cover these forms, the accepted maximum and all four interval styles.

SQLx 0.9.0's pinned `sqlx-postgres/src/connection/{executor,mod}.rs` separately
awaits uncached statement preparation and nonempty cache-close synchronization.
The five reset policy statements and one ordinary-acquisition verification (plus
one native assignment batch for profiles with custom settings) are
statement accounting, not measured network-round-trip or latency
claims. Existing retirement, redacted setup failures and independent atomic/
snapshot reset ownership remain unchanged; see `batter-md9y` for executed evidence.

PostgreSQL 18's [GUC assignment implementation](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/misc/guc.c)
checks parameter SET ACLs for `PGC_SUSET` assignments; `PGC_USERSET` assignments
need no explicit grant. Reading `current_setting` does not repeat assignment
authority checks. The [auto_explain module](https://github.com/postgres/postgres/blob/REL_18_STABLE/contrib/auto_explain/auto_explain.c)
registers `auto_explain.log_analyze` as `PGC_SUSET`; the native regression preloads
it before checking revocation, distinguishing it from an unrestricted placeholder.
Idle acquisition reassigns current custom values through native `set_config` in
one ordered array batch, then verifies the whole policy in a separate command.
This preserves permission/check-hook semantics and detects value drift without
repeating reset or clearing the statement cache. Native fast paths and already
held leases retain their documented limits.

## Forced connection close at shutdown (2026-11-11)

Rechecked for `batter-9wig.1` against the resolved Axum 0.8.9 and Hyper 1.11.1
sources in the locked registry.

- Axum `src/serve/mod.rs`: `WithGracefulShutdown` stops accepting when the
  signal fires, drops the listener, and returns only after
  `close_tx.closed()`, that is after every spawned connection task has dropped
  its receiver. Each connection task selects the Hyper connection future
  against the signal and calls `graceful_shutdown` once. The builder is
  `hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())` with no
  timer, so Hyper's 30 second `header_read_timeout` default, which
  [requires a timer](https://docs.rs/hyper/1.11.1/hyper/server/conn/http1/struct.Builder.html#method.header_read_timeout),
  never applies. Axum offers no handle that ends an accepted connection.
- Hyper `src/proto/h1/conn.rs` and `dispatch.rs`: `disable_keep_alive` closes a
  connection only when it is idle or `has_initial_read_write_state`, which
  requires an empty read buffer. A partly received request head therefore keeps
  the connection open after graceful shutdown. While a response is being
  written and the request was read, `poll_read_keep_alive` calls
  `mid_message_detect_eof`, which reads from the transport (`force_io_read`)
  only while half-close is not allowed and its read buffer is empty
  (`conn.rs:487-493`). Bytes of a following request already buffered, or a read
  side Hyper has closed (`conn.rs:434-435`), leave it waiting with no transport
  interest, so a connection whose reads and writes were all ready can then wait
  on its response body alone. Every dispatcher pass still flushes
  (`dispatch.rs:172-175`), and with an empty write buffer `Buffered::poll_flush`
  calls the transport's `poll_flush` (`io.rs:271-275`) unless `pipeline_flush`
  is enabled, which Axum's builder leaves off. An I/O error from a read, write
  or flush ends the connection future with an error; Axum's task then logs at
  trace level and drops its receiver. Checked against Hyper 1.11.1 for the
  `batter-9wig` follow-up review.
- Hyper has no write timeout. A peer that stops reading leaves the connection
  future pending in `poll_flush` for as long as TCP keeps the connection.
- `serve_connection_with_upgrades` resolves its connection future when the
  upgrade is handed over, so an upgraded connection no longer holds Axum's
  graceful wait.
- Hyper `src/proto/h1/dispatch.rs` `poll_write`: a response body frame error
  becomes `Error::new_user_body` and ends the connection future, so a chunked
  message whose body errors receives no final chunk. It polls the body only
  while `can_write_body` holds and the write buffer accepts more, so a body is
  not polled while its peer is not reading. Checked for `batter-9wig.6`; a
  real-socket test asserts the missing final chunk.
- Hyper `poll_write` stops polling a body without asking for its end in three
  cases: once `is_end_stream` reports the end after a data frame
  (`dispatch.rs:408-417`), once `can_write_body` is false because the
  advertised length has been written (`dispatch.rs:391-397`), and after a
  trailers frame (`dispatch.rs:426-430`). A body that yields its end with a
  `Content-Length` still outstanding makes `end_body` fail:
  `Encoder::end` returns `NotEof` for `Kind::Length(n)` (`encode.rs:124`) and
  `end_body` maps it to a body-write-aborted error and closes writing
  (`conn.rs:807-809`). A short final chunk after `is_end_stream` is written by
  `write_body_and_end`, which closes writing because `encode_and_end` returns
  false for a chunk shorter than the remaining length (`conn.rs:764-781`,
  `encode.rs:248-251`). Either way the peer receives a truncated
  message, so the owned body reports such an end as `failed`. Checked against
  Hyper 1.11.1 on 2026-11-12; real-socket tests assert both short ends.
- Hyper writes no response body, whatever length is declared, to a HEAD
  request, for a 2xx answer to CONNECT and for 1xx, 204 and 304 statuses
  (`role.rs:515-527`, `can_have_body`, forcing the body to zero at
  `role.rs:965`). Axum 0.8.9 also replaces a HEAD response's body with an empty
  one after setting its length (`routing/route.rs:166-171`). The owned body
  therefore takes no advertised length from such a response. Checked on
  2026-11-12; in-process tests cover a 304 and a HEAD response.
- Hyper frames a body whose size it does not know by the `Content-Length`
  header present when it encodes the head (`role.rs:746-766`), and Axum 0.8.9
  applies a tuple's headers after the inner response is built
  (`axum-core` `response/into_response.rs:396-416`,
  `response/into_response_parts.rs:149-157`). A length declared after
  `into_streaming_response` returns therefore frames the message, so the owned
  body takes its advertised length from the response request admission
  returns. `http` 1.x `Response::into_body` discards the response's parts,
  extensions included, so the started body hands its framing to admission
  through the admission record rather than the response. Checked on
  2026-11-12 and 2026-11-13; real-socket tests cover a length declared through
  a tuple and on a response rebuilt around the body, both short and met.
- Hyper answers an HTTP/1.0 request in HTTP/1.0 (`conn.rs:692-699`,
  `enforce_version`) and, with no declared length, delimits the body by
  closing the connection (`role.rs:921-925`, `Encoder::close_delimited`). A
  body error still closes the connection (`dispatch.rs:401-405`), so such a
  peer receives an orderly end of stream after the data already written and
  cannot tell an interrupted body from a complete one; a declared length keeps
  the shortfall visible. Checked on 2026-11-12; a real-socket HTTP/1.0 test
  observes the clean close.

Batter therefore fails the accepted connection's own `AsyncRead`/`AsyncWrite`
operations instead of owning the accept loop. A connection need not be waiting
in the transport when the gate closes, so the gate records the task of every
I/O poll, not only a pending one, and wakes it; that task's next pass flushes
and fails. Local real-socket tests observed the pending body, the stalled
reader and the half-sent head each holding the wrapper until abort without the
close and ending inside the cancel allowance with it; a pending body after
pipelined request bytes did the same when only pending polls were recorded.
HTTP/2 and an upgraded connection were not exercised.

