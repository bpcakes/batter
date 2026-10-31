//! Deterministic coordinator tests with an injected signal source.
//!
//! A test-owned [`Injected`] source stands in for the native listeners: each
//! `()` sent on its paired sender is one delivery, and that sender reports
//! `is_closed` once the coordinator has dropped the source. Nothing here
//! installs a process-wide signal disposition.

mod checked;
mod delivery;
mod removal;
mod unclaimed;

use super::{
    Command, CommandFailure, CommandFuture, CommandOwner, CommandScope, CommandSuccess,
    SharedCommandReport,
    signals::{self, WorkSignals},
};
use crate::{cleanup::CleanupBudget, operation::OperationOwner};
use std::{
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::mpsc;

/// Ordered evidence shared by drop markers, cleanup hooks and the source.
type Log = Arc<Mutex<Vec<&'static str>>>;

const LISTENERS_DROPPED: &str = "listeners-dropped";

fn push(log: &Log, entry: &'static str) {
    log.lock().unwrap().push(entry);
}

fn entries(log: &Log) -> Vec<&'static str> {
    log.lock().unwrap().clone()
}

/// A stop-signal source driven by the test instead of the kernel.
struct Injected {
    deliveries: mpsc::UnboundedReceiver<()>,
    log: Log,
}

impl WorkSignals for Injected {
    fn poll_received(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        match self.deliveries.poll_recv(cx) {
            Poll::Ready(Some(())) => Poll::Ready(()),
            // With every sender gone no delivery can arrive.
            Poll::Ready(None) | Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for Injected {
    fn drop(&mut self) {
        push(&self.log, LISTENERS_DROPPED);
    }
}

/// A fresh source, its delivery handle and the log its removal is written to.
fn injected() -> (Injected, mpsc::UnboundedSender<()>, Log) {
    let (sender, deliveries) = mpsc::unbounded_channel();
    let log = Log::default();
    let source = Injected {
        deliveries,
        log: log.clone(),
    };
    (source, sender, log)
}

/// Writes its entry to the log when destroyed.
struct Marker {
    log: Log,
    entry: &'static str,
}

impl Marker {
    fn new(log: &Log, entry: &'static str) -> Self {
        Self {
            log: log.clone(),
            entry,
        }
    }
}

impl Drop for Marker {
    fn drop(&mut self) {
        push(&self.log, self.entry);
    }
}

fn budget() -> CleanupBudget {
    let second = Duration::from_secs(1);
    CleanupBudget::new(second, second, second).unwrap()
}

/// A live parent whose budget no test reaches; deadlines here are failure guards.
fn parent() -> OperationOwner {
    OperationOwner::new(Duration::from_secs(60)).unwrap()
}

/// Start through the one signal start path with the injected source installed.
fn start_injected<F, T, E>(command: Command<F>, source: Injected) -> CommandOwner<T, E>
where
    F: for<'a> FnOnce(&'a mut CommandScope) -> CommandFuture<'a, T, E> + Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
{
    match signals::start_with(command, move || Ok(source)) {
        Ok(owner) => owner,
        Err(error) => panic!("injected installation cannot fail: {error}"),
    }
}

/// The listeners were removed, and removed once.
fn assert_removed_once(sender: &mpsc::UnboundedSender<()>, log: &Log) {
    assert!(sender.is_closed(), "listeners outlived the command's work");
    let removals = entries(log)
        .into_iter()
        .filter(|entry| *entry == LISTENERS_DROPPED)
        .count();
    assert_eq!(removals, 1, "{:?}", entries(log));
}

fn failed_report<T, E>(
    result: Result<CommandSuccess<T>, CommandFailure<T, E>>,
) -> SharedCommandReport<T, E> {
    match result {
        Err(CommandFailure::Report(report)) => report,
        Err(CommandFailure::Coordinator(_)) => panic!("coordinator ended without a report"),
        Ok(success) => panic!("command unexpectedly succeeded: {success}"),
    }
}

fn succeeded<T, E>(result: Result<CommandSuccess<T>, CommandFailure<T, E>>) -> CommandSuccess<T> {
    match result {
        Ok(success) => success,
        Err(failure) => panic!("command failed: {failure}"),
    }
}
