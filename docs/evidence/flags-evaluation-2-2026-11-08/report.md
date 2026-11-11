# The agent's final report

Archived verbatim from the agent's reply on 2026-11-08. It is the agent's own
account; the maintainer's scoring is in [README.md](README.md).

---

I built the checkout service in `<scratch>/eval2-consumer`. It compiles and its offline tests pass, but nothing has run against PostgreSQL. That leaves unexercised: the migrations, `verify_schema`, flags registration and refresh, LISTEN, the worker running `receipt.send`, and the operator command writing to a real store.

## What I built, file by file

- **`Cargo.toml`**: `batter` is still the only Batter dependency, with the same three features. I added axum, chrono, serde, serde_json, sqlx (postgres, runtime-tokio, json, macros, migrate), tokio and tracing. `Cargo.lock` gained no packages; only the project's own dependency list changed.
- **`src/main.rs`**: installs the redacting panic hook as the first statement, reads the process environment once, and dispatches `serve` (the default) or `migrate`. Flags resolve from the full settings before the application narrows its own names.
- **`src/config.rs`**: `DATABASE_URL`, `CHECKOUT_MIGRATION_DATABASE_URL`, `CHECKOUT_FLAGS_LAST_GOOD_DIR` (required), and `CHECKOUT_BIND`, `CHECKOUT_REQUEST_TIMEOUT_MS`, `CHECKOUT_WORKER_ID` with defaults. It refuses any ambient `PG*` variable and redacts the URL in debug output. 3 tests.
- **`src/flags.rs`**: the catalog with the `FLAG_` settings prefix.
  - `checkout_halt` is a kill switch, released by default, holding its last known state when the flags data goes stale.
  - `express_receipt` is a release flag, off by default, expiring 2027-03-31. It is deliberately not governed by the halt.
  - 2 tests.
- **`src/serve.rs`**: one `Startup::protected` with Unix signals, in this order:
  - pool via `pool_in`, registered first so it closes last, then `RunledgerDatabase` and `verify_schema`;
  - flags `control_plane::register_in` using `start_on_defaults` (the settings supply the startup values), the last-good file and LISTEN;
  - the receipts job catalog, `sync_definitions`, the native worker and `runledger::register_in`;
  - the HTTP gate on `checkout_halt`, boundary assembly and `register_in`;
  - then `finish_with_output` followed by `wait_checked`.
- **`src/http.rs`** and **`src/http/tests.rs`**: `/live` and `/ready` probes, plus the gated `checkout` group with `POST /checkout` and `GET /checkout/status`.
  - `POST /checkout` validates the body, pins `express_receipt` from the request's view and maps every atomic outcome. An unknown commit returns 500 `outcome_unknown`, meaning "retry with the same order_ref".
  - 3 tests run the real boundary in-process with test flags and a database handle that never connects:
    - engaged halt: both group routes return 503 `disabled`, while `/live` still answers;
    - released halt: the status route serves, and a bad body gets 400;
    - unreachable database: 503, never reported as accepted.
- **`src/orders.rs`**: inserts the order with its pin and enqueues `receipt.send` in one transaction. A repeated `order_ref` returns `already_accepted`. It also loads and records receipts, with explicit pending/sent states.
- **`src/receipts.rs`**: the `receipt.send` job.
  - The gate uses `DirectWhenTripped::Defer` (30 s recheck), so while halted receipts are continued later without using up attempts; max attempts is 25.
  - The handler replays the stored pin. Only an honoured pin decides the receipt; an incompatible or overridden pin defers the job rather than reading the live flag.
  - Delivery is a placeholder run under the work phase; the "sent" record runs under the finalization phase.
  - 4 tests, including "pin taken while on still decides after the flag is off" and "an incompatible pin defers".
- **`src/migrate.rs`**: a `command::Command` that runs Runledger's migrations, then the app's (with `set_ignore_missing(true)`), then the flags store's.
- **`migrations/20261108090000_checkout_orders.sql`**: the orders table with a `jsonb` pin column. A CHECK makes "receipt pending" and "receipt sent" the only representable states.
- **`ops/engage-halt.sh`**: the operator procedure.
  - It reads the switch's revision N and writes `kill checkout_halt engage --if-revision N`. It never uses `--yes`.
  - Exit 0: written, unchanged or already engaged. It then re-checks the switch and waits on `fleet --version V` until every live process has applied it.
  - Exit 1 or 3: reports that nothing was written. `revision_mismatch` re-reads the switch and stops rather than overwriting another operator's write. `store_behind_fleet` and a full NOTIFY queue (SQLSTATE 54000) point to `FLAG_CHECKOUT_HALT=engaged` plus a restart.
  - Unknown commit (exit 4, panic 101, a signal, or no report): checks `history` for the proposed version, then re-issues the same write with the same N, at most 3 times. The store answers an already-satisfied write `unchanged` before checking revisions, so this cannot write twice or undo a later release. It ends with exit 30 if the outcome is still unknown.
