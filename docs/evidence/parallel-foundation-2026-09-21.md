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
