# ADR-010: Optimize integration for coding-agent consumers

## Status

Accepted guidance, 2026-09-11. Owning Bead: `batter-o4h`.
No library redesign is implied.

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

## Recurring example review defects

The implementation agent must initiate an assessment when the same confirmed
invariant fails again after repair, or when a repair introduces a confirmed
failure in a coupled lifecycle phase, such as cleanup racing unfinished work.
Do not wait for the user to notice. An existing review-round limit is a backstop,
not a reason to postpone assessment; reviewer comment counts alone are not a
trigger or proof of a design gap. "It is only example code" cannot dismiss the
signal: examples are consumer contracts for the library's intended agents.

Before another dependent repair:

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