- **`tests/engage_halt.rs`**: 11 cases that run the script against a scripted stand-in for `batter-flags`, using the child process's own environment. They assert the exact commands run, the same `--if-revision` on every re-issue, and the exit status.

## Commands and results

- `SQLX_OFFLINE=true cargo check --offline --all-targets`: passes, no warnings.
- `SQLX_OFFLINE=true cargo test --offline`: 12 unit tests and 11 script tests pass.
- `cargo clippy --all-targets` and `cargo fmt --check`: clean. There is one `#[allow(clippy::result_large_err)]`, with a reason, on `orders::accept`.
- Binary run with no database: startup fails with "startup failed at database". Bad arguments, a bad `FLAG_` value and a missing migration URL each fail with a clear message.
- I also built the real `batter-flags` command offline with `--locked` into a scratch target directory, so nothing was written into the Batter repo, then deleted that build. The script against it with an unreachable database exits 20; with `PGHOST` set it exits 2.

## Friction with Batter's API and docs

1. **The flags example contradicts the canonical startup path.** `crates/batter/examples/rollout.rs` calls `supervisor.start()` directly, while `crates/batter/src/lib.rs` (the facade's own doc comments) and `crates/batter-flags/src/control_plane.rs` say to use `Startup::protected`. Its `migrate` also builds and closes a pool by hand instead of using `command::Command`. There is no example combining the HTTP gate, the job gate and pinning; I assembled those from `crates/batter-flags/README.md` and the doc comments in `http.rs` and `jobs.rs`.
2. **The canonical handler test path can't be used offline.** `runledger-test-support` (`crates/batter-runledger/Cargo.toml`) pulls Docker-related crates that are not in the lockfile, and `JobFlags` has no test constructor. So I could not drive the gated handler itself; I tested the decision on `PinnedResolution` with `TestFlags::view()`.
3. **The protected phase API doesn't fit a flag-aware handler.** The README says to use `job_phases(...)?.into_operation(work, finalize)`, which needs owned callbacks. But `JobFlags` is borrowed and the pin has to be read from the row first. I used the manual sequencing that `crates/batter-runledger/src/phases.rs` labels "not the protected path".
4. **Receipts can still dead-letter on a first boot.** Before the first flags publication the job gate always uses up an attempt, whatever `when_tripped` says (`crates/batter-flags/README.md`). The last-good file and 25 attempts cover about 15 minutes. A longer flags-store outage on first boot would dead-letter receipts; I couldn't choose continuation for that window.
5. **The operator command's JSON payloads are undocumented** (`crates/batter-flags-cli/README.md`). I had to read source and migration SQL:
   - `show` returns its state as `{"encoding":1,"switch":…}`;
   - a `history` entry's `after` field is the full row image, so the state is at `.after.state.switch`;
   - `fleet --version` reports `rollout.reached` and `live`, or `applied` and `behind`;
   - only one of the two "uncertain" code paths includes `proposed_version` (`src/writes.rs`).
6. **"Re-run with --if-revision" doesn't say which revision** (`docs/operations.md` and the command's own message). Using the current revision could re-engage a switch someone released in between. The safe choice, the original revision, depends on the "equality is tested before CAS" rule, which is stated only in `crates/batter-flags-postgres/README.md`.
7. **Read failures are worded like write failures.** Against an unreachable database, `show` and `fleet` (reads) report "the timeout passed before COMMIT; nothing written", and a refused connection shows up as a timeout.
8. **Smaller items:**
   - The operator command is unpublished, so operators must build it from the Batter repo (the script accepts `BATTER_FLAGS_BIN`).
   - `flags-postgres` selects no TLS provider and the lockfile has none, so the service cannot use `sslmode=verify-full`.
   - There's no documented way to refuse `PG*` variables; I used `select_environment(&[], &["PG"])`. I believe, but have not checked, that SQLx's URL parsing also reads them.
   - Narrowing settings before `declared::resolve` silently resets every flag to default, and this is stated only in rustdoc (`crates/batter-flags/src/declared.rs`).
   - No helper produces today's `CivilDate`; the example hand-rolls one and I used chrono.
   - Sharing `_sqlx_migrations` with an application migrator is undocumented; `set_ignore_missing` and the migration order were my guess (`docs/reference-compatibility.md` covers only Runledger with Runlimit).
   - `SharedCommandReport` doesn't implement `Display`.

## Not done

- No exact-role grants manifest for the serving login.
- No check that the app's own table exists at startup (`verify_schema` covers only Runledger).
- No dependency health monitor; readiness reflects the lifecycle only.
- Receipt delivery is a placeholder, so a delivery that succeeds but fails to record would be re-sent on retry.
- No README.
- I read Batter's CLI and store source, but nothing under `docs/evidence/`.
