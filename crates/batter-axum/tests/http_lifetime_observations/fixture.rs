use super::{
    resource::{DropEvent, Events, StreamBody, UploadBody},
    transport::TrackedListener,
};
use axum::{
    Extension, Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use batter::{
    cleanup::SkipReason,
    lifecycle::{
        Readiness, RunningSupervisor, SharedShutdownReport, ShutdownHandle, Supervisor,
        SupervisorObserver, TaskOutcome,
    },
    operation::OperationContext,
};
use batter_axum::{RequestPolicy, observe_http, request_admission};
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{net::TcpListener, sync::Notify};

#[derive(Clone)]
pub struct Fixture {
    pub events: Events,
    pub handle: ShutdownHandle,
    observer: SupervisorObserver,
    abort_checkpoint: bool,
    pub address: SocketAddr,
    pub release_handler: Arc<Notify>,
    pub release_body: Arc<Notify>,
    pub release_graceful: Arc<Notify>,
    pub context: Arc<Mutex<Option<OperationContext>>>,
}
#[derive(Clone)]
struct HandlerState {
    events: Events,
    release_handler: Arc<Notify>,
    release_body: Arc<Notify>,
    context: Arc<Mutex<Option<OperationContext>>>,
}
impl HandlerState {
    fn enter(&self, context: OperationContext) -> DropEvent {
        *self.context.lock().unwrap() = Some(context);
        self.events.record("handler-entry");
        DropEvent(self.events.clone(), "handler-drop")
    }
}
async fn pending_handler(
    State(state): State<HandlerState>,
    Extension(context): Extension<OperationContext>,
) -> &'static str {
    let _drop = state.enter(context);
    state.release_handler.notified().await;
    state.events.record("response-constructed");
    "done"
}
async fn upload(
    State(state): State<HandlerState>,
    Extension(context): Extension<OperationContext>,
    request: Request,
) -> Response {
    let _drop = state.enter(context);
    // Announce the exact poll boundary, not an elapsed-time guess about upload entry.
    state.events.record("upload-read");
    let body = Body::new(UploadBody {
        inner: Some(request.into_body()),
        events: state.events.clone(),
    });
    let bytes = to_bytes(body, 1024).await.unwrap();
    state.events.record("upload-complete");
    bytes.into_response()
}
async fn streaming(
    State(state): State<HandlerState>,
    Extension(context): Extension<OperationContext>,
) -> Body {
    let _drop = state.enter(context);
    state.events.record("response-constructed");
    Body::new(StreamBody {
        events: state.events.clone(),
        first: true,
        released: Some(Box::pin(async move { state.release_body.notified().await })),
    })
}
impl Fixture {
    pub async fn start(case: &str) -> (Self, RunningSupervisor) {
        let events = Events::default();
        let mut supervisor = Supervisor::new(super::limits::shutdown(case));
        let handle = supervisor.handle();
        let state = HandlerState {
            events: events.clone(),
            release_handler: Arc::default(),
            release_body: Arc::default(),
            context: Arc::default(),
        };
        let policy = RequestPolicy::new(
            handle.clone(),
            if matches!(
                case,
                "incomplete_upload_deadline"
                    | "streaming_cooperative_drain"
                    | "blocked_body_wrapper_abort"
            ) {
                Duration::from_millis(150)
            } else {
                Duration::from_secs(60)
            },
        )
        .unwrap();
        let app = router(state.clone(), policy);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let release_graceful = Arc::new(Notify::new());
        register_server(
            &mut supervisor,
            listener,
            app,
            events.clone(),
            release_graceful.clone(),
            case,
        );
        let cleanup_events = events.clone();
        let cleanup_failure = case == "dual_failure";
        let delayed_report = matches!(
            case,
            "delayed_shutdown_report" | "delayed_missing_reconciliation_event"
        );
        let late_entry = case == "late_handler_entry";
        supervisor
            .on_cleanup("dependency", move || async move {
                if delayed_report {
                    tokio::time::sleep(super::limits::REPORT_DELAY).await;
                }
                if late_entry {
                    cleanup_events.wait("exercise-complete").await;
                    cleanup_events.record("fast-entry");
                }
                cleanup_events.record("finalizer");
                if cleanup_failure {
                    return Err(std::io::Error::other("cleanup-failure-control").into());
                }
                Ok(())
            })
            .unwrap();
        let running = supervisor.start();
        handle.mark_ready();
        // Return ownership before readiness waiting: the driver must retain
        // this running supervisor even when the startup deadline expires.
        let fixture = Self {
            events,
            handle,
            observer: running.observer(),
            abort_checkpoint: case == "blocked_body_wrapper_abort",
            address,
            release_handler: state.release_handler,
            release_body: state.release_body,
            release_graceful,
            context: state.context,
        };
        (fixture, running)
    }
    pub fn drain(&self) {
        self.handle.request();
        assert_eq!(self.handle.readiness(), Readiness::Draining);
        self.events.record("drain");
    }
    pub fn assert_cancelled(&self) {
        assert!(
            self.context.lock().unwrap().as_ref().unwrap().check()
                == Err(batter::operation::Interruption::Cancelled)
        );
        self.events.record("context-cancelled");
    }
    pub async fn abort_checkpoint_report(&self) -> SharedShutdownReport {
        assert!(self.abort_checkpoint, "terminal report belongs to teardown");
        // This deliberate mid-exercise observation proves that the body outlives
        // wrapper abort. The enclosing exercise owns its only diagnostic deadline.
        let report = self.observer.wait().await.unwrap();
        self.events.record("report");
        report
    }
    pub fn assert_clean(&self, report: &SharedShutdownReport) {
        assert!(report.is_success(), "{report:?}");
        assert_eq!(report.tasks.len(), 1);
        assert_eq!(report.tasks[0].name, "http");
        assert_eq!(report.tasks[0].outcome, TaskOutcome::Stopped);
        assert!(report.tasks[0].error.is_none());
        assert_eq!(report.cleanup.records.len(), 1);
        assert_eq!(self.events.count("finalizer"), 1);
        self.events.before("server-ok", "finalizer");
        self.events.before("server-socket-drop", "server-ok");
    }
    pub fn assert_aborted(&self, report: &SharedShutdownReport) {
        assert!(!report.is_success());
        assert!(report.forced_cancellation);
        assert_eq!(report.abort_requested, ["http"]);
        assert!(report.unjoined.is_empty(), "{report:?}");
        assert_eq!(report.tasks.len(), 1);
        assert_eq!(report.tasks[0].name, "http");
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
        assert_eq!(report.cleanup.skipped.len(), 1);
        assert_eq!(report.cleanup.skipped[0].name, "dependency");
        assert_eq!(report.cleanup.skipped[0].reason, SkipReason::UnsafeTaskExit);
        assert_eq!(self.events.count("finalizer"), 0);
        assert_eq!(self.events.count("server-ok"), 0);
        assert_eq!(self.events.count("server-drop"), 1);
    }
}
fn router(state: HandlerState, policy: RequestPolicy) -> Router {
    let boundary = state.events.clone();
    let response = state.events.clone();
    let fast = state.events.clone();
    Router::new()
        .route(
            "/fast",
            get(move || async move {
                fast.record("fast-entry");
                "ok"
            }),
        )
        .route("/pending", get(pending_handler))
        .route("/upload", post(upload))
        .route("/stream", get(streaming))
        .with_state(state)
        .route_layer(middleware::from_fn(move |request: Request, next: Next| {
            let policy = policy.clone();
            let boundary = boundary.clone();
            async move {
                boundary.record("admission-boundary");
                request_admission(State(policy), request, next).await
            }
        }))
        .layer(middleware::from_fn(observe_http))
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            let response = response.clone();
            async move {
                let result = next.run(request).await;
                response.record("response-observed");
                result
            }
        }))
}
fn register_server(
    supervisor: &mut Supervisor,
    listener: TcpListener,
    app: Router,
    events: Events,
    release: Arc<Notify>,
    case: &str,
) {
    let withheld = case == "admission_with_graceful_withheld";
    let acknowledge = case != "startup_timeout";
    supervisor
        .register("http", move |shutdown| async move {
            let _drop = DropEvent(events.clone(), "server-drop");
            if acknowledge {
                shutdown.mark_started();
            }
            let signal_events = events.clone();
            let listener = TrackedListener {
                inner: listener,
                events: events.clone(),
            };
            let result = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    shutdown.draining().await;
                    if withheld {
                        release.notified().await;
                    }
                    signal_events.record("graceful-signal-ready");
                })
                .await;
            events.record(if result.is_ok() {
                "server-ok"
            } else {
                "server-error"
            });
            result.map_err(Into::into)
        })
        .unwrap();
}
