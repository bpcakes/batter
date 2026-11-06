# Documentation

Batter is a native Rust/Tokio operational foundation with optional adapters.
Start with the [repository README](../README.md) for package selection and runnable
examples, then use the guides below for the boundary you are changing.

## Using Batter

| Guide | Purpose |
| --- | --- |
| [Usage](usage.md) | Canonical service and command composition, operations, retry and cleanup. |
| [Guarantees](guarantees.md) | Implemented contracts, preconditions and explicit limits. |
| [Operations](operations.md) | Startup, shutdown, settings and diagnostics. |
| [Integrations](integrations.md) | Adapter ownership, feature selection and application responsibilities. |
| [Reference compatibility](reference-compatibility.md) | Pinned dependency composition, single-dependency consumers and scoped live evidence. |
| [Security policy](../SECURITY.md) | Reporting vulnerabilities and security boundaries. |

Crate READMEs and rustdoc own package-level API detail; the
[package table](../README.md#packages) links to them. Prefer the protected
integration path in [usage](usage.md#canonical-agent-consumer-path).

## Maintaining Batter

| Guide | Purpose |
| --- | --- |
| [Repository instructions](../AGENTS.md) and [agent map](../agent-map.md) | Reading order, ownership and change rules; read the nearest package guide before editing. |
| [Architecture](architecture.md) and [ADRs](adr/README.md) | Dependency direction and accepted design decisions. |
| [Implementation status](status.md) | Source capabilities and the limits of recorded verification. |
| [Testing](testing.md#verification) | Current local and CI verification commands and failure-contract coverage. |
| [Delivery tracker](roadmap.md) | Beadroll access; delivery scope, priority and dependencies live in the tracker. |
| [Primary references](references.md) | Dated upstream source checks, grouped by topic. |

## Rationale and evidence

The [Effect v4 brief](effect-v4-brief.md) and
[backend design evidence](design-evidence.md) explain design choices.
The [evidence index](evidence/README.md) links to current API assessments,
surface diffs and implementation evidence.

Current contracts live in the guides and rustdoc. Dated evidence describes its
recorded revision and environment; it does not establish a successful run against
the current checkout. Record new verification in the owning Bead.

Keep this page as navigation. Put API details in the owning guide, dated source
checks under [references](references.md), and historical artifacts under
[evidence](evidence/README.md). Preserve existing section anchors when moving
current content, and keep delivery task lists in Beadroll. Delete superseded
material whose contracts and evidence are already owned elsewhere; Git retains
the history.
