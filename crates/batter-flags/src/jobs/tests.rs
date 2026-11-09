//! Failure contracts through the actual native invocation drive.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]
use super::*;
use crate::{reader::Reader, test::TestFlags};
use batter_flags_core::{
    Catalog, CatalogBuilder, PrePublication, ProcessKind, SafeState, SettingsPrefix, StaleRule,
    SwitchState, Version,
    low_level::{BaseBuilder, Candidate, CandidateRow},
};
use batter_runledger::test_support::{HandlerExit, HandlerInvocation};
use runledger_core::jobs::{
    JobCompletionDisposition, JobContract, JobDeadLetterOrigin, JobDeadLetterReason,
    JobFailureKind, JobRetryTiming, JobSpec,
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::time::Instant;

mod pins;
mod timing;

fn world() -> (Catalog, ProcessKind, KillSwitch) {
    let (mut b, worker, _) =
        CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap(), "worker", "hosted").unwrap();
    let halt = b
        .kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
        .declare()
        .unwrap();
    (b.build(), worker, halt)
}
fn recheck() -> Recheck {
    Recheck::after(Duration::from_secs(30)).unwrap()
}
fn policy(when_tripped: DirectWhenTripped) -> GatePolicy<DirectWhenTripped> {
    GatePolicy {
        when_tripped,
        awaiting: Recheck::after(Duration::from_secs(60)).unwrap(),
    }
}
struct Probe(Arc<AtomicUsize>);
#[async_trait]
impl FlagAwareHandler for Probe {
    fn job_type(&self) -> JobType<'static> {
        JobType::new("test.probe")
    }
    async fn execute(
        &self,
        _: JobExecution<'_>,
        _: &JobFlags,
        _: Value,
    ) -> Result<JobCompletion, JobFailure> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(JobCompletion::success())
    }
}
async fn invoke(handler: &dyn JobHandler) -> Result<JobCompletion, JobFailure> {
    let result = HandlerInvocation::new(Duration::from_secs(5))
        .unwrap()
        .execute(handler, Value::Null)
        .await;
    assert!(result.exit_hook_panics.is_empty());
    match result.exit {
        HandlerExit::Returned(value) => value,
        other => panic!("unexpected exit: {other:?}"),
    }
}
fn assert_retry(failure: JobFailure, minimum: Duration) {
    assert_eq!(failure.kind, JobFailureKind::Retryable);
    assert_eq!(failure.code, "flags.job_type_deferred");
    let Some(JobRetryTiming::After(delay)) = failure.retry_timing() else {
        panic!("missing retry lower bound")
    };
    assert!((minimum..=minimum.mul_f64(1.25)).contains(&delay));
}

#[test]
fn recheck_rejects_invalid_intervals_and_includes_both_endpoints() {
    for d in [
        Duration::ZERO,
        Duration::from_secs(30) - Duration::from_nanos(1),
        Duration::from_secs(3600) + Duration::from_nanos(1),
        Duration::MAX,
    ] {
        assert_eq!(Recheck::after(d), Err(RecheckError));
    }
    for d in [Duration::from_secs(30), Duration::from_secs(3600)] {
        assert_eq!(Recheck::after(d).unwrap().interval(), d);
    }
}

