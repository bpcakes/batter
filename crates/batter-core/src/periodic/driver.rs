//! The one serial loop that owns a periodic component's schedule.
//!
//! There is exactly one directly owned future per registered component: no
//! detached per-run task, no event queue and no second scheduler.

use super::{
    History, PeriodicCompletion, PeriodicFailure, PeriodicInitializationExpired, PeriodicPolicy,
};
use crate::{
    BoxError,
    lifecycle::{
        ComponentExit, ComponentStartup, PeriodicAdmission, RunningComponent, SupportObligation,
    },
    operation::{Interruption, OperationContext, OperationError, OperationOwner, RootDeadline},
};
use std::{future::Future, sync::Arc, time::Duration};
use tokio::time::{Instant, Interval, MissedTickBehavior, interval, sleep_until};

/// Why the loop stopped admitting further runs.
enum Ending<E> {
    /// Drain was observed while initialization was still pending.
    Abandoned,
    /// The first-success allowance expired before any run succeeded.
    InitializationExpired,
    /// The applicable stopping point was reached.
    Stopped,
    /// A run returned an explicit fatal failure.
    Fatal(E),
}

/// What the schedule produced while waiting for the next invocation.
enum Due {
    Run,
    Abandon,
    Expired,
    Stopped,
}

/// One classified run outcome. Exactly one of these is recorded per run.
enum Run<E> {
    Succeeded,
    Recoverable(E),
    DeadlineExceeded,
    Interrupted,
    Fatal(E),
}

pub(super) async fn drive<F, Fut, E>(
    name: &'static str,
    policy: PeriodicPolicy,
    mut work: F,
    history: Arc<History>,
    startup: ComponentStartup,
    admission: PeriodicAdmission,
    obligation: SupportObligation,
) -> Result<ComponentExit, BoxError>
where
    F: FnMut(OperationContext) -> Fut,
    Fut: Future<Output = Result<(), PeriodicFailure<E>>>,
    E: std::error::Error + Send + Sync + 'static,
{
    // Nothing has run yet, so an abandoned or never-polled registration
    // performs no application work at all.
    if startup.shutdown().is_draining() {
        history.finished(PeriodicCompletion::AbandonedDuringStartup);
        return Ok(startup.abandon());
    }
    // The allowance is measured from this first live execution onward and
    // covers failed runs and the interval waits between them. Registration
    // already rejected an unrepresentable allowance.
    let initialization = policy
        .startup()
        .initialization_allowance()
        .map(|allowance| {
            Instant::now()
                .checked_add(allowance)
                .unwrap_or_else(Instant::now)
        });
    let mut pending = Some(startup);
    let mut running: Option<RunningComponent> = None;
    if initialization.is_none() {
        // Immediate acknowledgement states that the loop is initialized, not
        // that any maintenance has succeeded.
        history.acknowledged();
        obligation.assume();
        running = Some(
            pending
                .take()
                .expect("startup is pending before acknowledgement")
                .acknowledge_started(),
        );
    }
    let mut ticker = interval(policy.interval());
    // Missed ticks are skipped. One overdue tick may run immediately after an
    // overrun and the schedule then realigns, so no burst of missed work is
    // replayed. This is not completion-plus-delay: the interval is measured
    // between invocation starts.
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut invocation = 0u64;
    let ending = loop {
        let initializing = pending.is_some();
        match next_due(&mut ticker, &admission, initializing, initialization).await {
            Due::Run => {}
            Due::Abandon => break Ending::Abandoned,
            Due::Expired => break Ending::InitializationExpired,
            Due::Stopped => break Ending::Stopped,
        }
        let Some(grant) = admission.admit() else {
            // No run is admitted after its applicable stopping point. A
            // rejection can also follow another component's failure before
            // drain is observable, so wait for the actual stopping point
            // instead of exiting early as an unexpected success.
            if initializing {
                admission.draining().await;
                break Ending::Abandoned;
            }
            admission.stopping().await;
            break Ending::Stopped;
        };
        invocation = invocation.saturating_add(1);
        history.admitted();
        let deadline = run_deadline(
            policy.run_budget(),
            grant.forced_at,
            initialization.filter(|_| initializing),
        );
        // `execute` destroys its owned work future before returning, so no
        // application code is live while the evidence below is published.
        match execute(name, &mut work, deadline, &admission, initializing).await {
            Run::Succeeded => {
                history.succeeded();
                if pending.is_some() {
                    // The run boundary is cooperative and does not reread its
                    // clock after the work's own poll returns, so a blocking
                    // poll or destructor can cross the initialization deadline
                    // and still return success. Keep that run counted, but do
                    // not let it acknowledge startup after the allowance
                    // expired: expiry must still prevent Ready.
                    if initialization.is_some_and(|deadline| Instant::now() >= deadline) {
                        break Ending::InitializationExpired;
                    }
                    // A qualifying success acknowledges exactly once. Later
                    // run failures never revoke it, and it establishes no
                    // continuing lease, renewal or dependency health.
                    history.acknowledged();
                    obligation.assume();
                    running = Some(
                        pending
                            .take()
                            .expect("initialization is pending before acknowledgement")
                            .acknowledge_started(),
                    );
                }
            }
            Run::Recoverable(error) => history.recoverable(invocation, Arc::new(error)),
            Run::DeadlineExceeded => history.deadline_exceeded(),
            Run::Interrupted => {
                history.stop_interrupted();
                break if initializing {
                    Ending::Abandoned
                } else {
                    Ending::Stopped
                };
            }
            Run::Fatal(error) => break Ending::Fatal(error),
        }
    };
    finish(name, &history, ending, pending, running)
}

