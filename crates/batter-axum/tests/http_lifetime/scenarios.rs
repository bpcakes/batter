use super::{
    client::Client,
    http_graceful, limits,
    server::{Access, Server},
    state::State,
    trace_capture::Capture,
};
use batter::{
    BoxError,
    cleanup::SkipReason,
    lifecycle::{Readiness, SharedShutdownReport, ShutdownCause, TaskOutcome},
};
use std::{io::Write, sync::Arc, time::Duration};

fn emit(event: &str) {
    let mut output = std::io::stdout().lock();
    writeln!(output, "\nbatter-fixture:{event}").unwrap();
    output.flush().unwrap();
}

pub fn run(scenario: &str) {
    if scenario == "http-stall" {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                emit("http-runtime-stalled");
                loop {
                    std::thread::park();
                }
            });
        return;
    }
    let capture = Arc::new(Capture::with_filter(http_graceful::FILTER));
    capture.block_on(async {
        let state = State::default();
        let deadline = tokio::time::Instant::now() + limits::STARTUP;
        let built = tokio::time::timeout_at(
            deadline,
            Server::start(scenario, capture.clone(), state.clone()),
        )
        .await
        .map_err(|error| -> BoxError { error.into() })
        .and_then(|result| result);
        let server = built.unwrap_or_else(|error| {
            panic!(
                "startup construction: {error:?}; teardown: no running owner; events: {:?}; trace: {}",
                state.snapshot(), capture.text()
            )
        });
        let startup = tokio::time::timeout_at(deadline, server.access.handle.wait_ready()).await;
        let access = server.access.clone();
        let name = scenario.to_owned();
        let exercise_limit = if matches!(scenario, "http-missing-event" | "http-teardown-stuck") {
            limits::MISSING_EVENT
        } else {
            limits::EXERCISE
        };
        // Join the exercise even on panic, then independently drive teardown.
        // Both failures are retained in the outer assertion.
        let exercise = if matches!(startup, Ok(Ok(()))) {
            Some(tokio::spawn(async move {
                tokio::time::timeout(exercise_limit, exercise(&name, access)).await
            }).await)
        } else {
            None
        };
        if matches!(exercise, Some(Ok(Ok(Ok(()))))) {
            server.access.state.record("exercise-complete");
        }
        let teardown = tokio::time::timeout(limits::TEARDOWN, server.teardown()).await;
        assert!(
            matches!(startup, Ok(Ok(())))
                && matches!(exercise, Some(Ok(Ok(Ok(())))))
                && matches!(&teardown, Ok(Ok(report)) if report.all_direct_tasks_joined()),
            "startup: {startup:?}; exercise: {exercise:?}; teardown: {teardown:?}; events: {:?}; trace: {}",
            server.access.state.snapshot(),
            capture.text(),
        );
        let report = teardown.unwrap().unwrap();
        server.access.state.record("terminal-report");
        reconcile(scenario, &server.access, &report);
        check_observation(scenario, &server.access.state, &capture.text());
    });
}

fn check_observation(scenario: &str, state: &State, text: &str) {
    let completions: Vec<_> = text
        .lines()
        .filter(|line| line.contains("HTTP response boundary finished"))
        .collect();
    let expected = if scenario == "http-admission" { 2 } else { 1 };
    if scenario == "http-reject" {
        if state.count("drain-request-rejected") == 1 {
            assert_eq!(completions.len(), 2, "{text}");
        } else {
            // A socket can close after response construction but before delivery;
            // closure alone cannot prove the absence of an observed response.
            assert!((1..=2).contains(&completions.len()), "{text}");
        }
        if completions.len() == 2 {
            assert!(
                completions[1].contains("status=503")
                    && completions[1].contains("http_outcome=\"server_error\""),
                "{text}"
            );
        }
    } else {
        assert_eq!(completions.len(), expected, "{text}");
    }
    if scenario == "http-admission" {
        assert!(
            completions[1].contains("status=503")
                && completions[1].contains("http_outcome=\"server_error\""),
            "{text}"
        );
    }
    let event = completions[0];
    match scenario {
        "http-upload" | "http-cancel" => {
            assert!(
                event.contains("status=503") && event.contains("http_outcome=\"server_error\""),
                "{event}"
            );
        }
        "http-disconnect-handler" => {
            assert!(
                event.contains("http_outcome=\"dropped\"") && !event.contains("status="),
                "{event}"
            );
        }
        _ => assert!(
            event.contains("status=200") && event.contains("http_outcome=\"completed\""),
            "{event}"
        ),
    }
}

