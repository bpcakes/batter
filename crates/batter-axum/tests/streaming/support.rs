//! Shared fixtures: an in-process boundary on a paused clock, and a supervised
//! process whose drain and forced cancellation the test drives.

use crate::capture::Capture;
use axum::{
    body::{Body, Bytes, HttpBody},
    extract::Request,
    response::Response,
};
use batter_axum::{
    BodyBudget, GuardedRouter, HttpBoundary, InProcessClient, RequestPolicy,
    ResponseConstructionBudget,
};
use batter_core::{
    cleanup::CleanupBudget,
    lifecycle::{ShutdownBudget, ShutdownHandle, ShutdownReport, Supervisor},
    operation::OperationContext,
};
use std::{
    future::{Future, poll_fn},
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

pub const SECOND: Duration = Duration::from_secs(1);

/// Names the spans a registry closes.
#[derive(Clone, Default)]
pub struct Closed(Arc<Mutex<Vec<&'static str>>>);

impl Closed {
    pub fn names(&self) -> Vec<&'static str> {
        self.0.lock().unwrap().clone()
    }
}

impl<S> tracing_subscriber::Layer<S> for Closed
where
    S: tracing::Subscriber + for<'lookup> tracing_subscriber::registry::LookupSpan<'lookup>,
{
    fn on_close(&self, id: tracing::span::Id, context: tracing_subscriber::layer::Context<'_, S>) {
        if let Some(span) = context.span(&id) {
            self.0.lock().unwrap().push(span.name());
        }
    }
}
pub const FINISHED: &str = "HTTP response body finished";
pub const DESTROYED: &str = "application body destroyed";

/// Marks the application's stream being destroyed, under whatever subscriber
/// is current at that moment.
pub struct Marker;

impl Drop for Marker {
    fn drop(&mut self) {
        tracing::info!("{DESTROYED}");
    }
}

/// Run on a paused clock with the capture as this thread's only subscriber.
pub fn paused<F: Future>(capture: &Capture, future: F) -> F::Output {
    paused_with(&capture.dispatch, future)
}

/// Run on a paused clock with `dispatch` as this thread's only subscriber.
pub fn paused_with<F: Future>(dispatch: &tracing::Dispatch, future: F) -> F::Output {
    tracing::dispatcher::with_default(dispatch, || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .unwrap()
            .block_on(future)
    })
}

pub fn ready() -> ShutdownHandle {
    let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
    approval.approve();
    handle
}

pub fn budget(total: u64, idle: u64) -> BodyBudget {
    BodyBudget::new(Duration::from_secs(total), Duration::from_secs(idle)).unwrap()
}

pub async fn client(handle: &ShutdownHandle, guarded: GuardedRouter) -> InProcessClient {
    let policy = RequestPolicy::new(
        handle.operation_admission(),
        ResponseConstructionBudget::new(SECOND).unwrap(),
    )
    .with_infrastructure_json();
    HttpBoundary::new(policy)
        .assemble(guarded)
        .await
        .unwrap()
        .in_process()
}

pub async fn get(client: &InProcessClient, path: &str) -> Response {
    client
        .request(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
}

/// The next data frame, as the transport would poll it.
pub async fn next(body: &mut Body) -> Option<Result<Bytes, axum::Error>> {
    loop {
        let frame = poll_fn(|cx| Pin::new(&mut *body).poll_frame(cx)).await?;
        match frame.map(|frame| frame.into_data()) {
            Ok(Ok(data)) => return Some(Ok(data)),
            Ok(Err(_trailers)) => {}
            Err(error) => return Some(Err(error)),
        }
    }
}

/// Every data frame up to the body's end or its first error.
pub async fn drain(body: &mut Body) -> (String, Option<String>) {
    let mut text = String::new();
    loop {
        match next(body).await {
            Some(Ok(data)) => text.push_str(std::str::from_utf8(&data).unwrap()),
            Some(Err(error)) => {
                let source = std::error::Error::source(&error).unwrap().to_string();
                return (text, Some(source));
            }
            None => return (text, None),
        }
    }
}

/// The captured body-completion events, one line each.
pub fn finished(capture: &Capture) -> Vec<String> {
    capture
        .text()
        .lines()
        .filter(|line| line.contains(FINISHED))
        .map(str::to_owned)
        .collect()
}

/// The single completion event, which must name `outcome` at `level`.
pub fn assert_finished(capture: &Capture, level: &str, outcome: &str) -> String {
    let events = finished(capture);
    assert_eq!(events.len(), 1, "{}", capture.text());
    let event = events.into_iter().next().unwrap();
    assert!(event.trim_start().starts_with(level), "{event}");
    assert!(
        event.contains(&format!("body_outcome=\"{outcome}\"")),
        "{event}"
    );
    assert!(event.contains("request_id=\""), "{event}");
    event
}

/// A context the stream handed out, for inspection after the body ended.
#[derive(Clone, Default)]
pub struct Escaped(Arc<Mutex<Option<OperationContext>>>);

impl Escaped {
    pub fn keep(&self, context: &OperationContext) {
        *self.0.lock().unwrap() = Some(context.clone());
    }

    pub fn context(&self) -> OperationContext {
        self.0.lock().unwrap().clone().expect("the stream ran")
    }
}

/// When the supervised component of [`Process`] returns.
#[derive(Clone, Copy)]
pub enum Stops {
    AtDrain,
    AtForcedCancellation,
}

/// A supervised process with one component, so drain and forced cancellation
/// follow the real phase clock.
pub struct Process {
    pub handle: ShutdownHandle,
    driver: tokio::task::JoinHandle<ShutdownReport>,
}

impl Process {
    pub async fn start(drain: Duration, stops: Stops) -> Self {
        let cleanup = CleanupBudget::new(SECOND, SECOND, SECOND).unwrap();
        let mut supervisor =
            Supervisor::new(ShutdownBudget::new(drain, SECOND, SECOND, cleanup).unwrap());
        let handle = supervisor.handle();
        let (started, running) = tokio::sync::oneshot::channel();
        supervisor
            .register("component", move |startup| async move {
                let component = startup.acknowledge_started();
                started.send(()).unwrap();
                match stops {
                    Stops::AtDrain => component.draining().await,
                    Stops::AtForcedCancellation => component.cancelled().await,
                }
                Ok(component.stopped())
            })
            .unwrap();
        let driver = tokio::spawn(supervisor.run_until(std::future::pending()));
        running.await.unwrap();
        Self { handle, driver }
    }

    pub async fn report(self) -> ShutdownReport {
        self.driver.await.unwrap()
    }
}
