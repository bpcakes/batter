# Backlog location

Beadroll (`bead`) is the sole source of truth for delivery scope, priority,
status, acceptance criteria and dependencies. The former Markdown roadmap's
completed milestones and explicitly deferred outcomes are retained in the
tracker. This page is navigation only; do not add task lists here.

From the repository root:

```sh
bead prime
bead ready --type task --json
bead list --all --deferred --label roadmap --limit 0 --json
bead show <id> --json
```

Inspect an issue with `bead show` before claiming it with `bead claim <id>`.
Follow the current workflow from `bead prime`, record progress and verification
with `bead comments add`, and run `bead sync` before stopping. Close completed
work with `bead close <id> -r "what was done"`; release unfinished work with
`bead release <id>`.

One tracker is shared across worktrees, clones and machines. State lives outside
the checkout and syncs through a Git ref, not a code branch. Claims belong to
agent threads and become exclusive once accepted by the remote; `bead prime`
explains resuming work in a new session and handling conflicts.

The tracked [legacy Beads export](../.beads/issues.jsonl) and other `.beads/`
files are historical data. Do not edit them or run `br` or `bd` in this repository.
Use `bead export` when another tool needs a JSONL snapshot, and use `bead` for
current state and ownership. The legacy export is not an input to local
verification or CI.

Legacy roadmap identifiers are retained in tracker external references and
descriptions. Implemented capability facts remain in [status](status.md),
behavioral contracts in [guarantees](guarantees.md) and
[integrations](integrations.md). These documents are not parallel backlogs.
Archived plans are historical records, not open work; new work does not require
an ExecPlan.

Closing technical tasks does not authorize publishing to a crate registry.
