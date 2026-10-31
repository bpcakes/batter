//! Library-owned SIGTERM/SIGINT listeners for one command's work phase.

use super::{Command, CommandFuture, CommandOwner, CommandScope, driver};
use std::{
    io,
    task::{Context, Poll},
};
use tokio::signal::unix::{Signal, SignalKind, signal};

/// A stop-signal source polled only by the command coordinator while work can
/// still be cancelled. The coordinator stops polling after the first `Ready`.
pub(super) trait WorkSignals {
    fn poll_received(&mut self, cx: &mut Context<'_>) -> Poll<()>;
}

/// The unsignalled paths: never ready, registers no waker, installs nothing.
pub(super) struct NoSignals;

impl WorkSignals for NoSignals {
    fn poll_received(&mut self, _: &mut Context<'_>) -> Poll<()> {
        Poll::Pending
    }
}

/// Native listeners. Dropping them removes both listeners; Tokio never
/// restores the process-wide disposition it replaced at installation.
pub(super) struct UnixStopSignals {
    terminate: Signal,
    interrupt: Signal,
}

impl UnixStopSignals {
    /// SIGTERM, then SIGINT, as protected startup installs them. A failed
    /// SIGINT installation drops the SIGTERM listener without rollback.
    pub(super) fn install() -> io::Result<Self> {
        let terminate = signal(SignalKind::terminate())?;
        let interrupt = signal(SignalKind::interrupt())?;
        Ok(Self {
            terminate,
            interrupt,
        })
    }
}

impl WorkSignals for UnixStopSignals {
    fn poll_received(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        // Poll both so each registers the waker; either one stops work.
        let terminate = self.terminate.poll_recv(cx).is_ready();
        let interrupt = self.interrupt.poll_recv(cx).is_ready();
        if terminate || interrupt {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

/// The only signal start path, with an injectable installer for tests.
/// Nothing is installed for a context that is already interrupted, because
/// no work can run; the coordinator then records that interruption.
pub(super) fn start_with<F, T, E, S>(
    command: Command<F>,
    install: impl FnOnce() -> io::Result<S>,
) -> io::Result<CommandOwner<T, E>>
where
    F: for<'a> FnOnce(&'a mut CommandScope) -> CommandFuture<'a, T, E> + Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    S: WorkSignals + Send + 'static,
{
    if command.context.check().is_err() {
        return Ok(driver::start_owned(command, NoSignals));
    }
    // On error the unstarted command, and with it the factory, drops here.
    let signals = install()?;
    Ok(driver::start_owned(command, signals))
}
