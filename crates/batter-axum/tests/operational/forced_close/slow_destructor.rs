//! A connection whose destruction is still running when Axum counts it
//! finished: Hyper destroys the pending body or in-flight handler it holds
//! only after that, so the server waits for it or is aborted.

use super::*;

/// Destroying it hands its worker over with `block_in_place`, as a destructor
/// doing blocking work may, and waits for `release` or `hold`, then records
/// that it finished.
struct SlowDrop {
    release: std::sync::mpsc::Receiver<()>,
    hold: Duration,
    destroyed: Arc<AtomicBool>,
}

impl Drop for SlowDrop {
    fn drop(&mut self) {
        let (release, hold) = (&self.release, self.hold);
        let _ = tokio::task::block_in_place(|| release.recv_timeout(hold));
        self.destroyed.store(true, Ordering::SeqCst);
    }
}

/// One frame, then pending, owning a [`SlowDrop`].
struct SlowlyDestroyed {
    first: bool,
    _guard: SlowDrop,
}

impl http_body::Body for SlowlyDestroyed {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, Infallible>>> {
        if std::mem::take(&mut self.first) {
            return Poll::Ready(Some(Ok(http_body::Frame::data(Bytes::from_static(
                b"first",
            )))));
        }
        Poll::Pending
    }
}

/// What Hyper still holds when it ends the connection: a pending response
/// body, or an in-flight handler future.
#[derive(Clone, Copy)]
enum Holder {
    Body,
    Handler,
}

/// A served [`SlowDrop`] that sets `destroyed` once its destruction finished.
struct SlowRoute {
    release: std::sync::mpsc::Sender<()>,
    entered: Arc<AtomicBool>,
}

fn slowly_destroyed_route(
    router: Router,
    holder: Holder,
    hold: Duration,
    destroyed: &Arc<AtomicBool>,
) -> (Router, SlowRoute) {
    let (release, released) = std::sync::mpsc::channel();
    let guard = Arc::new(Mutex::new(Some(SlowDrop {
        release: released,
        hold,
        destroyed: destroyed.clone(),
    })));
    let entered = Arc::new(AtomicBool::new(false));
    let router = match holder {
        Holder::Body => router.route(
            "/",
            get(move || async move {
                let guard = guard.lock().unwrap().take().unwrap();
                Body::new(SlowlyDestroyed {
                    first: true,
                    _guard: guard,
                })
            }),
        ),
        // A fallback is outside the route layer's admission, so forced
        // cancellation does not end it: Hyper still holds it in flight when
        // the gate fails the connection's I/O.
        Holder::Handler => {
            let reached = entered.clone();
            router
                .route("/admitted", get(|| async { "admitted" }))
                .fallback(move || async move {
                    let _guard = guard.lock().unwrap().take();
                    reached.store(true, Ordering::SeqCst);
                    std::future::pending::<&'static str>().await
                })
        }
    };
    (router, SlowRoute { release, entered })
}

/// Wait until Hyper holds the slowly destroyed value.
async fn holding(client: &mut TcpStream, holder: Holder, entered: &AtomicBool) {
    match holder {
        Holder::Body => {
            read_first_frame(client).await;
        }
        Holder::Handler => timeout(WAIT, async {
            while !entered.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap(),
    }
}

/// Axum counts a closed connection finished before Hyper destroys the pending
/// body or in-flight handler it still holds. A destructor still running then
/// holds the server open: finished within the allowance, the dependency
/// finalizer runs only after it; still running at the abort deadline, the
/// wrapper is aborted and cleanup is skipped instead of running beside it.
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn cleanup_waits_for_a_slow_body_destructor_or_is_skipped_at_the_abort_deadline() {
    slow_destructor(register_http, Holder::Body).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn peer_server_cleanup_waits_for_a_slow_body_destructor_or_is_skipped() {
    slow_destructor(register_http_with_connect_info_in, Holder::Body).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn cleanup_waits_for_a_slow_in_flight_handler_destructor_or_is_skipped() {
    slow_destructor(register_http, Holder::Handler).await;
}

async fn slow_destructor(register: RegisterHttp, holder: Holder) {
    destroyed_within_the_allowance(register, holder).await;
    still_destroying_at_the_abort_deadline(register, holder).await;
}

/// Closed one second after forced cancellation; destroyed 400 ms later, well
/// before the abort deadline.
async fn destroyed_within_the_allowance(register: RegisterHttp, holder: Holder) {
    let destroyed = Arc::new(AtomicBool::new(false));
    let observed = Arc::new(Mutex::new(None));
    let mut route = None;
    let served = Served::start_with(
        register,
        Duration::from_secs(2),
        |router| {
            let (router, slow) =
                slowly_destroyed_route(router, holder, Duration::from_millis(400), &destroyed);
            route = Some(slow);
            router
        },
        |supervisor| {
            let (observed, destroyed) = (observed.clone(), destroyed.clone());
            supervisor
                .on_cleanup("observer", move || async move {
                    *observed.lock().unwrap() = Some(destroyed.load(Ordering::SeqCst));
                    Ok(())
                })
                .unwrap();
        },
    )
    .await;
    let route = route.unwrap();
    let mut client = served.request(GET).await;
    holding(&mut client, holder, &route.entered).await;
    let report = served.shut_down().await;
    drop(route);
    assert!(report.is_success(), "{report:?}");
    assert!(report.forced_cancellation);
    assert!(report.abort_requested.is_empty(), "{report:?}");
    assert!(report.cleanup.skipped.is_empty(), "{report:?}");
    assert!(destroyed.load(Ordering::SeqCst));
    assert_eq!(
        *observed.lock().unwrap(),
        Some(true),
        "cleanup ran before the held value was destroyed"
    );
    drop(client);
}

/// Still being destroyed at the abort deadline, until released afterwards.
async fn still_destroying_at_the_abort_deadline(register: RegisterHttp, holder: Holder) {
    let destroyed = Arc::new(AtomicBool::new(false));
    let mut route = None;
    let served = Served::start(register, CANCEL, |router| {
        let (router, slow) = slowly_destroyed_route(router, holder, WAIT, &destroyed);
        route = Some(slow);
        router
    })
    .await;
    let route = route.unwrap();
    let mut client = served.request(GET).await;
    holding(&mut client, holder, &route.entered).await;
    let report = served.shut_down().await;
    let still_destroying = !destroyed.load(Ordering::SeqCst);
    // Release the destructor even if assertions fail.
    let _ = route.release.send(());
    timeout(WAIT, async {
        while !destroyed.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        still_destroying,
        "the destructor finished before the report"
    );
    assert!(report.forced_cancellation);
    assert_eq!(report.abort_requested, ["http"]);
    assert_eq!(report.cleanup.skipped.len(), 1);
    assert_eq!(report.cleanup.skipped[0].name, "dependency");
    assert_eq!(report.cleanup.skipped[0].reason, SkipReason::UnsafeTaskExit);
    assert!(!served.cleaned.load(Ordering::SeqCst));
    drop(client);
}
