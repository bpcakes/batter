# Sanitized evaluator results

Evaluator: root Codex session for `batter-lp2.4` closure.
Fresh author identity: `/root/fresh_consumer_author`, Codex based on GPT-5.
Repository input: Git `1cccca704d95811f5e0562e89ef593603e964eda` plus the
evaluator's narrow ordinary-start ordering repair.
Fixture: explicitly authorized disposable PostgreSQL 18.4 primary endpoint.
Credentials, endpoint coordinates, database data, and build products are omitted.

Frozen packet SHA-256:

- `packet.md`: `6da3c47e7daf1ef2f7765a120a8bc301a0956a107b8279d4512a999adfc0b058`
- `oracle/check.py`: `6abb0b36b5dde38641e87bc84a54ceb1b32a975f906e3c7f4e0d06a1d1a45b79`
- `modification.md`: `545e4364d18497c16bfb2212a9f2d4b45d730f5e3b59106f0fa982004c871e0a`

Initial source was submitted before any author-side compilation or execution.
The evaluator's first `--locked` invocation stopped because no lockfile existed;
this harness setup event did not compile author code and consumed no repair.
Cargo then generated the temporary package lock. The first actual compilation
failed with two `E0282` inference errors on the inner results returned through
`OperationContext::run`. Those diagnostics were retained and sent to the same
author. Repair 1 added only explicit `Result<_, SqlxFailure>` typing. It compiled
with one `unused_mut` warning and passed the independent initial oracle:

- real startup `SELECT 42` and fresh HTTP `/query` response;
- SIGTERM exit zero with complete `database.close` success record;
- later absent-relation native failure, nonzero exit, retained pool cleanup, and
  exact fixed `consumer failed` diagnostic.

The modification's first submission added a second owned dependency, registered
its asynchronous stop/join finalizer, added a fallible post-acquisition startup
stage, and traversed actual retained cleanup records. It compiled with the same
`unused_mut` warning and passed the independent modified oracle on its first
attempt:

- normal SIGTERM cleanup order `secondary.close`, then `database.close`;
- later native failure with the same complete LIFO cleanup and fixed diagnostic;
- injected startup failure before listener publication with the same complete
  LIFO cleanup and fixed diagnostic.

The author added no second signal installation or routing path. Separate target
directories bound each checked source variant to its executable. Binary SHA-256:

- initial repair 1: `281a1be7a7c21e647d701a4603e68b5ab49859348bbcc26073a113ceeb216ff1`
- modified submission: `7e2feb238b315ad3d5684c6b332037a5b444683afcfa5d118da3480488bde4ac`

The independent reviewer then found that initial repair 1 printed its cleanup
line from aggregate success without proving the named `database.close` record
was present. Initial repair 2, the final permitted initial repair, added that
exact record-presence check while retaining aggregate success. It compiled with
the same `unused_mut` warning and passed the complete initial oracle. Its binary
SHA-256 is
`50e286c6ba2b6d382ff3cf1436c64dcc0d55f92e1f7698f0472927398eb5441f`.

No reliability percentage is claimed. The initial first-attempt failure remains
part of the result. The reviewer must use only the frozen packet, submitted
source/manifests, snapshot hashes, and this sanitized result; it must not inspect
the oracle, repository plans/tests/examples/evidence/private modules, Git history,
or another agent transcript.
