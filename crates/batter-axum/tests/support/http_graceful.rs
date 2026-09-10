//! Version-specific observation of Axum 0.8.9's native connection shutdown.
use std::time::Duration;

pub const FILTER: &str = "info,axum::serve=trace";
pub const PENDING_WINDOW: Duration = Duration::from_millis(50);
const CONNECTION_EVENT: &str =
    "TRACE axum::serve: signal received in task, starting graceful shutdown";

pub async fn wait_for_connection(text: impl Fn() -> String) {
    // Axum emits this event immediately before synchronous graceful_shutdown(),
    // with no intervening await. On this runtime, the observing test cannot run
    // until that call returns. The producer/accept-loop events prove less.
    // Re-check this ordering when changing Axum: see docs/references.md.
    assert_eq!(
        tokio::runtime::Handle::current().runtime_flavor(),
        tokio::runtime::RuntimeFlavor::CurrentThread,
        "native graceful acknowledgement requires a current-thread fixture"
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let count = connection_events(&text());
            if count != 0 {
                assert_eq!(count, 1, "fixture must have exactly one native connection");
                return;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "missing native connection graceful acknowledgement: {}",
            text()
        )
    });
}

fn connection_events(text: &str) -> usize {
    text.lines()
        .filter(|line| line.ends_with(CONNECTION_EVENT))
        .count()
}

#[test]
fn producer_and_accept_loop_events_do_not_acknowledge_a_connection() {
    let text = "graceful-signal-ready\n\
        TRACE axum::serve: received graceful shutdown signal. Telling tasks to shutdown\n\
        TRACE axum::serve: signal received, not accepting new connections\n\
        TRACE fixture: signal received in task, starting graceful shutdown\n";
    assert_eq!(connection_events(text), 0);
    assert_eq!(connection_events(&format!("{text}{CONNECTION_EVENT}\n")), 1);
}