#[test]
fn rejection_trace_distinguishes_received_response_from_transport_closure() {
    let first = "HTTP response boundary finished status=200 http_outcome=\"completed\"";
    let both = format!(
        "{first}\nHTTP response boundary finished status=503 http_outcome=\"server_error\""
    );
    let rejected = State::default();
    rejected.record("drain-request-rejected");
    assert!(
        std::panic::catch_unwind(|| check_observation("http-reject", &rejected, first)).is_err()
    );
    check_observation("http-reject", &rejected, &both);
    let closed = State::default();
    closed.record("drain-request-transport-closed");
    check_observation("http-reject", &closed, first);
    check_observation("http-reject", &closed, &both);
}

async fn exercise(scenario: &str, access: Access) -> Result<(), BoxError> {
    let client = Client::connect(access.address, access.state.clone()).await?;
    match scenario {
        "http-missing-event" | "http-teardown-stuck" => {
            access.state.wait("never-emitted").await;
            Ok(())
        }
        "http-idle"
        | "http-reject"
        | "http-admission"
        | "http-delayed-report"
        | "http-late-handler" => keep_alive(scenario, access, client).await,
        "http-handler" | "http-cancel" | "http-upload" => handler(scenario, access, client).await,
        "http-stream" | "http-abort" => streaming(scenario, access, client).await,
        "http-disconnect-handler" | "http-disconnect-body" => {
            disconnect(scenario, access, client).await
        }
        _ => panic!("unknown HTTP scenario"),
    }
}

fn clean_report(access: &Access, report: &SharedShutdownReport) {
    assert!(report.is_success(), "{report:?}");
    assert_eq!(report.cause, ShutdownCause::Requested);
    assert_eq!(report.tasks.len(), 1);
    assert_eq!(report.tasks[0].name, "http.server");
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Stopped);
    assert!(report.tasks[0].error.is_none());
    assert!(report.abort_requested.is_empty());
    assert!(report.unjoined.is_empty());
    assert_eq!(access.state.count("cleanup"), 1);
    access.state.before("server-ok", "cleanup");
    access.state.before("server-dropped", "cleanup");
}

async fn keep_alive(scenario: &str, access: Access, mut client: Client) -> Result<(), BoxError> {
    client.send("/fast").await?;
    assert_eq!(client.response().await?, (200, b"ok".to_vec()));
    assert_eq!(access.state.count("handler-entered"), 1);
    access.handle.request();
    assert_eq!(access.handle.readiness(), Readiness::Draining);
    if scenario == "http-admission" {
        access.state.wait("drain-observed").await;
        client.send("/fast").await?;
        assert_eq!(client.response().await?.0, 503);
        assert_eq!(access.state.count("admission-reached"), 2);
        assert_eq!(access.state.count("handler-entered"), 1);
        assert_eq!(access.state.count("graceful-signal-ready"), 0);
        access.state.graceful.notify_one();
    } else if scenario == "http-reject" {
        rejected_or_closed(&mut client).await;
        assert_eq!(
            access.state.count("drain-request-rejected")
                + access.state.count("drain-request-transport-closed"),
            1
        );
    }
    client.eof().await?;
    assert_eq!(access.state.count("handler-entered"), 1);
    Ok(())
}

