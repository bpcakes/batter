//! Isolate process-wide callsite registration from other tests' dispatchers.
#[allow(dead_code)]
#[path = "../../../test-support/process/capture.rs"]
mod capture;
#[allow(dead_code)]
#[path = "../../../test-support/process/evidence.rs"]
mod evidence;
#[path = "../../../test-support/process/launch.rs"]
mod launch;
#[path = "../../../test-support/dispatch.rs"]
mod test_dispatch;
#[allow(dead_code)]
#[path = "../../../test-support/process/timing.rs"]
mod timing;
#[allow(dead_code)]
#[path = "../../../test-support/process/watchdog.rs"]
mod watchdog;

fn subject() -> tracing::Span {
    tracing::info_span!("unscoped.first.registration")
}

#[test]
fn unscoped_first_registration_preserves_later_scoped_interest() {
    let run = watchdog::ChildRun::run_with_policy(
        "callsite",
        timing::WaitPolicy::ExitAfter(std::time::Duration::from_secs(2)),
    );
    run.validate(
        watchdog::ExpectedExit::Success,
        &["scoped-interest-preserved"],
        &[],
    )
    .unwrap();
}

#[test]
fn bootstrap_keeps_macros_off_until_the_real_dispatch_is_registered() {
    let run = watchdog::ChildRun::run_with_policy(
        "bootstrap",
        timing::WaitPolicy::ExitAfter(std::time::Duration::from_secs(2)),
    );
    run.validate(
        watchdog::ExpectedExit::Success,
        &["scoped-interest-preserved"],
        &[],
    )
    .unwrap();
}

struct BootstrapProbe;
impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for BootstrapProbe {
    fn on_register_dispatch(&self, _: &tracing::Dispatch) {
        // Called after insertion, before the real dispatch's cache rebuild.
        // The sentinel must not have enabled macros in the preceding window.
        assert_eq!(
            tracing::level_filters::LevelFilter::current(),
            tracing::level_filters::LevelFilter::OFF
        );
        std::thread::spawn(|| assert!(subject().is_disabled()))
            .join()
            .unwrap();
    }
}

#[test]
fn child_fixture() {
    if let Some(name) = launch::scenario() {
        use tracing_subscriber::prelude::*;
        let dispatch = match name.as_str() {
            "callsite" => test_dispatch::new(tracing_subscriber::registry()),
            "bootstrap" => test_dispatch::new(tracing_subscriber::registry().with(BootstrapProbe)),
            _ => panic!("unknown dispatch case"),
        };
        tracing::dispatcher::with_default(&dispatch, || {
            // First hit is on a thread with no default subscriber. The helper
            // must not enable this span or silently install a global default.
            std::thread::spawn(|| {
                assert!(tracing::dispatcher::get_default(|d| {
                    d.is::<tracing::subscriber::NoSubscriber>()
                }));
                assert!(subject().is_disabled());
            })
            .join()
            .unwrap();
            assert!(
                !subject().is_disabled(),
                "registered subscriber lost callsite interest"
            );
        });
        use std::io::Write;
        writeln!(
            std::io::stdout().lock(),
            "\nbatter-fixture:scoped-interest-preserved"
        )
        .unwrap();
    }
}
