Owning Bead: `batter-1a5h`.

Scope: .github/workflows/ci.yml and supporting evidence documentation. Preserve all seven Rust jobs and semantic checks. Remove duplicate branch push runs, cancel superseded work, add pinned dependency caching and bounded timeouts. Validate Actions syntax, unchanged Rust command inventory, and repository policy regressions. No Rust source or dependency graph changes; hosted performance measurement follows publication.

Implemented 2026-09-22. Actionlint 1.7.12, the original matrix/command comparison,
all 11 existing Jig integration tests, and agent map/guide checks pass. The full
Jig profile on Rust 1.98.1 passes in `run_01M34830VSXGJQJ7RAP9NH04XQ`, including
`api:test` receipt `receipt_01M348XVMCWKK84G2ENBJA80X7`; evidence and gates report
fresh passes. An earlier run's commands passed but its receipts were rejected
because documentation/tracker edits occurred during execution; the successful
rerun held the tree fixed. Cache restore's metadata command leaves an outdated
lockfile unchanged on Rust 1.94.0 and 1.98.1. Hosted cache performance is unmeasured.
