# Backlog location

Beads is the sole source of truth for delivery scope, priority, status,
acceptance criteria and dependencies. The former Markdown roadmap has been
migrated to the repository's Rust Beads tracker (`br`), including completed
milestones and explicitly deferred outcomes. This page is navigation only;
do not add task lists here.

From the repository root:

```sh
bv --robot-triage
br ready --type task --json
br list --all --deferred --label roadmap --json
br show <id> --json
```

Verify an issue with `br show` before claiming it. Use `br` to update tasks and
dependencies, then `br sync --flush-only` to refresh the tracked
[Beads export](../.beads/issues.jsonl). Do not hand-edit the export or use `bd`
against this workspace. Check `bv`'s reported `source_path` and counts against
`br`; use the tracker result when a viewer snapshot is stale.

Legacy roadmap identifiers are retained in Beads external references and
descriptions. Implemented capability facts remain in [status](status.md),
behavioral contracts in [guarantees](guarantees.md) and
[integrations](integrations.md), and executed evidence in
[validation](validation.md). These documents are not parallel backlogs.
Task-local ExecPlans hold implementation steps and evidence for their owning
bead; archived plans are historical records, not open work.

Closing technical tasks does not authorize publishing or deployment.
