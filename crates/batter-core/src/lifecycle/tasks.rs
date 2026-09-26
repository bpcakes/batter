//! Direct-task ownership, join accounting and escalation bookkeeping.

#[cfg(test)]
mod tests;

use super::{
    Component, LifecycleCoordinator, RegisteredComponent, ShutdownCause, TaskOutcome, TaskRecord,
    process, receive_process,
};
use crate::{BoxError, scoped_dispatch};
use std::collections::HashMap;
use tokio::{
    sync::mpsc,
    task::{AbortHandle, Id, JoinError, JoinSet},
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
    pub(super) fn spawn_component(&mut self, component: Component) {
        let Component {
            name,
            lifecycle:
                RegisteredComponent {
                    startup,
                    coordinator,
                },
            factory,
        } = component;
        let classification = coordinator.clone();
        let span = tracing::info_span!(target: "batter", "batter.task", task = name).or_current();
        let abort = self.set.spawn(scoped_dispatch::scope(
            async move {
                let result = factory(startup, coordinator).await.map(|_exit| ());
                // Capture the state at completion, never at delayed observation.
                let expected = classification.is_draining();
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

    fn pending(&self, coordinator: &LifecycleCoordinator) -> bool {
        !self.set.is_empty() || coordinator.shared.has_finite_tasks()
    }

    pub(super) fn unfinished(&self, coordinator: &LifecycleCoordinator) -> bool {
        self.names.values().any(|task| !task.abort.is_finished())
            || coordinator.shared.has_finite_tasks()
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

    // A `None` cause means a successful finite completion or expected critical stop.
    // Actual failures are retained and initiate shutdown before drain.
    fn record(&mut self, result: TaskResult) -> RecordedExit {
        let (id, mut outcome, error) = classify_task_result(result);
        let TaskMetadata { name, finite, .. } = self
            .names
            .remove(&id)
            .expect("every owned task has metadata");
        if finite && outcome == TaskOutcome::Stopped {
            outcome = TaskOutcome::Completed;
        }
        let exit = |cause| RecordedExit {
            cause,
            name,
            finite,
            outcome,
        };
        if outcome == TaskOutcome::Completed {
            self.completed = self.completed.saturating_add(1);
            tracing::debug!(target: "batter", task = name, ?outcome, "process task exit observed");
            return exit(None);
        }
        let record = TaskRecord {
            name,
            outcome,
            error,
        };
        record.log_observation();
        self.records.push(record);
        exit(if outcome == TaskOutcome::Stopped {
            None
        } else if finite {
            Some(ShutdownCause::FiniteTaskExit(name))
        } else {
            Some(ShutdownCause::ComponentExit(name))
        })
    }

    fn record_exit(
        &mut self,
        result: TaskResult,
        coordinator: &LifecycleCoordinator,
    ) -> Option<ShutdownCause> {
        let exit = self.record(result);
        if exit.cause.is_some() {
            coordinator.shared.fail_task();
        }
        // Recorder code runs only after a failure has closed admission.
        #[cfg(feature = "metrics")]
        crate::telemetry::metrics::task(exit.finite, exit.name, exit.outcome);
        exit.cause
    }

    pub(super) fn collect_ready(&mut self, coordinator: &LifecycleCoordinator) {
        // This nonblocking API can observe completed tasks even when Tokio's
        // cooperative poll budget is exhausted. Only this coordinator adds to
        // the JoinSet, so harvesting the currently spawned set is bounded.
        while let Some(result) = self.set.try_join_next_with_id() {
            self.record_exit(result, coordinator);
        }
    }

    pub(super) async fn collect_until(
        &mut self,
        queued: &mut Option<mpsc::Receiver<process::QueuedProcess>>,
        coordinator: &LifecycleCoordinator,
        allowance: std::time::Duration,
    ) {
        while self.pending(coordinator) {
            tokio::select! {
                biased;
                _ = coordinator.shared.phase_elapsed(allowance) => break,
                _ = self.next_exit(coordinator), if !self.is_empty() => {}
                Some(task) = receive_process(queued) => self.spawn_process(task),
            }
        }
        // Expired allowances stop waiting, not observation of results that are
        // already available. Reconcile before escalation or final reporting.
        self.collect_ready(coordinator);
    }

    pub(super) fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    /// Join and record in the same poll, before the coordinator sees a cause.
    /// Cancelling a pending wait cannot discard a consumed task result.
    pub(super) async fn next_exit(
        &mut self,
        coordinator: &LifecycleCoordinator,
    ) -> Option<ShutdownCause> {
        let result = self.set.join_next_with_id().await?;
        self.record_exit(result, coordinator)
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

/// One observed direct-task exit and the shutdown cause it selects, if any.
#[cfg_attr(not(feature = "metrics"), allow(dead_code))]
struct RecordedExit {
    cause: Option<ShutdownCause>,
    name: &'static str,
    finite: bool,
    outcome: TaskOutcome,
}
