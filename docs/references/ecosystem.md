# References: Dependency selection and Effect v4 rationale

[Reference index](../references.md) · [Documentation index](../README.md)

These source checks support current contracts. Each record names the
version and date actually inspected; it is not a new dependency audit.
Re-check the resolved dependency before changing an integration.

## SQLx 0.9.1 upgrade, 2026-11-18

For `batter-c5ci`, every SQLx requirement, the consumer manifests and the
SQLx CLI refresh check moved from 0.9.0 to 0.9.1; Cargo resolves one SQLx
0.9.1 graph and drops the SHA-2 0.10 duplicate
([#4301](https://github.com/transact-rs/sqlx/pull/4301)). Rust remains 1.100.0
(SQLx declares 1.94.0). The
[0.9.1 changelog](https://docs.rs/crate/sqlx/0.9.1/source/CHANGELOG.md) and a
diff of the downloaded 0.9.0 and 0.9.1 `sqlx-core`, `sqlx-postgres` and
`sqlx-macros-core` registry sources were inspected for behavior that current
contracts cite:

- BEGIN depth is counted before its round trip
  ([#4394](https://github.com/transact-rs/sqlx/pull/4394)); see
  [failed bounded BEGIN](postgresql.md#postgresql-18-guarded-index-removal-2026-10-27).
  Batter's owned atomic and snapshot transactions send `BEGIN` as raw
  commands and do not use that manager.
- Outside a transaction, the PostgreSQL executor now invalidates a cached
  statement and sends it again when the first response to the statement is
  `0A000` from `RevalidateCachedQuery` or `26000`
  ([#4375](https://github.com/transact-rs/sqlx/pull/4375),
  [`executor.rs`](https://docs.rs/crate/sqlx-postgres/0.9.1/source/src/connection/executor.rs)).
  This is an automatic upstream replay, not only a re-send of a rejected
  bind. It also applies to the simple-query path of `sqlx::raw_sql` and other
  unprepared execution. It matches `26000` by code alone and `0A000` only
  when the server reports the routine `RevalidateCachedQuery`. A statement
  that has already caused a non-transactional effect, such as `nextval` or a
  session advisory lock, before one of those errors repeats that effect:
  an external review's PostgreSQL 18 comparison (not a checked-in test)
  advanced a sequence twice with 0.9.1 and once with 0.9.0 for one autocommit
  `DO` that raised `26000`. Owned
  `run_atomic`, `low_level::PgAtomicTransaction` and snapshot work run inside
  a server transaction, so SQLx returns the error there; low-level
  `PgSession` and pooled execution outside a transaction can replay. Batter
  cannot disable this. Statement counts in the reset-accounting references
  assume no such error.
- With the statement cache disabled (`statement_cache_capacity(0)` or
  `statement-cache-capacity=0`), a persistent query now uses the unnamed
  statement ([#4337](https://github.com/transact-rs/sqlx/pull/4337)), but the
  re-parse after a custom-type catalog lookup applies only to nonpersistent
  queries ([#4361](https://github.com/transact-rs/sqlx/pull/4361)). The first
  query returning an uncached custom type, such as an enum, therefore fails
  with `26000` inside a transaction, including `run_atomic`; outside one the
  replay above hides it. The same external review reproduced this on PostgreSQL
  18 with 0.9.1, not 0.9.0. Unverified workarounds: keep the cache enabled or
  mark such queries
  `.persistent(false)`. No checked-in package disables the cache.
- The TCP connector trims leading and trailing brackets from host strings.
  The reference service checks decoded domain names before using the native
  setter, which preserves genuine IPv6 authority syntax and rejects encoded
  bracket delimiters. See the pinned
  [SQLx host implementation](https://docs.rs/crate/sqlx-core/0.9.1/source/src/net/mod.rs).
  IPv6 URL hosts are bracketed and unbracketed consistently
  ([#4529](https://github.com/transact-rs/sqlx/pull/4529)); see
  [native live URL compatibility](fixtures.md#native-live-url-compatibility-2026-09-10).
- `hostaddr` and `PGHOSTADDR` no longer replace `host`
  ([#4514](https://github.com/transact-rs/sqlx/pull/4514)). URL parsing starts
  from `new_without_pgpass`, which now reads an ambient `PGHOSTADDR` into a
  separate address that the connection prefers over the URL host; SQLx
  exposes no getter or reset for it. In 0.9.0 the URL host overrode the
  variable. This matches libpq: the PostgreSQL 18
  [environment variables](https://www.postgresql.org/docs/18/libpq-envars.html)
  page says `PGHOSTADDR` "behaves the same as the `hostaddr` connection
  parameter", and [connection setup](https://www.postgresql.org/docs/18/libpq-connect.html)
  fills unset parameters from their environment variables, so `psql` also
  connects to `PGHOSTADDR` beside a URL host. Where SQLx is an application's
  only PostgreSQL client, Batter therefore keeps libpq's semantics. SQLx still
  differs from libpq in one case: `make_socket_path` in
  [`options/mod.rs`](https://docs.rs/crate/sqlx-postgres/0.9.1/source/src/options/mod.rs)
  checks a Unix socket before `hostaddr`, while libpq says TCP "is always used
  when a nonempty string is specified" for `hostaddr`; that divergence is
  tracked for an upstream report.

  The invariant Batter does keep is narrower: where two parsers act on one
  endpoint, both reach the same server. Fixtures provision through the
  `postgres-test-harness` client (tokio-postgres 0.7.18, which reads no
  environment) and connect pools with SQLx. Fixture resolution, template
  preparation and the public `harness_connect_options` and
  `harness_admin_connect_options` therefore read the harness admin URL once
  with tokio-postgres, map its typed `Config` field by field into SQLx options,
  and take the database from the name the harness recorded, refusing any
  ambient client `PG*` variable (every `PG` name except `PGDATA` and
  `PG_`-prefixed ones), `hostaddr`, no host, several hosts or ports, a
  `dbname` parameter (the harness keeps the admin query in every lease URL)
  and settings SQLx cannot carry. Suites parse once and refuse before provisioning or template lookup. Template initializers receive a typed
  target whose SQLx options come from that parse, not a URL.
  The pinned [harness connector](https://github.com/bpcakes/postgres-test-harness/blob/3d525e6fc5745ce2e2437c7997de5cccdecff4ac/src/admin.rs)
  uses `NoTls`, so fixture clients use plaintext. A passwordless admin URL
  (trust or peer) is the harness login with an empty password.
  [tokio-postgres authentication](https://docs.rs/crate/tokio-postgres/0.7.18/source/src/connect_raw.rs)
  refuses a password challenge when the password is absent.
  [SQLx authentication](https://docs.rs/crate/sqlx-postgres/0.9.1/source/src/connection/establish.rs)
  uses an empty response credential for that case, so both fail it:
  PostgreSQL 18's [`CREATE ROLE`](https://www.postgresql.org/docs/18/sql-createrole.html)
  and [`ALTER ROLE`](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/commands/user.c)
  clear any password that verifies the empty string, pre-encrypted secrets
  included, and [`recv_password_packet`](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/libpq/auth.c)
  refuses an empty plaintext or MD5 response. Its
  [URL parser](https://docs.rs/crate/sqlx-postgres/0.9.1/source/src/options/parse.rs)
  first tries a passfile, so fixture code never hands SQLx a URL. The
  metadata refresh script refuses any ambient client `PG*` variable, parses
  the Unix bytes of `DATABASE_URL` as UTF-8 and treats an empty userinfo password
  as omitted. The [libpq 18 passfile](https://www.postgresql.org/docs/18/libpq-pgpass.html)
  matches the `host` parameter, but searches `localhost` for libpq's default
  socket directory and ignores a group- or world-accessible file on Unix;
  [SQLx 0.9.1's lookup](https://docs.rs/crate/sqlx-postgres/0.9.1/source/src/options/pgpass.rs)
  keys a socket URL by its own probed default host, keeps IPv6 brackets,
  checks permissions only on Linux and keeps a password's unescaped `:`, which
  libpq's [`passwordFromFile`](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/interfaces/libpq/fe-connect.c)
  treats as the end of the password. Passwordless refreshes therefore use the
  passfile only for a DNS or IPv4 host; for a socket or IPv6 host psql gets
  `PGPASSFILE=/dev/null` and the SQLx URL `password=`. The refresh gives `psql` the parts as its own PG* variables, gives
  SQLx CLI a URL rebuilt from the same parts, and asks a Cargo build script for
  the merged environment and selected compiler. Live preparation requires native
  rustc without compiler wrappers. Each actual compiler invocation checks its
  inherited settings immediately before executing rustc, including any
  package-specific `cargo::rustc-env` values. Runledger's fixtures refuse any client `PG*`
  variable in every mode, since their native test proxies dial the URL host
  themselves, and append `sslmode=disable` to every admin and derived
  database URL so every connection uses the native proxies' plaintext. An
  earlier `hostaddr=127.0.0.1` pin on owned-container URLs (a URL `hostaddr`
  overrides the environment for a TCP host in both SQLx and libpq, checked
  against SQLx 0.9.1 and psql 18 with an ambient `PGHOSTADDR`) was dropped
  once every mode refused client `PG*` variables before provisioning. Derivation
  re-appends only that query; an external admin URL's query is not carried, so an external URL
  with any other query is refused before provisioning (`sslmode=disable` and a
  `hostaddr` repeating its IP literal host are accepted), since leases would lose its endpoint or TLS
  settings. SQLx 0.9.0 honoured these parameters too, so that split
  predates this upgrade. Fixture endpoints accept `connect_timeout` and
  `channel_binding=disable` without carrying them into SQLx options, and refuse
  other connection tuning. Refusing every client `PG*` variable replaced earlier attempts to carry or
  translate inherited settings, each of which missed a variable that changed
  one client's connection. The metadata refresh
  drops a `hostaddr` equal to its IP literal host and refuses any other. The reference root already rejects every PG* variable.

  The invariant holds under three stated preconditions, which are not
  enforced: a host name resolves to the same server for the duration of a run
  (two lookups of one name can differ for any pair of clients, libpq
  included); a URL handed to another process, such as a template initializer's
  child or a consumer of `EphemeralDatabase::url()`, is read under that
  process's own environment; and configuration files other than Cargo's, which
  the refresh probe covers, and `sqlx.toml`, which the refresh refuses, are out
  of scope.

  An earlier revision of this unreleased branch instead added an app-wide URL
  boundary that tried to make SQLx, tokio-postgres and libpq read arbitrary URL
  text identically; ten external review rounds kept finding further
  divergences in that open-ended invariant, so it was replaced by the
  parse-once design above.
- A `.sql` file in a migration directory that does not match
  `<VERSION>_<DESCRIPTION>.sql` is now a resolve error
  ([#4367](https://github.com/transact-rs/sqlx/pull/4367)). Every checked-in
  migration directory conforms.
- Pool `release` now publishes the idle count before the connection becomes
  acquirable, and maintenance caps its idle sweep at `max_connections`
  ([#4289](https://github.com/transact-rs/sqlx/pull/4289)). TCP sockets set
  `TCP_NODELAY` ([#4336](https://github.com/transact-rs/sqlx/pull/4336)); the
  rustls connector is cached per options value
  ([#4242](https://github.com/transact-rs/sqlx/pull/4242)). Non-listener
  connections discard `NotificationResponse`
  ([#4485](https://github.com/transact-rs/sqlx/pull/4485)).
- The offline query-metadata format is unchanged; 0.9.1 only scopes `.env`
  lookup to the workspace and tracks fewer paths. The checked-in `.sqlx`
  caches compile unchanged.

Dated records below that cite 0.9.0 sources describe what was inspected then.

## Dependency refresh, 2026-11-04

For `batter-8e8r`, Cargo updated Tokio 1.54.1, thiserror 2.0.21,
serde_json 1.0.154, UUID 1.29.0, SHA-2 0.11.1, zeroize 1.9.1, h2 0.4.20,
tokio-rustls 0.26.6, proptest 1.12.0 and TOML 1.2.0. Rust remains 1.98.1.
The following primary sources were inspected in the downloaded registry source
for the exact versions linked here; the web renderer was unavailable.

- [AES-GCM 0.11.1 changelog](https://docs.rs/crate/aes-gcm/0.11.1/source/CHANGELOG.md)
  and [AEAD 0.6.1 API](https://docs.rs/aead/0.6.1/aead/trait.AeadInOut.html):
  the new in/out API accepts the same buffer for input and output. The at-rest
  adapter retains detached 16-byte tags appended to ciphertext and uses
  in-place slice conversions, with zeroizing scratch buffers on failure.
- [HMAC 0.13.0](https://docs.rs/hmac/0.13.0/hmac/) moves key construction to
  `KeyInit`; [HKDF 0.13.0](https://docs.rs/hkdf/0.13.0/hkdf/) retains the
  extract/expand path used here. NIST/RFC known answers and the existing
  independently generated Node envelope fixture cover unchanged cryptographic
  bytes; the fixture is not regenerated for this upgrade.
- [rand_core 0.10.1 changelog](https://docs.rs/crate/rand_core/0.10.1/source/CHANGELOG.md)
  removes `OsRng`. The leaf now directly uses
  [getrandom 0.4.3 `fill`](https://docs.rs/getrandom/0.4.3/getrandom/fn.fill.html),
  which returns an error on partial failure and leaves destination contents
  unspecified on error. The existing failure source now modifies half the
  destination before failing; seal and changed-key rewrap must still return
  only `RandomnessUnavailable`.
- [tower-http 0.7.1 request IDs](https://docs.rs/crate/tower-http/0.7.1/source/src/request_id.rs):
  comparison with 0.6.11 changes only constructors/accessors to `const fn`.
  Batter uses only this module, with `RequestId` privately wrapped; it does not
  expose tower-http types. Reqwest still resolves its own tower-http 0.6.11.
- [metrics-exporter-otel 0.3.1 manifest](https://docs.rs/crate/metrics-exporter-otel/0.3.1/source/Cargo.toml)
  requires OpenTelemetry 0.31. Keep the SDK, exporter, HTTP and protobuf family
  aligned with that bridge; updating the direct requirements to 0.33 would
  introduce incompatible meter types.
- [reqwest 0.13.5 features](https://docs.rs/crate/reqwest/0.13.5/source/Cargo.toml)
  remove `rustls-tls-webpki-roots`; `rustls` selects aws-lc-rs and the platform
  verifier. The follow-up `batter-7oob` migrates OTLP and the reference provider
  to 0.13.5 using explicit rustls configurations and bundled WebPKI roots.
  The production outbound exchange now uses owned low-level HTTP/1 transport;
  reqwest remains only in its control tests.

### macOS refusal fixture repair

`batter-8e8r.1` replaces bound-but-unconnected sockets in the facade example and
outbound fault tests. On the macOS arm64 host, a standalone socket probe found
that connecting to such a port times out; holding both ends of an established
loopback connection and targeting its client-side port instead produces
`ConnectionRefused`. This is executed OS behavior, not a claim that `bind`
alone guarantees refusal. Apple's [connect(2) manual](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/connect.2.html)
distinguishes refusal from connection-establishment timeout; Linux's
[bind(2) manual](https://man7.org/linux/man-pages/man2/bind.2.html) documents
address allocation and `EADDRINUSE`. The fixtures explicitly bind before
connecting, retain both stream owners until after the HTTP attempt, and bound
their own setup with a two-second timeout. Regression checks retain exact
`NotDispatched(Failed)` assertions, reject a competing non-reuse bind, and send
a marker over the held connection to detect unexpected HTTP bytes or closure.
This prevents accidental ephemeral allocation, not deliberate explicit
`SO_REUSEADDR`/`SO_REUSEPORT` rebinding by another process.

## Facade feature and resolver semantics: reviewed 2026-09-18; resolver rechecked 2026-09-30

- Cargo's [feature reference](https://doc.rust-lang.org/cargo/reference/features.html)
  defines optional dependency features, `dep:` names, additive feature
  forwarding, and feature unification. The facade uses those mechanisms to
  select adapter namespaces without moving native ownership into `batter`.
- Cargo's [resolver reference](https://doc.rust-lang.org/cargo/reference/resolver.html)
  explains why a workspace `--all-features` build cannot prove an isolated
  consumer graph. The facade runner therefore gives each temporary consumer an
  external workspace boundary and checks normal dependency reachability.
- Rechecked the [resolver 2 feature rules](https://doc.rust-lang.org/cargo/reference/resolver.html#feature-resolver-version-2)
  and [virtual workspace rules](https://doc.rust-lang.org/cargo/reference/workspaces.html#virtual-workspace)
  for `batter-0jsf`: resolver 2/3 separates dev-dependency features from ordinary
  builds when development targets are not built. Resolver 1 unifies them; a
  virtual workspace needs an explicit resolver because it has no root package
  edition. The recipe now records that precondition. This is an enabled build
  graph claim, not a claim about unused code surviving final binary linking.
- Cargo's [target reference](https://doc.rust-lang.org/cargo/reference/cargo-targets.html)
  defines `required-features`, used by the facade-owned runnable consumers.

The runner preserves the repository's resolved registry and Git source tuples,
uses a Cargo-generated temporary lock only for the temporary root package, and
then runs compilation with `--locked`. These primary Cargo semantics justify
the validation shape; they are not evidence that the facade runner itself has
passed.

## Effect v4

The 2026-09-08 review used Effect 4.0.0-rc.112 for the
[design rationale](../effect-v4-brief.md). The links below identify that reviewed
API rather than asserting the latest Effect release.

The [beta announcement](https://effect.website/blog/releases/effect/40-beta)
documents the runtime rewrite, coordinated first-party package versions, and
platform/RPC/cluster consolidation into `effect`. Concrete platform, database
driver, and provider packages still exist separately. Modules under `unstable`
may change in minor releases; this is an explicit compatibility policy, not
simply a packaging switch.

Version-pinned migration references:

- [Services](https://github.com/Effect-TS/effect/blob/effect%404.0.0-rc.112/migration/services.md):
  Context.Service replaces Context.Tag; construction still has explicit wiring.
- [Fiber references](https://github.com/Effect-TS/effect/blob/effect%404.0.0-rc.112/migration/fiberref.md):
  Context.Reference and scoped service provision replace FiberRef conventions;
  this is more than a spelling change.
- [Cause](https://github.com/Effect-TS/effect/blob/effect%404.0.0-rc.112/migration/cause.md):
  flat Fail/Die/Interrupt reasons no longer encode sequential versus parallel
  composition. Batter's internal reports are a smaller aggregation contract.
- [Layer memoization](https://github.com/Effect-TS/effect/blob/effect%404.0.0-rc.112/migration/layer-memoization.md):
  sharing spans provide calls, with explicit freshness/local overrides. The guide
  still recommends explicit layer composition.
- [Schema migration](https://github.com/Effect-TS/effect/blob/effect%404.0.0-rc.112/migration/schema.md)
  documents redesigned codec, refinement, transformation, and issue APIs.
  [Schema](https://github.com/Effect-TS/effect/blob/effect%404.0.0-rc.112/packages/effect/src/Schema.ts)
  and [JsonSchema](https://github.com/Effect-TS/effect/blob/effect%404.0.0-rc.112/packages/effect/src/JsonSchema.ts)
  are core modules. The [unstable tree](https://github.com/Effect-TS/effect/tree/effect%404.0.0-rc.112/packages/effect/src/unstable)
  has additional schema facilities, not a wholesale unstable Schema/JsonSchema API.
- [TxRef](https://effect.website/docs/v4/api/effect/TxRef): core STM operations
  coordinate through Effect.tx. Rust mutexes/atomics/channels do not reproduce
  those semantics merely by being native concurrency tools.

No primary benchmark substantiating the supplied blanket claim that streams and
batching are approximately 20 times faster was verified. Do not promote that
claim into a Batter performance comparison or implementation requirement.

The original architectural references remain useful:

- [Scope](https://effect.website/docs/v4/api/effect/Scope): managed finalization;
  Batter deliberately does not claim equivalent interruptibility guarantees.
- [Layer](https://effect.website/docs/v4/api/effect/Layer): construction/resource
  composition, distinct from Tower middleware layers. The page inspected
  identifies the v4 API as 4.0.0-rc.112.
- [ExecutionPlan](https://effect.website/docs/v4/api/effect/ExecutionPlan): the
  inspiration for coherent retry/fallback policy; Batter implements only a small
  bounded-retry subset.
- [TestClock](https://effect.website/docs/v4/api/effect/testing/TestClock): testable
  runtime time; Batter's authored timer tests use Tokio paused time instead.
- [The Effect type](https://effect.website/docs/v4/getting-started/the-effect-type):
  background for the preceding brief's Success/Error/Requirements comparison.

## Boundary conventions reviewed: 2026-09-08

- [RFC 9457](https://www.rfc-editor.org/rfc/rfc9457.html), sections 3.1, 3.2,
  4.2.1, and 5: Problem Details uses `type` as its primary identifier, allows
  extensions, and does not require `detail` or `instance`. `about:blank` carries
  HTTP status semantics; its title should match that status phrase. A separate
  `code` extension is not a standard problem-type identifier. Response data must
  be selected to avoid exposing internals. Batter's existing basic body and tests
  do not establish an application-wide problem-type taxonomy or full conformance.
- [Tower HTTP panic middleware](https://docs.rs/tower-http/latest/tower_http/catch_panic/index.html)
  provides a native adapter candidate. No tower-http version is selected and no
  catcher is implemented in Batter; inspect the chosen release before integrating.
  Rust's [catch_unwind contract](https://doc.rust-lang.org/std/panic/fn.catch_unwind.html)
  excludes aborting panics and runs the panic hook before catching an unwind.
  Sanitizing a response does not sanitize hook output or recover arbitrary state.
- [Cargo feature compatibility](https://doc.rust-lang.org/cargo/reference/features.html#semver-compatibility):
  optional APIs still require compatibility decisions. An `unstable` feature
  name is not itself an exemption from a crate's published stability policy.
