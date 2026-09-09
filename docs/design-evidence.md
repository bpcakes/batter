# Design grounded in backend review evidence

Re-audited on 2026-09-09 against commit `22848ea`. This document records design
rationale and the implemented boundary. Beads owns delivery scope, acceptance,
priority, status and dependencies; this is not a parallel roadmap.

The foundation's purpose is to preserve execution budgets, ownership and outcome
meaning as work crosses requests, transactions, external providers and durable
jobs. Ordinary futures, concrete domain errors, native transactions and explicit
dependencies remain the programming model. A shared convention earns its place
when it prevents a demonstrated mistake or makes that mistake observable.

## Evidence and limits

The private retrospective dated 2026-09-08 sampled 45 grouped mechanisms across
five backend project families. It used retained review reports, contemporaneous
remediation plans, selected Git changes, test source and Jig receipts. Repeated
review rounds and related checkouts were not counted as independent discoveries.
Most records were remediation histories; selected cases had original reports or
recorded failing-before/passing-after evidence. The audit did not replay those
applications or establish current defects, incident rates or AI-specific causation.

The full private provenance is retained outside this repository in
`/home/aa/Documents/batter-review-evidence-2026-09-08.md` and its JSON companion.
The generic cases below contain the design rationale needed by an implementer;
access to another application's private repository is not a prerequisite.

Jig receipts establish recorded execution scope. A harness contract check does
not test an application's behavior; a passing test with a lossy oracle cannot
establish a contract it never asserted. A matching revision/fingerprint improves
attribution, but does not turn a reviewer concern into a reproduced failure.

## What changed in the implemented foundation

The re-audit inspected the four commits after `e5f2f04`, including the actual
source and selected tests, rather than treating yesterday's gaps as current work.

- Lifecycle transitions and admission now share a private state owner. Stopped
  is terminal, unstarted-owner abandonment signals before capture destruction,
  and permanent closure precedes startup/capacity errors. See
  [state](../crates/batter/src/lifecycle/state.rs),
  [state tests](../crates/batter/src/lifecycle/state/tests.rs) and
  [ownership tests](../crates/batter/tests/lifecycle_state.rs).
- Non-yielding subprocess controls and bounded scheduling exploration now have
  recorded execution and defect-rejecting controls. These establish observable
  limits and tested orderings, not scheduler exhaustiveness or preemption. See
  [subprocess tests](../crates/batter/tests/non_yielding.rs) and
  [scheduling tests](../crates/batter/tests/scheduling.rs).
- HTTP observation is independent of admission. Response severity overrides,
  filtered-span correlation, full owned-future destruction and handler unwinding
  have explicit tests. The real example router retains observation across probes,
  fallback and rejection. See [adapter](../crates/batter-axum/src/lib.rs),
  [correlation tests](../crates/batter-axum/tests/observation/correlation.rs) and
  [example tests](../crates/batter-axum/examples/http_service/tests.rs).

The latest combined-source validation records macOS execution on both supported
toolchains; earlier Linux evidence retains its earlier source scope. Hosted CI
and live PostgreSQL execution remain unverified in that record. The re-audit
conclusions use inspected source and recorded evidence; checks for this planning
change are recorded separately in its Jig session. See [validation](validation.md).

Core operation, retry and semaphore admission production files are unchanged in
this commit range. The SQLx demonstration still acquires a pool, probes it and
explicitly tears it down. Runledger, Runlimit, typed application config, generated
contracts, durable correlation and a metrics/exporter recipe remain outside the
implemented capabilities. See [status](status.md) and [integrations](integrations.md).

## Failure mechanisms and ownership decisions

**Existing helpers can be bypassed.** Reviewed code acquired raw pool connections
outside an established deadline/disposition policy, and new provider runners
bypassed shared capacity. Test the complete command and actual configuration
consumers. Keep transaction arguments visible and pass one budget through the
real path. Another generic database wrapper would not establish adoption.

**Retryability and effect certainty are different.** A pre-dispatch failure can
permit a policy-defined retry or refund; a response lost after dispatch can leave
the mutation unknown. Retain those distinctions in concrete provider/application
types. `ReplaySafety` is an assertion by the caller, not proof of idempotency.
Share mechanics while keeping interactive and durable budgets and retry owners
distinct. Reconciliation requires an explicit provider protocol with declared
key retention and payload matching, not a universal transient-error classifier.

**The transaction must own the whole invariant.** Historical commands could
affect replacement generations, commit mutation separately from required durable
submission, or observe inconsistent snapshots. Application identity, fencing,
lock order and native transaction boundaries remain application-owned. A focused
database proof must use public commands, controlled interleavings, a constrained
pool and independent persisted-state assertions. Process cleanup is not durable
business recovery.

**Metadata is not authority.** Diagnostic events and untrusted forwarding data
were able to select business state or admission identity. Correlation propagation
must stay separate from authentication, proxy trust, exact durable ownership and
generation checks. Explicit extensions and arguments are the default. Ambient
access is conditional ergonomics, not the authority or task-lifetime mechanism.

**Observability has operational costs.** Diagnostic database writes defeated load
shedding, serialized unrelated transactions and allowed a timing constraint to
roll back business work. Optional integrations need bounded observation storage,
explicit saturation behavior and slow/failing-sink tests. Best-effort metrics are
separate from authoritative audit records. Existing synchronous `tracing`
callbacks do not isolate arbitrary blocking or panicking subscribers; a selected
exporter's operational noninterference contract must state that limit.

**Config and generated contracts must preserve meaning.** Reviewed validators
disagreed across runtime/deployment, stored settings were ignored, and schemas
silently lost constraints during successful conversion. Typed root configuration
must reach native resource constructors. Final serialized specifications and
generated clients must agree with actual routes, required fields and complete
domain success/failure states. Test terminal polling behavior as well as error
envelopes. Add compatibility cases for formats actually exposed or persisted.

**Fixtures and assertions must distinguish the failure.** Historical tests
disabled production authentication, expected duplicate replay events, omitted
producer variants or never checked that fixture setup changed a row. Preserve
the real router/config/transaction path, assert fixture preconditions and inspect
independent outcomes. A real database clock/interleaving is not simulated by
Tokio paused time. Multiple body/cleanup failures should remain individually
inspectable; checking only redacted output is insufficient.

## Architectural consequence

Reusable execution mechanics stay in Batter. Persistence protocols, authority,
provider meaning and successful-state projection stay in applications or their
upstream libraries. A small unpublished reference application is the place to
demonstrate their composition and failure behavior; the existing minimal SQLx
lifecycle demonstration can retain its current purpose.

Repeated failure classes justify that application proof. They do not establish
two real consumers of a new abstraction. Focused experimental operational
helpers may be implemented under their concrete Beads; stable promotion and
broader extraction remain conditional on actual adoption. Preserve concrete
errors and keep DI containers, repository facades, new workflow engines and
generic provider interfaces outside this adjustment.

No upstream dependency or external protocol is selected by this rationale. The
delivery graph retains a compatibility prerequisite that must check the actual
current public APIs before implementation. Existing completed lifecycle/HTTP work
remains evidence to reuse, rather than a reason to create another general
hardening task.

The implementation-readiness follow-up inspected candidate upstream source,
recorded in [references](references.md). It exposed handoff constraints beyond
the original failure categories: worker construction is not readiness, dropping
a fixture lease can trigger deletion, detached SQLx sessions outlive pool
accounting, and reconciliation must use identity retained before dispatch.
These constraints refine open acceptance criteria; they establish no executed
upstream compatibility or new implemented capability.
