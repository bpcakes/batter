# ADR-005: Preserve causes internally; sanitize automatic outputs

Status: accepted for the MVP source. Date: 2026-09-07.

Context: applications need typed domain failures, while a process supervisor
must collect errors from unrelated components. Operational interruption, an
expected business rejection, a panic, and an uncertain external outcome are not
interchangeable. Error strings can contain secrets and high-cardinality data.

Decision: retain generic E in operation/retry APIs. Only heterogeneous lifecycle
and cleanup boundaries box original errors. Reports retain all observed causes
and skipped work. Automatic tracing uses names, outcome categories, counts, and
timing; no global subscriber is installed. HTTP infrastructure errors expose
sanitized stable codes, not report/source serialization.

Consequences: internal reports require a trusted diagnostic sink; Debug is not a
redaction contract. Domain-to-HTTP mapping remains application-owned. Metrics
exporters and durable trace propagation are deferred, not secretly provided by
local spans. Rust's default panic hook may print payloads independently of tracing;
Batter does not claim whole-process panic redaction or install a global hook.

Amendment, 2026-09-08: ordinary INFO subscribers receive operation completion and
cleanup events without enabling span events; task failures and abnormal operation
outcomes use WARN. HTTP status/outcome/latency are separate from response
construction success. Automatic request dimensions are normalized method and
matched route, not raw URI/headers. Owned spawn boundaries carry the current
span and scoped subscriber. Finite task errors preserve the same original E
through a shared typed receipt and heterogeneous report source chain.

Cancellation hardening, 2026-09-08: dispatch propagation must cover destruction,
not only polling. Tracing's poll-scoped subscriber wrapper is insufficient for
abort-time events and nested span destruction under another dispatcher. Use one
private, allocation-free future wrapper, safely projected with pin-project-lite,
across observed operation/HTTP bodies and owned task boundaries. No public
runtime abstraction or consumer opt-in is introduced. Cleanup routes every skip
through the same report-and-event path.
