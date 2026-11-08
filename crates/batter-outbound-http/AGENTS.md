# Outbound exchange guide

## Purpose

Own one send-once provider HTTP exchange on Unix. Provider meaning, replay,
idempotency keys, Retry-After, dispatch fences and reconciliation belong to the
application. Experimental under ADR-015, originally `batter-3s1a`; connection
ownership and deadline correction belong to `batter-u387`.

## Key entrypoints

- `src/client.rs`: one-origin client, immutable shared rustls configuration,
  explicit ring, TLS 1.3/1.2, ALPN http/1.1, HTTPS platform verifier and empty
  loopback roots. No process-default crypto provider.
- `src/origin.rs`, `src/limits.rs`: validated origins and bounded limits.
- `src/request.rs`: draft validation, origin-form request/Host/framing assembly,
  non-Clone prepared request and consuming send; compile-fail ownership tests.
  `RequestDraft::trailing_slash()` preserves endpoint path endings without
  admitting empty or dot segments or changing the origin root.
- `src/transport.rs`: owned TCP/TLS establishment, clock/cancellation gate and
  guarded writes above and below TLS. TLS reads may flush encrypted records.
- `src/transport/dial.rs`: bounded family/address fallback, with at most two
  TCP candidates in the exchange future, under one setup deadline; losing
  candidates drop before TLS/HTTP.
- `src/send.rs`: pre-start window, setup timeout, immediate pre-send_request
  gate, inline hyper HTTP/1 driver, phase classification and bounded body.
- `src/exchange.rs`: exhaustive must-use Exchange, opaque unsent evidence,
  stable labels, status/body retention and redacted formatting.
- `src/operation.rs`: optional OperationContext bridge; only Batter dependency.
- `src/transport/tests.rs`: deterministic handoff/queued/partial-writer stalls.
- `src/transport/tls_write_tests.rs`: real TLS buffered-request/error-read
  regressions using the same TLS assembly and driver as production.
- `src/send/tests.rs`: a ready setup poll that consumes its connect budget.
- `tests/`: preparation, before-head, body, operation, wire, redaction,
  ClientHello and Linux trust-store integration suites.

## Edit here for X

Transport changes belong in client/transport/send, preparation in request/origin,
and evidence vocabulary in exchange. Keep README, crate rustdoc, guarantees,
integrations, architecture, SECURITY, operations, ADR-010/015 and references
consistent. Record actual checks in the owning Bead. Recheck exact resolved
upstream APIs before dependency changes. reqwest is dev-only for the proxy
control and feature-unification controls (gzip/HTTP2); do not use it in the
normal transport. The facade feature runner rejects reqwest and hyper-util's
legacy client in the normal leaf and shipped facade example graphs.
The facade consumer lives in `crates/batter/examples/outbound_exchange`.

## Invariants

Each exchange owns a fresh TCP connection, TLS handshake and low-level hyper
HTTP/1 connection driver. Never spawn that driver/writer, introduce a pool,
reuse/resend a request or infer unsent proof from upstream error kinds.
Before handoff, cancellation then timer win over connection progress. The final
gate immediately precedes synchronous send_request, with no await in between.
After handoff, available replies win, then cancellation, then deadline. Every
write/vectored-write/flush/shutdown is gated, including the TCP stream below
TLS. Reading an available reply after stopping must not flush a late request.
No HTTP/TLS writer task can progress while the exchange future is unpolled.
Already-entered polls and kernel transmission cannot be retracted; Unknown
remains conservative after handoff. Pre-handoff expiry/cancellation is
NotStarted, setup failure/timeout is NotDispatched. Both carry opaque evidence.

Preserve one head/body deadline, bounded body buffering, status retention,
sensitive header values and value-free formatting. Keep request ownership,
exhaustive outcomes and twelve stable labels. No provider policy or telemetry.
Do not generalize this ready-result precedence to arbitrary application futures.
No core dependency outside `operation`; never depend on the facade/adapters.
New Linux/macOS claims require executed evidence; historical macOS checks in
`batter-8e8r` predate the owned transport.

## Common commands

- `cargo test -p batter-outbound-http --all-features --locked`
- `cargo test -p batter-outbound-http --no-default-features --locked`
- `cargo clippy -p batter-outbound-http --all-targets --all-features --locked -- -D warnings`
- `cargo doc -p batter-outbound-http --all-features --no-deps --locked`
- `bash scripts/verify.sh`
