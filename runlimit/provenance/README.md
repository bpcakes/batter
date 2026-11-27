# Unexecuted import records

Imported SQL that a later forward change superseded. Each file is kept
byte-identical, and its digest is in [`../upstream-assets.json`](../upstream-assets.json),
so the import stays auditable. No code path executes these statements.

| Record | Executed successor | Why it was replaced |
| --- | --- | --- |
| `runlimit-postgres/cleanup_expired.sql` | `runlimit-postgres/src/cleanup_expired_by_key.sql` | It selected victims by `ctid`, which needs relation-level `SELECT`. |
| `runlimit-postgres/attempt_complete.sql` | `runlimit-postgres/src/attempt_complete_scheduled.sql` | It restated the backoff formula in SQL instead of reading `AttemptPolicy::delay_schedule`. |

This directory is outside every package, so the records are not published.
Do not copy a record back into a package, embed one with `include_str!` or
`include_bytes!`, or add a file here without a digest.
`scripts/check_runlimit_workspace.py` rejects an uninventoried file here and,
anywhere else in the tree, a `.sql` file with a record's bytes, a symlink into
this directory, a Rust source that contains a record's path, even in a
comment, or whose literal include path resolves to a record. It also rejects
a record's content in any native package's `cargo package --list` files.
Refer to a record by its file name. A new statement is a new file beside the
code that executes it.
