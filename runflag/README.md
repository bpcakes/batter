# runflag

Native typed flags and kill switches. `runflag-core` owns the flag key and
catalog grammar, validated snapshots, evaluation and pinned decisions;
`runflag-postgres` owns the audited PostgreSQL control-plane store; and
`runflag-cli` builds the `runflag` operator command. No package here depends on
a Batter package. The Batter lifecycle integration, declared mode and the
protected registration path live in `batter-flags` under `crates/`.

This family is authored in this repository on its own 0.1.0 version line and
takes the ownership tier of the imported native families: grammar, evaluation
and storage that work without Batter. The packages are scaffolds: manifests,
lints and feature shapes are settled, and the implementation arrives with the
delivery tasks of the flags epic.
