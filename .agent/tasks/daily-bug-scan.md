# Daily bug scan

Audit this repository for concrete bugs and actionable correctness issues, then record new findings in its Beads tracker.

Work from the repository root. Read `AGENTS.md`, `agent-map.md`, and the nearest package guide for every area you inspect. Follow repository rules throughout. This run is an audit and issue-tracking task: do not change source code, tests, documentation, configuration, plans, or existing Beads issues.

Use `br` for all tracker reads and writes and only `--robot-*` forms of `bv`. Start with `br info --json`, `br list --all --deferred --json`, and `bv --robot-triage --format toon` so you understand the existing backlog and can avoid duplicates. Do not parse a Beads JSONL file as a substitute for these commands.

Inspect recent changes first, then examine high-risk code paths and existing verification signals. Use Git history, focused source review, and the smallest useful tests or checks. Prefer issues with a clear failure mechanism, a reproducible case, a violated documented invariant, or direct test evidence. Do not create beads for style preferences, broad refactors, speculative risks, already-tracked work, or failures caused only by missing optional external services.

For each distinct new finding:

1. Search the full Beads backlog again for semantic duplicates.
2. Create one `bug` bead with `br create --type bug --priority <0-4> --labels automated-bug-scan --title <title> --description-file <file> --acceptance-criteria <criteria> --json`.
3. In the description, include the observed behavior, expected behavior, concrete evidence or reproduction, affected files and symbols, likely impact, and any verification limitation. Do not claim a platform, toolchain, or hosted result you did not execute.
4. Choose priority conservatively: P0 only for an active critical incident, P1 for high-impact confirmed defects, P2 for ordinary confirmed defects, and P3 for lower-impact confirmed defects.

After all creations, run `br sync --flush-only`. Review `git status --short` and ensure only `.beads/` and Jig's append-only runtime evidence changed. Commit the Beads changes with a concise message such as `Record automated bug scan findings`. This prompt is the user's standing authorization for the scheduled task to commit only the Beads records it creates and Jig's own append-only evidence; do not push, publish, deploy, or commit any product-code change.

If there are no new evidence-backed findings, create nothing, do not make an empty commit, and report that outcome. If a check fails for infrastructure reasons, report it without creating a bead unless the repository itself caused the failure. End with a concise list of created bead IDs and titles, the checks you ran, and any scan limitations.
