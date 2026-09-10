use super::{client::Client, fixture::Fixture};
use crate::capture::Capture;
use crate::http_graceful;
use std::time::Duration;

pub async fn exercise(case: &str, fixture: &Fixture, capture: &Capture) {
    let mut client = Client::connect(fixture.address, fixture.events.clone()).await;
    match case {
        "slow_failure" => {
            client.get("/fast").await;
            client.headers(200).await;
            let _drop =
                super::resource::DropEvent(fixture.events.clone(), "exercise-resource-drop");
            std::future::pending::<()>().await;
        }
        "missing_exercise_event" | "missing_wire_marker" => {
            client.get("/fast").await;
            client.headers(200).await;
            client.through(b"\r\n\r\nok").await;
            if case == "missing_wire_marker" {
                client.through(b"never-on-wire").await;
            } else {
                fixture.events.wait("never-exercised").await;
            }
        }
        "dual_failure" => {
            client.get("/fast").await;
            client.headers(200).await;
            panic!("exercise-failure-control");
        }
        "idle_keep_alive_drain"
        | "delayed_shutdown_report"
        | "missing_reconciliation_event"
        | "delayed_missing_reconciliation_event"
        | "late_handler_entry"
        | "existing_connection_drain"
        | "admission_with_graceful_withheld" => keep_alive(case, fixture, client).await,
        "admitted_handler_drain" => admitted_handler(fixture, client, capture).await,
        "incomplete_upload_deadline" => upload(fixture, client, capture).await,
        "streaming_cooperative_drain" | "blocked_body_wrapper_abort" => {
            stream(case, fixture, client, capture).await
        }
        "forced_handler_cancellation" => cancellation(fixture, client, capture).await,
        "missing_disconnect_event"
        | "disconnect_pending_handler"
        | "write_half_close_pending_handler"
        | "disconnect_streaming_body" => disconnect(case, fixture, client, capture).await,
        _ => panic!("unknown lifetime case {case}"),
    }
}
fn observe(capture: &Capture, status: u16, outcome: &str) {
    let text = capture.text();
    let lines: Vec<_> = text
        .lines()
        .filter(|line| line.contains("HTTP response boundary finished"))
        .collect();
    assert_eq!(lines.len(), 1, "{text}");
    let fields = lines[0]
        .split_once("HTTP response boundary finished")
        .unwrap()
        .1;
    assert!(
        fields
            .split_whitespace()
            .any(|field| field == format!("status={status}")),
        "{text}"
    );
    assert!(
        fields.contains(&format!("http_outcome=\"{outcome}\"")),
        "{text}"
    );
}
async fn keep_alive(case: &str, fixture: &Fixture, mut client: Client) {
    // A fully consumed prior response proves the server accepted this connection.
    client.get("/fast").await;
    client.headers(200).await;
    client.through(b"\r\n\r\nok").await;
    client.assert_fixed_complete();
    assert_eq!(fixture.events.count("fast-entry"), 1);
    fixture.drain();
    client.bytes.clear();
    if case == "admission_with_graceful_withheld" {
        admission_rejection(fixture, &mut client).await;
    } else if case == "existing_connection_drain" {
        // Drain directly drives native graceful shutdown. Transport rejection is
        // permitted here; the companion case must reach guarded routing.
        let write = client.socket.writable().await;
        assert!(write.is_ok());
        let sent = tokio::io::AsyncWriteExt::write_all(
            &mut client.socket,
            b"GET /fast HTTP/1.1\r\nHost: localhost\r\n\r\n",
        )
        .await;
        fixture.events.record(if sent.is_ok() {
            "second-request-written"
        } else {
            "second-request-write-error"
        });
    }
    if case == "existing_connection_drain" {
        client.transport_close().await;
    } else {
        client.eof().await;
    }
    assert_eq!(fixture.events.count("fast-entry"), 1);
}
async fn admitted_handler(fixture: &Fixture, mut client: Client, capture: &Capture) {
    client.get("/pending").await;
    fixture.events.wait("handler-entry").await;
    fixture.drain();
    http_graceful::wait_for_connection(|| capture.text()).await;
    fixture.events.record("connection-graceful");
    client.assert_pending().await;
    assert_eq!(
        fixture.events.count("handler-drop"),
        0,
        "handler must survive native graceful shutdown before release"
    );
    assert_eq!(fixture.events.count("response-constructed"), 0);
    assert_eq!(fixture.events.count("server-socket-drop"), 0);
    assert_eq!(fixture.events.count("server-ok"), 0);
    assert_eq!(fixture.events.count("finalizer"), 0);
    fixture.events.record("graceful-pending");
    assert_eq!(
        fixture.context.lock().unwrap().as_ref().unwrap().check(),
        Ok(())
    );
    fixture.events.record("handler-release");
    fixture.release_handler.notify_one();
    client.headers(200).await;
    client.eof().await;
    assert!(client.bytes.ends_with(b"done"));
    client.assert_fixed_complete();
    fixture.assert_cancelled();
    fixture.events.before("drain", "response-constructed");
    observe(capture, 200, "completed");
}

