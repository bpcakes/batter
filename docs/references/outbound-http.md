# References: Outbound HTTP transport

[Reference index](../references.md) · [Documentation index](../README.md)

These source checks support current contracts. Each record names the
version and date actually inspected; it is not a new dependency audit.
Re-check the resolved dependency before changing an integration.

## Reqwest consolidation, 2026-11-04

`batter-7oob` migrates the OTLP adapter and reference provider from reqwest
0.12.28 to 0.13.5; Cargo now resolves one reqwest version. OpenTelemetry remains
on the bridge-compatible 0.31 line. The implementation was checked against the
downloaded reqwest 0.13.5 and rustls 0.23.46 sources and the pinned
[ClientBuilder documentation](https://docs.rs/reqwest/0.13.5/reqwest/struct.ClientBuilder.html).

- `rustls-no-provider` requires a provider unless a preconfigured TLS backend
  is supplied. Both clients pass `tls_backend_preconfigured` an explicit ring
  configuration with TLS 1.2/1.3 and WebPKI roots (updated to 1.0.11). They do
  not use the platform verifier or install/read the global provider. The
  reqwest/rustls release lines must remain compatible: reqwest downcasts the
  configuration and rejects an unknown backend type at construction.
  Unrelated plain reqwest builders with only `rustls-no-provider` still require
  application-owned provider setup. That upstream constructor policy cannot be
  enforced by these private clients; their builders never rely on it.
- Their shipped 0.12 manifests did not enable HTTP/2 or decompression. Both
  now explicitly pin HTTP/1.1 in ALPN and the builder, and disable gzip, Brotli,
  deflate and zstd. This also prevents feature unification with the outbound
  package's dev dependencies from changing response bytes after consolidation.
  Previously, consumer features could enable those behaviors; they no longer
  change these private client profiles.
- The OTLP client retains `retry::never`; the reference provider now sets it
  explicitly too. With HTTP/2 compiled and forced in a test, the native default
  sends three attempts after `REFUSED_STREAM`, versus one with each production
  retry policy. Redirects and proxy discovery remain disabled. Connect/body
  error classification and application-owned replay decisions remain unchanged;
  the 0.13.5 `is_connect` source review in the next section applies to all three
  clients, superseding the historical 0.12 review for these two.
- New shared loopback regressions reject a self-signed certificate over TLS
  1.2 and 1.3, then complete a handshake using a separately trusting control;
  inspect the actual ClientHello ALPN; and assert no global provider appears.
  With gzip compiled, actual client response bytes stay encoded while an
  explicitly enabled control decodes them. These check the builder passed to
  reqwest, not just a standalone rustls configuration.

No public signatures or persisted formats change. The fixed profiles enforce
provider ownership and response-byte/replay policy internally; applications
cannot substitute a builder into these paths. Public-CA acceptance, the live
reference PostgreSQL effect suite and hosted CI are not established by these
loopback tests. `bash scripts/verify.sh` passed on macOS arm64 / Rust 1.98.1:
formatting, file budgets, all four Clippy configurations, all six matrix parts,
rustdoc and all HTTP smoke profiles. `cargo outdated -R --offline` now lists
only the bridge-constrained OpenTelemetry packages, with no obsolete reqwest
feature warnings. This follow-up did not run Linux or the ignored reference live
effect suite. Verification is recorded in `batter-7oob`.

## Owned outbound HTTP/1 connections, 2026-11-04

`batter-u387` removes reqwest from the leaf's normal dependency graph. Cargo
resolves hyper 1.11.1, hyper-util 0.1.20, Tokio 1.54.1, tokio-rustls 0.26.6,
rustls 0.23.46, rustls-platform-verifier 0.7.1 and http-body-util 0.1.5.
The exact downloaded registry sources were inspected; the web renderer could
not retrieve the pinned documentation URLs.

- [hyper HTTP/1 handshake](https://docs.rs/hyper/1.11.1/hyper/client/conn/http1/fn.handshake.html)
  returns a SendRequest and a Connection. In `src/client/conn/http1.rs`,
  handshake constructs these without spawning a task. Connection must be
  polled by its owner; Batter polls it alongside the response/body receiver.
- [SendRequest::send_request](https://docs.rs/hyper/1.11.1/hyper/client/conn/http1/struct.SendRequest.html#method.send_request)
  synchronously calls dispatch.send before returning the response future. Its
  documented request contract requires Host and sends the URI as supplied.
  Batter builds Host and an origin-form path/query at preparation. The final
  gate directly precedes this synchronous handoff.
- [Connection](https://docs.rs/hyper/1.11.1/hyper/client/conn/http1/struct.Connection.html)
  performs HTTP/1 I/O when polled. `src/proto/h1/dispatch.rs` polls reading,
  writing and flushing inside its loop. Batter owns this driver and never
  spawns it. Read-side progress after expiry is permitted only through I/O
  wrappers that block new writes/flushes; unsent proof is based on no handoff,
  not on a hyper error kind or the driver's internal loop order.
- [TokioIo](https://docs.rs/hyper-util/0.1.20/hyper_util/rt/struct.TokioIo.html)
  adapts Tokio read/write traits to hyper. Only hyper-util's tokio feature is
  required by the leaf. The legacy client/pool code is not invoked.
- [TlsConnector::connect](https://docs.rs/tokio-rustls/0.26.6/tokio_rustls/struct.TlsConnector.html#method.connect)
  returns a handshake future over the supplied stream and uses the supplied
  Arc<ClientConfig>. `src/common/mod.rs` can write TLS records while servicing
  a read, including an alert after packet processing fails. Therefore Batter
  guards TCP below TLS as well as plaintext writes above it. No detached TLS
  writer is created by this path.
- [Incoming](https://docs.rs/hyper/1.11.1/hyper/body/struct.Incoming.html) exposes
  framed response data and a body size hint. Batter uses its exact size hint,
  rather than a raw Content-Length header, so HEAD/bodyless replies remain
  empty even when the header describes a representation. BodyExt::frame reads
  data/trailers; trailers are consumed without becoming message-body bytes.
- Tokio's [TcpStream::connect](https://docs.rs/tokio/1.54.1/tokio/net/struct.TcpStream.html#method.connect)
  awaits resolved addresses serially (`src/net/tcp/stream.rs:121-137`). The
  earlier reqwest 0.13.5 connector used hyper-util's
  [300 ms family fallback](https://docs.rs/hyper-util/0.1.20/hyper_util/client/legacy/connect/struct.HttpConnector.html#method.set_happy_eyeballs_timeout)
  and [shared address timeouts](https://docs.rs/hyper-util/0.1.20/hyper_util/client/legacy/connect/struct.HttpConnector.html#method.set_connect_timeout).
  `batter-4p4a` therefore resolves through Tokio's
  [lookup_host](https://docs.rs/tokio/1.54.1/tokio/net/fn.lookup_host.html) and
  runs its own bounded TCP candidate futures. The other family starts after
  at most 300 ms or half the remaining setup budget; each family's remaining
  addresses share its remaining time. Losing candidates drop before TLS/HTTP.
  DNS can use blocking work, but it owns no HTTP request or connection writer.
- reqwest 0.13.5 `src/async_impl/client.rs:284-285` supplies default
  `Accept: */*`. The direct request assembler preserves it only when the caller
  supplied no Accept. The omission has equivalent negotiation meaning under
  [RFC 9110 section 12.4.1](https://httpwg.org/specs/rfc9110.html#section-12.4.1),
  but preserving the previous wire shape avoids an unrelated policy change.

The TLS failure-path test calls the production TLS assembly and HTTP driver
over controlled raw I/O. It leaves encrypted HTTP records buffered, then
injects an invalid TLS record while the writer becomes writable. Exact
tokio-rustls 0.26.6 `common/mod.rs:109-115` calls write_io on a packet-processing
error; rustls 0.23.46 `common_state.rs:633-644` queues its alert without clearing
pending records, and `conn.rs:812-813` drains that queue. Temporarily removing
the raw guard made the regression fail with request ciphertext written after
expiry. A second ablation removed the direct setup clock recheck and made the
late-ready setup test dispatch instead of returning an unsent timeout. Both
protections were restored and the unit suite passed. These focused Linux
experiments belong to `batter-4p4a`; they do not establish live dual-stack routing
or new macOS evidence.

TLS construction retains explicit ring, TLS 1.3/1.2, http/1.1 ALPN, the platform
verifier for HTTPS and empty roots for loopback. The earlier verifier/rustls
source observations below still apply: macOS verification may block a poll,
Linux roots follow SSL_CERT_FILE/SSL_CERT_DIR, and verifier/rustls logs can
contain certificate/destination identities. reqwest and legacy hyper-util
pool/destination logs no longer arise from this transport path. The
application still owns logger filtering and any default provider needed by
other dependencies. These are source observations, not claims of new macOS
execution. Exact performed verification is recorded in `batter-u387` and
the panel follow-up `batter-4p4a`.

## Public HTTPS address policy (2026-11-05)

The conservative exclusions were checked against the IANA
[IPv4 special-purpose registry](https://www.iana.org/assignments/iana-ipv4-special-registry/)
and [IPv6 special-purpose registry](https://www.iana.org/assignments/iana-ipv6-special-registry/).
The implementation also excludes globally reachable special-use ranges,
including AS112 192.31.196.0/24, 192.175.48.0/24 and 2620:4f:8000::/48, and
AMT 192.52.193.0/24. Literal boundary tests cover the first/last excluded
addresses and their permitted neighbors; dialer tests cover these blocks in
mixed and wholly disallowed answers. The registry does not guarantee routing
in a local network;
the API makes no such claim. Enforcement uses the inspected Tokio lookup
result directly, followed by the existing bounded dialer, with no second DNS
lookup. Literal and real localhost tests cover refusal before TCP; mixed-answer
tests observe dialer calls rather than issuing traffic to public addresses.

## Outbound response-body storage (2026-11-09)

`batter-53nf` replaces the body read's growing `Vec` with a fixed zeroizing
buffer. Cargo resolves bytes 1.12.1 and zeroize 1.9.1, and the exact downloaded
registry sources were inspected; these are source observations.

- [Bytes::from_owner](https://docs.rs/bytes/1.12.1/bytes/struct.Bytes.html#method.from_owner)
  moves the owner into a reference-counted `Owned<T>` (`src/bytes.rs:254-290`).
  `owned_clone` only increments that count (`:1114`), and clones, slices and
  splits keep the owner. `owned_drop_impl` decrements with `Release`, and only the
  last release takes an `Acquire` load and drops `Box<Owned<T>>` (`:1144-1160`).
  The destructor therefore runs once, after every other view is finished.
- `owned_is_unique` is always false (`:1140`), so `try_into_mut` returns `Err`.
  `owned_to_vec` deep-copies with `to_vec` and then releases its share (`:1129`),
  and `owned_to_mut` goes through it (`:1136`). Upstream documents that conversion
  to `BytesMut` always creates a deep copy. A sole handle of a body built by
  `Bytes::from(Vec)` is unique (`promotable_is_unique`, `:1306-1316`), so it could
  be reclaimed without a copy; that no longer happens.
- [`Zeroize for [Z]`](https://docs.rs/zeroize/1.9.1/zeroize/trait.Zeroize.html)
  writes the default value through `ptr::write_volatile` for every element
  (`src/lib.rs:452-467`, `:749-765`). Its `Vec` implementation is documented as
  best effort because it cannot reach storage released by earlier reallocations
  (`:512-533`), which is why the buffer does not grow a `Vec`. The `Zeroize for
  [Z]` used here needs no `alloc` feature; this package disables default features.
- [Incoming](https://docs.rs/hyper/1.11.1/hyper/body/struct.Incoming.html) frames
  are hyper-owned `Bytes` that `read_mem` splits from its read buffer
  (`src/proto/h1/io.rs:354-362`); the length, chunked and close-delimited decoders
  return them as frames (`src/proto/h1/decode.rs:151-170`, `:476-504`, `:224-236`).
  The package copies them out and cannot wipe that storage.

Each protection was removed in turn and the private tests were rerun on Linux:
the destructor's wipe (removed, limited to half the written bytes, and limited to
the written bytes), the destructor on replaced allocations, `Bytes::from_owner`
(copied out, then leaked), the growth ceiling, the absence of eager limit
allocation, the preallocation cap and, one at a time, the size-limit, transport,
cancellation and deadline paths. Each of the thirteen ablations failed the
matching tests, and all passed once restored.

In one release-build measurement on Linux x86-64, `zeroize` took 0.26 ms for
1 MiB, 2.1 ms for 8 MiB and 18 ms for 64 MiB, against 0.36 ms, 4.2 ms and 38 ms for
copying the same sizes into fresh zeroed memory. One growth step pays both for
its old allocation, so the largest steps approach `RESERVE`. A protected final
shrink would have paid both again in the exchange's last poll, which is why a
completed body keeps the allocation it grew into. Execution details and platform
limits are recorded in `batter-53nf`.
