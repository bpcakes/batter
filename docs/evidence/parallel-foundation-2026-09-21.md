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

## Authorized profile correction

The user authorized the profile/API correction after Runledger #21 merged.
The execution plan is `.agent/plans/attempt-profile-ownership.md`.
`PgProfiledPool` now owns the previously native connect/acquire/release hooks in
the SQLx foundation. Runledger delegates without changing its public database
API (follow-up PR #22). `AttemptRunner` requires that owner and one authoritative
schema; its retained completion uses the same owner through
`run_atomic_profiled_in`. An arbitrary pool no longer compiles as its constructor
argument. Native Runlimit policy, storage, budgets and migrations are unchanged.

Invalid-state review: no separate admission/completion profile setter exists;
native admission uses the owned pool, and profiled completion reapplies and
revalidates its exact declaration. Custom schema fallback is rejected at
construction. Native access remains a trusted SQL escape hatch, not endpoint
attestation or a privilege sandbox. The independent boundary investigator and
candidate bypass reviewer found no additional concrete issue in this design.

Focused execution on PostgreSQL 18.6: 142 SQLx unit tests, profile construction
tests and SQLx doctests passed; the shared pool live regression passed. All 14
live attempt tests and adapter tests/Clippy passed. New cases observe native
admission through an invoker trigger and compare restricted role, quoted custom
schema and RLS setting with the final decision. Forbidden table access remains
denied; missing authoritative tables cannot fall back to public; wrong-login
configuration invokes neither application factory; profile drift cannot publish
a committed result. Existing rejection/success audit and retry-state cases pass.
Temporarily restoring unprofiled completion caused the custom-schema regression
to fail with `AttemptError::Atomic`; the mutation was removed before acceptance.

Both full Rust 1.98.1 and 1.94.0 verification matrices passed, including Clippy,
rustdoc and all ten rebuilt HTTP smoke profiles. All 26 live atomic tests passed.
A fresh GitHub-only consumer compiled the profile-owned attempt runner and shared
Runledger profile identity with no sibling checkout and a deliberately invalid
`RUNLEDGER_BATTER_SOURCE`. Final Jig gate passed; `api:test` receipt is
`receipt_01M32CAHRTKVB1HCXT3KZ3RVXD` under plan
`plan_01M326CSKANTPZKQ2E292FDP0M`.

Native review of the original stable range `ac127530..f2a883c` reported no findings.
Runledger's final native review `a8f6283..a5048e4` also reported no findings; its
374 PostgreSQL and 497 runtime tests/doctests and nine packaged-consumer tests
passed. The old P1 is fixed by the ownership boundary, not a caller workaround.
Hosted results remain attached to PR #3 and companion PR #22. The existing native
tracker normalization conflict is documented in that companion; unrelated records
were not rewritten. Downstream adoption remains separate; no application limiter
code or consumer repository changes are included.
