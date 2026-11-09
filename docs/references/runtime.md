# References: Runtime, ownership, telemetry and settings

[Reference index](../references.md) · [Documentation index](../README.md)

These source checks support current contracts. Each record names the
version and date actually inspected; it is not a new dependency audit.
Re-check the resolved dependency before changing an integration.

## Native cleanup acquisition, 2026-11-05

`batter-rtxb` checked Cargo.lock's Tokio 1.54.1 against its downloaded source
(`src/sync/semaphore.rs`, `src/net/udp.rs`). The pinned primary references are
[Semaphore::acquire_owned](https://docs.rs/tokio/1.54.1/tokio/sync/struct.Semaphore.html#method.acquire_owned)
and [UdpSocket::bind](https://docs.rs/tokio/1.54.1/tokio/net/struct.UdpSocket.html#method.bind).
The web fetch was unavailable; the resolved package source supplied the evidence.

An owned semaphore acquisition yields `OwnedSemaphorePermit`; dropping it returns
the permit. Cancelling a queued acquisition loses its queue position. Batter
retains the permit exclusively in registered cleanup and makes no waiter-bound
claim. UDP binding receives a concrete SocketAddr, avoiding caller address
resolution and DNS. Tokio's bind constructs the socket before returning it;
Batter polls that native future directly. An Arc clone retains socket ownership,
so dropping the cleanup reference alone cannot close a socket retained elsewhere.
Tests establish local reference lifetime and native error behavior, not remote
network effects or arbitrary descendant termination.

## Rust 1.98.1 panic hooks for the redacting hook, 2026-11-02

For `batter-4q78`, the pinned toolchain, Rust 1.98.1, is every package's minimum
(`batter-wis2`); every API the hook uses is stable there (`PanicHookInfo` since
1.81, `AsFd` and descriptor duplication since 1.63). The documented contract
comes first; the facts after it were read in the 1.98.1 source (tag `1.98.1`:
`library/std/src/panicking.rs`, `panic.rs`, `rt.rs`, `thread/current.rs`,
`io/stdio.rs`, `sys/process/unix/unix.rs`, `os/fd/owned.rs`, and libtest's
`library/test/src/lib.rs` and `test_result.rs` read in the 1.99.0 tree, whose
`panicking.rs` is byte-identical), are implementation behaviour rather than
documented guarantees, and a toolchain upgrade must recheck them. The CI job
definitions run the `panic_hook` target on 1.98.1 in the Linux verification and
macOS jobs and on floating `stable` in the weekly Linux run; hosted results
belong to the owning Bead, not here.

- [`panic::set_hook`](https://doc.rust-lang.org/1.98.1/std/panic/fn.set_hook.html)
  replaces the process-global hook, runs it before the panic runtime under both
  the unwinding and aborting strategies, and panics when called from a
  panicking thread. [`take_hook`](https://doc.rust-lang.org/1.98.1/std/panic/fn.take_hook.html)
  returns the current hook and reinstates the default one, so between it and the
  next `set_hook` the default hook is installed; the atomic `update_hook` is
  unstable. Nothing reports which hook is installed without replacing it, so no
  library can check that an application installed a particular hook.
- [`PanicHookInfo`](https://doc.rust-lang.org/1.98.1/std/panic/struct.PanicHookInfo.html):
  `payload`, `payload_as_str` (since 1.91.0) and `location` are its accessors,
  and `location` currently always returns `Some`. Its `Display` (`panic.rs`)
  writes `panicked at {location}:\n{payload}` for a string payload, so a hook
  that formats the info discloses the payload. A `#[track_caller]` function such
  as `Result::unwrap` reports its caller's
  [`Location`](https://doc.rust-lang.org/1.98.1/std/panic/struct.Location.html),
  whose `Display` is `file:line:column`, and `unwrap`'s payload contains the
  error's `Debug`. A `panic!` with a constant message carries a `&'static str`
  payload; one with arguments carries a `String`.
- `panicking.rs` `panic_with_hook` builds the hook's `PanicHookInfo` with
  `payload.get()`, which formats a `panic!` message's arguments into a `String`
  before any custom hook runs. `default_hook` renders `thread '{name}' ({os id})
  panicked at {location}:\n{payload}` into a 512-byte stack buffer and writes it
  with one `write_all` when it fits, otherwise formats it directly in several
  writes, to the raw standard error stream rather than through the
  `std::io::Stderr` lock, or into libtest's output-capture sink when one is set.
  It reads the thread name through a private accessor, prints a backtrace as
  `RUST_BACKTRACE` selects, and forces a full one for a panic raised while the
  same thread is already panicking, unless that panic forces no backtrace, as
  std's own `panic in a destructor during cleanup` abort does.
- `panic_with_hook` prints without consulting any hook in two cases. A panic
  raised while the same thread prepares or runs its hook, including while
  `payload.get()` formats the arguments, prints `panicked at {location}:` and
  that nested panic's message only when it is a constant string
  (`StaticStrPayload` alone implements `as_str`), then aborts. After
  `panic::always_abort`, which `sys/process/unix/unix.rs` calls only in the
  child that `Command` forks, before
  [`pre_exec`](https://doc.rust-lang.org/1.98.1/std/os/unix/process/trait.CommandExt.html#tymethod.pre_exec)
  closures and `exec`, a panic prints `aborting due to panic at
  {location}:\n{payload}` and aborts. A child of the application's own `fork`
  keeps the installed hook. A panic that cannot unwind runs the hook and then
  prints `thread caused non-unwinding panic. aborting.` without its payload.
  `resume_unwind` never runs the hook.
- `rt.rs` `lang_start_internal` catches a panic escaping `main`, drops its
  payload without printing it and returns exit status 101. libtest runs each test
  on a thread named after it, reports a plain failed test with status 101 and
  never prints that test's payload; it prints a `#[should_panic(expected = ...)]`
  mismatch's payload. When it captures output it installs a wrapper around the
  built-in hook, and after every run, with or without capture, it takes the hook
  again, which reinstates the default hook.
- `thread/current.rs`: the only stable route to a thread's name is
  [`thread::current`](https://doc.rust-lang.org/1.98.1/std/thread/fn.current.html),
  which allocates a handle on a thread's first call and panics once the
  thread's handle has been dropped in its final runtime cleanup. A panic inside
  a hook makes std abort the process, so the redacting hook reads no thread name.
- [`BorrowedFd::try_clone_to_owned`](https://doc.rust-lang.org/1.98.1/std/os/fd/struct.BorrowedFd.html#method.try_clone_to_owned)
  duplicates a descriptor with `fcntl(F_DUPFD_CLOEXEC, 3)` on Linux and macOS
  (`os/fd/owned.rs`) and returns an error at the descriptor limit;
  `io::stderr().as_fd()` borrows descriptor 2 without locking. Every write
  through [`io::Stderr`](https://doc.rust-lang.org/1.98.1/std/io/struct.Stderr.html)
  takes its process-wide reentrant lock, which `eprint!` holds while it formats
  its arguments, so another thread can hold it indefinitely, and `eprint!`
  panics when the write fails (`io/stdio.rs` `print_to`).
- [POSIX `write`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/write.html):
  a write of at most `PIPE_BUF` bytes to a pipe is not interleaved with other
  writers. [Linux `pipe(7)`](https://man7.org/linux/man-pages/man7/pipe.7.html)
  gives `PIPE_BUF` as 4096 bytes; the locked libc 0.2.189 defines it as 512 for
  Apple targets (`src/unix/bsd/mod.rs`), so the hook's 512-byte single write is
  atomic on a pipe on both systems.
- [POSIX `fcntl`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/fcntl.html):
  a process's process-owned record locks on a file are removed when it closes
  any descriptor for that file, while open-file-description locks are removed
  only when every descriptor for that description is closed. Closing the hook's
  duplicate therefore releases the former and not the latter.
- [`io::Write::write_fmt`](https://doc.rust-lang.org/1.98.1/std/io/trait.Write.html#method.write_fmt)
  panics when a formatting implementation reports an error that the underlying
  writer did not; the hook formats only `&str` and `u32` values, which report
  only writer errors.
- Tokio 1.53.1 [`JoinError`](https://docs.rs/tokio/1.53.1/tokio/task/struct.JoinError.html):
  `runtime/task/error.rs` formats a string payload in both `Display`
  (`task {id} panicked with message {payload:?}`) and `Debug`. Tokio stores a
  task's payload in the `JoinError` and prints nothing itself;
  `runtime/scheduler/multi_thread/worker.rs` prints a fixed line only when its own
  worker loop panics under debug assertions. tokio-macros 2.7.2 `src/entry.rs`
  builds the runtime with `.expect("Failed building the Runtime")` before it runs
  the `#[tokio::main]` body, and Tokio's `src/loom/std/mod.rs` panics with the
  value of an invalid `TOKIO_WORKER_THREADS`.
- tracing-subscriber 0.3.23: `tracing_subscriber::fmt()` builds a subscriber
  with `log_internal_errors` enabled (`fmt/mod.rs`), while `fmt::layer()`
  defaults it to disabled despite its setter's documentation
  (`fmt/fmt_layer.rs`). With it enabled, `on_event` reports a failed event write
  with `eprintln!`, and a formatting failure by writing to the same writer, then
  with `eprintln!` if that write fails. A hook that emits a `tracing` event
  through such a subscriber can therefore panic inside the hook.
- Locked dependencies, searched on 2026-11-02 for `set_hook`, `take_hook` and
  `update_hook` outside tests: ratatui 0.30.2 `src/init.rs` chains the hook in
  `init`, `try_init`, `init_with_options`, `try_init_with_options` and `run`,
  none of which any workspace package calls (the operator TUI builds its
  `Terminal` directly), and proc-macro2 1.0.107 swaps it only under
  `cfg(no_is_available)`, for compilers older than 1.57. This is evidence about
  one `Cargo.lock`, not a test.

## Tokio 1.53.1 blocking tasks for bounded blocking work, 2026-10-31

For `batter-0772`, `Cargo.lock` resolves Tokio 1.53.1. The versioned
documentation below states the public contract that `Bulkhead::run_blocking`
relies on. The facts after it were read in the installed 1.53.1 source; they are
source-inspected implementation behaviour, not documented guarantees, and a
Tokio upgrade must recheck them.

- [`task::spawn_blocking`](https://docs.rs/tokio/1.53.1/tokio/task/fn.spawn_blocking.html):
  a started blocking task cannot be aborted, and `abort` before it starts "may"
  prevent it from starting. Runtime shutdown waits indefinitely for started
  blocking tasks; `shutdown_timeout` stops waiting but leaves them running.
  Cancelling a task that has not started "is possible", "but this is not
  guaranteed". Tokio recommends a semaphore or similar primitive to limit
  CPU-bound parallelism, and a current-thread runtime still runs blocking
  closures on separate threads.
- [`runtime::Handle::spawn_blocking`](https://docs.rs/tokio/1.53.1/tokio/runtime/struct.Handle.html#method.spawn_blocking)
  runs the closure on that handle's blocking pool, and
  [`Handle::current`](https://docs.rs/tokio/1.53.1/tokio/runtime/struct.Handle.html#method.current)
  panics outside a runtime context. `run_blocking` takes the handle as its first
  statement, so that panic precedes any permit or decision. It then builds an
  unpolled `Sleep`: `time/sleep.rs` `Sleep::new_timeout` looks the time driver
  up and panics when timers are disabled, and registers nothing until polled,
  so a runtime without `enable_time` also panics before admission instead of
  in the result wait after handoff. A `Wait` call can reach the handoff in a
  later poll made from another context, and `Sleep::new_timeout` reads the
  current context, so `run_blocking` polls its result wait inside
  [`Handle::enter`](https://docs.rs/tokio/1.53.1/tokio/runtime/struct.Handle.html#method.enter)
  of the runtime checked at first poll. A `Sleep` keeps the handle it was built
  with (`time/sleep.rs`), so the admission wait's timer, built at first poll,
  needs no guard. Both timers therefore live in the first-poll runtime: a
  current-thread runtime keeps its driver in its scheduler core
  (`runtime/scheduler/current_thread/mod.rs`) and processes timers only inside
  `block_on`, so a call finished from another context sees its deadline only
  while that runtime is driven, and `TimerEntry::poll_elapsed` panics once it
  has shut down. This reasoning was checked for Tokio's default timer only;
  finishing a call from another context under `--cfg tokio_unstable` with the
  alternative timer (`runtime/time_alt`) is unsupported and unverified.
- tracing-subscriber 0.3.23 `registry/sharded.rs`: closing a span releases its
  parent through the thread's default dispatcher, and `Registry::try_close`
  panics on an unknown span ID. `run_blocking` therefore closes its
  `batter.blocking_work` span, and destroys an abandoned value or panic payload
  that may hold a span created inside the closure, under the dispatcher it
  captured at first poll.
- [`task::JoinHandle`](https://docs.rs/tokio/1.53.1/tokio/task/struct.JoinHandle.html):
  dropping the handle detaches the task, so an abandoned wait leaves the closure
  running.
- [`task::JoinError`](https://docs.rs/tokio/1.53.1/tokio/task/struct.JoinError.html):
  `is_cancelled` and `try_into_panic` separate cancellation from a panic and
  return the original payload.
- [`Runtime::shutdown_timeout`](https://docs.rs/tokio/1.53.1/tokio/runtime/struct.Runtime.html#method.shutdown_timeout)
  and [`shutdown_background`](https://docs.rs/tokio/1.53.1/tokio/runtime/struct.Runtime.html#method.shutdown_background)
  bound or skip the wait for blocking tasks, which keep running;
  `shutdown_background` documents the resulting resource leak.
- [`runtime::Builder::max_blocking_threads`](https://docs.rs/tokio/1.53.1/tokio/runtime/struct.Builder.html#method.max_blocking_threads):
  the default is 512 threads, which also serve `fs` operations, DNS resolution
  through `ToSocketAddrs` and stdio. Beyond the limit tasks wait in a queue that
  applies no backpressure and "could potentially grow unbounded".
- [`sync::Semaphore::close`](https://docs.rs/tokio/1.53.1/tokio/sync/struct.Semaphore.html#method.close)
  prevents new permits and notifies pending waiters. The source
  (`sync/batch_semaphore.rs` `close`) only sets the closed bit and wakes the
  waiters, so permits already issued are not revoked.
- [`time::advance`](https://docs.rs/tokio/1.53.1/tokio/time/fn.advance.html)
  moves paused time in one jump, and
  [`time::pause`](https://docs.rs/tokio/1.53.1/tokio/time/fn.pause.html#preventing-auto-advance)
  documents that auto-advance is inhibited while a blocking task runs.

Source-inspected facts in the installed 1.53.1 crate:

- `runtime/blocking/pool.rs` `Spawner::spawn_task`: once the pool's shutdown
  flag is set, a new task is never queued; it is shut down immediately, which
  drops the closure and stores a cancelled `JoinError`. The returned handle
  therefore resolves cancelled, despite the "will never resolve" comment in
  `Spawner::spawn_blocking`. Otherwise the task is pushed onto the queue first,
  then an idle thread is notified or a new thread spawned. If spawning fails
  with any error other than `WouldBlock`, or with `WouldBlock` while the pool
  has no threads, `spawn_task` returns `NoThreads` and `Spawner::spawn_blocking`
  panics with "OS can't spawn worker thread" in the caller, after the task was
  queued. Batter leaves that panic uncaught because the queued closure already
  owns its permit. This NoThreads path is source-inspected, untested.
- `runtime/blocking/pool.rs` `Inner::run`: a busy worker's loop pops and runs
  queued tasks without checking the shutdown flag, so queued closures can still
  run after shutdown begins. Only an idle worker woken by shutdown drains the
  queue through `shutdown_or_run_if_mandatory`, which shuts `spawn_blocking`'s
  non-mandatory tasks down unstarted. Runtime shutdown therefore does not
  reliably drop queued blocking work, and which worker takes a queued task is
  not specified.
- `runtime/blocking/task.rs` `BlockingTask::poll`: the closure runs to
  completion inside one poll, so a started closure has no cancellation point.
- `runtime/blocking/schedule.rs`: with `test-util` on a current-thread runtime,
  `BlockingSchedule::new` inhibits paused-time auto-advance when the task is
  created at spawn, and `release` allows it again when the task completes. The
  inhibition therefore also covers a queued closure, not only a running one as
  the `pause` documentation words it. Paused tests with an outstanding closure
  must use `advance` or cancellation.
- `runtime/task/state.rs` `transition_to_notified_and_cancel`: `abort` on a
  queued task, which is already notified, only sets `CANCELLED`. When a worker
  later takes it, `transition_to_running` reports the cancellation and the
  closure is dropped unstarted with a cancelled `JoinError`. Batter's
  mapping-level `NotStarted` test relies on this; `run_blocking` never aborts.
- `runtime/task/harness.rs`: `shutdown` drops an unstarted task through
  `cancel_task` under `catch_unwind`, so a capture whose destructor panics
  yields a panic `JoinError` rather than a cancelled one; Batter reports it as
  `Panicked`. `complete` and `drop_join_handle_slow` drop the output of a task
  whose handle is gone under `catch_unwind`, on the completing blocking thread or
  where the handle is dropped, so an abandoned value or panic payload is
  destroyed unobserved. For a closure that ran, `complete` runs on the blocking
  thread outside the dispatcher `run_blocking` captured, so Batter catches a
  closure panic inside the closure and returns the value or payload in a private
  wrapper whose destructor drops it under the dispatcher captured at first poll.
  When `shutdown` destroys an unstarted closure and a capture's destructor
  panics, Batter catches that panic and rethrows its payload in the same
  wrapper, so `cancel_task` stores the wrapper as the panic `JoinError`; Tokio
  destroys an unclaimed one under the captured dispatcher, and `run_blocking`
  unwraps a claimed one into `Panicked` with the original payload.
- `runtime/task/error.rs`: `Repr` is only `Cancelled` or `Panic`, and
  `JoinError`'s `Debug` and `Display` print a string payload, so
  `BlockingError` never stores a `JoinError`.
- `runtime/time/mod.rs` `Driver::park_internal`: the timer driver processes
  elapsed timers after parking. A task woken before the next park can observe
  `Instant::now()` past a deadline whose timer has not fired, which is how
  `Wait` admission can grant a permit after the deadline and why `run_blocking`
  checks the context again before handoff.
- `runtime/time/entry.rs` `TimerEntry::poll_elapsed` asserts that the time
  driver has not shut down, so a deadline timer polled after runtime shutdown
  panics.
- `runtime/runtime.rs` `Drop for Runtime` and `runtime/scheduler/`: `Drop`
  calls the scheduler's `shutdown` before the `blocking_pool` field is dropped
  and shuts the pool down. The current-thread scheduler destroys every spawned task and then
  shuts its drivers down, so its own tasks never meet a shutting-down pool. The
  multi-thread `Handle::shutdown` only closes the scheduler: a worker keeps
  polling tasks until `Core::maintenance` or parking observes the close, and the
  last worker's `shutdown_core` shuts the drivers down
  (`scheduler/multi_thread/worker.rs`). Workers are blocking-pool threads, so
  that can happen while the pool's shutdown waits. `Runtime::shutdown_timeout`
  and `shutdown_background` call `Handle::shutdown`, a no-op for current-thread
  (`scheduler/mod.rs`), then shut the pool down and wait up to the timeout; for
  current-thread this all precedes `Drop`, which shuts the drivers down. In
  `blocking/pool.rs`, `Spawner::spawn_task` destroys a task spawned after the
  pool began shutting down, and idle threads destroy queued tasks with
  `shutdown_or_run_if_mandatory`. `NotStarted` therefore reaches a caller only
  while the pool is shutting down and the time driver has not yet shut down: a
  multi-thread runtime's own tasks or a call polled from outside it, after a
  plain drop as well as either shutdown method, or a call polled from outside a
  current-thread runtime during `shutdown_timeout` or `shutdown_background`. `a_task_on_a_dropping_multi_thread_runtime_is_not_started` and `an_outside_call_during_current_thread_shutdown_is_not_started` exercise one
  case of each; the latter also checks that a poll after the runtime is gone
  panics in the shut-down timer after handoff. That a context cancelled or
  already past its deadline before handoff returns `NotAdmitted` from
  admission's context checks, and that one cancelled after handoff returns
  `Abandoned` from `run_inner`'s initial `check` and biased `select!`, instead
  of that panic, is source-inspected and not exercised.
- `runtime/builder.rs`: a current-thread runtime caps blocking threads at
  `max_blocking_threads`, while a multi-thread runtime adds its worker-thread
  count to that cap.

## Metrics facade 0.24.6: reviewed 2026-09-25

For `batter-6vn`, checked the locked `metrics` 0.24.6 source (`src/recorder/mod.rs`,
`src/handles.rs`, `src/key.rs`) and its
[crate documentation](https://docs.rs/metrics/0.24.6/metrics/). Without an
installed recorder, macros use a no-op recorder. `set_global_recorder` is
one-time application setup; `set_default_local_recorder` scopes a recorder to
the current thread, which the tests use with current-thread runtimes. Counter and
histogram handles are infallible: a recorder cannot return a recording error to
the caller, so dropping or coalescing is recorder policy. The crate declares
Rust 1.71.1, below this repository's 1.98.1 minimum. Its normal dependencies
are `rapidhash` and, only on `cfg(target_pointer_width = "32")` targets,
`portable-atomic` with its `fallback` feature. Metric names follow
[Prometheus naming practice](https://prometheus.io/docs/practices/naming/)
(`_total` counters, base-unit `_seconds`) and its
[cardinality guidance](https://prometheus.io/docs/practices/instrumentation/#do-not-overuse-labels).
Recorder aggregation and exporter buffering were not reviewed for `batter-6vn`;
the selected exporter and its flush order are recorded below for `batter-8jr`.

Rechecked the locked recorder implementation on 2026-09-26: `with_recorder`
prefers a thread-local recorder, including for description macros. There is no
public global-recorder getter. Global installation requires `Sync`, without
`Send`, and returns the rejected recorder through `SetRecorderError<R>`. Batter
therefore retains a shared reference to the accepted recorder for direct catalog
publication; ordinary observations still use the facade's scoped dispatch.

## Reference metrics export: reviewed 2026-09-26

For `batter-8jr`, resolved with Cargo and read the locked sources of
`metrics-exporter-otel` 0.3.1, `opentelemetry` and `opentelemetry_sdk` 0.31.0,
`opentelemetry-otlp` 0.31.1, `opentelemetry-http` 0.31.0 and
`opentelemetry-proto` 0.31.0. Newer 0.33 OpenTelemetry releases exist, but the
exporter bridge requires `opentelemetry` 0.31.

- [metrics-exporter-otel source](https://docs.rs/crate/metrics-exporter-otel/0.3.1/source/):
  `storage.rs` creates one observable counter with its own callback and
  attribute vector per complete key and one histogram per key; `lib.rs` keeps a
  registry entry per key and `metadata.rs` one description per name/kind, with
  no bound. `metadata.rs::set_description` retains caller text, and `storage.rs`
  applies it when creating an instrument, so the adapter checks the exact
  catalog description before delegation. Histogram boundaries must be set
  before first creation.
- [SDK `ManualReader`](https://docs.rs/opentelemetry_sdk/0.31.0/opentelemetry_sdk/metrics/struct.ManualReader.html)
  and `metrics/mod.rs` export the reader, `MetricReader` and `Pipeline` only under
  `experimental_metrics_custom_reader`. `manual_reader.rs::force_flush` is a
  no-op and `shutdown_with_timeout` only detaches the producer; collection after
  shutdown fails. `meter_provider.rs` ignores shutdown timeouts.
  `pipeline.rs` applies a default 2,000-stream cardinality limit per instrument;
  the largest foundation instrument has 792 series.
  `Resource::builder()` reads `OTEL_*` resource detectors;
  `Resource::builder_empty()` does not.
- [OTLP HTTP metrics source](https://docs.rs/crate/opentelemetry-otlp/0.31.1/source/src/exporter/http/metrics.rs)
  treats every 2xx as success without decoding the response and formats a
  non-success body and custom client errors into `InternalFailure` strings.
  The [HTTP builder source](https://docs.rs/crate/opentelemetry-otlp/0.31.1/source/src/exporter/http/mod.rs)
  merges `OTEL_EXPORTER_OTLP_*HEADERS` even with explicit
  configuration, and reads endpoint, timeout and compression variables when not
  set explicitly. With a caller-supplied HTTP client, its selected timeout is
  retained in `_timeout` but not applied to the request; Batter's separate
  attempt and final deadlines bound yielding transport work. The exporter
  performs no retry.
- [`opentelemetry-http` `HttpClient`](https://docs.rs/opentelemetry-http/0.31.0/opentelemetry_http/trait.HttpClient.html)
  receives the encoded `Request<Bytes>`, which permits a payload ceiling before
  dispatch and a fixed, body-free response.
- [OTLP partial success](https://opentelemetry.io/docs/specs/otlp/#partial-success)
  requires distinguishing rejected points from full acceptance and forbids
  automatic retry of partially accepted requests; a message without rejected
  points is a warning. [OTLP/HTTP responses](https://opentelemetry.io/docs/specs/otlp/#otlphttp-response)
  require HTTP 200 for both full and partial success, and carry a protobuf
  `ExportMetricsServiceResponse` for protobuf requests. The adapter treats
  every other HTTP status as a typed failure, including other 2xx responses;
  the upstream client accepts any 2xx, so this is an adapter-owned check.
  [Binary Protobuf encoding](https://opentelemetry.io/docs/specs/otlp/#binary-protobuf-encoding)
  requires the collector response to declare `Content-Type: application/x-protobuf`.

The resolved graph was checked with `cargo tree -e features`: no
`internal-logs`, `reqwest-blocking-client` or `experimental_async_runtime`.
`http-proto` enables the exporter's trace feature and tonic-generated message
types. These are source facts, not proof of collector durability.

Extraction amendment, 2026-09-26 (`batter-i3ny`): the same Cargo-resolved bridge,
SDK and OTLP versions now live in optional `batter-otlp`. Rechecked the published
SDK 0.31.0 ManualReader documentation and OTLP response specification; the native
force-flush and response semantics above are unchanged. Core now owns service
completion and catalog definitions; adapter configuration remains explicit.
Synchronous SDK collection/closure is not made preemptible by async deadlines.

Review repair, 2026-09-26 (`batter-i3ny`): rechecked locked reqwest 0.12.28
`src/retry.rs` and `src/async_impl/client.rs` against its
[`ClientBuilder::retry` contract](https://docs.rs/reqwest/0.12.28/reqwest/struct.ClientBuilder.html#method.retry).
The default permits two extra dispatches for protocol NACKs, including HTTP/2
`REFUSED_STREAM` and remote graceful `GOAWAY`. Another consumer can enable HTTP/2
through Cargo feature unification. The adapter therefore selects
`reqwest::retry::never()` explicitly. A native h2c `REFUSED_STREAM` regression
compares the default policy with the production policy; it does not exercise TLS
certificate validation or ALPN negotiation.

Branch review repair, 2026-09-27 (`batter-i3ny`): rechecked the primary
[OTLP/HTTP response contract](https://opentelemetry.io/docs/specs/otlp/#otlphttp-response)
and locked Tokio 1.53.1
[`Instant::checked_add`](https://docs.rs/tokio/1.53.1/tokio/time/struct.Instant.html#method.checked_add).
OTLP requires HTTP 200 for full and partial success, so the adapter rejects
other 2xx statuses before decoding. `checked_add` returns `None` if a later
deadline no longer fits; schedule validation checks representability only at
construction, so the serial owner clamps an extreme allowance at each use.

## Tokio and Tokio-util

- [JoinSet 1.53.1](https://docs.rs/tokio/1.53.1/tokio/task/struct.JoinSet.html):
  direct task ownership, ID-aware joins, abort requests, and cancellation-safe joins.
- [JoinError](https://docs.rs/tokio/latest/tokio/task/struct.JoinError.html): named
  panic/cancellation observations at the task boundary.
- [Task cancellation](https://docs.rs/tokio/latest/tokio/task/): abortion requires
  runtime progress; blocking/non-yielding behavior is a separate concern.
- [CancellationToken 0.7.19](https://docs.rs/tokio-util/0.7.19/tokio_util/sync/struct.CancellationToken.html):
  downward child cancellation and drop guards; not transitive child-task joining.
- [TaskTracker](https://docs.rs/tokio-util/latest/tokio_util/task/task_tracker/struct.TaskTracker.html):
  task lifetime tracking is not an application failure supervisor.
- [Paused time](https://docs.rs/tokio/latest/tokio/time/fn.pause.html): a runtime
  testing facility, not control of database time.

### Periodic maintenance scheduling semantics, 2026-10-08

`Cargo.lock` resolves Tokio 1.53.1, and this section records the semantics
Batter's periodic components rely on against that exact pin, read from the
installed `src/time/interval.rs` and `src/time/sleep.rs`. The versioned links
below identify those primary APIs; a browser re-fetch during the scheduling
repair was unavailable, so the installed locked source supplies that check.

- [`tokio::time::interval`](https://docs.rs/tokio/1.53.1/tokio/time/fn.interval.html)
  starts its first deadline at `Instant::now()`, so the first tick completes
  immediately, and it panics when the period is zero. `PeriodicPolicy::new`
  validates a positive, representable interval before registration. The private
  periodic schedule now owns its cadence over `sleep_until`, rather than
  delegating missed-tick policy to `Interval`.
- The default missed-tick behaviour is
  [`Burst`](https://docs.rs/tokio/1.53.1/tokio/time/enum.MissedTickBehavior.html#variant.Burst),
  which replays every missed tick. The native alternative is
  [`Skip`](https://docs.rs/tokio/1.53.1/tokio/time/enum.MissedTickBehavior.html#variant.Skip)
  for skipping overdue work, subject to its lateness tolerance below.
- `Interval::poll_tick` returns the already elapsed deadline as soon as it is
  polled, so one overdue invocation runs immediately after an overrun. Only
  then does `Skip` realign: its next deadline is
  `now + period - ((now - missed_deadline) % period)`, measured from the
  original schedule. Tokio treats a
  tick as missed only when `now` exceeds its deadline by more than five
  milliseconds; within that grace the next deadline stays
  `missed_deadline + period`.
- That five-ms tolerance can replay multiple short-period ticks. Batter's sole
  private schedule applies the phase-preserving formula at every delivered tick,
  without a tolerance. It awaits
  [`sleep_until`](https://docs.rs/tokio/1.53.1/tokio/time/fn.sleep_until.html),
  then records the next strictly future cadence point before invoking work.
  One overdue invocation remains allowed; later missed ticks are not queued.
  Remainders use full `u128` duration nanoseconds and deadline addition is
  checked. If the next point cannot be represented, the schedule stays pending
  under the existing initialization/shutdown waits instead of overflowing.
- `Sleep` cancellation drops the timer without additional cleanup. The private
  schedule changes its deadline only after that sleep completes, so losing
  `select!` while waiting consumes no tick. Native timer resolution remains:
  Tokio documents millisecond granularity, not precise sub-millisecond wakeups.

These are start-to-start semantics. They are deliberately not the
completion-plus-delay schedule that `crates/batter-core/src/health.rs` uses for
dependency sampling, and periodic runs keep `OperationContext`'s documented
cooperative boundary instead of health's late-publication rule. Neither the
interval nor the per-run deadline preempts a factory, poll or destructor that
blocks its runtime thread, and dropping a run's future proves nothing about a
remote effect it already started.

### OTLP attempt callback boundary, rechecked 2026-11-09

Owning Bead: `batter-kwfa`. `Cargo.lock` still resolves `opentelemetry-otlp`
0.31.1 and `opentelemetry-http` 0.31.0. Their downloaded registry sources and
the matching immutable upstream revisions were inspected for the export-attempt
state assessment:

- [HTTP metrics export](https://github.com/open-telemetry/opentelemetry-rust/blob/febe718fa9ad80c281a82f754e399d3ba6c40c26/opentelemetry-otlp/src/exporter/http/metrics.rs)
  encodes/builds the request before calling the client, awaits one `send_bytes`
  inline, and formats client errors into native error strings. A failure before
  dispatch supplies no custom-client observation.
- [HTTP client trait](https://github.com/open-telemetry/opentelemetry-rust/blob/285dc925f98403ff426acc70968f104dc820d4f2/opentelemetry-http/src/lib.rs)
  receives shared `&self` and a request, without an attempt ownership token.
  Exclusive attempt ownership must therefore remain in Batter's private driver;
  the callback boundary requires a checked claim rather than relying on the trait
  to enforce single dispatch.

The revisions come from each published crate's `.cargo_vcs_info.json`. This
supports the [proposed representation](../evidence/api-assessments/lifecycle.md#export-attempt-state-representation-batter-kwfa),
not a claim that the Rust redesign or its proposed regressions have been executed.

### Runledger invocation phases, 2026-10-09

Cargo.lock still resolves Tokio 1.53.1. Its
[`Instant::from_std`](https://docs.rs/tokio/1.53.1/tokio/time/struct.Instant.html#method.from_std)
and [`into_std`](https://docs.rs/tokio/1.53.1/tokio/time/struct.Instant.html#method.into_std)
wrap and unwrap the same `std::time::Instant` (the locked source stores the std
value unchanged), and outside a paused test clock `Instant::now()` reads
`std::time::Instant::now()`. The Runledger worker computes its handler deadline
with Tokio's `Instant::now()`, exposes it through `into_std`, and enforces it
with Tokio timers; the private `job_phases` bridge used by `PhasedJob` converts
it back with `from_std`, so the bridge
and native enforcement use one value on one clock, including under
[paused time](https://docs.rs/tokio/1.53.1/tokio/time/fn.pause.html). A custom
runtime that reads `std::time::Instant::now()` while a Tokio test clock is paused
uses a different clock; that is its own obligation.

Runledger's exit hooks run inside
[`catch_unwind`](https://doc.rust-lang.org/std/panic/fn.catch_unwind.html),
which catches unwinding panics but not aborting ones. Its boundary can contain
a hook panic raised while `JobInvocationOwner::drop` runs during an outer
unwind, provided the nested panic reaches that boundary before escaping the
destructor. Two probes compiled against the native source on Rust 1.98.1 confirmed
that later hooks still run and the outer panic remains catchable; their evidence is
recorded in `batter-5h5y.3`. A second panic escaping destruction during unwinding
can abort the process, as the
[`Drop` panics section](https://doc.rust-lang.org/std/ops/trait.Drop.html#panics)
warns. `catch_unwind` neither suppresses panic-hook output nor recovers from
aborting panics, including `panic = "abort"`.

### Retry attempt deadline boundaries, 2026-09-14

Cargo.lock resolves Tokio 1.53.1 and tokio-util 0.7.19. The versioned
[`select!` contract](https://docs.rs/tokio/1.53.1/tokio/macro.select.html)
states that the macro cancels its remaining branches after one completes and
that `biased;` polls ready branches from top to bottom. Batter retains its
existing cancellation, deadline, then work ordering so cancellation wins over
deadline and deadline wins over simultaneously ready completion. Because work
can become ready during its own poll, the selector does not provide hard
wall-clock preemption or reread its clock after that poll returns. The retry
layer separately rechecks cancellation of the public attempt child before it
accepts a factory value or classifies an error; it does not turn the cooperative
deadline into a post-return clock check.

Tokio 1.53.1
[`Instant::checked_add`](https://docs.rs/tokio/1.53.1/tokio/time/struct.Instant.html#method.checked_add)
returns no value when the result is outside the clock representation. Batter's
shared positive-duration validator checks this before accepting a retry attempt
maximum. The later per-attempt derivation uses checked arithmetic and the
already representable input deadline wins if the requested instant cannot be
represented.

Tokio-util 0.7.19 documents that a
[`child_token`](https://docs.rs/tokio-util/0.7.19/tokio_util/sync/struct.CancellationToken.html#method.child_token)
is cancelled with its parent while cancellation of the child does not travel
upward. Its
[`drop_guard`](https://docs.rs/tokio-util/0.7.19/tokio_util/sync/struct.CancellationToken.html#method.drop_guard)
cancels that token and its descendants on drop unless disarmed. Retry attempt
caps reuse `OperationContext::run`, so the selected attempt future is destroyed
and its child token is cancelled without claiming that arbitrary spawned tasks
were joined.

### Service and finite-command examples: 2026-09-10

Rechecked Cargo.lock and the cached Tokio 1.53.1 source for the native primitives
used by `batter-7r3.7`; no version or feature change was needed.

- [Semaphore::acquire_owned](https://docs.rs/tokio/1.53.1/tokio/sync/struct.Semaphore.html#method.acquire_owned)
  consumes an `Arc` and yields an owned permit. Its native Drop returns capacity;
  the startup example keeps that permit in a finalizer until the service joins.
- [UdpSocket](https://docs.rs/tokio/1.53.1/tokio/net/struct.UdpSocket.html)
  supports native bind/send/receive and shared `Arc` ownership through `&self`
  methods. The finite command retains a socket owner for explicit cleanup;
  closing that native socket requires no async close method. This is a local
  resource-lifetime example, not a database/session termination analogue.



## Canonical adapter future size and depth, 2026-09-28

Rechecked for `batter-3q3` against the selected sources.

- SQLx 0.9.0 [`Pool::acquire`](https://github.com/launchbadge/sqlx/blob/v0.9.0/sqlx-core/src/pool/mod.rs)
  returns an unerased `impl Future + 'static`, while the pool
  [`Executor` implementation](https://github.com/launchbadge/sqlx/blob/v0.9.0/sqlx-core/src/pool/executor.rs)
  erases acquisition plus the query as one `BoxFuture`. Batter mirrors that:
  one erasure per operation, never one per internal layer. The unerased
  acquisition measured 2,528 bytes unoptimized and 8,608 optimized on Rust
  1.98.1 (macOS arm64), and was copied through every enclosing boundary.
- `AsyncFnOnce` is [stable since 1.85.0](https://github.com/rust-lang/rust/blob/1.98.1/library/core/src/ops/async_function.rs),
  but its `CallOnceFuture` and `Output` associated types remain unstable
  (`async_fn_traits`). Stable code cannot name or bound a callback's future, so a
  generic runner cannot coerce it into `dyn Future + Send`; a concretely typed
  `Pin<Box<F>>` keeps inferred auto traits. Nameable futures (native checks,
  quota work, native admission, pooled queries) use `dyn Future + Send`.
- The Reference's [`recursion_limit`](https://doc.rust-lang.org/reference/attributes/limits.html#the-recursion_limit-attribute)
  default in rustc is 128. Rust 1.98.1 reports `queries overflow the depth limit`
  with the query depth reached while computing a coroutine layout, and E0275 for
  auto-trait proofs, both against that limit. A heap pointer ends the layout
  query; only `dyn` erasure ends an auto-trait proof. At the pinned base the
  committed composed-handler consumer reached layout depth 130; it now compiles
  at limit 64 (probed in steps of 8), unoptimized and optimized.
- [`std::thread`](https://doc.rust-lang.org/std/thread/#stack-size) documents a
  2 MiB default for spawned threads on Tier-1 platforms, overridden by
  `RUST_MIN_STACK`; libtest runs each test on such a thread. A bare
  [`#[tokio::test]`](https://docs.rs/tokio-macros/2.7.2/tokio_macros/attr.test.html)
  builds a current-thread runtime whose `block_on` polls on that test thread.
  That consumer, compiled at the pinned base with a raised limit, needed more
  than 1 MiB of this stack unoptimized; it now runs in at most 320 KiB
  unoptimized and 96 KiB optimized, including the complete HTTP boundary.

## Async startup initializers and boxed errors, 2026-10-11

Rechecked for `batter-syah` against the selected sources and Rust 1.98.1.

- [RFC 3668](https://rust-lang.github.io/rfcs/3668-async-closures.html) states that
  async closures "unconditionally implement the (non-async) `FnOnce` trait" and
  always implement `AsyncFnOnce`. The built-in `AsyncFn*` implementations for
  other callables apply only to concrete callable types, because blanket impls
  would overlap the async-closure implementations. The `AsyncFn*` associated
  types are reserved implementation details and not nameable on stable.
  `Initializer` therefore states the `Send` future through a sealed higher-ranked
  `FnOnce(&'a mut ProtectedStartupScope) -> Fut` helper. Only the async-closure
  constructor requires `AsyncFnOnce`, which drives argument inference for an
  unannotated closure. On Rust 1.98.1 a generic `F` bounded only by that `FnOnce`
  form did not satisfy `AsyncFnOnce`, while concrete async closures, `async fn`
  items and boxed `StartupFuture` closures satisfied the helper.
- The Reference's [orphan rules](https://doc.rust-lang.org/reference/items/implementations.html#orphan-rules)
  let only the defining crate's future versions add impls for its foreign types.
  On Rust 1.98.1, an `Error` impl for `W<Box<dyn Error + Send + Sync>>` or
  `W<anyhow::Error>` beside `impl<E: Error> Error for W<E>` failed with E0119:
  "upstream crates may add a new impl of trait `std::error::Error`". The same
  pair compiles for a local type that does not implement `Error`, which is why
  `ApplicationError` deliberately omits it.
- anyhow 1.0.104's [`error.rs`](https://docs.rs/anyhow/1.0.104/src/anyhow/error.rs.html)
  implements `From<anyhow::Error> for Box<dyn StdError + Send + Sync>` through
  `object_boxed`, which boxes the private `ErrorImpl<E>` rather than `E`. Its
  `StdError::source` delegates to the inner error's `source`, so the head error
  is neither a direct downcast of that box nor in its source chain; its message
  remains available through `Display`. `typed_startup_consumer` pins this.

## Finite command signals and owned completion (2026-10-31)

Owning Bead: `batter-od4b`. Rechecked against the locked Tokio 1.53.1 sources in
the Cargo registry; no dependency version changed.

- Tokio's [signal registry](https://docs.rs/tokio/1.53.1/src/tokio/signal/registry.rs.html)
  keeps one `watch::Sender<()>` and one pending flag per signal number.
  `register_listener` returns `tx.subscribe()`, whose receiver starts at the
  current version, so it sees only later broadcasts; `record_event` sets the
  pending flag; `broadcast` swaps each set flag and sends to every subscriber,
  ignoring a send with no receivers. A [`Signal`](https://docs.rs/tokio/1.53.1/src/tokio/signal/mod.rs.html)
  owns its receiver through `RxFuture`, so dropping it removes the listener.
- In [`signal/unix.rs`](https://docs.rs/tokio/1.53.1/src/tokio/signal/unix.rs.html)
  the process-level handler records the event and writes one byte to a global
  pipe. `signal_enable` registers that handler once per signal through
  `signal_hook_registry` and never unregisters it, and returns
  `"signal driver gone"` from `check_inner` when the runtime's driver is gone.
  `signal` itself calls `Handle::current()`, which panics outside a runtime, and
  [`driver().signal()`](https://docs.rs/tokio/1.53.1/src/tokio/runtime/driver.rs.html),
  which panics with "there is no signal driver running" without the I/O driver.
  The `Signal` documentation states that, even after the listener is dropped,
  later deliveries stay captured and the default behaviour is not reset. After a
  command drops its listeners, a SIGTERM or SIGINT is therefore recorded and
  broadcast to no receiver: consumed without effect. A failed SIGINT
  installation cannot roll back SIGTERM's handler.
- Each runtime's [signal driver](https://docs.rs/tokio/1.53.1/src/tokio/runtime/signal/mod.rs.html)
  registers its own `try_clone` duplicate of the one global pipe receiver,
  expecting at least one duplicate to see a wakeup; `process` drains it and
  calls `globals().broadcast()`, which reaches every listener in the process,
  whichever runtime created it. That is why the `command_signals` oracle runtime
  can make a signal visible to a command whose runtime is blocked, and why a
  signal arriving between two signal-owning commands reaches the second only if
  a driver broadcasts after its listener subscribed.
- [`sleep_until`](https://docs.rs/tokio/1.53.1/src/tokio/time/sleep.rs.html)
  needs the time driver and otherwise panics where the sleep is created; for a
  command that is inside its coordinator task, which `finish` reports as
  `CommandFailure::Coordinator`.
- [`JoinHandle`](https://docs.rs/tokio/1.53.1/src/tokio/runtime/task/join.rs.html)
  detaches its task when dropped. In the [task harness](https://docs.rs/tokio/1.53.1/src/tokio/runtime/task/harness.rs.html),
  `complete` drops the output itself when join interest is already unset, and
  `drop_join_handle_slow` drops a completed output when the handle drops, under
  `catch_unwind`; neither runs inside the task's future, so a dispatcher wrapper
  around that future does not cover the output's destruction. The private
  `ScopedOutput` guard therefore carries the coordinator's dispatcher and span
  into the output. Runtime shutdown calls `shutdown`, whose `cancel_task` drops
  the future and stores `JoinError::cancelled` (or a panic error if that drop
  panicked); an owner awaiting from another runtime receives that error, which
  `finish` returns as `CommandFailure::Coordinator` instead of panicking.


## VictoriaMetrics v1.156.0: reviewed 2026-11-09

Owning Bead: `batter-cw5f`. The upstream release API selected
[v1.156.0](https://github.com/VictoriaMetrics/VictoriaMetrics/releases/tag/v1.156.0).
Local Docker inspection pulled
`victoriametrics/victoria-metrics:v1.156.0` at digest
`sha256:da8a90ac0912f16589be2b96a6441a2fca4bca415c83783d972d92e3b5b66970`;
the live regression pins both tag and digest.

- [OTLP integration](https://docs.victoriametrics.com/victoriametrics/integrations/opentelemetry/)
  specifies HTTP Protobuf ingestion at `/opentelemetry/v1/metrics`, recommends
  cumulative temporality and promotes resource attributes to labels by default.
  Batter already uses cumulative snapshots and fixed explicit histogram bounds.
- [Collector configuration](https://docs.victoriametrics.com/victoriametrics/data-ingestion/opentelemetry-collector/)
  documents single-node port 8428 and the cluster path
  `/insert/<accountID>/opentelemetry/v1/metrics` on vminsert port 8480.
- [v1.156.0 handler](https://github.com/VictoriaMetrics/VictoriaMetrics/blob/v1.156.0/app/vminsert/main.go)
  calls `opentelemetry.InsertHandler` and `firehose.WriteSuccessResponse`.
  [Insertion](https://github.com/VictoriaMetrics/VictoriaMetrics/blob/v1.156.0/app/vminsert/opentelemetry/request_handler.go)
  parses Protobuf, converts time series and flushes its insert buffer.
  A local wire probe observed HTTP 200, zero response bytes and no Content-Type.
  Consequently only Batter's explicit VictoriaMetrics profile accepts that
  empty/untyped success; ordinary OTLP remains strict. This is response evidence,
  not proof of durable storage or a claim about every VictoriaMetrics release.

The live regression separately queries counters, explicit histogram buckets,
sums/counts, cleanup and shutdown metrics for two distinct instance IDs. Cluster,
authentication and non-Linux execution have not been exercised by this evidence.

Cleanup follow-up (`batter-pdfk`, 2026-11-09):
[Docker CLI v29.7.2 removal source](https://github.com/docker/cli/blob/v29.7.2/cli/command/container/rm.go)
submits removal directly and ignores typed NotFound only when force is selected;
other errors are joined and returned. A local client/server 29.7.2 probe of
`docker rm --force --volumes` on a random absent name returned exit 0 with empty
stdout/stderr. The fixture therefore uses checked forced removal without a
fallible listing or string-based absence classification. Successful absence at
that instant is not proof that an uncertain daemon operation has settled.


For the same fixture follow-up, Python 3.14.7 is the executed interpreter.
[Python 3.14 subprocess documentation](https://docs.python.org/3.14/library/subprocess.html#subprocess.Popen)
specifies that `start_new_session=True` invokes `setsid()` before executing a
POSIX child. Bounded diagnostic/removal commands use it to keep terminal/process-
group stop signals from reaching the cleanup CLI. A real subprocess surrogate
regression sends signals to the harness group while that child is active and
confirms the child completes. This does not cover SIGKILL, daemon failure or
arbitrary descendants outside the owned process boundary.


[Python urllib timeout semantics](https://docs.python.org/3.14/library/urllib.request.html#urllib.request.urlopen)
apply the timeout to blocking operations, not the aggregate response lifetime.
The executed Python 3.14.7 parser can therefore keep reading progressively
arriving headers/body without an idle timeout. The polling helper runs in a
parent-owned child with a complete-attempt timeout and bounded stop/reap; real
loopback progressing-response regressions verify this boundary (`batter-pdfk`).
