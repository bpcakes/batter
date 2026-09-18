# Runlimit adapter

## Purpose

Own quota-to-work execution and optional authenticated HTTP assembly. Native
Runlimit retains policy validation, hashing, atomic storage and transaction
disposition. This is a Unix-only adapter, not a limiter or authentication service.

## Key entrypoints

- `src/quota.rs`: nonempty native checks, typed consumption bridge and factory execution.
- `src/http.rs`: owned async authentication, native-peer subject selection and opaque serving.
- `batter-axum/src/quota_observation.rs`: the retained HTTP fact writer and read-only observation.
- `tests/quota.rs`, `tests/http.rs`: native memory, interrupted execution and actual serving.
- `examples/shape.rs`: runnable library consumer without the reference service.

## Edit here for X

Native backend algorithms belong upstream. This package only classifies pinned
native failures and composes operational ownership. Keep optional memory,
PostgreSQL and HTTP features independent; tests must not hide graph leakage via
workspace feature unification. Future facade imports must follow actual layout.

## Invariants

One native atomic batch precedes work. Never split it into sequential checks or
automatically retry/refund. Same parent deadline bounds admission and work.
An admitted quota does not establish factory invocation or application success.
Take the observation writer before calling application authentication/handlers;
forwarded headers and prior principal extensions are not subject authority.
Protected routes are the default; public probes require `with_public_probes`
and a `PublicProbes` builder of literal GET paths. Reject capture and
catch-all patterns at registration before Axum can match protected paths. Guard the complete protected
Router, including its custom fallbacks. Never accept an arbitrary Router for
public probes: Axum retains custom method fallbacks after `reset_fallback`.
Only declared GET/HEAD probe handlers may bypass admission, authentication and
quota; unsupported methods use Axum's default 405 without a custom handler.
Preserve a protected root fallback when provided, and the default unmatched
404 otherwise.
Handlers extract `Authenticated<P>`, never raw `Extension<P>` for authority.
The allowed result retains native scalar decision metadata without exposing a
denial arm; unknown future native denial kinds use an other-denial label.
Authentication meaning and explicitly unguarded probe routes remain application
policy. PostgreSQL setup/maintenance is not implemented or implicitly performed.
No body-stream, detached-task or remote rollback guarantee is added.

## Common commands

```sh
cargo test -p batter-runlimit --all-features --locked
cargo run -p batter-runlimit --features axum,memory --example quota_service --locked
python3 scripts/check_runlimit_features.py
```
