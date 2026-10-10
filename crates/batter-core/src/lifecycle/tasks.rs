//! Direct-task ownership, join accounting and escalation bookkeeping.

#[cfg(test)]
mod tests;

use super::{
    Component, ComponentClass, LifecycleCoordinator, RegisteredComponent, ShutdownCause,
    TaskOutcome, TaskRecord, managed::ManagedObserver, process, receive_process,
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

/// How one owned task is classified for stopping and escalation.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TaskKind {
    /// A directly registered ordinary component.
    Component,
    /// A directly registered component that supports others through drain.
    Support,
    /// An admitted finite process task.
    Finite,
}

struct TaskMetadata {
    name: &'static str,
    kind: TaskKind,
    abort: AbortHandle,
}

type TaskResult = Result<(Id, TaskExit), JoinError>;

#[derive(Default)]
pub(super) struct TaskSet {
    set: JoinSet<TaskExit>,
    names: HashMap<Id, TaskMetadata>,
    records: Vec<TaskRecord>,
    completed: u64,
    // A joined panic or abort is observed termination, not evidence that the
    // task's descendants released their dependencies. Retained here so the
    // support boundary cannot treat it as settled work.
    unsafe_ordinary_exit: bool,
}

impl TaskSet {
    pub(super) fn spawn_component(&mut self, component: Component) {
        let Component {
            name,
            class,
            lifecycle:
                RegisteredComponent {
                    startup,
                    coordinator,
                },
            factory,
        } = component;
        let classification = coordinator.clone();
        let kind = match &class {
            ComponentClass::Ordinary => TaskKind::Component,
            ComponentClass::PeriodicSupport(_) => TaskKind::Support,
        };
        let span = tracing::info_span!(target: "batter", "batter.task", task = name).or_current();
        let abort = self.set.spawn(scoped_dispatch::scope(
            async move {
                let result = factory(startup, coordinator).await.map(|_exit| ());
                // Capture the state at completion, never at delayed observation.
                // Initialized support work is expected to stop only at its own
                // later point, so a success during ordinary drain remains an
                // early exit. Support that never initialized abandons on drain
                // exactly like any other component.
                let expected = match &class {
                    ComponentClass::PeriodicSupport(obligation) => {
                        classification.shared.is_support_stopping()
                            || (!obligation.assumed() && classification.is_draining())
                    }
                    ComponentClass::Ordinary => classification.is_draining(),
                };
                TaskExit { result, expected }
            }
            .instrument(span),
        ));
        self.names
            .insert(abort.id(), TaskMetadata { name, kind, abort });
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
                kind: TaskKind::Finite,
                abort,
            },
        );
    }

    fn sorted_names(&self, keep: impl Fn(&TaskMetadata) -> bool) -> Vec<&'static str> {
        let mut values: Vec<_> = self
            .names
            .values()
            .filter(|entry| keep(entry))
            .map(|entry| entry.name)
            .collect();
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
        let TaskMetadata { name, kind, .. } = self
            .names
            .remove(&id)
            .expect("every owned task has metadata");
        let finite = kind == TaskKind::Finite;
        if finite && outcome == TaskOutcome::Stopped {
            outcome = TaskOutcome::Completed;
        }
        if kind != TaskKind::Support
            && matches!(outcome, TaskOutcome::Panicked | TaskOutcome::Aborted)
        {
            self.unsafe_ordinary_exit = true;
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
        // Recorder code runs only after a failure has closed process admission;
        // root admission closes when the driver requests drain.
        crate::telemetry::record::task(exit.finite, exit.name, exit.outcome);
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
        self.collect_while(queued, coordinator, allowance, |_, _| {})
            .await;
    }

    /// Drive the drain phase, closing support admission once the work that
    /// support exists for has actually finished.
    ///
    /// Support components are excluded from the predicate they await. Native
    /// settlement is observed here, during drain, rather than after support has
    /// stopped: a joined wrapper or a published-but-unsafe report is not a
    /// stopped proof, so the retained evidence must also allow dependency
    /// cleanup. When the boundary is never reached, support closes at the
    /// existing global forced-cancellation boundary instead.
    pub(super) async fn drain_phase(
        &mut self,
        queued: &mut Option<mpsc::Receiver<process::QueuedProcess>>,
        coordinator: &LifecycleCoordinator,
        allowance: std::time::Duration,
        managed: &[(&'static str, ManagedObserver)],
    ) {
        self.collect_while(queued, coordinator, allowance, |tasks, coordinator| {
            if !coordinator.shared.is_support_stopping()
                && tasks.ordinary_work_settled(coordinator)
                && managed
                    .iter()
                    .all(|(_, observer)| observer.snapshot().allows_dependency_cleanup())
            {
                coordinator.shared.close_support();
            }
        })
        .await;
    }

    /// Collect exits within one phase allowance, reassessing `each` before
    /// every wait. The hook only observes the task set and the coordinator; it
    /// neither owns nor joins work.
    async fn collect_while(
        &mut self,
        queued: &mut Option<mpsc::Receiver<process::QueuedProcess>>,
        coordinator: &LifecycleCoordinator,
        allowance: std::time::Duration,
        mut each: impl FnMut(&Self, &LifecycleCoordinator),
    ) {
        loop {
            each(self, coordinator);
            if !self.pending(coordinator) {
                break;
            }
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

    /// Whether every ordinary direct task has been joined with termination
    /// evidence adequate for dependency cleanup, and no queued or active finite
    /// work or admitted descendant remains.
    ///
    /// A joined panic or abort is deliberately not adequate: the same exit
    /// makes the coordinator skip finalizers, and its descendants may still
    /// need support. Support then closes at forced cancellation instead.
    fn ordinary_work_settled(&self, coordinator: &LifecycleCoordinator) -> bool {
        !self.unsafe_ordinary_exit
            && !self
                .names
                .values()
                .any(|task| task.kind != TaskKind::Support)
            && !coordinator.shared.has_finite_tasks()
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
        let unjoined = self.sorted_names(|_| true);
        // Finite labels are a separate vocabulary that may repeat and may
        // match a registered component, so component-scoped reconciliation
        // cannot use the combined diagnostic list.
        let unjoined_components = self.sorted_names(|entry| entry.kind != TaskKind::Finite);
        let unsafe_exit = !unjoined.is_empty()
            || self.records.iter().any(|record| {
                matches!(record.outcome, TaskOutcome::Panicked | TaskOutcome::Aborted)
            });
        TaskSummary {
            records: self.records,
            completed: self.completed,
            unjoined,
            unjoined_components,
            unsafe_exit,
        }
    }
}

pub(super) struct TaskSummary {
    pub(super) records: Vec<TaskRecord>,
    pub(super) completed: u64,
    /// Every direct task whose completion was not observed, by diagnostic name.
    pub(super) unjoined: Vec<&'static str>,
    /// Only the registered components among them, whose names are unique.
    pub(super) unjoined_components: Vec<&'static str>,
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
struct RecordedExit {
    cause: Option<ShutdownCause>,
    name: &'static str,
    finite: bool,
    outcome: TaskOutcome,
}
