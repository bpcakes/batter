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

### At-rest rewrap pairing assessment (`batter-t2uq`)

The earlier wrapper-only rotation returned a `WrappedKey` that the caller had to
attach to the input descriptor. Attaching it to another descriptor compiled and
failed authentication only when used. The canonical
`Keyring::rewrap_envelope` now authenticates once and returns the complete
`Envelope` with the input descriptor, so ordinary rotation has no separate
attachment step. A complete result cannot be passed directly as a `WrappedKey`
to another envelope's composer. The old wrapper-only signature and split-part
constructors remain for compatibility; their rustdoc marks them as lower-level
paths, and extraction can still reintroduce the mismatch. Cross-descriptor
authentication tests cover that deliberate escape hatch.

This local shape proves only the pairing returned by that call. Decoded and
reconstructed envelopes share the same type and may be unauthenticated; the
method does not read the body. Storage must retain header/body association,
compare-and-swap a complete prior header or covering revision, fence current
key policy, and enforce durable wrapper-encryption budgets. These remote and
application-owned effects cannot be proved by the returned Rust value. Fresh
agent implementation and modification evaluations remain proposed and
unexecuted for this API.

### Operation authority assessment (`batter-tc9w.1`)

A cloneable context previously let a consumer cancel the shared operation or
construct an unrelated root while processing a child callback. That made a
sibling or parent interruption possible through an ordinary execution argument.
The canonical path now passes cloneable `OperationContext` only for observation
and execution. Non-cloneable `OperationOwner` retains explicit cancellation;
child owners derive their deadline and token from the complete parent context.
`RootDeadline` preserves absolute time for an explicitly independent root, and
`OperationAdmission` returns an owner linked to process cancellation only after
readiness. Compile-fail rustdocs reject context cancellation, independent root
construction through a context, and owner cloning. Runtime regressions cover
child isolation, deadline clamping and process cancellation.

The application root still chooses when an independent operation is appropriate;
local types cannot prove that policy. An owner may deliberately give up its
authority with `into_context`, and dropping it does not cancel detached work.
The application must still await any work it owns and keep cleanup separate from
process cancellation. These are explicit ownership limits, not claims of async
drop or remote effect rollback. Fresh-agent usability evaluation is proposed
and unexecuted.

### Native installation validation assessment (`batter-wloc`)

`MigrationHistory` requires an explicit choice between bundled checksums and
application-managed history. `InstallationError` separates incompatible local
schema/grant observations from database and timeout failures; callers cannot
receive a success value carrying unresolved issues. Both modes run the same
schema and effective-grant inspection, so choosing application-managed history
does not bypass those requirements. The canonical `validate_installation` call
owns transaction setup, timeout, rollback, and connection disposal.

Calling validation before migrations returns an incompatibility rather than a
false success. A service can still choose not to call it, or its remote schema,
role or search path may change after success. A Rust capability cannot prove
those external facts for the later admission and cleanup connections; the API
documents the snapshot boundary and requires a uniform pool configuration.
Fresh agent implementation and modification evaluations for this port are
proposed and unexecuted.

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