async fn upload(fixture: &Fixture, mut client: Client, capture: &Capture) {
    client.send(b"POST /upload HTTP/1.1\r\nHost: localhost\r\nContent-Length: 100\r\nConnection: close\r\n\r\npart").await;
    fixture.events.wait("upload-pending").await;
    fixture.drain();
    client.headers(503).await;
    client.eof().await;
    assert!(client.text().contains("deadline_exceeded"));
    client.assert_fixed_complete();
    assert_eq!(fixture.events.count("upload-complete"), 0);
    fixture.events.wait("handler-drop").await;
    fixture.assert_cancelled();
    observe(capture, 503, "server_error");
    fixture.events.before("handler-drop", "response-observed");
    fixture.events.before("upload-pending", "upload-body-drop");
    fixture
        .events
        .before("upload-body-drop", "response-observed");
}
async fn cancellation(fixture: &Fixture, mut client: Client, capture: &Capture) {
    client.get("/pending").await;
    fixture.events.wait("handler-entry").await;
    fixture.drain();
    client.headers(503).await;
    client.eof().await;
    assert!(client.text().contains("operation_cancelled"));
    client.assert_fixed_complete();
    assert!(!client.text().contains("deadline_exceeded"));
    fixture.events.wait("handler-drop").await;
    fixture.assert_cancelled();
    observe(capture, 503, "server_error");
    fixture.events.before("handler-drop", "response-observed");
}
async fn pending_stream_checkpoint(fixture: &Fixture, client: &mut Client) {
    client.assert_pending().await;
    assert_eq!(
        fixture.events.count("body-drop"),
        0,
        "body must survive native graceful shutdown before release"
    );
    assert_eq!(fixture.events.count("body-complete"), 0);
    assert_eq!(fixture.events.count("server-socket-drop"), 0);
    assert_eq!(fixture.events.count("server-ok"), 0);
    assert_eq!(fixture.events.count("finalizer"), 0);
    fixture.events.record("graceful-pending");
}

async fn body_outlives_request_deadline(fixture: &Fixture, client: &mut Client, capture: &Capture) {
    fixture.assert_cancelled();
    observe(capture, 200, "completed");
    assert_eq!(fixture.events.count("body-drop"), 0);
    assert_eq!(fixture.events.count("body-complete"), 0);
    let deadline = fixture.context.lock().unwrap().as_ref().unwrap().deadline();
    tokio::time::sleep_until(deadline + Duration::from_millis(1)).await;
    client.assert_pending().await;
    fixture.events.record("body-pending-after-request-deadline");
    assert_eq!(fixture.events.count("body-drop"), 0);
    assert!(!client.bytes.ends_with(b"0\r\n\r\n"));
}

async fn stream(case: &str, fixture: &Fixture, mut client: Client, capture: &Capture) {
    client.get("/stream").await;
    client.headers(200).await;
    client.through(b"5\r\nfirst\r\n").await;
    fixture.events.wait("body-pending").await;
    body_outlives_request_deadline(fixture, &mut client, capture).await;
    fixture.drain();
    http_graceful::wait_for_connection(|| capture.text()).await;
    fixture.events.record("connection-graceful");
    if case == "blocked_body_wrapper_abort" {
        inspect_blocked_abort(fixture, &mut client).await;
    } else {
        pending_stream_checkpoint(fixture, &mut client).await;
    }
    fixture.events.record("body-release");
    fixture.release_body.notify_one();
    client.eof().await;
    client.assert_chunk_complete();
    fixture.events.wait("body-drop").await;
    assert_completed_stream(fixture);
    observe(capture, 200, "completed");
    fixture.events.wait("server-socket-drop").await;
    if case == "blocked_body_wrapper_abort" {
        fixture.events.before("report", "body-drop");
        assert_eq!(fixture.events.count("finalizer"), 0);
    }
}
async fn disconnect(case: &str, fixture: &Fixture, mut client: Client, capture: &Capture) {
    let streaming = case == "disconnect_streaming_body";
    client
        .get(if streaming { "/stream" } else { "/pending" })
        .await;
    fixture.events.wait("handler-entry").await;
    if streaming {
        client.headers(200).await;
        client.through(b"5\r\nfirst\r\n").await;
        assert!(!client.bytes.ends_with(b"0\r\n\r\n"));
        fixture.events.wait("body-pending").await;
        observe(capture, 200, "completed");
    }
    if case == "write_half_close_pending_handler" {
        client.half_close().await;
        client.eof().await;
        assert!(client.bytes.is_empty(), "no response constructed");
    } else {
        client.close();
    }
    tokio::time::timeout(super::limits::DISCONNECT, async {
        if case == "missing_disconnect_event" {
            fixture.events.wait("never-disconnected").await;
        }
        fixture
            .events
            .wait(if streaming {
                "body-drop"
            } else {
                "handler-drop"
            })
            .await;
        fixture.events.wait("server-socket-drop").await;
    })
    .await
    .expect("transport must drop the resource and socket before release or drain");
    fixture.events.record("transport-drop-before-release");
    fixture.assert_cancelled();
    assert_eq!(fixture.events.count("body-complete"), 0);
    if streaming {
        observe(capture, 200, "completed");
    } else {
        assert_eq!(fixture.events.count("response-constructed"), 0);
        observe_dropped(capture);
    }
    fixture.drain();
}