async fn rejected_or_closed(client: &mut Client) {
    // Ordinary graceful shutdown may close transport before routing. Only
    // closure/reset or the configured rejection is permitted.
    match client.send("/fast").await {
        Ok(()) => match client.response().await {
            Ok((status, _)) => {
                assert_eq!(status, 503);
                client.state.record("drain-request-rejected");
                emit("drain-request-rejected");
            }
            Err(error) => record_transport_rejection(client, &error),
        },
        Err(error) => record_transport_rejection(client, &error),
    }
}

fn record_transport_rejection(client: &Client, error: &std::io::Error) {
    assert!(is_closed(error), "{error}");
    client.state.record("drain-request-transport-closed");
    emit("drain-request-transport-closed");
}

fn is_closed(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionAborted
    )
}

async fn handler(scenario: &str, access: Access, mut client: Client) -> Result<(), BoxError> {
    if scenario == "http-upload" {
        client
            .raw("POST /upload HTTP/1.1\r\nHost: localhost\r\nContent-Length: 100\r\n\r\nx")
            .await?;
    } else {
        client.send("/pending").await?;
    }
    access.state.wait("handler-entered").await;
    if scenario != "http-upload" {
        drain_handler(scenario, &access).await;
    }
    if scenario == "http-handler" {
        access.state.record("handler-release");
        access.state.release.notify_one();
    }
    check_handler_response(scenario, &mut client).await?;
    access.state.wait("handler-dropped").await;
    assert!(access.state.context_cancelled());
    if scenario == "http-upload" {
        access.handle.request();
    }
    client.eof().await?;
    assert_eq!(
        access.state.count("response-constructed"),
        usize::from(scenario == "http-handler")
    );
    Ok(())
}

async fn drain_handler(scenario: &str, access: &Access) {
    assert!(!access.state.context_cancelled());
    access.handle.request();
    access.state.wait("drain-observed").await;
    if scenario == "http-handler" {
        http_graceful::wait_for_connection(|| access.trace.text()).await;
        access.state.record("connection-graceful");
        tokio::time::sleep(http_graceful::PENDING_WINDOW).await;
        assert_eq!(
            access.state.count("handler-dropped"),
            0,
            "handler must survive native graceful shutdown before release"
        );
        assert!(!access.state.context_cancelled());
        assert_eq!(access.state.count("response-constructed"), 0);
        assert_eq!(access.state.count("server-ok"), 0);
        assert_eq!(access.state.count("cleanup"), 0);
        access.state.record("graceful-pending");
    }
}

async fn check_handler_response(scenario: &str, client: &mut Client) -> Result<(), BoxError> {
    let (status, body) = client.response().await?;
    assert_eq!(status, if scenario == "http-handler" { 200 } else { 503 });
    if scenario == "http-handler" {
        assert_eq!(body, b"done");
    } else {
        let code = if scenario == "http-cancel" {
            "operation_cancelled"
        } else {
            "deadline_exceeded"
        };
        assert!(std::str::from_utf8(&body)?.contains(code));
    }
    Ok(())
}

async fn pending_stream_checkpoint(access: &Access) {
    tokio::time::sleep(http_graceful::PENDING_WINDOW).await;
    assert_eq!(
        access.state.count("body-dropped"),
        0,
        "body must survive native graceful shutdown before release"
    );
    assert_eq!(access.state.count("body-complete"), 0);
    assert_eq!(access.state.count("server-ok"), 0);
    assert_eq!(access.state.count("cleanup"), 0);
    access.state.record("graceful-pending");
}

