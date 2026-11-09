//! The one serial loop that owns a periodic component's schedule.
//!
//! There is exactly one directly owned future per registered component: no
//! detached per-run task, no event queue and no second scheduler.

use crate::lifecycle::StartupPhase;

use super::{
    History, PeriodicCompletion, PeriodicFailure, PeriodicInitializationExpired, PeriodicOutcome,
    history::Cause, schedule::Schedule,
};
use crate::{
    BoxError,
    lifecycle::{
        ComponentExit, ComponentStartup, PeriodicAdmission, PeriodicGrant, RunningComponent,
        SupportObligation,
    },
    operation::{Interruption, OperationContext, OperationError, OperationOwner, RootDeadline},
};
use std::{error::Error, fmt, future::Future, sync::Arc, time::Duration};
use tokio::time::{Instant, sleep_until};

#[cfg(test)]
mod tests;

/// The one capability owned by the loop, with an allowance only before startup.
enum ComponentState {
    Initializing {
        startup: ComponentStartup,
        deadline: Instant,
    },
    Running(RunningComponent),
}

impl ComponentState {
    fn running(
        startup: ComponentStartup,
        history: &History,
        obligation: &SupportObligation,
    ) -> Self {
        history.acknowledged();
        obligation.assume();
        Self::Running(startup.acknowledge_started())
    }

    fn phase(&self) -> StartupPhase {
        match self {
            Self::Initializing { .. } => StartupPhase::Initializing,
            Self::Running(_) => StartupPhase::Running,
        }
    }

    fn initialization_deadline(&self) -> Option<Instant> {
        match self {
            Self::Initializing { deadline, .. } => Some(*deadline),
            Self::Running(_) => None,
        }
    }

    fn acknowledge(self, history: &History, obligation: &SupportObligation) -> Self {
        match self {
            Self::Initializing { startup, .. } => Self::running(startup, history, obligation),
            Self::Running(running) => Self::Running(running),
        }
    }
}

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
    Run(PeriodicGrant),
    Rejected,
    Abandon,
    Expired,
    Stopped,
}

/// One classified run outcome. Exactly one of these is recorded per run.
///
/// Outcomes the run itself produced are already retained, because they are
/// classified while its future is still alive. The boundary's own outcomes are
/// recorded by the loop, which is the only place that observes them.
enum Run {
    Succeeded,
    Skipped,
    Recoverable,
    DeadlineExceeded,
    Interrupted,
    Fatal(Cause),
}

/// What a run that returned `Ok` did, already retained in bounded history.
enum Ran {
    Worked,
    Skipped,
}

/// A run's own outcome, already retained in bounded history.
enum Classified {
    Recoverable,
    Fatal(Cause),
}