fn observe_dropped(capture: &Capture) {
    let text = capture.text();
    let lines: Vec<_> = text
        .lines()
        .filter(|l| l.contains("HTTP response boundary finished"))
        .collect();
    assert_eq!(lines.len(), 1, "{text}");
    let fields = lines[0]
        .split_once("HTTP response boundary finished")
        .unwrap()
        .1;
    assert!(fields.contains("http_outcome=\"dropped\""), "{text}");
    assert!(!fields.contains("status="), "{text}");
}

async fn admission_rejection(fixture: &Fixture, client: &mut Client) {
    assert_eq!(fixture.events.count("graceful-signal-ready"), 0);
    client.get("/fast").await;
    client.headers(503).await;
    client.through(b"service_unavailable").await;
    assert_eq!(fixture.events.count("admission-boundary"), 2);
    assert_eq!(fixture.events.count("fast-entry"), 1);
    fixture.events.record("rejection-witnessed");
    fixture.release_graceful.notify_one();
}

async fn inspect_blocked_abort(fixture: &Fixture, client: &mut Client) {
    let report = fixture.abort_checkpoint_report().await;
    fixture.assert_aborted(&report);
    assert_eq!(
        fixture.events.count("body-drop"),
        0,
        "body still owned at report checkpoint"
    );
    assert_eq!(fixture.events.count("body-complete"), 0);
    // Probe the actual socket while the blocked resource and runtime remain
    // live. Non-observation is bounded; it is not indefinite survival.
    client.assert_pending().await;
    assert_eq!(fixture.events.count("server-socket-drop"), 0);
    fixture.events.record("body-owned-after-report");
}

fn assert_completed_stream(fixture: &Fixture) {
    assert_eq!(fixture.events.count("body-complete"), 1);
    assert_eq!(fixture.events.count("body-drop"), 1);
    fixture
        .events
        .before("handler-entry", "response-constructed");
    fixture
        .events
        .before("response-constructed", "response-observed");
    fixture.events.before("response-observed", "headers");
    fixture.events.before("headers", "body-release");
    fixture.events.before("body-release", "body-complete");
    fixture.events.before("body-complete", "body-drop");
    fixture.events.before("response-observed", "body-release");
}

// Terminal report checks belong to the separately bounded teardown owner.
pub fn check_shutdown(
    case: &str,
    fixture: &Fixture,
    report: &batter::lifecycle::SharedShutdownReport,
) {
    if case == "blocked_body_wrapper_abort" {
        fixture.assert_aborted(report);
        return;
    }
    fixture.assert_clean(report);
    if matches!(
        case,
        "idle_keep_alive_drain"
            | "existing_connection_drain"
            | "admission_with_graceful_withheld"
            | "delayed_shutdown_report"
            | "missing_reconciliation_event"
            | "delayed_missing_reconciliation_event"
            | "late_handler_entry"
    ) {
        assert_eq!(
            fixture.events.count("fast-entry"),
            1,
            "terminal handler entry count"
        );
    }
    match case {
        "forced_handler_cancellation" => {
            assert!(report.forced_cancellation);
            fixture.events.before("handler-drop", "server-ok");
        }
        "admitted_handler_drain" => {
            assert!(!report.forced_cancellation);
            fixture.events.before("handler-drop", "server-ok");
        }
        "streaming_cooperative_drain" => fixture.events.before("body-drop", "server-ok"),
        "idle_keep_alive_drain"
        | "existing_connection_drain"
        | "admission_with_graceful_withheld"
        | "delayed_shutdown_report" => assert!(!report.forced_cancellation),
        _ => {}
    }
    if case == "delayed_shutdown_report" {
        fixture.events.before("exercise-complete", "finalizer");
        fixture.events.before("finalizer", "terminal-report");
    }
}
