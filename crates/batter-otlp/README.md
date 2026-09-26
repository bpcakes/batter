# batter-otlp

Optional OTLP/HTTP metrics export for Batter's protected service lifecycle.
This 0.0.1 publication candidate targets Unix, including Linux and macOS.

`prepare(endpoint, service_name, Schedule::new(interval, attempt, final_allowance)?)`
returns an inert, non-cloneable pipeline. Pass it to
`batter_core::service::start(protected_startup, prepared)`; through the facade,
select feature `otlp` and use `batter::otlp` with `batter::service`.
See the `prepare` rustdoc and `tests/service.rs` for complete consumers.

The application selects configuration sources, identity, timing, endpoint trust
and exit policy. This crate reads no configuration values itself, but rejects
ambient `OTEL_*` names because the selected native builder merges those values.
Do not mutate the process environment during preparation. HTTP(S) endpoints must
name `/v1/metrics` without URL credentials, query or fragment. The reference
service adds its own literal-loopback, plain-HTTP restriction.

Batter installs the recorder before startup and owns the service through cleanup
and diagnostic completion. Owner drop requests drain; cancelling a borrowed
waiter does not. Service and diagnostics have separate retained outcomes,
including synchronous installation and asynchronous diagnostic panics. Diagnostics
are process-global: a second installation is rejected and closes only its own
pipeline. After successful installation the recorder cannot be reset/reloaded.

The adapter admits only Batter's catalog through its shared schema, caps complete
keys before bridge allocation, uses fixed histogram buckets, and exports
cumulative manual-reader snapshots with one serial owner. It has no queue or
retry, limits requests to 2 MiB and responses to 64 KiB, and retains typed failure
categories. It is a dedicated Batter-only pipeline; custom application metrics
need an explicitly composed recorder elsewhere.

Final collection follows the retained startup/shutdown result. Any in-flight
periodic attempt settles first, then final export receives its own allowance.
Exporter/provider closure follows the attempt exactly once on normal completion.
Allowances bound yielding I/O, not synchronous SDK collection/closure or blocking
recorder code. Diagnostic panic is retained but cannot guarantee resource closure.
Report coverage does not prove unsupervised work stopped; acknowledgement does
not prove durable collector storage. No runtime-death or async-Drop guarantee.

Upstream versions: metrics-exporter-otel 0.3.1, OpenTelemetry SDK 0.31.0 and
OTLP 0.31.1. Native aggregation and encoding remain upstream. The experimental
custom-reader interface is private to this adapter. See `docs/references.md` at
the workspace root for inspected sources.

Run `cargo test -p batter-otlp --locked` and the workspace verification script.
