# Typed root settings that reach native constructors


Owning Bead: `batter-5pm`. Beads owns delivery scope, acceptance, priority and
dependencies; this file supplies execution details under `.agent/PLANS.md`.
Audited baseline: `0e47f7dbd5d0c04d878d290c5d8c9181d1162b18`, 2026-09-10.
Implementation completed under Jig plan `plan_01M262XKYHQ8SPX2XT63H2TN2M`.
The prior audit supplied this plan; current evidence is recorded below.

## Purpose


An application root will read explicitly chosen sources once, reject invalid
settings before acquisition, and pass validated values to native request, pool,
process-admission and worker constructors. Changing a setting must change the
constructed policy and its demonstrated behavior. Formatting settings or ordinary
startup failures must not print credentials. Small HTTP roots can reuse loading,
bounded parsing and redaction without depending on the durable reference.

## Progress


- [x] (2026-09-10) Audit current source, tracker dependencies, native APIs and
  verification commands; distinguish inspected semantics from executed behavior.
- [x] (2026-09-10) Specify source policy, ownership, constructor coverage,
  diagnostic limits and the producer/consumer acceptance boundary.
- [x] Implement shared settings mechanics and failure tests.
- [x] Implement reference settings and native constructor paths with tests.
- [x] Adopt shared mechanics in the runnable HTTP example and reference live probes.
  All nineteen live cases passed on PostgreSQL 18.6 under both supported toolchains.
- [x] Execute both toolchain matrices, relevant live tests, rebuilt HTTP smokes
  and Jig gates; update contracts/status/evidence and close the owning Bead.

## Surprises & Discoveries


The reference package already exists. Its `src/lib.rs` currently contains only
native type-identity functions. `tests/support/worker.rs::probe` builds a real
Runledger supervisor from hardcoded `JobsConfig`; there is no production command,
router or worker host yet. `batter-4t6` and owned startup task `batter-7r3.3` are
closed. Waiting for the future host before completing its config prerequisite
would create an acceptance deadlock.

The HTTP example's `Config::load` reads `BATTER_BIND` and
`BATTER_REQUEST_TIMEOUT_MS`; `router` hardcodes `Bulkhead::new(32)`. Its logging
module already retains native causes behind sanitized Debug/Display. The SQLx
lifecycle example reads `DATABASE_URL` directly and hardcodes pool options.
These are source-backed reuse examples, not proof of two deployed adopters.

Selected dotenvy 0.15.7 has nonmutating iterators, but variable substitution reads
`std::env::var` ahead of file values. Its parse error retains and displays the
offending line. It cannot directly establish an injected-source-only contract.
SQLx 0.9.0 derives Debug for `PgConnectOptions`, including its password, reads
`PG*` environment defaults even in `new_without_pgpass`, and its URL parser warns
with both unknown query keys and values. `PgPoolOptions` silently clamps minimum
connections internally. Runledger's selected `JobsConfig::from_env` silently
defaults malformed input and clamps other values. Use its typed validation and
explicit `Supervisor::builder` instead. Sources are recorded in
`docs/references.md` under the dated configuration audit.

## Decision Log


2026-09-10: Put narrow, policy-free, std-based mechanics in
`crates/batter/src/settings.rs` and `settings/`, exported as `batter::settings`.
Application-specific names, defaults, precedence, secret requirements and native
SQLx/Runledger constructors stay in the reference or HTTP root. This introduces
no new package, adapter dependency or configuration registry. If `batter-tmx`
has extracted the foundation first, implement the module in `batter-core` and
re-export the same path through `batter`; if extraction follows, its public-path
inventory must include this module. Neither task requires a new blocking edge.

2026-09-10: Use a small explicitly documented literal dotenv dialect, with one
shared implementation. Do not use dotenvy's ambient interpolation or implement
shell expansion. This is the settings source reader authorized by this Bead,
not a second application/deployment validator. No TOML/YAML, dynamic reload,
remote secret store, CLI framework, DI graph or placeholder provider token is
needed. Another dialect/library requires equivalent hermetic tests and an
updated primary-source decision before adoption.

