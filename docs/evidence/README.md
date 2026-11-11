# Evidence index

[Documentation index](../README.md) · [Implementation status](../status.md) ·
[Current verification](../testing.md#verification)

These records describe particular revisions, experiments and execution
environments. They are retained evidence, not current setup instructions or a
delivery backlog. A passing historical run does not verify today's checkout.
Owning Beads retain delivery decisions and verification outcomes.

## Design assessments

- [API invalid-state assessments](api-assessments/README.md) record the ownership
  and type-shape decisions behind current APIs, grouped by operational boundary.
- [Implicit state-machine audit](implicit-state-machines-2026-11-05.md) retains the
  2026-11-05 inventory and linked follow-up outcomes.
- [Flags control-plane evaluation, 2026-11-08](flags-evaluation-2-2026-11-08/README.md)
  scores a fresh agent's flag-aware service against an oracle frozen before
  launch, and archives the consumer and the agent's report.

## API surfaces and migration evidence

| Record | Scope |
| --- | --- |
| [Public API diff since 0.0.1](public-api-diff-2026-10-09/README.md) | Extraction method, package surfaces and later operation/readiness comparisons. |
| [Managed objects, 2026-10-15](managed-objects-api-diff-2026-10-15/README.md) | Managed-object API surface. |
| [Constraint triggers, 2026-10-16](managed-constraint-triggers-api-diff-2026-10-16/README.md) | Constraint-trigger API surface. |
| [Browser credentials, 2026-10-17](browser-credentials-api-diff-2026-10-17/README.md) | Browser credential API surface. |
| [Owned outbound connections, 2026-11-04](outbound-owned-connection-api-diff-2026-11-04.md) | Transport ownership comparison. |
| [Startup handoff, 2026-11-05](startup-handoff-2026-11-05.md) | Consuming handoff API and verification. |
| [Managed release fence, 2026-11-05](managed-release-fence-2026-11-05.md) | Release-ordering evidence and [API surface](managed-release-fence-api-2026-11-05.txt). |
| [Owned profiled pool, 2026-11-05](owned-profiled-pool-2026-11-05.md) | Pool ownership evidence and [API surface](owned-profiled-pool-api-2026-11-05.txt). |
| [Intent reconciliation, 2026-11-05](intent-reconciliation-2026-11-05.md) | Reconciliation evidence and [API surface](intent-reconciliation-api-2026-11-05.txt). |
| [Typed flags, 2026-11-08](flags-public-api-2026-11-08/README.md) | Flags packages and their seams in existing packages ([ADR-016](../adr/016-typed-flags-control-plane.md)). |
| [Flags store, T-12](flags-store-t12.md) | Store implementation-run results. |

## Retention

Keep evidence that supports a current contract, public migration or unresolved
assessment. Superseded prototypes, old frozen consumer packages, repair diaries
and the publication dossier have been removed; recover historical snapshots from
Git when needed. Record new execution results in the owning Bead, and upstream
semantics in [primary references](../references.md).
