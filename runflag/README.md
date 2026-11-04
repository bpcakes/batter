# runflag

Native typed flags and kill switches. `runflag-core` owns the flag key and
catalog grammar, validated snapshots, evaluation and pinned decisions;
`runflag-postgres` owns the audited PostgreSQL control-plane store; and
`runflag-cli` builds the `runflag` operator command. No package here depends on
a Batter package. The Batter lifecycle integration, declared mode and the
protected registration path live in `batter-flags` under `crates/`.

This family is authored in this repository on its own 0.1.0 version line and
takes the ownership tier of the imported native families: grammar, evaluation
and storage that work without Batter. `runflag-core` implements the catalog:
typed declarations of release flags, mode flags and kill switches, branded
handles, the catalog and declaration digests and, with its `serde` feature, the
manifest and the manifest diff. Its snapshots and evaluation, the PostgreSQL
store and the operator command are still scaffolds; their implementation
arrives with later delivery tasks of the flags epic.
