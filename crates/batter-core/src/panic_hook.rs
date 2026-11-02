use std::{
    fs::File,
    io::{self, Cursor, Write},
    os::fd::AsFd,
    panic::PanicHookInfo,
};

/// Bytes rendered on the stack so that one write carries the whole line.
const LINE_CAPACITY: usize = 512;

/// Construct a panic hook that reports where a panic happened, never what it carried.
///
/// Batter never installs a panic hook. Every binary installs this one itself,
/// unwrapped, as the first statement of `main`: each service, operator command
/// and mode, before argument parsing, configuration loading, tracing
/// initialization or mode dispatch.
///
/// ```no_run
/// #[tokio::main]
/// async fn main() {
///     std::panic::set_hook(batter_core::redacting_panic_hook());
///     // Parse arguments, load configuration and initialize tracing after it.
/// }
/// ```
///
/// `#[tokio::main]` builds the runtime before the first statement of the
/// body, so Rust's default hook still prints a panic raised while building it,
/// such as Tokio's message for an invalid `TOKIO_WORKER_THREADS` value. When
/// that output matters, install the hook in a synchronous `main` and build the
/// runtime after it. Never install it in library code, in in-process tests, or
/// in an initialization helper that tests also call: the hook is process-wide,
/// its line bypasses libtest's output capture, and libtest prints a plain
/// failing test's panic message, such as an assertion's, only through the
/// default hook, for every test in that binary; it prints a
/// `#[should_panic(expected = ...)]` mismatch's payload itself.
///
/// A discarded hook is a compile-time warning, rejected here as an error:
///
/// ```compile_fail
/// #![deny(unused_must_use)]
/// fn main() {
///     batter_core::redacting_panic_hook();
/// }
/// ```
///
/// For every panic the hook writes one line to standard error, after a newline
/// so that it starts on a line of its own:
///
/// ```text
/// a thread panicked at src/main.rs:12:5; panic payload redacted
/// ```
///
/// That text is part of this contract: alerts may match `panicked at`, and a
/// change to it is recorded in the changelog. The hook never reads, writes or
/// forwards the payload. Rust formats a `panic!` message's arguments, or the
/// `Debug` of an error passed to `unwrap` or `expect`, into the payload before
/// any hook runs; that payload is what `catch_unwind`, a thread join, a Tokio
/// `JoinError` and Batter reports retain, and this hook never writes it. The
/// location is the one the panic reports: the caller of a `#[track_caller]`
/// function such as `unwrap`, and a dependency's own line for a panic inside
/// that dependency. The hook prints no thread name, because reading one can
/// panic while a thread is being torn down, and no backtrace, whatever
/// `RUST_BACKTRACE` says. It calls neither the previous nor the default hook,
/// emits no `tracing` event and returns normally, so unwinding, `catch_unwind`,
/// joins, `JoinError`s and the exit status 101 of an uncaught panic in `main`
/// are unchanged.
///
/// The line goes through a duplicate of the standard error descriptor, in one
/// write when it fits in 512 bytes, and never through the [`std::io::Stderr`]
/// lock, which another thread can hold indefinitely; Rust's default hook avoids
/// that lock too. When the descriptor cannot be duplicated, for example at the
/// process descriptor limit, the hook writes nothing. Write errors are
/// ignored, and no thread state is read, so the hook itself does not panic.
///
/// # Limits
///
/// The hook covers payload printing at panic time only. A value cannot prove
/// that a binary installed it, installed it early enough or kept it:
/// `std::panic` can replace the process hook but cannot report which one is
/// installed.
///
/// - Keep it unwrapped; Batter provides no wrapper and no `tracing` variant. A
///   wrapper or a later hook, the application's own or a dependency
///   initializer's such as a crash reporter, discloses the payload again when
///   it formats, logs or sends [`PanicHookInfo`], whose `Display` includes a
///   string payload, or the payload itself, or when it calls the default hook.
///   The common chain of [`std::panic::take_hook`] around a `tracing` event
///   writes the payload to the log while this hook's line still says
///   `redacted`. A later [`std::panic::set_hook`] replaces this hook, and
///   between `take_hook` and the next `set_hook` the default hook is installed.
///   A hook that only restores state, such as a terminal, and then calls this
///   one keeps the guarantee.
/// - Wrapper code that can itself panic aborts the process: Rust cannot unwind
///   a panic raised while a hook runs, and `catch_unwind` inside the hook does
///   not help. `eprintln!` to a closed pipe, `std::thread::current()` during
///   thread teardown and a `tracing` subscriber that reports its own write
///   failure with `eprintln!` all panic that way.
/// - Rust aborts when a panic occurs while it prepares or runs a hook,
///   including while it formats a `panic!` message's arguments; it then prints
///   that nested panic's location, and its message only when the message is a
///   constant string, but not the original payload. In a child that
///   [`std::process::Command`] forks, Rust disables hooks before running
///   [`CommandExt::pre_exec`](std::os::unix::process::CommandExt::pre_exec)
///   closures and `exec`, so a panic there prints the full payload without
///   consulting any hook.
/// - Tokio's `JoinError` formats a string payload in its `Debug` and `Display`.
///   Payloads retained in Batter reports, such as
///   [`PanicPayload`](crate::PanicPayload), stay reachable through their
///   explicit fields and [`PanicPayload::try_inspect`](crate::PanicPayload::try_inspect).
/// - Closing the duplicate after each report releases the process-owned POSIX
///   `fcntl` record locks the process holds on the file behind standard error,
///   as closing any descriptor for that file does; open-file-description locks
///   are unaffected.
/// - The hook does not sanitize the process: logs, other output and error
///   formatting keep their own disclosure behaviour.
#[must_use = "the hook does nothing until the application root installs it"]
pub fn redacting_panic_hook() -> Box<dyn Fn(&PanicHookInfo<'_>) + Send + Sync + 'static> {
    Box::new(|info: &PanicHookInfo<'_>| {
        let site = info
            .location()
            .map(|location| Site::new(location.file(), location.line(), location.column()));
        report(site);
    })
}

