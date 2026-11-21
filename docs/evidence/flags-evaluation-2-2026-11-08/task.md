# Task given to the agent

Launched 2026-11-08 as a fresh general-purpose agent with no other context. Verbatim:

> You are building a small Rust service on top of the Batter library, as an application developer would. Work only in this directory: `<scratch>/eval2-consumer`. It is a cargo project named `checkout-service` that already depends on Batter (one dependency, `batter`, a path dependency with the features `flags-postgres`, `flags-axum` and `flags-runledger`) and has a stub `src/main.rs`.
>
> Batter lives at /Users/aa/Documents/batter. You may read anything there to learn how to use it: README.md, docs/usage.md, docs/integrations.md, docs/operations.md, the crates' rustdoc comments and READMEs, and the runnable examples in crates/batter/examples (examples are the library's consumer contract). Do NOT modify anything under /Users/aa/Documents/batter, and do NOT open anything under /Users/aa/Documents/batter/docs/evidence/ (that holds an evaluation of this exercise). Do not use git in the Batter repository.
>
> What the service must do:
>
> 1. Serve an HTTP route group named `checkout` (for example `POST /checkout` and `GET /checkout/status`) through Batter's HTTP boundary.
> 2. Run a background job type `receipt.send` with Batter's Runledger integration, enqueued when a checkout is accepted.
> 3. Use Batter's typed flags in control-plane mode: values come from an audited PostgreSQL flags store, with the process's settings supplying the startup values. The database URL and other settings come from the process's configuration at the application root.
> 4. Add a kill switch `checkout_halt`. While it is engaged, the checkout route group must be unavailable and receipt jobs must not run; queued receipts must be retried later, not lost or dropped.
> 5. Add a release flag `express_receipt` (on or off). The value it has when a checkout is accepted must be the one the receipt job acts on, even if the flag changes before the job runs. Store whatever is needed with the order in the application's own table.
> 6. Write an operator procedure as a script `ops/engage-halt.sh` that engages `checkout_halt` with Batter's operator command. It must handle every outcome the command can report, including the case where the command cannot tell whether its write committed, without risking a second unintended write.
> 7. Add at least one test that runs without a database.
>
> Constraints: keep `batter` as the only Batter dependency (you may add ordinary ecosystem crates such as tokio, serde, serde_json or axum, but they must already be resolvable offline from the existing Cargo.lock). The project must build with `SQLX_OFFLINE=true cargo check --offline --all-targets` and its offline tests must pass with `SQLX_OFFLINE=true cargo test --offline`. You do not need a running database. Do not change the process environment in code or tests. Follow the library's documented canonical path rather than low-level escape hatches.
>
> When you are done, reply with: what you built (file by file), the commands you ran and their results, and every point of friction you hit with Batter's API or documentation (unclear docs, things you had to guess, errors that misled you), each with the file you were reading. Be specific and honest about anything you could not finish.

The starting project was `Cargo.toml` with that one dependency, a copy of the workspace `Cargo.lock`, and a `src/main.rs` that installed `batter::redacting_panic_hook()` and did nothing else.