/// Publish the loop's own final marker and return its exit proof or failure.
///
/// The marker is the runner's claim about itself; the coordinator reconciles it
/// with its own join evidence when it freezes the report.
fn finish<E>(
    name: &'static str,
    history: &History,
    ending: Ending<E>,
    pending: Option<ComponentStartup>,
    running: Option<RunningComponent>,
) -> Result<ComponentExit, BoxError>
where
    E: std::error::Error + Send + Sync + 'static,
{
    match ending {
        Ending::InitializationExpired => {
            history.finished(PeriodicCompletion::InitializationExpired);
            Err(Box::new(PeriodicInitializationExpired { name }) as BoxError)
        }
        Ending::Fatal(error) => {
            history.finished(PeriodicCompletion::Fatal);
            Err(Box::new(error) as BoxError)
        }
        Ending::Abandoned | Ending::Stopped => match running {
            Some(running) => {
                history.finished(PeriodicCompletion::Stopped);
                Ok(running.stopped())
            }
            None => {
                history.finished(PeriodicCompletion::AbandonedDuringStartup);
                Ok(pending
                    .expect("an unacknowledged component still owns its startup")
                    .abandon())
            }
        },
    }
}

async fn next_due(
    ticker: &mut Interval,
    admission: &PeriodicAdmission,
    initializing: bool,
    initialization: Option<Instant>,
) -> Due {
    let expiry = async {
        match initialization.filter(|_| initializing) {
            Some(deadline) => sleep_until(deadline).await,
            None => std::future::pending().await,
        }
    };
    tokio::select! {
        biased;
        // Pending initialization abandons on global drain, including support.
        _ = admission.draining(), if initializing => Due::Abandon,
        _ = admission.stopping(), if !initializing => Due::Stopped,
        () = expiry => Due::Expired,
        _ = ticker.tick() => Due::Run,
    }
}

/// Each run gets a fresh deadline, capped by whichever recorded clock is
/// earlier. A budget longer than the interval is valid and is not shortened
/// while the process is running normally.
fn run_deadline(
    budget: Duration,
    forced_at: Option<Instant>,
    initialization: Option<Instant>,
) -> Instant {
    let mut deadline = Instant::now()
        .checked_add(budget)
        .unwrap_or_else(Instant::now);
    for cap in [forced_at, initialization].into_iter().flatten() {
        deadline = deadline.min(cap);
    }
    deadline
}

async fn execute<F, Fut, E>(
    name: &'static str,
    work: &mut F,
    deadline: Instant,
    admission: &PeriodicAdmission,
    initializing: bool,
) -> Run<E>
where
    F: FnMut(OperationContext) -> Fut,
    Fut: Future<Output = Result<(), PeriodicFailure<E>>>,
{
    // The run's cancellation lineage descends from process forced cancellation,
    // so children cannot outlive it and cannot exceed its deadline. Ending one
    // run cancels only its own child scope, never a sibling or a future run.
    let parent = admission.operation_token();
    let owner = OperationOwner::under(RootDeadline::at(deadline), &parent);
    let attempt = owner.context().run(name, work);
    tokio::pin!(attempt);
    tokio::select! {
        biased;
        _ = admission.draining(), if initializing => Run::Interrupted,
        _ = admission.stopping(), if !initializing => Run::Interrupted,
        result = &mut attempt => match result {
            Ok(()) => Run::Succeeded,
            Err(OperationError::Failed(PeriodicFailure::Recoverable(error))) => {
                Run::Recoverable(error)
            }
            Err(OperationError::Failed(PeriodicFailure::Fatal(error))) => Run::Fatal(error),
            Err(OperationError::Interrupted(Interruption::DeadlineExceeded)) => {
                Run::DeadlineExceeded
            }
            Err(OperationError::Interrupted(Interruption::Cancelled)) => Run::Interrupted,
        },
    }
}
