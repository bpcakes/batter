use super::{
    limits,
    state::{ControlledBody, Lifetime, State},
    trace_capture::Capture,
};
use axum::{
    Extension, Router,
    body::{Body, to_bytes},
    extract::{Request, State as AxumState},
    middleware::{self, Next},
    response::Response,
    routing::get,
};
use batter::{
    BoxError,
    lifecycle::{
        RunningSupervisor, SharedShutdownReport, ShutdownHandle, Supervisor, SupervisorObserver,
    },
    operation::OperationContext,
};
use batter_axum::{RequestPolicy, observe_http, request_admission};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::net::TcpListener;

pub struct Server {
    pub running: RunningSupervisor,
    pub access: Access,
}

#[derive(Clone)]
pub struct Access {
    pub trace: Arc<Capture>,
    pub state: State,
    pub address: SocketAddr,
    pub handle: ShutdownHandle,
    observer: SupervisorObserver,
    abort_checkpoint: bool,
}

async fn boundary(AxumState(state): AxumState<State>, request: Request, next: Next) -> Response {
    state.record("admission-reached");
    next.run(request).await
}

async fn handler(
    AxumState(state): AxumState<State>,
    Extension(context): Extension<OperationContext>,
    request: Request,
) -> Response {
    *state.context.lock().unwrap() = Some(context);
    let _lifetime = Lifetime(state.clone(), "handler-dropped");
    state.record("handler-entered");
    let body = match request.uri().path() {
        "/pending" => {
            state.release.notified().await;
            Body::from("done")
        }
        "/upload" => {
            let bytes = to_bytes(request.into_body(), 1024).await.unwrap();
            Body::from(bytes)
        }
        "/stream" => Body::new(ControlledBody::new(state.clone())),
        _ => Body::from("ok"),
    };
    state.record("response-constructed");
    Response::new(body)
}

impl Server {
    pub async fn start(
        scenario: &str,
        capture: Arc<Capture>,
        state: State,
    ) -> Result<Self, BoxError> {
        let force = matches!(scenario, "http-cancel" | "http-abort");
        let budget = if scenario == "http-delayed-report" {
            crate::http_process::delayed_report_budget()
        } else {
            limits::shutdown_budget(force)
        };
        let mut supervisor = Supervisor::new(budget);
        let handle = supervisor.handle();
        let request_budget = if scenario == "http-upload" {
            Duration::from_millis(150)
        } else {
            Duration::from_secs(30)
        };
        let policy = RequestPolicy::new(handle.clone(), request_budget)?;
        let router = Router::new()
            .route("/{*path}", get(handler).post(handler))
            .route_layer(middleware::from_fn_with_state(policy, request_admission))
            .route_layer(middleware::from_fn_with_state(state.clone(), boundary))
            .layer(middleware::from_fn(observe_http))
            .with_state(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server_state = state.clone();
        let delayed = scenario == "http-admission";
        let startup_stuck = scenario == "http-startup-stuck";
        let unresponsive = startup_stuck || scenario == "http-teardown-stuck";
        let dispatch = capture.dispatch.clone();
        supervisor.register("http.server", move |signal| async move {
            let _lifetime = Lifetime(server_state.clone(), "server-dropped");
            let graceful_state = server_state.clone();
            if !startup_stuck {
                signal.mark_started();
            }
            if unresponsive {
                std::future::pending::<()>().await;
            }
            let serve = axum::serve(listener, router).with_graceful_shutdown(async move {
                signal.draining().await;
                graceful_state.record("drain-observed");
                if delayed {
                    graceful_state.graceful.notified().await;
                }
                graceful_state.record("graceful-signal-ready");
            });
            // The child scenario uses a current-thread runtime with this dispatcher
            // installed for its entire drive, covering Axum's spawned connections.
            let result = tracing::dispatcher::with_default(&dispatch, || {
                batter::telemetry::with_current_dispatch(async move { serve.await })
            })
            .await;
            server_state.record(if result.is_ok() {
                "server-ok"
            } else {
                "server-error"
            });
            result.map_err(Into::into)
        })?;
        let cleanup_state = state.clone();
        let delayed_report = scenario == "http-delayed-report";
        let late_entry = scenario == "http-late-handler";
        supervisor.on_cleanup("dependency", move || async move {
            if delayed_report {
                tokio::time::sleep(crate::http_process::REPORT_DELAY).await;
            }
            if late_entry {
                cleanup_state.wait("exercise-complete").await;
                cleanup_state.record("handler-entered");
            }
            cleanup_state.record("cleanup");
            Ok(())
        })?;
        handle.mark_ready();
        let running = supervisor.start();
        // Transfer the owner before waiting for readiness, so a timed-out
        // initialization can still drive and inspect shutdown separately.
        let observer = running.observer();
        Ok(Self {
            running,
            access: Access {
                trace: capture,
                state,
                address,
                handle,
                observer,
                abort_checkpoint: scenario == "http-abort",
            },
        })
    }

    pub async fn teardown(&self) -> Result<SharedShutdownReport, BoxError> {
        self.access.state.release.notify_one();
        self.access.state.graceful.notify_one();
        self.access.handle.request();
        let report = self
            .running
            .wait()
            .await
            .map_err(|_| std::io::Error::other("coordinator failed"))?;
        // Preserve the actual report, including after an exercise failure. The
        // caller checks direct joins without discarding abort/cleanup details.
        Ok(report)
    }
}

impl Access {
    pub async fn abort_checkpoint_report(&self) -> SharedShutdownReport {
        assert!(self.abort_checkpoint, "terminal report belongs to teardown");
        self.observer.wait().await.unwrap()
    }
}