2026-09-10: Treat native connection options and retained causes as an explicit
trusted boundary. Redacted wrappers do not sanitize intentional raw exposure,
third-party source-chain formatting, the default panic hook, process memory or
upstream runtime logs. Do not claim encryption or zeroization. Reject URL
parameters that would trigger SQLx's value-bearing warning before native parsing.

2026-09-10: Complete reusable helpers, concrete reference constructors, current
native consumer tests and startup failure evidence in this Bead. Full production
router adoption belongs to `batter-kpd`, hosted worker adoption to `batter-0cp`,
and an authorized pinned external migration to `batter-7r3.6`. Their runtime
acceptance remains required there; none is evidence that this producer already
provides a complete reference service.

## Outcomes & Retrospective


Shared mechanics and both application consumers are implemented. Both final Rust
1.98.1/1.94.0 verify matrices pass (754 executions each), and all ten rebuilt HTTP
smokes pass. Following the user's Docker provisioning request, native preflight
and all nineteen live cases passed on both toolchains against a dedicated
PostgreSQL 18.6 container. This includes the three new configuration cases and
the stronger committed-lease worker oracle. Use docs/validation.md for executed
evidence and managed Jig evidence for the final current api:test receipt.
The disposable endpoint is removed after verification. No commit, publication,
full reference-service implementation or deployed adoption is implied.


## Context and Orientation


Read `AGENTS.md`, `agent-map.md`, and the nearest package guides first.
`crates/batter/src/validation.rs` owns the existing operational bounds;
`ConfigurationError` remains its existing public type. `Bulkhead::new` bounds
concurrent held permits; `Supervisor::with_process_capacity` bounds queued plus
running finite tasks. These are distinct capacities. `RequestPolicy::new` in
`crates/batter-axum/src/lib.rs` validates the response-construction budget.
`Startup` and cleanup reservations already own partial initialization; use them
without adding another lifetime owner.

Create `examples/reference-service/src/config.rs` with private submodules when
needed, export it from the example library, and add ordinary
`tests/configuration.rs`. Constructors belong here because they depend on native
SQLx 0.9.0 and Runledger Git revision
`0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4`. Add direct Batter/Axum dependencies
only to the application package where used. A URL syntax parser dependency may
be added here using the already resolved `url` version; keep it out of the
foundation. Cargo must generate any lockfile changes. Do not modify upstreams.

The existing reference live owner is `tests/reference_live.rs`, with helpers in
`tests/support/`, and the runner is `scripts/reference_live.py` through
`scripts/test_reference_live.sh`. It checks an exact inventory of sixteen cases;
new cases require the runner inventory and its Python controls to change together.
The harness is external PostgreSQL 18, never an ordinary-test prerequisite.

## Milestone 1: shared sources and redaction


Provide a value-hiding `SettingsSource`, a `SecretString` with explicit
`expose_secret(&self) -> &str`, and narrow bounded integer/duration parsing.
`SettingsSource` accepts injected `(OsString, OsString)` pairs so missing and
non-Unicode values stay distinct. Its Debug must not dump either keys or values.
Lookup/exposure is deliberate; do not implement implicit string dereferencing,
automatic serialization or broad configuration traits. `SecretString` Debug and
Display produce a fixed redacted marker, including alternate/pretty formatting.
Keep causes concrete. A generic redacted diagnostic wrapper may retain `E`
through `Error::source`; its own Debug/Display must use only static field names,
source category and reason. This does not make raw source traversal redacted.

Expose a reader over supplied bytes/Read plus an exact-path file helper. The
root decides whether to call it and whether a missing selected file is fatal.
There is no default `.env` discovery or parent traversal. Define the supported
dialect precisely: UTF-8, LF or CRLF, blank lines, full-line `#` comments, ASCII
keys `[A-Za-z_][A-Za-z0-9_]*`, an equals separator, trimmed outer ASCII spaces,
and single-line unquoted or wholly single/double-quoted literal values. Remove
matching outer quotes; keep interior bytes literally. Unquoted values cannot
contain whitespace or quote delimiters; quote values needing those characters.
Dollar signs and backslashes are literal, with no escaping, interpolation,
multiline values, inline comments or `export` statements. Reject unsupported
syntax and duplicate keys with a sanitized line-number/category error. A raw
line, unknown key or path is not suitable automatic diagnostic text.

