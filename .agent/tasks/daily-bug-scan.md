# Daily bug scan

Audit this repository for concrete bugs and actionable correctness issues, then record new findings in its beadroll tracker.

Work from the repository root. Read `AGENTS.md`, `agent-map.md`, and the nearest package guide for every area you inspect. Follow repository rules throughout. This run is an audit and issue-tracking task: do not change source code, tests, documentation, configuration, plans, or existing issues.

Use `bead` for all tracker reads and writes. Start with `bead prime`, then inspect `bead list --all --deferred --limit 0 --json` and `bead claims --json` so you understand the full backlog and current ownership. Use `bead search --all <query> --json` for focused duplicate checks. The `.beads/` directory is historical data; do not use its export as current tracker state.

Inspect recent changes first, then examine high-risk code paths and existing verification signals. Use Git history, focused source review, and the smallest useful tests or checks. Prefer issues with a clear failure mechanism, a reproducible case, a violated documented invariant, or direct test evidence. Do not create beads for style preferences, broad refactors, speculative risks, already-tracked work, or failures caused only by missing optional external services.

For each distinct new finding:

1. Search the full beadroll backlog again for semantic duplicates, including closed and deferred issues.
2. Create one `bug` bead with `bead create --type bug --priority <0-4> --labels automated-bug-scan --title <title> --description-file <file> --acceptance-criteria <criteria> --json`. Keep temporary description files outside the checkout.
3. In the description, include the observed behavior, expected behavior, concrete evidence or reproduction, affected files and symbols, likely impact, and any verification limitation. Do not claim a platform, toolchain, or hosted result you did not execute.
4. Choose priority conservatively: P0 only for an active critical incident, P1 for high-impact confirmed defects, P2 for ordinary confirmed defects, and P3 for lower-impact confirmed defects.

After all creations, run `bead sync`. Tracker writes are published through beadroll's dedicated Git ref and require no code-branch commit. Compare `git status --short` with the starting state and leave checkout files unchanged. This task authorizes creating and syncing bug reports only; do not commit, push code, publish or deploy.

If there are no new evidence-backed findings, create nothing and report that outcome. If a check fails for infrastructure reasons, report it without creating a bead unless the repository itself caused the failure. End with a concise list of created bead IDs and titles, the checks you ran, and any scan limitations.