/// The only panic facts the hook renders. The payload has no way in.
#[derive(Clone, Copy)]
struct Site<'a> {
    file: &'a str,
    line: u32,
    column: u32,
}

impl<'a> Site<'a> {
    fn new(file: &'a str, line: u32, column: u32) -> Self {
        Self { file, line, column }
    }
}

/// Write through a duplicate of descriptor 2, never through the `Stderr`
/// lock; without a descriptor to spare, write nothing.
fn report(site: Option<Site<'_>>) {
    if let Ok(descriptor) = io::stderr().as_fd().try_clone_to_owned() {
        emit(&mut File::from(descriptor), site);
    }
}

/// Write the line with one `write_all` when it fits on the stack, otherwise
/// directly in several writes. Errors are ignored.
fn emit(output: &mut dyn Write, site: Option<Site<'_>>) {
    let mut line = [0_u8; LINE_CAPACITY];
    let mut cursor = Cursor::new(&mut line[..]);
    let _ = match render(&mut cursor, site) {
        Ok(()) => {
            let rendered = usize::try_from(cursor.position()).unwrap_or(LINE_CAPACITY);
            output.write_all(&line[..rendered])
        }
        Err(_) => render(output, site),
    };
}

fn render(output: &mut dyn Write, site: Option<Site<'_>>) -> io::Result<()> {
    match site {
        Some(Site { file, line, column }) => write!(
            output,
            "\na thread panicked at {file}:{line}:{column}; panic payload redacted\n"
        ),
        None => output
            .write_all(b"\na thread panicked at an unknown location; panic payload redacted\n"),
    }
}

#[cfg(test)]
mod tests;