The file helper takes a caller-provided byte limit and rejects excess data by
reading at most limit + 1 with checked arithmetic; the reference chooses 64 KiB.
An absent path means no file input; an explicitly selected missing, unreadable,
invalid-UTF-8 or malformed file is an error, even when environment values could
cover it. A source-reader test checks a parent directory containing `.env` and
an explicitly missing child path; the parent is never consumed.

Root source order is defaults < explicit file < captured environment < explicit
in-memory overrides. Validate source structure first, then merge, then parse the
winning value. Thus an invalid overridden numeric value may be replaced, while
a malformed file, duplicate or unknown file key still fails. Never fall back
after an invalid winning value. Empty differs from missing. Ambient environment
selection is application-owned: ignore unrelated names, reject unknown names
under the root's reserved prefix, and allow explicitly named external settings
such as DATABASE_URL and RUST_LOG. Dedicated files and overrides reject every
unknown key. Helpers receive this policy; they do not embed application names.
Test both source permutations and these negative cases without set_var/remove_var.

## Milestone 2: validated reference settings and native constructors


Use private fields in `RootSettings` with `PoolSettings` and worker settings;
no public raw fields permit post-validation invalid states. Constructors require
validated settings rather than Option values with hidden defaults. Root loading
and validation do no network work, bind no listener and spawn no task. Use
checked conversion into u32, usize, i64 and i32 before calling native APIs.
The following are selected example defaults, not production recommendations.

Keep HTTP names/defaults `BATTER_BIND=127.0.0.1:3000` and
`BATTER_REQUEST_TIMEOUT_MS=2000`; accept ephemeral port 0 for tests. Add
`BATTER_BULKHEAD_CAPACITY=32` and `BATTER_PROCESS_CAPACITY=32`, each positive and
no larger than the actual native constructor bound. Use
`BATTER_POOL_MAX_CONNECTIONS=8`, `BATTER_POOL_MIN_CONNECTIONS=0`, and
`BATTER_POOL_ACQUIRE_TIMEOUT_MS=3000`; require max > 0, min <= max and a
representable u32. Reject zero or over-one-year positive durations rather than
truncate or clamp. Delegate Batter's exact duration and capacity checks to its
constructors; do not widen the existing invariant.

Map all eight selected `JobsConfig` fields explicitly. Retain upstream setting
names and documented initial defaults: JOBS_WORKER_ID is required for a serving
root (setup may omit it because no worker is constructed), JOBS_POLL_INTERVAL_MS
500, JOBS_CLAIM_BATCH_SIZE 16, JOBS_LEASE_TTL_SECONDS 60,
JOBS_MAX_GLOBAL_CONCURRENCY 32, JOBS_REAPER_INTERVAL_SECONDS 15,
JOBS_SCHEDULE_POLL_INTERVAL_SECONDS 30 and JOBS_REAPER_RETRY_DELAY_MS 30000.
Require a nonblank worker ID, batch 1..=JOBS_CLAIM_BATCH_SIZE_MAX, positive lease
TTL/retry delay fitting i32, positive concurrency within Tokio's capacity,
and positive bounded intervals. Do not adopt from_env's silent defaults or
its stronger 10-second/1000-ms clamps: direct native validation permits 1.
Call `JobsConfig::validate` as well as application upper-bound checks. Native
intent promotion deliberately derives polling and batch size from JobsConfig;
do not silently read JOBS_INTENT_PROMOTER_* or enable a second source path.
Unknown JOBS_* settings are rejected. Runlimit settings wait for `batter-97p`.

