# Parallel foundation integration

Owning Bead: `batter-979`.

The integration preserves master's session-profile contracts and PR #3's
`run_atomic_in`/attempt composition. The initial merge is a foundation checkpoint;
whole-workspace acceptance requires the following coordinated Runledger pin.

The core and SQLx packages expose their actual source directories through Cargo
dependency metadata. No native library is linked. The `links` identities also
prevent two foundation implementations in the resolved graph. SQLx forwards its
actual core source so the native consumer can check both compiled packages,
instead of trusting a separately selected sibling checkout. This adds no runtime
API or foundation dependency on an adapter.

Runledger owns validation against its reviewed foundation revision. Adapter-only
pin changes must not alter that foundation identity; inherited foundation
manifest settings still require validation. This is an unpublished source-graph
contract, not registry publication evidence or a tamper-resistant build sandbox.

Initial checkpoint checks: `cargo test -p batter-sqlx --lib --locked` passed
141 tests; `cargo fmt --all -- --check` passed. Full integration validation is
pending the native companion update.

The coordinated native pin is Runledger `41bca4c`, retaining `b83abe4`'s profile
restoration fix and validating actual Cargo foundation sources. Runledger PR #20
merged during integration; the follow-up is PR #21. Runlimit now selects merged
PR #9 (`12e035d`) consistently in the adapter and facade examples.

On macOS/Rust 1.98.1 with PostgreSQL 18.6, all-target/all-feature workspace
compilation, ten live attempt tests, and all 25 owned atomic-scope tests passed.
The atomic suite includes session-profile restoration and redaction. Full
two-toolchain verification, HTTP smokes, Jig and final review remain separate
acceptance checks.

## Escalation: explicit profile ownership for attempts

Native Codex review of `ac1275307b448244c4350f7174227e5b584031cb` through
`1c5c558883f413566fedbc0bd1eff3fa156fe642` found a P1 integration defect:
`AttemptRunner` admits using its pool's policy, then `run_atomic_in` invokes
unprofiled `run_atomic`, whose reset discards that policy. The review's independent
PostgreSQL 18 reproduction observed `restricted` before the runner and `postgres`
inside its final application callback. Source inspection confirms the missing
profile field and unprofiled completion path.

This is an authority-contract gap, not another pin adjustment. A canonical
profile-bearing attempt owner must enforce the same explicit policy across
admission and completion, preserving retained atomic outcomes. The API/ownership
change is escalated for direction rather than implemented speculatively. PR #3
is draft; downstream adoption is on hold. No DealSafe changes were made.

Completed checks before escalation: Rust 1.98.1 full verification and five HTTP
smokes, the live tests above, and a fresh GitHub-only facade/Runledger/Runlimit
consumer. The Rust 1.94.0 full run and final Jig collection were interrupted when
the escalation stopped implementation; neither has a final passing receipt.
An earlier Jig run was invalidated by integration-document edits during its
read-only collection. Runledger PR #21's final native review is clean and all
five required hosted checks passed at `d574cbf`.
