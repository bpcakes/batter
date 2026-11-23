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

## API surfaces and migration evidence

| Record | Scope |
| --- | --- |
| [VictoriaMetrics, 2026-11-09](victoriametrics-2026-11-09.md) | Direct OTLP profile, API comparison and pinned Docker ingestion/read-back. |
| [Public API diff since 0.0.1](public-api-diff-2026-10-09/README.md) | Extraction method, package surfaces and later operation/readiness comparisons. |
| [Managed objects, 2026-10-15](managed-objects-api-diff-2026-10-15/README.md) | Managed-object API surface. |
| [Constraint triggers, 2026-10-16](managed-constraint-triggers-api-diff-2026-10-16/README.md) | Constraint-trigger API surface. |
| [Browser credentials, 2026-10-17](browser-credentials-api-diff-2026-10-17/README.md) | Browser credential API surface. |
| [Owned outbound connections, 2026-11-04](outbound-owned-connection-api-diff-2026-11-04.md) | Transport ownership comparison. |
| [Startup handoff, 2026-11-05](startup-handoff-2026-11-05.md) | Consuming handoff API and verification. |
| [Managed release fence, 2026-11-05](managed-release-fence-2026-11-05.md) | Release-ordering evidence and [API surface](managed-release-fence-api-2026-11-05.txt). |
| [Owned profiled pool, 2026-11-05](owned-profiled-pool-2026-11-05.md) | Pool ownership evidence and [API surface](owned-profiled-pool-api-2026-11-05.txt). |
| [Intent reconciliation, 2026-11-05](intent-reconciliation-2026-11-05.md) | Reconciliation evidence and [API surface](intent-reconciliation-api-2026-11-05.txt). |

## Retention

Keep evidence that supports a current contract, public migration or unresolved
assessment. Superseded prototypes, old frozen consumer packages, repair diaries
and the publication dossier have been removed; recover historical snapshots from
Git when needed. Record new execution results in the owning Bead, and upstream
semantics in [primary references](../references.md).
