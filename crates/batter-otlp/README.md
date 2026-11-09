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
service adds its own literal-loopback, plain-HTTP restriction. For direct
VictoriaMetrics ingestion use the explicit profile below.

## Direct VictoriaMetrics export

With facade feature `otlp`, prepare the same protected lifecycle using:

```rust,no_run
let diagnostics = batter::otlp::prepare_victoriametrics(
    "http://127.0.0.1:8428/opentelemetry/v1/metrics",
    "example-service",
    "worker-1",
    batter::otlp::Schedule::new(
        std::time::Duration::from_secs(10),
        std::time::Duration::from_secs(3),
        std::time::Duration::from_secs(5),
    )?,
)?;
let owner = batter::service::start(startup, diagnostics);
let completion = owner.wait().await;
// Inspect both completion.service() and completion.diagnostics().
```

Direct-package consumers use `batter_otlp::prepare_victoriametrics`. This profile
requires a bounded nonempty `service.instance.id` in addition to `service.name`.
Choose a distinct ID for each concurrent producer and retain it for that process's
lifetime. The library validates syntax and length, not cross-process uniqueness.
Keep these resource attributes when configuring backend label promotion.

Endpoints must end in `/opentelemetry/v1/metrics`; nonempty prefix segments allow
cluster URLs such as `/insert/0/opentelemetry/v1/metrics` and proxy prefixes.
Credentials, query strings and fragments remain forbidden. Authentication and
private-CA termination require an appropriately configured local proxy/collector;
this API adds no authentication headers or custom trust roots.

VictoriaMetrics v1.156.0 responds to successful ingestion with an empty HTTP 200
and no Content-Type. Only this explicit profile accepts that response. A present
wrong media type, nonempty untyped body, non-200 status, malformed Protobuf or
partial rejection still fails. A typed Protobuf success remains accepted. The
ordinary `prepare` profile keeps its strict endpoint and response requirements.

`python3 scripts/check_victoriametrics.py` provisions a pinned Docker image on a
random loopback port, exports from two isolated processes, queries stored
counters/histograms and final cleanup/shutdown observations, and removes its
container. It runs in the workspace verification matrix. Executed evidence covers
Linux and single-node v1.156.0; cluster deployment, authentication and macOS Docker
execution are not claimed. See the workspace
[evidence](../../docs/evidence/victoriametrics-2026-11-09.md).

The reqwest 0.13 transport uses an explicit rustls configuration with the ring
provider, TLS 1.2/1.3 and bundled WebPKI roots. It neither reads the platform
trust store nor installs a process-default crypto provider. HTTP/1.1, no redirects,
no proxy discovery, no protocol retries and no response decompression are fixed
even when another dependency enables reqwest's HTTP/2 or compression features.

This selects reqwest's `rustls-no-provider` feature. An application building a
separate plain `reqwest::Client` without a provider-enabled feature must supply
its own TLS configuration or install a process-default provider at its root.
This adapter does neither on the application's behalf.

Batter installs the recorder before startup and owns the service through cleanup
and diagnostic completion. Owner drop requests drain; cancelling a borrowed
waiter does not. Service and diagnostics have separate retained outcomes,
including synchronous installation and asynchronous diagnostic panics. Diagnostics
are process-global: a second installation is rejected and closes only its own
pipeline. After successful installation the recorder cannot be reset/reloaded.
The adapter's completed report is either `Exported` or `InstallationRejected`.
Endpoint absence belongs to application configuration, while a panicked or failed
diagnostic task remains a separate core `DiagnosticOutcome`.

The adapter admits only Batter's catalog names, labels and descriptions through
its shared schema, caps complete keys before bridge allocation, uses fixed
histogram buckets, and exports
cumulative manual-reader snapshots with one serial owner. It has no queue or
retry, limits requests to 2 MiB and responses to 64 KiB, and retains typed failure
categories. An ordinary OTLP success must declare the Protobuf media type even
when its body is empty; the explicit VictoriaMetrics profile additionally accepts
the empty untyped 200 described above. A lookalike media type cannot acknowledge
an export.
It is a dedicated Batter-only pipeline; custom application metrics
need an explicitly composed recorder elsewhere.

Final collection follows the retained startup/shutdown result. Any in-flight
periodic attempt settles first, then final export receives its own allowance.
Exporter/provider closure follows the attempt exactly once on normal completion.
Guard closure waits for in-flight recorder delegation and rejects later registrations
and descriptions. Previously obtained handles remain outside that guard.
Allowances bound yielding I/O, not synchronous SDK collection/closure or blocking
recorder code. Diagnostic panic is retained but cannot guarantee resource closure.
Report coverage does not prove unsupervised work stopped; acknowledgement does
not prove durable collector storage. No runtime-death or async-Drop guarantee.

Upstream versions: metrics-exporter-otel 0.3.1, OpenTelemetry SDK 0.31.0 and
OTLP 0.31.1. Native aggregation and encoding remain upstream. The experimental
custom-reader interface is private to this adapter. See `docs/references.md` at
the workspace root for inspected sources.

Run `cargo test -p batter-otlp --locked` and the workspace verification script.
The adapter tests rerun affected cases in clean child processes when the invoking
shell has `OTEL_*` settings; a separate child proves public preparation still
rejects those settings.

Services returning typed startup output use
`service::start_with_readiness(startup, prepared, Some(absolute_deadline))` and
consume `into_ready().await` before using that output. Core owns the deadline,
readiness observation, drain and output handoff; the exporter keeps its existing
completion witness and final-export ordering. The facade HTTP example selects
this adapter with the `otlp` feature and `BATTER_OTLP_ENDPOINT`.
