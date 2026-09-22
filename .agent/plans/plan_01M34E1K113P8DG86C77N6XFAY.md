Owning Bead: batter-jtnk. Reproduce the failing CI assertion with a controlled scheduler delay. Use persisted logical database time to assert exact replenishment boundaries; retain a separate server-clock sampling test. Run default/all-feature PostgreSQL tests on both supported Rust toolchains and independent negative controls, then freeze files for Jig validation. Push the repair to PR #9 and inspect hosted verification.

The original test fails its denial assertion after an injected 220 ms pause;
the repaired test passes with that pause before every decision. Period-bucket
and zero-server-clock production mutations fail the respective assertions; both
production files were restored. Default and all-feature native PostgreSQL suites
each pass 71 live cases on Rust 1.94.0 and 1.98.1, using PostgreSQL 16.15 in an
owned disposable container. The new clock module is covered by the existing
exhaustive `runlimit/*/tests/**` Jig scopes. Extracting the clock test puts the
parent below 800 lines, so its 810-line import exception was removed. The native
file-budget gate passes against the original plan baseline.

Full frozen-tree Jig validation passed on Rust 1.98.1 in
`run_01M34EJHG52QDFY1EK2S3DGWYT`, including `api:test` receipt
`receipt_01M34FCSXH9T2HRESEEC46SC4E`. Evidence and gates are fresh and passing.
The final tracker update requires only a policy refresh; Rust inputs, toolchain,
environment and prerequisites remain unchanged. Hosted validation of the repair
commit will follow in PR #9 and is not yet claimed as passing.
