## Progress
- Fix manual CI comparison authority, cache the selected Jig runtime, and use normal merging for Markdown plans.
- Validate GitHub-style ref layouts, cache configuration, merge behavior, and existing repository checks.

## Surprises & Discoveries
The review reproduced missing local master on manual workflow dispatch and contradictory union merges of edited plans. CI currently rebuilds Jig on every runner.

## Decision Log
Preserve exact pull-request/push/merge-group comparison authority. Cache the repository-local runtime using source revision, contract/profile, platform, and toolchain identity. Preserve union merging for append-only JSONL records.

## Outcomes & Retrospective
Pending verification. Update docs and stage fixes; do not commit or publish.

## Progress update
Implemented all three review fixes: manual CI uses origin/master, a pinned actions/cache step restores only runtime cache directories with complete identity inputs, and a project-owned text merge override protects editable plans after template refreshes. JSONL union merging is preserved.

## Validation update
All seven committed standard-library Python regression tests pass against the selected runtime. They enforce actual event budgets, unavailable-base rejection, zero push-before behavior, cold/warm cache behavior with Cargo blocked, Git conflict/union semantics, and source ZIP exclusions. Workflow syntax/cache configuration and static package checks pass. One Jig Rust check correctly blocked when validation documentation changed during its planning window; rerunning after staging the settled files.

## Outcomes & Retrospective update
Hosted cache service behavior remains unverified. No project commit or publication performed.