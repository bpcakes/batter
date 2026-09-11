Owning issue: batter-39e.

## Progress

- Migrated the launcher and resolved contract from epoch 7 to epoch 9, pinned to upstream commit 10a3dc9ae63547b09a48b05a463495bce2101f37.
- Declared exhaustive inputs for api:clippy, api:fmt, api:test and api:test-locked. Preserved all five required verify targets as independent siblings.
- Preserved existing application changes, customized workflows, agent guides and harness files outside this scoped migration.
- Added disposable Git repository controls for tracker/documentation reuse across Git states, relevant source/helper invalidation and compatible native file-budget refresh.
- Jig doctor, contract, agent-map and agent-guides checks passed. The full verify profile passed with original Rust receipt IDs below.

## Decisions and observations

Contract and file-budget retain whole-repository freshness and may refresh after tracker edits. Rust inputs cover manifests, lockfiles, toolchain/Cargo/lint configuration, package source/tests/examples/benches/migrations, shared test sources, and Python/shell test helpers. Root tracker data is not globally excluded. Exhaustive prefixes avoid ignored build and cache trees.

The default two-second inspection budget returned collection_limit; a read-only inspection with --freshness-timeout-ms 30000 passed in roughly 2.3 seconds. Agent guidance documents this recovery without rerunning checks.

## Original Rust evidence

- api:clippy: receipt_01M26HYEBK7NSSFCBHBZPV63B7
- api:fmt: receipt_01M26HYEQAKD434B519YHM3PFQ
- api:test: receipt_01M26HYF3T5JSP5A55Y77AZKYM

## Validation outcome

All 10 Jig integration tests passed in 348.319 seconds. The native-policy refresh control, tracker/documentation edits across dirty/staged/committed states, and source/fixture/migration/helper invalidation controls all passed. The full verification profile passed once. Actual tracker closeout then reused the exact original three Rust receipts and executed only contract and file-budget. No application source, test commands or workflow definitions were changed by this task.

## Concurrent staging during finalization

After the successful closeout/reuse proof, another process staged the checkout at approximately 21:05 UTC. This task did not stage or commit repository changes. Contract v9 distinguishes committed, index and worktree projections, so staging relevant source invalidated its original receipts despite unchanged current bytes. Preserved the staging and started a new final verification for the settled index. Tracker-only staging/commits remain covered by the passing reuse regression; source-staging reuse would require a separate Jig runtime change.

The final verification against the staged checkout passed all five targets, and `work finish` closed this plan successfully. No commit was created by this task.
