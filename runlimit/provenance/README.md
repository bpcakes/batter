# Unexecuted import records

Imported SQL that a later forward change superseded. Each file is kept
byte-identical, and its digest is in [`../upstream-assets.json`](../upstream-assets.json),
so the import stays auditable. No code path executes these statements.

| Record | Executed successor | Why it was replaced |
| --- | --- | --- |
| `runlimit-postgres/cleanup_expired.sql` | `runlimit-postgres/src/cleanup_expired_by_key.sql` | It selected victims by `ctid`, which needs relation-level `SELECT`. |
| `runlimit-postgres/attempt_complete.sql` | `runlimit-postgres/src/attempt_complete_scheduled.sql` | It restated the backoff formula in SQL instead of reading `AttemptPolicy::delay_schedule`. |

This directory is outside every package, so the records are not published.
Do not copy their statements, embed them with `include_str!`, or add new
files here without a digest: `scripts/check_runlimit_workspace.py` rejects
both. A new statement is a new file beside the code that executes it.
