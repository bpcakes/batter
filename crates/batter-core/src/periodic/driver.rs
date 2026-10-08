//! The one serial loop that owns a periodic component's schedule.
//!
//! There is exactly one directly owned future per registered component: no
//! detached per-run task, no event queue and no second scheduler.

use super::{
    History, PeriodicCompletion, PeriodicFailure, PeriodicInitializationExpired, PeriodicPolicy,
    history::Cause,
};
use crate::{
    BoxError,
    lifecycle::{
        ComponentExit, ComponentStartup, PeriodicAdmission, RunningComponent, SupportObligation,
    },
    operation::{Interruption, OperationContext, OperationError, OperationOwner, RootDeadline},
};
use std::{error::Error, fmt, future::Future, sync::Arc, time::Duration};
use tokio::time::{Instant, Interval, MissedTickBehavior, interval, sleep_until};

/// Why the loop stopped admitting further runs.
enum Ending {
    /// Drain was observed while initialization was still pending.
    Abandoned,
    /// The first-success allowance expired before any run succeeded.
    InitializationExpired,
    /// The applicable stopping point was reached.
    Stopped,
    /// A run returned an explicit fatal failure, already retained.
    Fatal(Cause),
}

/// What the schedule produced while waiting for the next invocation.
enum Due {
    Run,
    Abandon,
    Expired,
    Stopped,
}

/// One classified run outcome. Exactly one of these is recorded per run.
///
/// Causes arrive already shared, because a terminal one is retained while its
/// run future is still alive.
enum Run {
    Succeeded,
    Recoverable(Cause),
    DeadlineExceeded,
    Interrupted,
    Fatal(Cause),
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
        let retention = Retention {
            history: &history,
            invocation,
        };
        match execute(
            name,
            &mut work,
            deadline,
            &admission,
            initializing,
            retention,
        )
        .await
        {
            Run::Succeeded => {
                history.succeeded();
                if pending.is_some() {
                    // The run boundary is cooperative: it neither rereads its
                    // clock nor rechecks drain after the work's own poll
                    // returns, so a blocking poll or destructor can cross
                    // either boundary and still return success. Keep that run
                    // counted, but never acknowledge startup afterwards.
                    // Pending initialization abandons on observed drain, and an
                    // expired allowance stays a retained initialization
                    // failure; both must still prevent Ready, and neither may
                    // assume the support obligation.
                    if admission.is_draining() {
                        break Ending::Abandoned;
                    }
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
            Run::Recoverable(error) => history.recoverable(invocation, error),
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
    // The factory and its application captures are destroyed when this future
    // returns, so the ending's concrete cause is retained before that.
    finish(name, &history, invocation, ending, pending, running)
}

/// Await one run without consuming its future, retaining a terminal cause
/// while that future is still alive.
///
/// Do not await the run by value: it would be destroyed at the end of that
/// statement, and a run future whose own destructor panics would then leave
/// only the panic in the report. The enclosing execution boundary destroys
/// this future after this body has returned, so the cause is already retained
/// by then. Recoverable causes are shared here too, so one classification
/// handles both and nothing is cloned.
async fn retained<Fut, E>(work: Fut, retention: Retention<'_>) -> Result<(), PeriodicFailure<Cause>>
where
    Fut: Future<Output = Result<(), PeriodicFailure<E>>>,
    E: Error + Send + Sync + 'static,
{
    tokio::pin!(work);
    match work.as_mut().await {
        Ok(()) => Ok(()),
        Err(PeriodicFailure::Recoverable(error)) => {
            Err(PeriodicFailure::Recoverable(Arc::new(error)))
        }
        Err(PeriodicFailure::Fatal(error)) => {
            let error: Cause = Arc::new(error);
            retention
                .history
                .terminal(retention.invocation, error.clone());
            Err(PeriodicFailure::Fatal(error))
        }
    }
}

/// Where one run's classified outcome is retained.
#[derive(Clone, Copy)]
struct Retention<'a> {
    history: &'a History,
    invocation: u64,
}

/// A terminal cause shared with the component's independently retained
/// history. `Error::source` exposes the original error, exactly as a finite
/// process task's shared failure does, so concrete downcasts survive.
struct SharedTerminal(Arc<dyn Error + Send + Sync>);

impl fmt::Debug for SharedTerminal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SharedPeriodicTerminalFailure")
    }
}

impl fmt::Display for SharedTerminal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("periodic run failed terminally")
    }
}

impl Error for SharedTerminal {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&*self.0)
    }
}

/// Publish the loop's own final marker and return its exit proof or failure.
///
/// The marker is the runner's claim about itself; the coordinator reconciles it
/// with its own join evidence when it freezes the report.
fn finish(
    name: &'static str,
    history: &History,
    invocation: u64,
    ending: Ending,
    pending: Option<ComponentStartup>,
    running: Option<RunningComponent>,
) -> Result<ComponentExit, BoxError> {
    match ending {
        Ending::InitializationExpired => {
            // This library error is reconstructible, so the report keeps a
            // directly downcastable copy alongside the retained one.
            history.terminal(invocation, Arc::new(PeriodicInitializationExpired { name }));
            history.finished(PeriodicCompletion::InitializationExpired);
            Err(Box::new(PeriodicInitializationExpired { name }) as BoxError)
        }
        Ending::Fatal(error) => {
            // The cause was already retained while its run future was alive.
            history.finished(PeriodicCompletion::Fatal);
            Err(Box::new(SharedTerminal(error)) as BoxError)
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
    retention: Retention<'_>,
) -> Run
where
    F: FnMut(OperationContext) -> Fut,
    Fut: Future<Output = Result<(), PeriodicFailure<E>>>,
    E: Error + Send + Sync + 'static,
{
    // The run's cancellation lineage descends from process forced cancellation,
    // so children cannot outlive it and cannot exceed its deadline. Ending one
    // run cancels only its own child scope, never a sibling or a future run.
    let parent = admission.operation_token();
    let owner = OperationOwner::under(RootDeadline::at(deadline), &parent);
    let attempt = owner
        .context()
        .run(name, |scope| retained(work(scope), retention));
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