async fn streaming(scenario: &str, access: Access, mut client: Client) -> Result<(), BoxError> {
    client.send("/stream").await?;
    let head = client.head().await?;
    assert_eq!(head.status, 200);
    access.state.wait("body-pending").await;
    check_observation(scenario, &access.state, &access.trace.text());
    assert!(access.state.context_cancelled());
    assert_eq!(access.state.count("body-dropped"), 0);
    assert_eq!(access.state.count("body-complete"), 0);
    access.handle.request();
    http_graceful::wait_for_connection(|| access.trace.text()).await;
    access.state.record("connection-graceful");
    if scenario == "http-abort" {
        aborted_report(&access).await;
    } else {
        pending_stream_checkpoint(&access).await;
    }
    access.state.record("body-release");
    access.state.release.notify_one();
    assert_eq!(client.body(&head).await?, b"startend");
    client.eof().await?;
    access.state.wait("body-dropped").await;
    access
        .state
        .before("response-constructed", "headers-received");
    access.state.before("body-release", "body-complete");
    access.state.before("body-complete", "body-dropped");
    if scenario == "http-abort" {
        access
            .state
            .before("abort-report-inspected", "body-dropped");
        assert_eq!(access.state.count("cleanup"), 0);
    }
    Ok(())
}

async fn aborted_report(access: &Access) {
    let report = access.abort_checkpoint_report().await;
    assert!(!report.is_success());
    assert_eq!(report.abort_requested, ["http.server"]);
    assert!(report.unjoined.is_empty());
    assert_eq!(report.tasks.len(), 1);
    assert_eq!(report.tasks[0].name, "http.server");
    assert_eq!(report.tasks[0].outcome, TaskOutcome::Aborted);
    assert!(
        report.tasks[0]
            .error
            .as_ref()
            .unwrap()
            .downcast_ref::<tokio::task::JoinError>()
            .unwrap()
            .is_cancelled()
    );
    assert_eq!(report.cleanup.skipped[0].name, "dependency");
    assert_eq!(report.cleanup.skipped[0].reason, SkipReason::UnsafeTaskExit);
    assert!(report.cleanup.records.is_empty());
    assert_eq!(access.state.count("cleanup"), 0);
    assert_eq!(access.state.count("server-ok"), 0);
    assert_eq!(access.state.count("body-dropped"), 0);
    assert_eq!(access.state.count("body-complete"), 0);
    access.state.record("abort-report-inspected");
}

async fn disconnect(scenario: &str, access: Access, mut client: Client) -> Result<(), BoxError> {
    let streaming = scenario == "http-disconnect-body";
    client
        .send(if streaming { "/stream" } else { "/pending" })
        .await?;
    access.state.wait("handler-entered").await;
    if streaming {
        assert_eq!(client.head().await?.status, 200);
        access.state.wait("body-pending").await;
    }
    client.disconnect()?;
    let dropped = if streaming {
        "body-dropped"
    } else {
        "handler-dropped"
    };
    tokio::time::timeout(Duration::from_secs(1), access.state.wait(dropped)).await
        .expect("resolved HTTP/1.1 transport must drop the resource after full disconnect before test release");
    access.state.before("client-full-close", dropped);
    assert!(access.state.context_cancelled());
    assert_eq!(access.state.count("body-complete"), 0);
    assert_eq!(
        access.state.count("response-constructed"),
        usize::from(streaming)
    );
    access.handle.request();
    Ok(())
}

fn reconcile(scenario: &str, access: &Access, report: &SharedShutdownReport) {
    if scenario == "http-abort" {
        // The exercise already inspected the intentional report before body release.
        assert_eq!(report.abort_requested, ["http.server"]);
        return;
    }
    clean_report(access, report);
    if matches!(
        scenario,
        "http-idle"
            | "http-reject"
            | "http-admission"
            | "http-delayed-report"
            | "http-late-handler"
    ) {
        assert_eq!(
            access.state.count("handler-entered"),
            1,
            "terminal handler entry count"
        );
    }
    match scenario {
        "http-handler" | "http-cancel" | "http-upload" => {
            assert_eq!(report.forced_cancellation, scenario == "http-cancel");
            access.state.before("handler-dropped", "cleanup");
        }
        "http-stream" => access.state.before("body-dropped", "server-ok"),
        "http-delayed-report" => {
            access.state.before("exercise-complete", "cleanup");
            access.state.before("cleanup", "terminal-report");
        }
        _ => {}
    }
}
