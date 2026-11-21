# ADR-010: Optimize integration for coding-agent consumers

## Status

Accepted guidance, 2026-09-11. Owning Bead: `batter-o4h`.
Strengthened 2026-09-20 by `batter-j6f`. This ADR does not authorize speculative
redesign, but evidence that the canonical path represents a locally preventable
invalid state requires the assessment below rather than another caller warning.

## Context

While implementing an example, the user reported several review/repair rounds
that did not converge. This prompted the agent-only consumption policy: example
repair churn can reveal operational protocols that consumers must reconstruct
instead of invariants enforced by Batter. Follow-up Bead `batter-lxm` codifies
the implementation agent's responsibility to investigate that signal.
This is the user's reported motivation, not a replay or independent audit of
those rounds, a confirmed API diagnosis, or an executed usability benchmark.

## Decision

Batter's only consumers are coding agents generating or modifying applications.
Its public integration path therefore prioritizes convergence on correct native
Rust/Tokio composition. Operational invariants should follow from
library-owned execution, constrained interfaces, validated configuration, and
executable checks. A repeated caller obligation to coordinate cancellation,
joining, registration, finalization, deadline relationships, or error
retention is design feedback and debt, even when accurately documented.

On the canonical path, make invalid operational states unrepresentable whenever
Rust ownership, types, or API shape can express the invariant. Documentation,
examples and semantic tests demonstrate the contract but do not replace API
enforcement. A known-invalid ordering, nesting, paired call, phase transition,
empty input, cleanup sequence or omitted outcome must not remain an ordinary
peer of the supported composition merely because guidance describes the right
choice.

The repository keeps one clear canonical supported path. Examples are consumer
contracts. A lower-level escape hatch may remain for a justified application
boundary, but it must disclose its obligations and must not appear equivalent to
the protected path. Application-specific protocols stay in applications;
upstream-specific lifecycle obligations belong in supported adapters when a
real adapter exists. Agent-only consumption does not justify opaque DSLs,
excessive abstractions, giant instruction files, or claims that types prove
arbitrary remote effects.

Design proposals are assessed with independent failure scenarios and fresh-agent
implementation or modification tasks. Clean review or test volume alone is not
evidence of agent usability. Such evaluations are proposed and unexecuted until
the repository records concrete execution evidence.

## Public API invalid-state review

Every new or materially changed public API must be reviewed before delivery for
invalid states that its canonical consumers can still construct. Do not wait for
a recurring example failure. Ask:

1. Can a caller construct a value, phase or route that execution must later
   reject even though pure construction had enough information to reject it?
2. Must a caller remember ordering, middleware nesting, a paired completion,
   cleanup sequencing, non-emptiness, deadline relationships or exhaustive
   outcome handling for the invariant to hold?
3. Can authority or lifecycle state be represented by a narrower capability,
   opaque validated witness, consuming transition or exhaustive enum instead of
   a broad cloneable handle, raw value, boolean or temporal convention?
4. Can library-owned assembly or execution make the supported composition the
   only ordinary path, rather than documenting which combination of individually
   valid calls is safe?
5. Is the remaining obligation truly application policy, upstream protocol
   behavior, or an unverifiable remote effect? If so, keep that boundary explicit
   and do not imply a stronger local guarantee.

When a common misuse can reach execution, or still compiles where a type-state
transition could prevent it, redesign the canonical path or record the evidence-
backed reason it cannot own the invariant. Typical tools are private constructors,
opaque validated values, nonempty collections, capability tokens, typestate or
consuming transitions, exhaustive results, and library-ordered composition.
These are means, not a requirement to introduce a framework or DSL. Prefer the
smallest native Rust boundary that removes the caller-memory obligation.

A justified low-level escape hatch may expose a weaker contract, but its name,
placement and documentation must distinguish it from the protected path and list
the obligations it leaves with the caller. Its existence does not justify making
the same invalid state representable through the canonical API.

## Recorded API assessments