DATABASE_URL has no default and is secret as a whole, including percent-encoded
userinfo, query password, host and path. The reference's explicit `ConfigMode`
distinguishes Serve from Setup. Both require an endpoint; Serve requires an
explicit nonempty password, Setup permits an explicit passwordless endpoint for
externally provisioned local trust-authenticated TCP fixtures. These are example root
policies, never generic library rules or authorization checks. Do not invent an
API/provider secret before its real consumer exists. Both modes validate every
supplied field; Setup does not acquire service resources or construct workers.
Give `PoolSettings` and `WorkerSettings` their own typed constructors over
explicit selected inputs, used internally by RootSettings. Existing probes can
exercise those same sections without manufacturing a serving root or imposing
Serve's password requirement on the externally provisioned fixture. Calling a
worker constructor still requires its explicit valid worker ID. Setup must not
silently synthesize a worker configuration when that identity is absent.

Before SQLx parsing, use native URL syntax parsing to require postgres/postgresql,
an explicit host, username and database, no fragment, and a valid nonzero port
(default 5432). Initially support only the query keys password, sslmode and
application_name, once each; reject all others, including values/keys containing
fake-secret markers, before SQLx can warn. Reject conflicting password sources.
Decode URL userinfo/database/query values exactly once, preserving native
credential meaning. Reject malformed percent encodings and decoded invalid UTF-8
with sanitized errors. Cover IPv4/DNS/IPv6 hosts and encoded credential characters;
Unix socket URLs are outside this initial reference subset, not outside Batter's
Unix platform scope. Assert native endpoint/username/database/SSL getters and
explicit trusted password URL round-tripping with fake values so a successful
connection to a trust-authenticated test server cannot hide a lost password.
Require explicit sslmode so TLS policy is not inferred; existing local live tests
use disable. Preserve selected TLS semantics; never downgrade a requested mode.
This is a documented reference URL subset, not a claim of full libpq support.

SQLx lacks an environment-free default constructor. The reference consequently
rejects native PG* configuration from its captured process environment
(including password, host, TLS/certificate, options and passfile settings) before
native option construction, with a fixed diagnostic. Reject even empty values
when the variable is present; empty can affect native defaults. Explicitly
overwrite endpoint, password (including empty in Setup), SSL mode and application
name, using `new_without_pgpass`/setters after validated URL decomposition so no
passfile lookup occurs. The root captures the environment once and never mutates
it; native construction must occur in that same unchanged process environment.
Tests exercise a `RootSettings::from_process` entry through a bounded Unix test
child with env overrides (not a new config-check binary) and prove PG*
inputs fail before native construction. The injected loader alone cannot prove
an ambient-free SQLx API. Document this root restriction; downstream roots may
choose their own explicit native-source policy rather than inherit it silently.

Provide methods equivalent to `request_policy(ShutdownHandle)`, `pool_options()`,
`connect_options_from_process()`, `supervisor(ShutdownBudget)`, `bulkhead()` and
`jobs_config()`; return native types and concrete errors. Exact return types are
RequestPolicy, PgPoolOptions, PgConnectOptions, Batter Supervisor, Bulkhead and
JobsConfig. Do not expose native connection options in a derived Debug of a
larger settings/options aggregate. At explicit native access, document that its
Debug and URL conversion may reveal secrets. No new aggregate service container.

## Milestone 3: consumption and failure proofs


Adopt shared source/bounds/diagnostic mechanics in
`crates/batter-axum/examples/http_service.rs` and its logging module. Keep its
existing bind, timeout, RUST_LOG default/filter behavior and fixed failure exit.
Pass configured Bulkhead capacity into the actual router; retain its existing
default 32 and all readiness/observation tests. The HTTP example has its own
schema and does not gain SQLx, worker settings, or a mandatory secret. An
explicit BATTER_ENV_FILE path can select the optional source; do not look it up
from inside that file. Unknown file keys are checked against this root's schema.
Document and execute the example with a temporary file and fake values, including
an environment override that changes its observed request deadline.

