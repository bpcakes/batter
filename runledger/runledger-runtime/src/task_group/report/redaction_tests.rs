use super::record_result;
use crate::shutdown::ShutdownSignal;
use crate::test_logs::{LogBuffer, SentinelPayload, panicked_join_error};

#[tokio::test]
async fn supervised_loop_join_logs_identity_without_payloads() {
    let mut errors = Vec::new();
    for payload in SentinelPayload::ALL {
        errors.push(panicked_join_error(payload).await);
    }
    let cancelled = tokio::spawn(std::future::pending::<()>());
    cancelled.abort();
    errors.push(cancelled.await.expect_err("task was aborted"));

    for error in errors {
        let expected = [
            format!("task_id={}", error.id()),
            format!("is_cancelled={}", error.is_cancelled()),
            format!("is_panic={}", error.is_panic()),
        ];
        let log = LogBuffer::default();
        let writer = log.clone();
        let _capture = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_max_level(tracing::Level::DEBUG)
                .with_writer(move || writer.clone())
                .finish(),
        );
        let (shutdown, _) = ShutdownSignal::channel();
        let mut records = Vec::new();
        record_result("scheduler", Err(error), false, &mut records, &shutdown);
        let message = "supervised runtime loop join failed";
        log.assert_redacted_event(message);
        let text = log.text();
        let event = text
            .lines()
            .find(|line| line.contains(message))
            .expect("join event");
        for field in expected {
            assert!(
                event.split_whitespace().any(|part| part == field),
                "{event}"
            );
        }
        assert!(records[0].result.is_err(), "join evidence remains retained");
    }
}