pub(super) async fn drive<F, Fut, S, E>(
    registration: super::Registration<F>,
    startup: ComponentStartup,
    admission: PeriodicAdmission,
    obligation: SupportObligation,
) -> Result<ComponentExit, BoxError>
where
    F: FnMut(OperationContext) -> Fut,
    Fut: Future<Output = Result<S, PeriodicFailure<E>>>,
    S: crate::periodic::PeriodicSuccess,
    E: std::error::Error + Send + Sync + 'static,
{
    let super::Registration {
        name,
        policy,
        mut work,
        history,
        scheduling,
    } = registration;
    // Nothing has run yet, so an abandoned or never-polled registration
    // performs no application work at all.
    if startup.shutdown().is_draining() {
        history.finished(PeriodicCompletion::AbandonedDuringStartup);
        return Ok(startup.abandon());
    }
    // The allowance is measured from this first live execution onward and
    // covers failed runs and the interval waits between them. Registration
    // already rejected an unrepresentable allowance.
    let mut component = match policy.startup().initialization_allowance() {
        Some(allowance) => ComponentState::Initializing {
            startup,
            deadline: Instant::now()
                .checked_add(allowance)
                .unwrap_or_else(Instant::now),
        },
        // Immediate acknowledgement states that the loop is initialized, not
        // that any maintenance has succeeded.
        None => ComponentState::running(startup, &history, &obligation),
    };
    let mut ticker = Schedule::new(policy.interval(), scheduling);
    let mut invocation = 0u64;
    let ending = loop {
        let phase = component.phase();
        let initializing = phase == StartupPhase::Initializing;
        let grant = match next_due(&mut ticker, &admission, &component).await {
            Due::Run(grant) => grant,
            Due::Abandon => break Ending::Abandoned,
            Due::Expired => break Ending::InitializationExpired,
            Due::Stopped => break Ending::Stopped,
            Due::Rejected => {
                // A rejection can precede observable drain when another
                // component fails. Wait for the applicable stopping boundary
                // rather than reporting an unexpected successful exit.
                if initializing {
                    admission.draining().await;
                    break Ending::Abandoned;
                }
                admission.stopping().await;
                break Ending::Stopped;
            }
        };
        invocation = invocation.saturating_add(1);
        history.admitted();
        let deadline = run_deadline(
            policy.run_budget(),
            grant.forced_at,
            component.initialization_deadline(),
        );
        // `execute` destroys its owned work future before returning, so no
        // application code is live while the evidence below is published.
        let retention = Retention {
            history: &history,
            invocation,
        };
        match execute(name, &mut work, deadline, &admission, phase, retention).await {
            Run::Succeeded => {
                if let ComponentState::Initializing { deadline, .. } = &component {
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
                    if Instant::now() >= *deadline {
                        break Ending::InitializationExpired;
                    }
                    // A qualifying success acknowledges exactly once. Later
                    // run failures never revoke it, and it establishes no
                    // continuing lease, renewal or dependency health.
                    component = component.acknowledge(&history, &obligation);
                }
            }
            // Already retained while its run future was alive. A skipped run
            // did no work, so it never acknowledges startup.
            Run::Skipped | Run::Recoverable => {}
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
    drop(ticker);
    finish(name, &history, invocation, ending, component)
}

/// Await one run without consuming its future, retaining everything that run
/// produced while that future is still alive.
///
/// Do not await the run by value: it would be destroyed at the end of that
/// statement, and a run future whose own destructor panics would then leave
/// only the panic in the report, with no retained cause, sample or count for
/// the invocation. The enclosing execution boundary destroys this future only
/// after this body has returned, so its evidence is already published by then
/// and the loop must not count it again.
///
/// The boundary's own outcomes, an expired run deadline and a stop
/// interruption, are not visible here and carry no application cause; the loop
/// counts those instead.
async fn retained<Fut, S, E>(work: Fut, retention: Retention<'_>) -> Result<Ran, Classified>
where
    Fut: Future<Output = Result<S, PeriodicFailure<E>>>,
    S: crate::periodic::PeriodicSuccess,
    E: Error + Send + Sync + 'static,
{
    tokio::pin!(work);
    match work.as_mut().await {
        Ok(success) => match success.outcome() {
            PeriodicOutcome::Ran => {
                retention.history.succeeded();
                Ok(Ran::Worked)
            }
            PeriodicOutcome::Skipped => {
                retention.history.skipped();
                Ok(Ran::Skipped)
            }
        },
        Err(PeriodicFailure::Recoverable(error)) => {
            retention
                .history
                .recoverable(retention.invocation, Arc::new(error));
            Err(Classified::Recoverable)
        }
        Err(PeriodicFailure::Fatal(error)) => {
            let error: Cause = Arc::new(error);
            retention
                .history
                .terminal(retention.invocation, error.clone());
            Err(Classified::Fatal(error))
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
    component: ComponentState,
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
        Ending::Abandoned | Ending::Stopped => match component {
            ComponentState::Running(running) => {
                history.finished(PeriodicCompletion::Stopped);
                Ok(running.stopped())
            }
            ComponentState::Initializing { startup, .. } => {
                history.finished(PeriodicCompletion::AbandonedDuringStartup);
                Ok(startup.abandon())
            }
        },
    }
}

async fn next_due(
    ticker: &mut Schedule,
    admission: &PeriodicAdmission,
    component: &ComponentState,
) -> Due {
    let phase = component.phase();
    let initializing = phase == StartupPhase::Initializing;
    let expiry = async {
        match component.initialization_deadline() {
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
        grant = ticker.admit(admission, phase) => match grant {
            Some(grant) => Due::Run(grant),
            None => Due::Rejected,
        },
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

async fn execute<F, Fut, S, E>(
    name: &'static str,
    work: &mut F,
    deadline: Instant,
    admission: &PeriodicAdmission,
    initializing: StartupPhase,
    retention: Retention<'_>,
) -> Run
where
    F: FnMut(OperationContext) -> Fut,
    Fut: Future<Output = Result<S, PeriodicFailure<E>>>,
    S: crate::periodic::PeriodicSuccess,
    E: Error + Send + Sync + 'static,
{
    let initializing = initializing == StartupPhase::Initializing;
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
            Ok(Ran::Worked) => Run::Succeeded,
            Ok(Ran::Skipped) => Run::Skipped,
            Err(OperationError::Failed(Classified::Recoverable)) => Run::Recoverable,
            Err(OperationError::Failed(Classified::Fatal(error))) => Run::Fatal(error),
            Err(OperationError::Interrupted(Interruption::DeadlineExceeded)) => {
                Run::DeadlineExceeded
            }
            Err(OperationError::Interrupted(Interruption::Cancelled)) => Run::Interrupted,
        },
    }
}
