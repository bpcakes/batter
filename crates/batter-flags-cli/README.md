# batter-flags-cli

The `batter-flags` operator command for the flags control plane. The package
is not published; build it from the repository:

```sh
cargo run -p batter-flags-cli -- --help
```

Experimental. The command reads `BATTER_FLAGS_DATABASE_URL`, captured once,
and refuses to run while any ambient `PG*` variable is set. The URL is parsed
natively and built through setters without a password file: a host off the
loopback interface requires `sslmode=verify-full` (the default, optionally
with an absolute `sslrootcert`), while `disable` and `require` are accepted
only on loopback. The tables live in the fixed `flags` schema. No diagnostic
renders the URL, the connection options or a database message; failures name
a SQLSTATE at most.

| Command | Does |
| --- | --- |
| `list [--expired \| --orphaned \| --declared-only]` | every stored or declared key, with its `revision` |
| `show <key>` | one key; an absent key is revision 0 |
| `set <key> <variant>` | sets a release or mode flag |
| `kill <switch> engage\|release` | engages or releases a kill switch |
| `archive <key>`, `restore <key>` | archives or restores a key |
| `history <key>` | the key's audited history |
| `fleet [--version N \| --flag KEY]` | the fleet, a rollout check, or a key's declarers |

Writes require `--actor`, `--reason` and exactly one of `--if-revision N` or
`--yes`; every argument is validated before the command connects. `set` and
`kill` write only keys a retained catalog declares. `archive` and `restore`
also act on a key no retained catalog declares any more, because archiving a
key whose declarations were all pruned is the cleanup archive exists for. A
flip a retained catalog marks coordinated is refused, inside the write's own
transaction, unless `--accept-rolling-flip` is given, because processes apply
it poll by poll. Releasing a kill switch warns with the number of Live
processes whose settings floor keeps it engaged.

There is one end-to-end `--timeout` (1 to 60 s) and no operation is ever
retried. The pool holds at most one connection at a time, but each store
operation retires its connection when it ends, so a command that reads before
it writes opens one connection per operation, and connection establishment
follows SQLx's own retry within the timeout. Every command first refuses a
standby and a schema older than it requires. SIGINT or SIGTERM before COMMIT
is sent, including during the continuity check just before it, rolls the
write back; once COMMIT is sent the signal waits and the real outcome is
reported.

Exit codes: 0 ok, 1 failed with nothing written (a database rejection of
COMMIT included), 2 usage, 3 precondition not met (a revision mismatch, every
store refusal, and a coordinated flip), 4 commit outcome unknown (re-run with
`--if-revision` after `batter-flags history <key>`), 101 panic. Under
`--output json` every exit writes exactly one JSON document, except the
argument parser's own exits (a usage error, `--help` and `--version`) and a
panic. The document has `command`, `exit`, `outcome`,
`message`, and optionally `data` and `warnings`; renaming any of them is a
breaking change.

The ignored live suite runs against a disposable PostgreSQL 18 server:

```sh
BATTER_FLAGS_POSTGRES_TEST_DATABASE_URL=postgres://... \
  cargo test -p batter-flags-cli --test live -- --include-ignored --test-threads=1
```