#[tokio::test]
async fn every_direct_trip_policy_skips_inner_work() {
    let (catalog, worker, halt) = world();
    let test = TestFlags::builder(catalog, worker)
        .unwrap()
        .engage(&halt)
        .build()
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let deferred = test
        .flags()
        .gated_direct(
            Probe(calls.clone()),
            &halt,
            policy(DirectWhenTripped::Defer(recheck())),
        )
        .unwrap();
    let completion = invoke(&deferred).await.unwrap();
    let JobCompletionDisposition::ContinueAfter(delay) = completion.disposition() else {
        panic!("not a continuation")
    };
    assert!((Duration::from_secs(30)..=Duration::from_millis(37500)).contains(&delay));
    assert_eq!(completion.checkpoint_value(), None);
    let retry = test
        .flags()
        .gated_direct(
            Probe(calls.clone()),
            &halt,
            policy(DirectWhenTripped::DeferAttempt(recheck())),
        )
        .unwrap();
    assert_retry(invoke(&retry).await.unwrap_err(), Duration::from_secs(30));
    let drop = test
        .flags()
        .gated_direct(Probe(calls.clone()), &halt, policy(DirectWhenTripped::Drop))
        .unwrap();
    let failure = invoke(&drop).await.unwrap_err();
    assert_eq!(
        (failure.kind, failure.code),
        (JobFailureKind::Terminal, "flags.job_type_dropped")
    );
    assert_eq!(failure.retry_timing(), None);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn workflow_policies_and_open_and_ungated_dispatch() {
    let (catalog, worker, halt) = world();
    let calls = Arc::new(AtomicUsize::new(0));
    let test = TestFlags::builder(catalog, worker)
        .unwrap()
        .engage(&halt)
        .build()
        .unwrap();
    for when_tripped in [
        StepWhenTripped::DeferAttempt(recheck()),
        StepWhenTripped::Drop,
    ] {
        let handler = test
            .flags()
            .gated_workflow_step(
                Probe(calls.clone()),
                &halt,
                GatePolicy {
                    when_tripped,
                    awaiting: recheck(),
                },
            )
            .unwrap();
        let failure = invoke(&handler).await.unwrap_err();
        match when_tripped {
            StepWhenTripped::DeferAttempt(_) => assert_retry(failure, Duration::from_secs(30)),
            StepWhenTripped::Drop => assert_eq!(failure.code, "flags.job_type_dropped"),
        }
    }
    invoke(&test.flags().flagged(Probe(calls.clone()), recheck()))
        .await
        .unwrap();
    let (catalog, worker, halt) = world();
    let open = TestFlags::builder(catalog, worker)
        .unwrap()
        .release(&halt)
        .build()
        .unwrap();
    invoke(
        &open
            .flags()
            .gated_direct(Probe(calls.clone()), &halt, policy(DirectWhenTripped::Drop))
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn construction_refuses_foreign_and_unowned_switches() {
    let (catalog, worker, _) = world();
    let (_, _, foreign) = world();
    let test = TestFlags::builder(catalog, worker)
        .unwrap()
        .build()
        .unwrap();
    let probe = || Probe(Arc::new(AtomicUsize::new(0)));
    assert!(matches!(
        test.flags()
            .gated_direct(probe(), &foreign, policy(DirectWhenTripped::Drop)),
        Err(GateError::ForeignHandle { .. })
    ));
    let (mut b, worker, _) =
        CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap(), "worker", "hosted").unwrap();
    let serving = b.process_kind("serving").unwrap();
    let halt = b
        .kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
        .owned_only_by(&serving)
        .declare()
        .unwrap();
    let test = TestFlags::builder(b.build(), worker)
        .unwrap()
        .build()
        .unwrap();
    assert!(matches!(
        test.flags().gated_workflow_step(
            probe(),
            &halt,
            GatePolicy {
                when_tripped: StepWhenTripped::Drop,
                awaiting: recheck()
            }
        ),
        Err(GateError::NotOwned { .. })
    ));
}

#[tokio::test]
async fn missing_publication_always_retries_even_drop_and_defer() {
    for when in [
        DirectWhenTripped::Drop,
        DirectWhenTripped::Defer(recheck()),
        DirectWhenTripped::DeferAttempt(recheck()),
    ] {
        let (catalog, worker, halt) = world();
        let base = BaseBuilder::new(catalog, worker).unwrap().finish();
        let (_writer, reader) = Reader::live(
            base,
            PrePublication::ServeDefaults,
            Duration::from_secs(100),
            Instant::now(),
        );
        let flags = Flags::new(reader);
        let calls = Arc::new(AtomicUsize::new(0));
        let handler = flags
            .gated_direct(Probe(calls.clone()), &halt, policy(when))
            .unwrap();
        assert_retry(invoke(&handler).await.unwrap_err(), Duration::from_secs(60));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        // An ungated handler remains callable before publication.
        invoke(&flags.flagged(Probe(calls.clone()), recheck()))
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

struct Typed {
    malformed: Arc<AtomicUsize>,
    dead: Arc<Mutex<Vec<Value>>>,
}
struct Contract;
impl JobContract for Contract {
    type Payload = u32;
    fn spec() -> JobSpec {
        JobSpec::new(JobType::new("test.typed")).unwrap()
    }
}
#[async_trait]
impl FlagAwareTypedHandler for Typed {
    type Contract = Contract;
    async fn execute(
        &self,
        _: JobExecution<'_>,
        _: &JobFlags,
        value: u32,
    ) -> Result<JobCompletion, JobFailure> {
        Ok(JobCompletion::with_output(value.into()))
    }
    fn malformed_payload(&self, _: &serde_json::Error) -> JobFailure {
        self.malformed.fetch_add(1, Ordering::SeqCst);
        JobFailure::terminal("test.malformed", "Invalid payload.")
    }
    async fn on_dead_letter(&self, _: JobContext, payload: Value, _: JobDeadLetterInfo) {
        self.dead.lock().unwrap().push(payload);
    }
}

#[tokio::test]
async fn typed_decode_is_after_gate_and_dead_letter_receives_raw_payload() {
    let malformed = Arc::new(AtomicUsize::new(0));
    let dead = Arc::new(Mutex::new(Vec::new()));
    let typed = || {
        Typed {
            malformed: malformed.clone(),
            dead: dead.clone(),
        }
        .into_flag_aware()
    };
    for state in [SwitchState::Engaged, SwitchState::Released] {
        let (catalog, worker, halt) = world();
        let builder = TestFlags::builder(catalog, worker).unwrap();
        let test = match state {
            SwitchState::Engaged => builder.engage(&halt),
            SwitchState::Released => builder.release(&halt),
        }
        .build()
        .unwrap();
        let handler = test
            .flags()
            .gated_direct(typed(), &halt, policy(DirectWhenTripped::Drop))
            .unwrap();
        let failure = invoke(&handler).await.unwrap_err();
        assert_eq!(
            failure.code,
            if state == SwitchState::Engaged {
                "flags.job_type_dropped"
            } else {
                "test.malformed"
            }
        );
        let context = context();
        let raw = serde_json::json!({"undecodable": "secret-shaped"});
        handler
            .on_dead_letter(
                context,
                raw.clone(),
                JobDeadLetterInfo::new(
                    failure,
                    JobDeadLetterReason::FailureKindNonRetryable,
                    Some(1),
                    JobDeadLetterOrigin::Worker,
                ),
            )
            .await;
        assert_eq!(dead.lock().unwrap().last(), Some(&raw));
    }
    assert_eq!(malformed.load(Ordering::SeqCst), 1);
    assert_eq!(dead.lock().unwrap().len(), 2);
    let (catalog, worker, _) = world();
    let test = TestFlags::builder(catalog, worker)
        .unwrap()
        .build()
        .unwrap();
    let handler = test.flags().flagged(typed(), recheck());
    let run = HandlerInvocation::new(Duration::from_secs(5))
        .unwrap()
        .execute(&handler, 42.into())
        .await;
    assert!(
        matches!(run.exit, HandlerExit::Returned(Ok(ref c)) if c.output() == Some(&Value::from(42)))
    );
    assert_eq!(handler.job_type(), JobType::new("test.typed"));
}
fn context() -> JobContext {
    // The native context's identity is read from a JSON literal, avoiding a
    // separate UUID dependency solely for worker-free fixtures.
    serde_json::from_value(serde_json::json!({"job_id":"00000000-0000-0000-0000-000000000123", "run_number":1,"attempt":1,"worker_id":"test","organization_id":null,"checkpoint":null})).unwrap()
}

#[tokio::test]
async fn legacy_dispatch_refuses_without_entering_inner_and_spread_is_stable() {
    let (catalog, worker, halt) = world();
    let test = TestFlags::builder(catalog, worker)
        .unwrap()
        .build()
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let flagged = test.flags().flagged(Probe(calls.clone()), recheck());
    let gated = test
        .flags()
        .gated_direct(Probe(calls.clone()), &halt, policy(DirectWhenTripped::Drop))
        .unwrap();
    for handler in [&flagged as &dyn JobHandler, &gated as &dyn JobHandler] {
        assert_eq!(
            handler
                .execute(context(), Value::Null)
                .await
                .unwrap_err()
                .code,
            "job.execution_services_required"
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let mut next = context();
    next.run_number = 3;
    next.attempt = 2;
    assert_eq!(recheck().delay(&context()), recheck().delay(&next));
}

#[tokio::test]
async fn an_admitted_invocation_keeps_its_view_across_publication() {
    struct ReadTwice {
        halt: KillSwitch,
        entered: Arc<tokio::sync::Notify>,
        resume: Arc<tokio::sync::Notify>,
    }
    #[async_trait]
    impl FlagAwareHandler for ReadTwice {
        fn job_type(&self) -> JobType<'static> {
            JobType::new("test.read_twice")
        }
        async fn execute(
            &self,
            _: JobExecution<'_>,
            flags: &JobFlags,
            _: Value,
        ) -> Result<JobCompletion, JobFailure> {
            assert_eq!(flags.switch(&self.halt).value(), SwitchState::Released);
            self.entered.notify_one();
            self.resume.notified().await;
            assert_eq!(flags.switch(&self.halt).value(), SwitchState::Released);
            Ok(JobCompletion::success())
        }
    }
    let (catalog, worker, halt) = world();
    let base = BaseBuilder::new(catalog, worker).unwrap().finish();
    let candidate = |version, state| {
        let v = Version::new(version);
        Candidate::new(&base.validation_scope(), v).row(CandidateRow::switch(&halt, state, v))
    };
    let open = candidate(1, SwitchState::Released);
    let tripped = candidate(2, SwitchState::Engaged);
    let (mut writer, reader) = Reader::live(
        base,
        PrePublication::ServeDefaults,
        Duration::from_secs(100),
        Instant::now(),
    );
    writer.read_at(open, Instant::now());
    let entered = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    let handler = Flags::new(reader)
        .gated_direct(
            ReadTwice {
                halt: halt.clone(),
                entered: entered.clone(),
                resume: resume.clone(),
            },
            &halt,
            policy(DirectWhenTripped::Drop),
        )
        .unwrap();
    let run = invoke(&handler);
    let publish = async {
        entered.notified().await;
        writer.read_at(tripped, Instant::now());
        resume.notify_one();
    };
    let (result, ()) = tokio::join!(run, publish);
    result.unwrap();
    assert_eq!(
        invoke(&handler).await.unwrap_err().code,
        "flags.job_type_dropped"
    );
}

#[tokio::test]
async fn explicit_settings_floor_can_drop_before_publication() {
    let (catalog, worker, halt) = world();
    let mut base = BaseBuilder::new(catalog, worker).unwrap();
    base.switch(halt.key(), SwitchState::Engaged).unwrap();
    let (_writer, reader) = Reader::live(
        base.finish(),
        PrePublication::ServeDefaults,
        Duration::from_secs(100),
        Instant::now(),
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let handler = Flags::new(reader)
        .gated_direct(Probe(calls.clone()), &halt, policy(DirectWhenTripped::Drop))
        .unwrap();
    assert_eq!(
        invoke(&handler).await.unwrap_err().code,
        "flags.job_type_dropped"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

mod unready;
