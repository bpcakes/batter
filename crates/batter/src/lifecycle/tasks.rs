//! Direct-task ownership, join accounting and escalation bookkeeping.

#[cfg(test)]
mod tests;

use super::{
    Component, ShutdownCause, ShutdownHandle, TaskOutcome, TaskRecord, process, receive_process,
};
use crate::{BoxError, scoped_dispatch};
use std::{
    collections::HashMap,
    sync::{Arc, atomic::AtomicBool},
};
use tokio::{
    sync::mpsc,
    task::{AbortHandle, Id, JoinError, JoinSet},
    time::Instant,
};
use tracing::Instrument;

pub(super) struct TaskExit {
    pub(super) result: Result<(), BoxError>,
    pub(super) expected: bool,
}

struct TaskMetadata {
    name: &'static str,
    finite: bool,
    abort: AbortHandle,
}

type TaskResult = Result<(Id, TaskExit), JoinError>;

#[derive(Default)]
pub(super) struct TaskSet {
    set: JoinSet<TaskExit>,
    names: HashMap<Id, TaskMetadata>,
    records: Vec<TaskRecord>,
    completed: u64,
}

impl TaskSet {
    pub(super) fn spawn_component(&mut self, component: Component, handle: &ShutdownHandle) {
        let name = component.name;
        let mut signal = handle.signal();
        signal.startup = Some(Arc::new(AtomicBool::new(false)));
        let handle = handle.clone();
        let span = tracing::info_span!(target: "batter", "batter.task", task = name).or_current();
        let abort = self.set.spawn(scoped_dispatch::scope(
            async move {
                let result = (component.factory)(signal).await;
                // Capture the state at completion, never at delayed observation.
                let expected = handle.is_draining();
                TaskExit { result, expected }
            }
            .instrument(span),
        ));
        self.names.insert(
            abort.id(),
            TaskMetadata {
                name,
                finite: false,
                abort,
            },
        );
    }

    pub(super) fn spawn_process(&mut self, task: process::QueuedProcess) {
        let name = task.name;
        // The future already carries the submitting operation's span and
        // subscriber, independent of the coordinator's tracing context.
        let abort = self.set.spawn(task.future);
        self.names.insert(
            abort.id(),
            TaskMetadata {
                name,
                finite: true,
                abort,
            },
        );
    }

    fn sorted_names(&self) -> Vec<&'static str> {
        let mut values: Vec<_> = self.names.values().map(|entry| entry.name).collect();
        values.sort_unstable();
        values
    }

    fn pending(&self, handle: &ShutdownHandle) -> bool {
        !self.set.is_empty() || handle.shared.has_finite_tasks()
    }

    pub(super) fn unfinished(&self, handle: &ShutdownHandle) -> bool {
        self.names.values().any(|task| !task.abort.is_finished())
            || handle.shared.has_finite_tasks()
    }

    // Membership means a result remains unobserved, not necessarily that the
    // task is still running. Preserve uncertainty only for actual abort requests.
    pub(super) fn abort_unfinished(&self) -> Vec<&'static str> {
        let mut requested = Vec::new();
        for task in self.names.values() {
            if !task.abort.is_finished() {
                task.abort.abort();
                requested.push(task.name);
            }
        }
        requested.sort_unstable();
        requested
    }

    // None means a successful finite completion or expected critical stop.
    // Actual failures are retained and initiate shutdown before drain.
    fn record(&mut self, result: TaskResult) -> Option<ShutdownCause> {
        let (id, mut outcome, error) = classify_task_result(result);
        let TaskMetadata { name, finite, .. } = self
            .names
            .remove(&id)
            .expect("every owned task has metadata");
        if finite && outcome == TaskOutcome::Stopped {
            outcome = TaskOutcome::Completed;
            self.completed = self.completed.saturating_add(1);
            tracing::debug!(target: "batter", task = name, ?outcome, "process task exit observed");
            return None;
        }
        let record = TaskRecord {
            name,
            outcome,
            error,
        };
        record.log_observation();
        self.records.push(record);
        if outcome == TaskOutcome::Stopped {
            None
        } else if finite {
            Some(ShutdownCause::FiniteTaskExit(name))
        } else {
            Some(ShutdownCause::ComponentExit(name))
        }
    }

    fn record_exit(
        &mut self,
        result: TaskResult,
        handle: &ShutdownHandle,
    ) -> Option<ShutdownCause> {
        let cause = self.record(result);
        if cause.is_some() {
            handle.shared.fail_task();
        }
        cause
    }

    pub(super) fn collect_ready(&mut self, handle: &ShutdownHandle) {
        // This nonblocking API can observe completed tasks even when Tokio's
        // cooperative poll budget is exhausted. Only this coordinator adds to
        // the JoinSet, so harvesting the currently spawned set is bounded.
        while let Some(result) = self.set.try_join_next_with_id() {
            self.record_exit(result, handle);
        }
    }

    pub(super) async fn collect_until(
        &mut self,
        queued: &mut Option<mpsc::Receiver<process::QueuedProcess>>,
        handle: &ShutdownHandle,
        deadline: Instant,
    ) {
        while self.pending(handle) {
            tokio::select! {
                biased;
                _ = tokio::time::sleep_until(deadline) => break,
                _ = self.next_exit(handle), if !self.is_empty() => {}
                Some(task) = receive_process(queued) => self.spawn_process(task),
            }
        }
        // Expired allowances stop waiting, not observation of results that are
        // already available. Reconcile before escalation or final reporting.
        self.collect_ready(handle);
    }

    pub(super) fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    /// Join and record in the same poll, before the coordinator sees a cause.
    /// Cancelling a pending wait cannot discard a consumed task result.
    pub(super) async fn next_exit(&mut self, handle: &ShutdownHandle) -> Option<ShutdownCause> {
        let result = self.set.join_next_with_id().await?;
        self.record_exit(result, handle)
    }

    /// Release task ownership before the caller can start dependency cleanup.
    pub(super) fn finish(self) -> TaskSummary {
        let unjoined = self.sorted_names();
        let unsafe_exit = !unjoined.is_empty()
            || self.records.iter().any(|record| {
                matches!(record.outcome, TaskOutcome::Panicked | TaskOutcome::Aborted)
            });
        TaskSummary {
            records: self.records,
            completed: self.completed,
            unjoined,
            unsafe_exit,
        }
    }
}

pub(super) struct TaskSummary {
    pub(super) records: Vec<TaskRecord>,
    pub(super) completed: u64,
    pub(super) unjoined: Vec<&'static str>,
    pub(super) unsafe_exit: bool,
}

fn classify_task_result(result: TaskResult) -> (Id, TaskOutcome, Option<BoxError>) {
    match result {
        Ok((
            id,
            TaskExit {
                result: Ok(()),
                expected,
            },
        )) => (
            id,
            if expected {
                TaskOutcome::Stopped
            } else {
                TaskOutcome::UnexpectedExit
            },
            None,
        ),
        Ok((
            id,
            TaskExit {
                result: Err(error), ..
            },
        )) => (id, TaskOutcome::Failed, Some(error)),
        Err(error) => {
            let outcome = if error.is_panic() {
                TaskOutcome::Panicked
            } else {
                TaskOutcome::Aborted
            };
            (error.id(), outcome, Some(Box::new(error) as BoxError))
        }
    }
}