Reference constructor tests must inspect native PgPoolOptions getters, validate
all JobsConfig fields and use the actual native builder path. Request tests
exercise RequestPolicy through request_admission with short/long budgets under
paused Tokio time. Bulkhead tests hold N permits and reject N+1; process tests
hold N admitted factories and reject N+1 after acknowledged startup, then
release, join and inspect the report. No assertion merely compares a parsed
struct against itself. Test maximum accepted and first rejected boundaries,
numeric overflow, negative values, min > max and native errors without catch-all
panic acceptance. Share root accepted/rejected cases with any additional real
validator that implementation introduces; create no deployment parser for tests.

Extend the existing reference worker probe to obtain its JobsConfig from the
same WorkerSettings constructor via explicit test overrides (20 ms poll, batch/concurrency 1 and
the current witness identity), preserving all migration, real handler witness,
shutdown and independent SUCCEEDED assertions. It must pass the constructed
object to `Supervisor::builder`, never builder_from_env. Cover changed worker
concurrency with held handlers and actual started-handler counts in a controlled
live case, so replacing the supplied config with defaults is detected. Other
field mappings use direct native-object assertions plus native validation.

Add a reference live pool/startup case using constructor-derived PgPoolOptions
with max 1/min 0: hold the sole checkout, witness a pending second acquisition
and its acquire timeout, release, then acquire successfully. Native SQLx timers
and PostgreSQL use real time, with barriers/independent process bounds; paused
Tokio time is not a database clock. FixtureSuite/FixtureScope accept native pool
options through ConnectionPlan and can prove the PoolSettings consumption path;
their FixtureDatabase deliberately exposes pools/name, not a database URL.
For the startup case that also exercises RootSettings::connect_options_from_process, use the
existing native lease pattern in tests/support/leases.rs: retain an awaited
PostgresHarness::empty_database lease, obtain its database_url, load Setup
settings, and pass both constructed options to connect_with. Keep that lease
owned until the startup result and pool finalization have been inspected, then
await lease cleanup and deferred drain, retaining all results before assertions.
Do not add a raw-URL escape to FixtureDatabase or silently replace native connect
options in this test. Keep Setup and serving credential policies distinct and
preserve the compatibility tests' existing authority assumptions.

Use `Startup`, reserve cleanup before acquisition, register the acquired pool
before a later fallible stage, then inject an initialization failure and a
separate cleanup failure. Inspect the retained primary error and all cleanup
records, reverse ordering, exact counts and absence of readiness. A generic
offline resource can return an actual cleanup error; SQLx Pool::close returns
unit, so do not fabricate a pool-close error. The live case independently proves
pool closure before lease cleanup. Invalid configuration must record zero
acquisition calls. Reuse existing waiter-cancellation ownership tests where the
path is unchanged; add a barrier case if this composition changes the transfer.

Capture tracing plus normal/alternate Debug and Display of sources, nested
RootSettings, SecretString, config failures, startup failure wrappers and actual
HTTP-example/test-child stderr. Use distinctive fake markers in passwords, encoded credentials,
unknown keys/parameters, malformed URLs, non-Unicode input and parse failures.
Require expected sanitized categories/nonzero exit as well as marker absence;
empty output is not a passing diagnostic oracle. Deliberate source inspection
must still recover the original typed cause. Native raw option Debug, arbitrary
source-chain reporters and panic-hook output are outside this automatic-output
claim. Never format those raw objects merely to build a sanitized message.

## Concrete steps and verification


From `/home/aa/Documents/batter`, verify `br show batter-5pm --json` and
`br ready --type task --json`, claim implementation only when starting it, and
start a new Jig implementation session using this file as its body. Recheck
Cargo.lock/source revisions before native changes. Ordinary focused commands
are `cargo test -p batter --locked`,
`cargo test -p batter-axum --example http_service --locked` and
`cargo test -p batter-example-reference-service --all-targets --all-features --locked`.
The HTTP example currently lacks `test = true` in its explicit manifest entry;
add it so the new and existing example-local tests run in the normal matrix as
well as the focused command. Verify the test names in actual Cargo output.
The proposed configuration targets do not exist yet; once added, ensure Cargo
discovers them and their tests actually run. New public helpers need runnable
rustdoc examples; use `cargo test -p batter --doc --locked`.

