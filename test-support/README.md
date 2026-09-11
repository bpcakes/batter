# Private repository test machinery

`process/` contains std-only Unix launch, output capture, timing and kill/reap
mechanics shared by integration-test binaries. It is not a Cargo package or a
public Batter API. The root remains a virtual workspace with no test target.

Callers supply a `WaitPolicy`; shared code knows no application scenario names,
package-relative assets, lifecycle APIs, or adapter types. Foundation wrappers
in `crates/batter/tests/non_yielding/` include these sources and attach their own
white-box controls. That suite alone owns the non-yielding fixture and Linux
Python parent-death probe. HTTP tests import only these shared mechanics and
provide their own fixture dispatcher and fixed deadline.

Keep controls attached explicitly by the owning suite. Do not put nested test
module inclusions here: every consumer is already compiled with `cfg(test)`, so
that flag cannot distinguish consumers. Do not resolve a suite's assets using
the consuming package's environment from a shared module.

The workspace-relative location remains stable when foundation tests move to a
different same-depth crate. Cargo fmt follows the HTTP module paths; Cargo/Jig
workspace tests compile both consumers, and Jig's `**/*.rs` inputs include these
sources. The foundation controls continue to test the same included implementation.

`events.rs` owns synchronous event storage and returns owned snapshots. It exposes
no mutex guard: failed ordering/wait assertions must leave resource Drop recording
available. Suite-local wrappers still own notifications, timeouts and diagnostics.

`dispatch.rs` constructs test subscribers with one retained OFF-filtered registry
before any real dispatch. Keeping the bootstrap maximum OFF prevents concurrent
macros from registering while the first real dispatch is being constructed; an
ordinary NoSubscriber has no maximum-level hint and prematurely enables TRACE.
The two-registration arrangement addresses tracing-core 0.1.36's callsite cache
bug ([upstream #2874](https://github.com/tokio-rs/tracing/issues/2874)) when a thread
without a subscriber first reaches a callsite. It installs no thread or global
default and does not retain real capture subscribers indefinitely. The isolated
`crates/batter/tests/tracing_dispatch.rs` first-hit and bootstrap regressions must pass before removing this
workaround on an upstream upgrade. Enabled and filtered callback assertions remain
in their owning suites; production code does not use this constructor.

The HTTP adapter's shared wrapper also owns complete startup/exercise/teardown
allowances below its process deadline. Keep phase policy there; the generic
`process/` machinery does not know HTTP phases. Exercise timeouts belong inside
joined tasks, and observed reports must outlive later reconciliation failures.
The filtered telemetry fixture no longer retains real dispatches in a static
vector; its storage-release regression permits brief concurrent cache borrows.

Terminal HTTP report checks run after exercise, using the retained report. Scenario
access restricts observer waiting to the intentional blocked-body abort checkpoint.
Delayed real cleanup success and missing-event reconciliation controls test both
sides of this boundary; keep semantic body/resource assertions with their fixtures.

Raw `WithSubscriber::with_subscriber(subscriber)` arguments also convert through
`Into<Dispatch>`; construct those dispatches with the shared helper. Already-built
`Dispatch` clones need no additional construction. Observation wait guards retain
interrupted event/wire diagnostics on destruction under the governing phase or
semantic checkpoint deadline. Keep capture and reports outside those futures;
recheck keep-alive handler counts after terminal reports in both HTTP fixtures.

`temp_dir.rs` allocates private Unix fixture directories with exclusive creation
and mode 0700. Existing directories or symlinks are rejected; bounded retries
choose another candidate. Tests retain the owner and explicitly check `close`;
Drop is only a best-effort panic fallback. Settings and configuration fixtures
share this private source, with collision/symlink controls in the foundation
settings target. It is not a public crate API or a database fixture.
