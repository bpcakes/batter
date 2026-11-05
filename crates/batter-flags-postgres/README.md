# batter-flags-postgres

Audited PostgreSQL control-plane storage for `batter-flags`: the schema, its
forward-only migrations, compare-and-set writes, history, catalog publication,
process presence and the fleet query. Evaluation stays in `batter-flags-core`;
loading, refreshing and readiness stay in `batter-flags`, which re-exports this
crate as `batter_flags::postgres` under its `postgres` feature.

Experimental scaffold: the manifest is settled and the store arrives with the
control-plane delivery tasks of the flags epic.
