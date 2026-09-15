# Frozen consumer packet

Evaluation root: `/Users/aa/.cache/batter-lp2.4-eval.iL867u`.
Repository root: `/Users/aa/Documents/batter` at
`1cccca704d95811f5e0562e89ef593603e964eda` plus the evaluator's narrow
ordinary-start ordering repair.

You may read only these public consumer inputs before the first submission:

- `/Users/aa/Documents/batter/README.md`
- `/Users/aa/Documents/batter/docs/usage.md`
- `/Users/aa/Documents/batter/docs/integrations.md`
- `/Users/aa/Documents/batter/docs/guarantees.md`
- `/Users/aa/Documents/batter/crates/batter/Cargo.toml`
- `/Users/aa/Documents/batter/crates/batter/src/lib.rs`
- `/Users/aa/Documents/batter/crates/batter/src/startup.rs`
- `/Users/aa/Documents/batter/crates/batter/src/lifecycle.rs`
- `/Users/aa/Documents/batter/crates/batter/src/registration.rs`
- `/Users/aa/Documents/batter/crates/batter-axum/Cargo.toml`
- `/Users/aa/Documents/batter/crates/batter-axum/src/lib.rs`
- `/Users/aa/Documents/batter/crates/batter-axum/src/serving.rs`
- `/Users/aa/Documents/batter/crates/batter-sqlx/Cargo.toml`
- `/Users/aa/Documents/batter/crates/batter-sqlx/src/lib.rs`

Do not read repository tests, examples, plans, evidence, private module files,
Git history, or another agent's transcript. Do not run a compiler, test, binary,
formatter, linter, metadata command, or network command before submitting the
first source. The evaluator owns compilation and the independent oracle.

Create an independent Rust package in `submission/initial` using public Batter,
Batter Axum, and Batter SQLx paths. Do not change the repository or `oracle/`.
The package must build one Unix service which:

1. Acquires a native PostgreSQL pool during protected startup, registers its
   owned close before returning it, and performs a real bounded `SELECT 42`.
2. Binds a loopback HTTP listener during startup. Once running, print exactly one
   `consumer-listening:http://HOST:PORT` line. `GET /query` must execute a fresh
   native query and return status 200 with body `42`.
3. Owns SIGTERM/SIGINT through the public startup path. A TERM after `/query`
   must produce a successful complete shutdown, exit zero, print exactly one
   `consumer-cleanup:database.close:succeeded`, then
   `consumer-result:success`, and never print native error contents.
4. `GET /trigger-failure` must return 202 with body `accepted`, then cause
   process-owned application work to execute a native query against a definitely
   absent relation. The later failure must drain the service, close the pool,
   exit nonzero, print the complete named cleanup record followed by
   `consumer-result:failure`, and write exactly `consumer failed` to stderr.
5. Diagnostics are fixed. Credentials, URLs, SQLx errors, and retained native
   error text must never be formatted to stdout or stderr.

The evaluator will supply `DATABASE_URL`, `RUST_LOG=off`, and a disposable
PostgreSQL 18 endpoint. Choose public APIs from the supplied docs; the packet
deliberately does not prescribe ownership function names.

After the initial variant is independently checked, a second prompt will request
one modification. At most two repair submissions are allowed for each variant.