Delivery-specific reviews live in the [assessment index](../evidence/api-assessments/README.md).
The links below retain this ADR's existing assessment anchors.

- <a id="tracked-operation-invocation-assessment-batter-2hqe"></a>[Tracked operation invocation assessment (batter-2hqe)](../evidence/api-assessments/operations.md#tracked-operation-invocation-assessment-batter-2hqe)
- <a id="startup-handoff-assessment-batter-r4ir"></a>[Startup handoff assessment (batter-r4ir)](../evidence/api-assessments/lifecycle.md#startup-handoff-assessment-batter-r4ir)
- <a id="native-online-index-repair-assessment-batter-fnj810"></a>[Native online-index repair assessment (batter-fnj8.10)](../evidence/api-assessments/postgresql.md#native-online-index-repair-assessment-batter-fnj810)
- <a id="managed-application-sql-assessment-batter-aouh"></a>[Managed application SQL assessment (batter-aouh)](../evidence/api-assessments/postgresql.md#managed-application-sql-assessment-batter-aouh)
- <a id="at-rest-rewrap-pairing-assessment-batter-t2uq"></a>[At-rest rewrap pairing assessment (batter-t2uq)](../evidence/api-assessments/operations.md#at-rest-rewrap-pairing-assessment-batter-t2uq)
- <a id="operation-authority-assessment-batter-tc9w1"></a>[Operation authority assessment (batter-tc9w.1)](../evidence/api-assessments/operations.md#operation-authority-assessment-batter-tc9w1)
- <a id="bounded-metrics-setup-assessment-batter-6vn"></a>[Bounded metrics setup assessment (batter-6vn)](../evidence/api-assessments/lifecycle.md#bounded-metrics-setup-assessment-batter-6vn)
- <a id="protected-diagnostic-completion-assessment-batter-i3ny"></a>[Protected diagnostic completion assessment (batter-i3ny)](../evidence/api-assessments/lifecycle.md#protected-diagnostic-completion-assessment-batter-i3ny)
- <a id="private-response-referrer-assessment-batter-lto"></a>[Private-response referrer assessment (batter-lto)](../evidence/api-assessments/http.md#private-response-referrer-assessment-batter-lto)
- <a id="http-route-group-assessment-batter-tc9w3"></a>[HTTP route group assessment (batter-tc9w.3)](../evidence/api-assessments/http.md#http-route-group-assessment-batter-tc9w3)
- <a id="reference-http-boundary-assessment-batter-tc9w4"></a>[Reference HTTP boundary assessment (batter-tc9w.4)](../evidence/api-assessments/http.md#reference-http-boundary-assessment-batter-tc9w4)
- <a id="admitted-router-assessment-batter-tc9w8"></a>[Admitted router assessment (batter-tc9w.8)](../evidence/api-assessments/http.md#admitted-router-assessment-batter-tc9w8)
- <a id="probe-renderer-and-readiness-condition-assessment-batter-tc9w5"></a>[Probe renderer and readiness condition assessment (batter-tc9w.5)](../evidence/api-assessments/http.md#probe-renderer-and-readiness-condition-assessment-batter-tc9w5)
- <a id="admitted-request-extractor-assessment-batter-tc9w6"></a>[Admitted request extractor assessment (batter-tc9w.6)](../evidence/api-assessments/http.md#admitted-request-extractor-assessment-batter-tc9w6)
- <a id="native-package-reachability-assessment-batter-0jsf"></a>[Native package reachability assessment (batter-0jsf)](../evidence/api-assessments/native-runtime.md#native-package-reachability-assessment-batter-0jsf)
- <a id="generic-serving-listener-assessment-batter-tc9w7"></a>[Generic serving listener assessment (batter-tc9w.7)](../evidence/api-assessments/http.md#generic-serving-listener-assessment-batter-tc9w7)
- <a id="native-grant-fragment-assessment-batter-qjxi"></a>[Native grant fragment assessment (batter-qjxi)](../evidence/api-assessments/postgresql.md#native-grant-fragment-assessment-batter-qjxi)
- <a id="sealed-http-assembly-assessment-batter-tc9w2"></a>[Sealed HTTP assembly assessment (batter-tc9w.2)](../evidence/api-assessments/http.md#sealed-http-assembly-assessment-batter-tc9w2)
- <a id="probe-response-policy-assessment-batter-probe-response-policy-2wz4"></a>[Probe response policy assessment (batter-probe-response-policy-2wz4)](../evidence/api-assessments/http.md#probe-response-policy-assessment-batter-probe-response-policy-2wz4)
- <a id="periodic-maintenance-assessment-batter-s6vu"></a>[Periodic maintenance assessment (batter-s6vu)](../evidence/api-assessments/lifecycle.md#periodic-maintenance-assessment-batter-s6vu)
- <a id="runledger-invocation-phases-assessment-batter-5h5y"></a>[Runledger invocation phases assessment (batter-5h5y)](../evidence/api-assessments/native-runtime.md#runledger-invocation-phases-assessment-batter-5h5y)
- <a id="typed-startup-initializer-assessment-batter-syah"></a>[Typed startup initializer assessment (batter-syah)](../evidence/api-assessments/lifecycle.md#typed-startup-initializer-assessment-batter-syah)
- <a id="maintained-runlimit-storage-assessment-batter-z4m9"></a>[Maintained Runlimit storage assessment (batter-z4m9)](../evidence/api-assessments/postgresql.md#maintained-runlimit-storage-assessment-batter-z4m9)
- <a id="whole-application-baseline-batter-k64t"></a>[Whole-application baseline (batter-k64t)](../evidence/api-assessments/native-runtime.md#whole-application-baseline-batter-k64t)
- <a id="managed-database-objects-assessment-batter-54d0"></a>[Managed database objects assessment (batter-54d0)](../evidence/api-assessments/postgresql.md#managed-database-objects-assessment-batter-54d0)
- <a id="browser-credential-gate-assessment-batter-52s3"></a>[Browser credential gate assessment (batter-52s3)](../evidence/api-assessments/http.md#browser-credential-gate-assessment-batter-52s3)
- <a id="fixture-login-pool-assessment-batter-7r38"></a>[Fixture login pool assessment (batter-7r3.8)](../evidence/api-assessments/postgresql.md#fixture-login-pool-assessment-batter-7r38)
- <a id="handler-test-facility-assessment-batter-3upk"></a>[Handler test facility assessment (batter-3upk)](../evidence/api-assessments/native-runtime.md#handler-test-facility-assessment-batter-3upk)
- <a id="finite-command-signal-and-owned-completion-assessment-batter-od4b"></a>[Finite command signal and owned completion assessment (batter-od4b)](../evidence/api-assessments/lifecycle.md#finite-command-signal-and-owned-completion-assessment-batter-od4b)
- <a id="bounded-blocking-work-assessment-batter-0772"></a>[Bounded blocking work assessment (batter-0772)](../evidence/api-assessments/operations.md#bounded-blocking-work-assessment-batter-0772)
- <a id="redacting-panic-hook-assessment-batter-4q78"></a>[Redacting panic hook assessment (batter-4q78)](../evidence/api-assessments/lifecycle.md#redacting-panic-hook-assessment-batter-4q78)
- <a id="public-origin-invalid-state-review-batter-321j"></a>[Public-origin invalid-state review (batter-321j)](../evidence/api-assessments/http.md#public-origin-invalid-state-review-batter-321j)
- <a id="response-body-storage-invalid-state-review-batter-53nf"></a>[Response-body storage invalid-state review (batter-53nf)](../evidence/api-assessments/http.md#response-body-storage-invalid-state-review-batter-53nf)
- <a id="outbound-exchange-assessment-batter-3s1a"></a>[Outbound exchange assessment (batter-3s1a)](../evidence/api-assessments/http.md#outbound-exchange-assessment-batter-3s1a)
- <a id="owned-phase-and-acquisition-handoff-assessment-batter-jxj2"></a>[Owned phase and acquisition handoff assessment (batter-jxj2)](../evidence/api-assessments/operations.md#owned-phase-and-acquisition-handoff-assessment-batter-jxj2)
- <a id="protected-service-readiness-and-typed-output-batter-m9rp"></a>[Protected service readiness and typed output (batter-m9rp)](../evidence/api-assessments/lifecycle.md#protected-service-readiness-and-typed-output-batter-m9rp)
- <a id="intent-reconciliation-assessment-batter-hw3k"></a>[Intent reconciliation assessment (batter-hw3k)](../evidence/api-assessments/postgresql.md#intent-reconciliation-assessment-batter-hw3k)
- <a id="flags-store-assessment-t-12-batter-jkw612"></a>[Flags store assessment (T-12, batter-jkw6.12)](../evidence/api-assessments/flags.md#flags-store-assessment-t-12-batter-jkw612)
- <a id="flags-catalog-and-declarations-assessment-t-04-batter-jkw64"></a>[Flags catalog and declarations assessment (T-04, batter-jkw6.4)](../evidence/api-assessments/flags.md#flags-catalog-and-declarations-assessment-t-04-batter-jkw64)
- <a id="flags-snapshots-evaluation-and-pins-assessment-t-05-batter-jkw65"></a>[Flags snapshots, evaluation and pins assessment (T-05, batter-jkw6.5)](../evidence/api-assessments/flags.md#flags-snapshots-evaluation-and-pins-assessment-t-05-batter-jkw65)
- <a id="declared-mode-reader-and-test-flags-assessment-t-06-batter-jkw66"></a>[Declared mode, reader and test flags assessment (T-06, batter-jkw6.6)](../evidence/api-assessments/flags.md#declared-mode-reader-and-test-flags-assessment-t-06-batter-jkw66)
- <a id="route-group-gate-seam-assessment-t-07-batter-jkw67"></a>[Route-group gate seam assessment (T-07, batter-jkw6.7)](../evidence/api-assessments/flags.md#route-group-gate-seam-assessment-t-07-batter-jkw67)
- <a id="http-flags-assessment-t-08-batter-jkw68"></a>[HTTP flags assessment (T-08, batter-jkw6.8)](../evidence/api-assessments/flags.md#http-flags-assessment-t-08-batter-jkw68)
- <a id="job-flags-assessment-t-09-batter-jkw69"></a>[Job flags assessment (T-09, batter-jkw6.9)](../evidence/api-assessments/flags.md#job-flags-assessment-t-09-batter-jkw69)
- <a id="periodic-flags-assessment-t-10-batter-jkw610"></a>[Periodic flags assessment (T-10, batter-jkw6.10)](../evidence/api-assessments/flags.md#periodic-flags-assessment-t-10-batter-jkw610)
- <a id="flags-presence-prune-and-fleet-assessment-t-13-batter-jkw613"></a>[Flags presence, prune and fleet assessment (T-13, batter-jkw6.13)](../evidence/api-assessments/flags.md#flags-presence-prune-and-fleet-assessment-t-13-batter-jkw613)
- <a id="control-plane-loader-assessment-t-14-batter-jkw614"></a>[Control-plane loader assessment (T-14, batter-jkw6.14)](../evidence/api-assessments/flags.md#control-plane-loader-assessment-t-14-batter-jkw614)
- <a id="last-good-file-assessment-t-15-batter-jkw615"></a>[Last-good file assessment (T-15, batter-jkw6.15)](../evidence/api-assessments/flags.md#last-good-file-assessment-t-15-batter-jkw615)
- <a id="operator-command-assessment-t-16-batter-jkw616"></a>[Operator command assessment (T-16, batter-jkw6.16)](../evidence/api-assessments/flags.md#operator-command-assessment-t-16-batter-jkw616)
- <a id="flags-facade-and-consumer-assessment-t-17-batter-jkw617"></a>[Flags facade and consumer assessment (T-17, batter-jkw6.17)](../evidence/api-assessments/flags.md#flags-facade-and-consumer-assessment-t-17-batter-jkw617)
- <a id="control-plane-invariants-assessment-t-21-batter-jkw621"></a>[Control-plane invariants assessment (T-21, batter-jkw6.21)](../evidence/api-assessments/flags.md#control-plane-invariants-assessment-t-21-batter-jkw621)
- <a id="flags-control-plane-fresh-agent-evaluation-t-24-batter-jkw624"></a>[Flags control-plane fresh-agent evaluation (T-24, batter-jkw6.24)](../evidence/api-assessments/flags.md#flags-control-plane-fresh-agent-evaluation-t-24-batter-jkw624)
- <a id="branch-state-protocol-repairs-batter-jkw636"></a>[Branch state-protocol repairs (batter-jkw6.36)](../evidence/api-assessments/flags.md#branch-state-protocol-repairs-batter-jkw636)
- <a id="search-path-refusal-batter-jkw634"></a>[Search-path refusal (batter-jkw6.34)](../evidence/api-assessments/flags.md#search-path-refusal-batter-jkw634)

## Recurring example review defects

The implementation agent must initiate an assessment when the same confirmed
invariant fails again after repair, or when a repair introduces a confirmed
failure in a coupled lifecycle phase, such as cleanup racing unfinished work.
Do not wait for the user to notice. An existing review-round limit is a backstop,
not a reason to postpone assessment; reviewer comment counts alone are not a
trigger or proof of a design gap. "It is only example code" cannot dismiss the
signal: examples are consumer contracts for the library's intended agents.

The proactive review above applies even without a failure. When recurrence or a
coupled-phase failure supplies stronger evidence, complete these steps before
another dependent repair:

1. Identify the recurring defects with source, failure-scenario, and repair
   evidence. Separate confirmed defects from duplicate or unsupported reviewer
   concerns and changed requirements; label missing execution evidence.
2. Trace each invariant to what the public API enforces and what the caller must
   remember. Identify the consumer, foundation, adapter, or upstream owner.
3. Classify the cause using the distinctions below. State whether another local
   patch removes the cause or merely covers another case of the same protocol.
4. Record the concern, evidence, alternatives, and recommended owner/remedy in
   the owning Bead; create a linked design Bead when separate scope is needed.
   Explicitly report the assessment to the user before continuing dependent
   repairs. Beads owns any resulting delivery scope and dependencies.

| Cause | Response |
| --- | --- |
| Implementation mistake | The supported API already enforces the invariant when used correctly. Fix the consumer and clarify misleading examples or guidance. |
| Library or adapter design gap | The consumer repeatedly reconstructs ownership, cleanup, timing, registration, or error-retention protocols. Assess moving the shared obligation into library-owned execution or a justified supported adapter. Another caller instruction alone does not close the design concern. |
| Application or upstream protocol complexity | The behavior belongs to application policy or the upstream dependency. Reassess whether the example needs it and whether reusable upstream lifecycle mechanics justify an adapter; preserve the upstream's responsibilities. |

Causes can coexist; keep uncertain classifications explicit. Assess reducing
optional example behavior without weakening required consumer contracts or
semantic failure tests to make a review pass. Clean review is not proof of
agent usability.

Implement a bounded remedy when evidence supports it and it is already within
the user's authorized scope. If the remedy expands scope or changes a public
contract beyond that authorization, present the concrete problem, alternatives,
and recommended scope change for a user decision before dependent changes.
Continue independent authorized work. Do not silently redesign the library or
continue an indefinite sequence of patches around the same caller obligation.
Validate an accepted remedy against the recurring failure scenarios and retain
the distinction between executed checks and proposed fresh-agent evaluations.

## Consequences and limits

This guidance preserves native errors, optional adapters, Unix scope, dependency
direction, and the existing honest limits: no asynchronous `Drop`, runtime-death
survival, or detached-task termination guarantee. It does not make unimplemented
adapters available, redesign APIs, or turn every application root into a
foundation protocol.
