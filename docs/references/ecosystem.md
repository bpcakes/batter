# References: Dependency selection and Effect v4 rationale

[Reference index](../references.md) · [Documentation index](../README.md)

These source checks support current contracts. Each record names the
version and date actually inspected; it is not a new dependency audit.
Re-check the resolved dependency before changing an integration.

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
