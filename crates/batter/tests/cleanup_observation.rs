#[path = "../../../test-support/dispatch.rs"]
mod test_dispatch;

// Isolate scoped log capture from cleanup tests that first register the same
// tracing callsites without a subscriber on other threads.
use batter::{
    BoxError,
    cleanup::{CleanupBudget, CleanupOutcome, CleanupReport, CleanupStack, SkipReason},
};
use std::{
    future::pending,
    io::{self, Write},
    sync::{Arc, Mutex},
    time::Duration,
};
use tracing::instrument::WithSubscriber;

#[derive(Clone)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl Write for LogBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct CaptureOrder {
    name: &'static str,
    dropped: Arc<Mutex<Vec<&'static str>>>,
}

impl Drop for CaptureOrder {
    fn drop(&mut self) {
        self.dropped.lock().unwrap().push(self.name);
    }
}

struct SkippedEvidence {
    report: CleanupReport,
    logs: String,
    factories_invoked: Vec<&'static str>,
    captures_dropped: Vec<&'static str>,
}

async fn exhaust_budget_before_dependencies(names: &[&'static str]) -> SkippedEvidence {
    let output = Arc::new(Mutex::new(Vec::new()));
    let writer = LogBuffer(output.clone());
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::INFO)
        .finish();
    let invoked = Arc::new(Mutex::new(Vec::new()));
    let dropped = Arc::new(Mutex::new(Vec::new()));
    let mut cleanup = CleanupStack::new();
    for &name in names {
        let capture = CaptureOrder {
            name,
            dropped: dropped.clone(),
        };
        let invoked = invoked.clone();
        cleanup
            .push(name, move || {
                invoked.lock().unwrap().push(name);
                async move {
                    drop(capture);
                    Ok(())
                }
            })
            .unwrap();
    }
    cleanup
        .push("uses.budget", pending::<Result<(), BoxError>>)
        .unwrap();
    let report = cleanup
        .close(
            CleanupBudget::new(
                Duration::from_millis(5),
                Duration::from_secs(1),
                Duration::from_millis(1),
            )
            .unwrap(),
        )
        .with_subscriber(test_dispatch::new(subscriber))
        .await;
    SkippedEvidence {
        report,
        logs: String::from_utf8(output.lock().unwrap().clone()).unwrap(),
        factories_invoked: invoked.lock().unwrap().clone(),
        captures_dropped: dropped.lock().unwrap().clone(),
    }
}

#[tokio::test(start_paused = true)]
async fn budget_exhaustion_reports_and_logs_each_skipped_hook_once_in_dependency_order() {
    let cases: &[(&[&str], &[&str])] = &[
        (&["dependency.close"], &["dependency.close"]),
        (
            &["root.close", "middle.close", "leaf.close"],
            &["leaf.close", "middle.close", "root.close"],
        ),
    ];
    for (registered, expected) in cases {
        let evidence = exhaust_budget_before_dependencies(registered).await;
        assert_eq!(evidence.report.records.len(), 1);
        assert_eq!(evidence.report.records[0].name, "uses.budget");
        assert_eq!(evidence.report.records[0].outcome, CleanupOutcome::TimedOut);
        assert!(!evidence.report.is_success());
        assert!(evidence.factories_invoked.is_empty());
        assert_eq!(
            evidence
                .report
                .skipped
                .iter()
                .map(|hook| hook.name)
                .collect::<Vec<_>>(),
            *expected
        );
        assert!(
            evidence
                .report
                .skipped
                .iter()
                .all(|hook| hook.reason == SkipReason::BudgetExhausted)
        );
        let events = evidence
            .logs
            .lines()
            .filter(|line| line.contains("cleanup skipped"))
            .collect::<Vec<_>>();
        assert_eq!(events.len(), expected.len(), "{}", evidence.logs);
        for (event, name) in events.iter().zip(*expected) {
            assert!(event.contains("WARN"), "{event}");
            assert!(event.contains(&format!("cleanup=\"{name}\"")), "{event}");
            assert!(event.contains("reason=BudgetExhausted"), "{event}");
        }
    }
}

#[tokio::test(start_paused = true)]
async fn budget_exhaustion_drops_skipped_captures_in_dependency_order_without_invoking_factories() {
    let evidence =
        exhaust_budget_before_dependencies(&["root.close", "middle.close", "leaf.close"]).await;
    assert!(evidence.factories_invoked.is_empty());
    assert_eq!(
        evidence.captures_dropped,
        ["leaf.close", "middle.close", "root.close"]
    );
}