Run `bash scripts/test_reference_live.sh` with POSTGRES_TEST_ADMIN_URL selected
externally for a disposable local PostgreSQL 18 server. The existing preflight
requires CREATE DATABASE and catalog-lock fault-injection privilege, not merely
a reachable server. Run it on both supported toolchains and require the updated
exact inventory to execute with zero ignored/filtered cases. Do not print the
connection value. Ordinary all-feature tests compile these tests as ignored;
they never substitute for live evidence. Update docs/reference-compatibility.md
only with actually executed expanded probe evidence.

Final code/dependency verification is:

    bash scripts/verify.sh
    RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
    cargo build -p batter-axum --example http_service --locked
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline

Rebuild the HTTP example with RUSTUP_TOOLCHAIN=1.94.0 and repeat all five smokes
against that binary. Run `python3 -m unittest discover -s scripts -p test_reference_live.py`
if changing the live inventory, plus existing smoke controls when changing their
runner. Both verification matrices include doctests, Clippy and warning-denied
rustdoc. HTTP smokes are separate evidence. Do not weaken semantic tests to get
a pass or interpret source inspection as execution.

Update the relevant settings contract in docs/guarantees.md, integrations.md,
operations.md and testing.md, the config row in docs/status.md, root/package
rustdoc and guides, example usage, and docs/references.md. Record actual commands,
toolchains, platform and live prerequisites in docs/validation.md. Keep Linux,
macOS and hosted claims separate. Inspect `scripts/jig work evidence --plan-id
<implementation-plan-id>` and `scripts/jig work gates --plan-id <implementation-plan-id>`,
then `scripts/jig work check --plan-id <implementation-plan-id>`. Require the
successful current api:test receipt and all applicable gates; reuse a fresh
receipt only under the repository's unchanged-environment/toolchain rules.
Review the complete diff and owning Bead acceptance before finishing that session.

## Validation and acceptance ownership


Bead AC1/AC2 are proven by source-policy and bound/error tests; AC3 by secret and
diagnostic tests. AC4 is proven by native option assertions and behavior tests;
AC5 by HTTP example and live reference constructor consumption; AC6 by startup
failure/cleanup proof; AC7 by preserved application boundaries and consumer
handoffs; AC8 by the complete executed validation and documentation evidence.
Mark completion only when all eight have evidence. A configured option printed
by an unused diagnostic binary does not prove consumption.

The later command must use these constructors in its production root and show
deadline/pool/capacity effects there. The worker host must use the same typed
JobsConfig path and show configured concurrency in its hosted runtime. External
adoption requires authorized access and an immutable pin under batter-7r3.6.
Those are downstream deliverables, not hidden prerequisites for this Bead.

## Idempotence, compatibility and recovery


There is no schema migration, persisted setting format, queued payload change,
dependency upgrade or external-repository edit in the intended work. Preserve
existing example defaults and explicitly document the new optional file dialect
and validation rules. Test files contain fake values only. Keep source readers
and constructors separately testable so failed parsing cannot leave resources.
Live tests retain cleanup ownership and use the existing external fixture
recovery path; never assert success after skipping a failed teardown.

If interrupted, inspect the current Bead, worktree, generated lock and live
resource owners before resuming. Preserve unrelated changes and append-only Jig
state. Update the living plan when real signatures or ownership move. Do not
commit, publish, deploy or edit an inferred downstream checkout without the
separate user authorization those actions require.

## Revision note


2026-09-10: Initial handoff prepared from the current repository and pinned
native sources. Corrects the obsolete new-package assumption, the consumer
acceptance deadlock, silent native fallback/clamping, ambient dotenv/SQLx
inputs and diagnostics that can expose credentials before error wrapping.


## Implementation discoveries (2026-09-10)

