# Unexecuted import records

Imported SQL that a later forward change superseded. Each file is kept
byte-identical, and its digest is in [`../upstream-assets.json`](../upstream-assets.json),
so the import stays auditable. No code path executes these statements.

| Record | Executed successor | Why it was replaced |
| --- | --- | --- |
| `runlimit-postgres/cleanup_expired.sql` | `runlimit-postgres/src/cleanup_expired_by_key.sql` | It selected victims by `ctid`, which needs relation-level `SELECT`. |
| `runlimit-postgres/attempt_complete.sql` | `runlimit-postgres/src/attempt_complete_scheduled.sql` | It restated the backoff formula in SQL instead of reading `AttemptPolicy::delay_schedule`. |

This directory is outside every package, so the records are not published. Do
not copy a record back into a package or embed one with `include_str!`; a new
statement is a new file beside the code that executes it.
`scripts/check_runlimit_workspace.py` requires each superseded record, by
digest, to be inventoried only here under a canonical path, every other file
here except this README and OS metadata to be inventoried, and no symlink here
or at this directory. It then packages and verifies every native crate, with
default and with all features, so a library that embeds a record fails to build
from its own archive, and rejects an archive member, under any name, that
contains a record's complete bytes, alone or within a larger file. Packaged test
targets are not built from the archive (`batter-88bd`).