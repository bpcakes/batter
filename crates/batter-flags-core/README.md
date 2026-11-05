# batter-flags-core

The typed flag and kill-switch catalog of `batter-flags`. An application
declares every release flag, mode flag and kill switch once, in code, with a
`CatalogBuilder`; each declaration mints a typed, catalog-branded handle, and
the built `Catalog` carries its digest and, with the `serde` feature, the
manifest deployment tooling reads.

This crate depends on no other Batter crate and reads no clock. The audited
PostgreSQL store is `batter-flags-postgres`, the operator command is
`batter-flags-cli`, and declared mode, the protected registration and the
lifecycle integration live in `batter-flags`, which re-exports this crate as
`batter_flags::core` (and the facade as `batter::flags::core`).

```sh
cargo test -p batter-flags-core --locked
cargo test -p batter-flags-core --all-features --locked
```

Experimental. The catalog is implemented; snapshots, evaluation and pinned
decisions arrive with later delivery tasks of the flags epic.