SQLx 0.9's `to_url_lossy` panics for a setter-provided bare IPv6 host. Native
Tokio connection needs that bare host, so the root retains it; DNS/IPv4 URLs
prove credential round-tripping and an independent IPv6 handshake proves exact
username/database/password bytes. No upstream patch is included. The selected
graph has no SQLx TLS backend; native mode preservation is tested, negotiation
remains unverified and requires an application-selected native TLS feature.

A failed Startup before running-driver transfer leaves the handle Draining,
not Stopped. The offline test asserts that native state plus zero task spawns
and real LIFO cleanup outcomes. Native SQLx pool close still returns unit.

The injected loader's PG* rejection cannot constrain ambient SQLx defaults by
itself. `connect_options_from_process` also rejects current real PG* variables before native
construction; callers must still keep process environment unchanged. Explicit
empty userinfo passwords preserve presence for conflict checking even though
Url normalizes its password accessor to None.


## Current acceptance audit

AC1-AC4 have offline source, boundary, redaction and native runtime evidence in
settings.rs, the twelve HTTP example tests and nineteen reference configuration
tests. AC5 has HTTP adoption and real native pool/worker execution. AC6 has real
offline LIFO failure evidence and the executed pool-close-before-lease probe.
AC7 has public rustdoc, the runnable independent HTTP consumer, docs and concrete
batter-kpd/batter-0cp handoffs. AC8 has both complete verification matrices, ten
rebuilt smokes, nineteen live cases on each supported toolchain, and passing Jig
gates; final managed receipts are refreshed after documentation/tracker updates.
The user authorized disposable Docker provisioning, resolving the earlier
endpoint blocker. No application source or test changes were required by the
live runs. All accepted review corrections are applied; TLS negotiation, macOS
execution for these changes, hosted CI and deployed adoption remain unclaimed.


## Review corrections (2026-09-10)

Addressed both collated review findings: the live startup probe retains its
StartupError report alongside observation/channel errors through a shared
private checker, exercised offline with actual startup/cleanup failures. Four
settings fixture consumers use exclusive mode-0700 directory creation instead
of accepting PID-derived preexisting paths. Collision/symlink regression and
an isolated child-process canary confirm that precreated paths cannot redirect
the new fixture writes. No production API or dependency change. Verification
results are recorded in docs/validation.md; live PostgreSQL acceptance remains
separate and the owning Bead stays in progress.

## Structural corrections after independent review (2026-09-10)

Research precedes implementation in this round; primary evidence and resolved
open questions are recorded in docs/references.md. The defects were local
boundary-design failures: a pure-looking test crossed a process-global native
constructor, and an independent Python/psql preflight recognized a different
configuration than the actual consumer. The inventory sentence was a separate
manual bookkeeping omission. No framework or backend-neutral abstraction is
needed.

The native constructor is explicitly named connect_options_from_process; tests
use the existing cleared child-process owner. The normal verification matrix
runs the full configuration target with hostile parent PG* entries to prevent
recurrence. Native Rust preflight and fixture acquisition share the actual root
validator and live endpoint policy; preflight authenticates through explicit
SQLx options and Python only schedules commands/checks the exact inventory.
Documentation refers to the authoritative inventory without a second numerical
breakdown. The live worker oracle uses committed lease counts rather than a quiet
time interval, grounded in the pinned native claim transaction.

Validate targeted clean/hostile tests, actual preflight rejection without fixture
acquisition, both full toolchains, rebuilt HTTP profiles and final Jig evidence.
Real PostgreSQL preflight and all nineteen live cases subsequently passed on
both toolchains using the user-authorized disposable Docker endpoint.
The upstream-native environment-free-constructor opportunity is a future upstream
concern, not permission to simulate purity by mutating global environment here.

The actual native preflight entrypoint is exercised in bounded child processes
by normal Cargo discovery; its shared-validator tests are not the only rejection
oracle. Python orchestration controls require preflight before inventory and
execution, and the matrix's scheduling controls require the hostile-environment
pass and propagate its failure. These checks address recurrence at both sides
of the process boundary rather than only testing helper return values.
