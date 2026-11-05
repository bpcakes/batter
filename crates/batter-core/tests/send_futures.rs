//! Every public future is `Send` for `Send` inputs, as `tokio::spawn` and Axum
//! handlers require. The closures are only type-checked, never called, so a
//! regression fails to compile. Bulkhead blocking runs are pinned in
//! `admission/blocking_outcomes.rs`.
use batter_core::{
    admission::{Admission, Bulkhead},
    cleanup::{CleanupBudget, CleanupStack},
    command::{CommandObserver, CommandOwner, RunningCommand},
    health::HealthMonitor,
    lifecycle::{
        InstalledSignals, LifecycleStatus, ManagedObserver, ProcessReceipt, RunningComponent,
        RunningSupervisor, ShutdownSignal, Supervisor, SupervisorObserver, UnapprovedSupervisor,
    },
    operation::{OperationContext, PhasedObserver, RunningPhasedOperation},
    retry::{self, Attempt, ReplaySafety, RetryDecision, RetryOptions, RetryPolicy},
    service::{DiagnosticCompletion, ServiceObserver, ServiceOwner, StartingService},
    startup::{StartingSupervisor, StartupObserver},
    telemetry::with_current_dispatch,
};
use std::{
    future::{Future, Ready, pending, ready},
    io,
};

fn send<F: Future + Send>(_: F) {}

type Probe = fn() -> Ready<Result<(), io::Error>>;

#[test]
fn supervisor_drivers_and_waits_are_send() {
    let _ = |supervisor: Supervisor| send(supervisor.run_until(pending::<()>()));
    let _ = |supervisor: Supervisor| send(supervisor.run_until_unapproved(pending::<()>()));
    let _ = |supervisor: Supervisor| {
        send(
            supervisor
                .run_until_unapproved(pending::<()>())
                .approve_readiness(),
        );
    };
    let _ = |running: &RunningSupervisor| {
        send(running.wait());
        send(running.wait_checked());
        send(running.wait_report());
        send(running.shutdown());
        send(running.shutdown_checked());
        send(running.shutdown_report());
    };
    let _ = |unapproved: &UnapprovedSupervisor| {
        send(unapproved.wait());
        send(unapproved.wait_checked());
        send(unapproved.wait_report());
        send(unapproved.shutdown());
        send(unapproved.shutdown_checked());
        send(unapproved.shutdown_report());
    };
    let _ = |observer: &SupervisorObserver| {
        send(observer.wait());
        send(observer.wait_checked());
        send(observer.wait_report());
    };
}

#[test]
fn lifecycle_signals_and_receipts_are_send() {
    let _ = |status: &LifecycleStatus| send(status.wait_ready());
    let _ = |shutdown: &ShutdownSignal| {
        send(shutdown.draining());
        send(shutdown.cancelled());
    };
    let _ = |component: &RunningComponent| {
        send(component.draining());
        send(component.cancelled());
    };
    let _ = |observer: &ManagedObserver| send(observer.wait());
    let _ = |signals: &mut InstalledSignals| send(signals.received());
    let _ = |receipt: ProcessReceipt<u8, io::Error>| send(receipt.wait());
    let _ = |monitor: HealthMonitor<Probe, io::Error>, shutdown: ShutdownSignal| {
        send(monitor.run(shutdown));
    };
    let _ = |slot: batter_core::cleanup::CleanupSlot<'_>,
             semaphore: std::sync::Arc<tokio::sync::Semaphore>| {
        send(slot.acquire_semaphore(semaphore))
    };
    let _ = |slot: batter_core::cleanup::CleanupSlot<'_>, address: std::net::SocketAddr| {
        send(slot.bind_udp(address))
    };
    let _ = |stack: CleanupStack, budget: CleanupBudget| send(stack.close(budget));
}

#[test]
fn startup_command_and_service_waits_are_send() {
    let _ = |starting: StartingSupervisor<io::Error>| send(starting.finish());
    let _ = |starting: StartingSupervisor<io::Error, RunningSupervisor, u8>| {
        send(starting.finish_with_output());
    };
    let _ = |observer: &StartupObserver<io::Error>| send(observer.wait());
    let _ = |command: &RunningCommand<u8, io::Error>| send(command.wait());
    let _ = |observer: &CommandObserver<u8, io::Error>| send(observer.wait());
    let _ = |owner: CommandOwner<u8, io::Error>| send(owner.finish());
    let _ = |owner: &ServiceOwner<io::Error, u8>| send(owner.wait());
    let _ = |observer: &ServiceObserver<io::Error, u8>| send(observer.wait());
    let _ = |observer: &ServiceObserver<io::Error, u8>| send(observer.wait_ready());
    let _ = |owner: &ServiceOwner<io::Error, u8>| send(owner.wait_ready());
    let _ = |owner: StartingService<io::Error, u8, String>| send(owner.into_ready());
    let _ = |completion: DiagnosticCompletion| send(completion.wait());
}

#[test]
fn operation_retry_and_admission_futures_are_send() {
    let _ = |owner: &RunningPhasedOperation<u8, io::Error, u8, io::Error>| send(owner.wait());
    let _ = |observer: &PhasedObserver<u8, io::Error, u8, io::Error>| send(observer.wait());
    let work = |_: OperationContext| ready(Ok::<_, io::Error>(1_u8));
    let attempt = |_: Attempt| ready(Ok::<_, io::Error>(1_u8));
    let classify = |_: &io::Error| RetryDecision::Stop;
    let _ = |context: &OperationContext| {
        send(context.cancelled());
        send(context.run_tracked("send.check", work));
        send(context.run("send.check", work));
        send(context.run_resolved("send.check", work, |result| result));
    };
    let _ = |context: &OperationContext, policy: &RetryPolicy| {
        let safety = ReplaySafety::Idempotent;
        send(retry::execute(
            context,
            "send.check",
            safety,
            policy,
            attempt,
            classify,
        ));
        send(retry::execute_with_options(
            context,
            "send.check",
            safety,
            policy,
            RetryOptions::new(),
            attempt,
            classify,
        ));
        send(retry::execute_with_jitter(
            context,
            "send.check",
            safety,
            policy,
            || 0,
            attempt,
            classify,
        ));
    };
    let _ = |bulkhead: &Bulkhead, context: &OperationContext| {
        send(bulkhead.enter(context, Admission::Reject));
    };
    let _ = || send(with_current_dispatch(ready(())));
}
