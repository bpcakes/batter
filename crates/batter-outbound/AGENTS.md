# Outbound exchange guide

## Purpose

Own the transport part of one outbound provider call: a library-built HTTP/1.1
client, send-once request preparation, a start gate inside the caller's
deadline, a bounded body read under the same deadline, and the exhaustive
`Exchange`. Provider meaning, replay, fences and reconciliation stay with the
crate that owns the provider protocol. Experimental (`batter-3s1a`, ADR-015,
ADR-014 track 1), not a provider adapter. Standalone leaf like `batter-at-rest`.
Unix only; Linux/macOS execution claims require actual recorded checks, and the
tests have executed on Linux only.

## Key entrypoints

- `src/lib.rs`: crate documentation, outcome table, deadline and limit
  non-claims, and re-exports (`http`, `Bytes`).
- `src/origin.rs`: validated HTTPS or literal-loopback HTTP `Origin`.
- `src/limits.rs`: `ConnectTimeout` and `ResponseLimit` bounds.
- `src/client.rs`: the fixed reqwest profile, the rustls configuration it hands
  to reqwest (*ring* passed explicitly, TLS 1.3 and 1.2, ALPN `http/1.1`, the
  platform verifier for HTTPS and an empty root store for loopback) and
  `Client::request`.
- `src/request.rs`: `RequestDraft` preparation rules, `PreparedRequest::send`,
  the sensitive-header unit test and the `compile_fail` doctests that keep
  `PreparedRequest` non-`Clone` and send-once.
- `src/send.rs`: `RESERVE`, `MINIMUM_ATTEMPT`, the start gate, the biased
  selects, transport classification and the bounded body read.
- `src/exchange.rs`: `Exchange`, the opaque `NotStarted` and `NotDispatched`
  evidence with a `compile_fail` doctest against a hand-built one, their
  reasons, `Response`, `Body`, `proves_unsent` and `label`.
- `src/operation.rs`: `PreparedRequest::send_in` behind feature `operation`.
- `tests/preparation.rs`, `tests/before_head.rs`, `tests/body.rs`,
  `tests/operation.rs` (feature `operation`), `tests/wire.rs` and
  `tests/redaction.rs`: refusals without a connection, and loopback fault tests
  that pin every outcome, the raw request and value-free formatting.
- `tests/tls_hello.rs`: the ClientHello's ALPN offer and TLS versions, with
  reqwest's `http2` compiled into the test build.
- `tests/trust_store.rs` (Linux only): a child process with an empty
  certificate store, where an HTTPS client fails with `ClientError::Build` and a
  loopback client still builds.

## Edit here for X

Change the client profile only in `src/client.rs`, transport classification and
deadline handling in `src/send.rs`, the outcome vocabulary in `src/exchange.rs`,
and preparation rules in `src/request.rs` and `src/origin.rs`. Record every
upstream fact the classification relies on in `docs/references.md`, and keep
`README.md`, `docs/guarantees.md` and `docs/integrations.md` in step, and
`SECURITY.md` when a TLS or limit setting changes. When reqwest, hyper-util or
rustls-platform-verifier change, recheck the log events listed in
`docs/references.md` and their disclosure in the README, the crate rustdoc,
`SECURITY.md`, `docs/operations.md` and the ADR-010 assessment. Status
interpretation, retry or replay, idempotency keys, Retry-After, fences,
reconciliation and telemetry belong to the application or the crate that owns
the provider protocol, never here. The facade namespace lives in
`crates/batter/src/lib.rs`; the runnable consumer is the facade's
`outbound_exchange` example. The scripted loopback fault peer lives at
`crates/batter-outbound/tests/support/peer.rs`; it counts connections and keeps
raw requests, so every fault test can assert one request or none. The
dev-dependencies compile reqwest's `gzip` and `http2` into test builds; keep
them, because `tests/tls_hello.rs` and the encoded-body test in `tests/body.rs`
rely on them to show the profile's overrides.

## Invariants

The reqwest profile is fixed and private: no public builder, setter, accessor or
reqwest type, and no conversion from a caller's `reqwest::Client` or `Request`.
TLS goes through the package's own rustls configuration with the *ring*
provider passed explicitly: never install or rely on a process-default crypto
provider, and never select reqwest's `rustls` feature, which adds aws-lc-rs as a
second provider. reqwest accepts that configuration only while both crates use
the same rustls release line; otherwise `Client::new` fails with
`ClientError::Build` (source-inspected). The `http/1.1`-only ALPN list in that
configuration is what keeps HTTP/2 out; `http1_only()` is defence in depth. A
loopback client keeps its empty root store, so it never reads the host
certificate store.
`NotDispatched` comes only from the connect kind (`reqwest::Error::is_connect`);
every other failure before a response head is `Unknown`. `is_timeout` only
separates `ConnectFailure::TimedOut` from `Failed`. Deadline reasons come only
from this package's clock: never set reqwest's `timeout` or `read_timeout`, and
never map `is_timeout` to a deadline reason. Keep the start-gate check
immediately before the first poll, with no await between them; no test detects
an inserted await, so review guards it. Keep the biased order request, then
cancellation, then timer, in both selects:
`head_ready_with_the_expired_timer_is_kept` and
`head_ready_with_cancellation_is_kept` fail if a ready head or the end of an
empty body loses to the timer or a cancellation, and
`cancellation_ready_with_the_expired_timer_is_reported` and
`cancellation_ready_with_the_expired_timer_during_the_body_is_reported` fail if
the timer wins over a cancellation before a head or during the body. A ready
non-empty body chunk tying with the timer or a cancellation is untested, so
review still guards that case. Ready results win only inside the exchange:
the biased select is sound because no redirect or replay can start another
request, so do not reuse it for arbitrary futures or move it into batter-core.
Keep the status when the body fails. No provider
meaning, replay, retry, fences, telemetry or HTTP/2. Formatted output never
carries URLs, queries, header values or bodies: hand-written `Debug`, value-free
error messages and sensitive header values in both directions; a `Response`
still holds the received headers and body behind its accessors. `Exchange`
stays exhaustive and `#[must_use]`, `NotStarted` and `NotDispatched` keep private
fields with no public constructor or `Clone`, `PreparedRequest` stays
non-`Clone` with a consuming `send`, and labels are stable. The fault tests ran
only against the versions this repository's lockfile resolves: re-run them on
every reqwest, hyper or hyper-util bump and recheck `docs/references.md`. No
batter-core dependency outside the `operation` feature; never depend on the
facade or an adapter.

## Common commands

- `cargo test -p batter-outbound --all-features --locked`
- `cargo test -p batter-outbound --no-default-features --locked` (without `batter-core`)
- `cargo clippy -p batter-outbound --all-targets --all-features --locked -- -D warnings`
- `cargo doc -p batter-outbound --all-features --no-deps --locked`
- `bash scripts/verify.sh`
