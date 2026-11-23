use super::*;
use crate::test_logs::{LogBuffer, SentinelPayload, capture_logs, panicked_join_error};

fn job() -> JobObserverLogContext {
    JobObserverLogContext {
        job_id: Uuid::nil(),
        job_type: "jobs.test.worker.observer.redaction".to_owned(),
        run_number: 1,
        attempt: 1,
    }
}

fn assert_classified(log: &LogBuffer, message: &str) {
    log.assert_redacted_event(message);
    let text = log.text();
    assert!(text.contains("task_id="), "{text}");
    assert!(text.contains("is_panic=true"), "{text}");
    assert!(text.contains("is_cancelled=false"), "{text}");
}

/// A task set holding one task that has already panicked with the sentinel.
async fn panicked_task_set(payload: SentinelPayload) -> TaskSet<()> {
    let mut tasks = TaskSet::new();
    let task = tasks.spawn(async move { payload.raise() });
    while !task.is_finished() {
        tokio::task::yield_now().await;
    }
    tasks
}

#[tokio::test]
async fn running_notification_join_logs_omit_the_payload() {
    type Helper = fn(&JobObserverLogContext, &tokio::task::JoinError);
    let helpers: [(Helper, &str); 3] = [
        (
            log_running_notification_panic,
            "job running observer task panicked; continuing worker job task",
        ),
        (
            log_running_notification_cancelled,
            "job running observer task was cancelled; continuing worker job task",
        ),
        (
            log_running_notification_join_failure,
            "job running observer task join failed; continuing worker job task",
        ),
    ];
    for payload in SentinelPayload::ALL {
        let log = LogBuffer::default();
        let _capture = capture_logs(&log);
        log_running_notification_join_error(&job(), Arc::new(panicked_join_error(payload).await));
        assert_classified(&log, helpers[0].1);

        // Each helper is safe even when handed a join error it does not expect.
        for (helper, message) in helpers {
            let error = panicked_join_error(payload).await;
            let log = LogBuffer::default();
            let _capture = capture_logs(&log);
            helper(&job(), &error);
            assert_classified(&log, message);
        }
    }
}

#[tokio::test]
async fn observer_task_set_join_logs_omit_the_payload() {
    type Drain = fn(&mut TaskSet<()>);
    for payload in SentinelPayload::ALL {
        let cases: [(&str, Drain); 2] = [
            (
                "terminal observer task crashed",
                drain_finished_terminal_observer_tasks,
            ),
            (
                "running observer drain task crashed",
                drain_finished_running_observer_tasks,
            ),
        ];
        for (message, drain) in cases {
            let mut tasks = panicked_task_set(payload).await;
            let log = LogBuffer::default();
            let _capture = capture_logs(&log);
            drain(&mut tasks);
            assert_classified(&log, message);
        }

        let mut tasks = panicked_task_set(payload).await;
        let log = LogBuffer::default();
        let capture = capture_logs(&log);
        drain_terminal_observer_tasks(&mut tasks).await;
        assert_classified(
            &log,
            "terminal observer task crashed while draining worker shutdown",
        );
        drop(capture);

        let mut tasks = panicked_task_set(payload).await;
        let log = LogBuffer::default();
        let capture = capture_logs(&log);
        assert_eq!(drain_aborted_terminal_observer_tasks(&mut tasks).await, 0);
        assert_classified(
            &log,
            "terminal observer task failed after worker shutdown abort",
        );
        drop(capture);

        let mut tasks = panicked_task_set(payload).await;
        let log = LogBuffer::default();
        let _capture = capture_logs(&log);
        assert_eq!(drain_aborted_running_observer_tasks(&mut tasks).await, 0);
        assert_classified(
            &log,
            "running observer drain task failed after worker shutdown abort",
        );
    }
}
