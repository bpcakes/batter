//! Loopback OTLP/HTTP metrics collector with scripted responses.
//!
//! Self-contained so library unit tests, the metrics integration target and
//! the live suite share one fixture. Requests are decoded with the upstream
//! generated OTLP types; stalled responses end when released or closed.

use axum::{
    Router,
    body::{Body, Bytes},
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use opentelemetry_proto::tonic::{
    collector::metrics::v1::{
        ExportMetricsPartialSuccess, ExportMetricsServiceRequest, ExportMetricsServiceResponse,
    },
    common::v1::{AnyValue, any_value},
    metrics::v1::{metric, number_data_point},
};
use prost::Message;
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};
use tokio::sync::{Notify, oneshot, watch};

/// Scripted collector response for every following request.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Behavior {
    /// 200 with an empty protobuf response.
    Accept,
    /// 200 with OTLP partial success rejecting the given points.
    PartialReject(i64),
    /// A non-success status with a body that must not be retained.
    Status(u16),
    /// 200 declaring protobuf with undecodable bytes.
    Malformed,
    /// 200 with a non-empty body that is not protobuf.
    WrongContentType,
    /// 200 declaring a body above any reasonable response ceiling.
    DeclaredOversized,
    /// 200 streaming a body above the response ceiling without a length.
    StreamedOversized,
    /// Read the request, then send no response headers until released.
    StallHeaders,
    /// Send headers and part of the body, then stall until released.
    StallBody,
}

/// Collector-controlled marker that must never reach retained diagnostics.
pub const BODY_MARKER: &str = "collector-body-marker";
const OVERSIZED: usize = 1024 * 1024;

struct Shared {
    behavior: Mutex<Behavior>,
    requests: Mutex<Vec<ExportMetricsServiceRequest>>,
    received: Notify,
    closing: watch::Receiver<bool>,
}

impl Shared {
    fn behavior(&self) -> Behavior {
        *self.behavior.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A running loopback collector.
pub struct Collector {
    address: SocketAddr,
    shared: Arc<Shared>,
    closing: watch::Sender<bool>,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<std::io::Result<()>>>,
}

impl Collector {
    pub async fn start(behavior: Behavior) -> std::io::Result<Self> {
        let (closing, closed) = watch::channel(false);
        let shared = Arc::new(Shared {
            behavior: Mutex::new(behavior),
            requests: Mutex::new(Vec::new()),
            received: Notify::new(),
            closing: closed,
        });
        let app = Router::new()
            .route("/v1/metrics", post(export))
            .layer(axum::extract::DefaultBodyLimit::max(8 * 1024 * 1024))
            .with_state(shared.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let (shutdown, stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = stopped.await;
                })
                .await
        });
        Ok(Self {
            address,
            shared,
            closing,
            shutdown: Some(shutdown),
            task: Some(task),
        })
    }

    pub fn endpoint(&self) -> String {
        format!("http://{}/v1/metrics", self.address)
    }

    #[allow(dead_code)]
    pub fn set(&self, behavior: Behavior) {
        *self
            .shared
            .behavior
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = behavior;
    }

    /// Every decoded request, in arrival order, including stalled ones.
    pub fn requests(&self) -> Vec<ExportMetricsServiceRequest> {
        self.shared
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Wait until at least `count` requests arrived.
    #[allow(dead_code)]
    pub async fn wait_requests(
        &self,
        count: usize,
        limit: Duration,
    ) -> Result<Vec<ExportMetricsServiceRequest>, String> {
        tokio::time::timeout(limit, async {
            loop {
                let received = self.shared.received.notified();
                let requests = self.requests();
                if requests.len() >= count {
                    return requests;
                }
                received.await;
            }
        })
        .await
        .map_err(|_| format!("collector did not receive {count} request(s)"))
    }

    /// End stalled responses: stalled headers become 503, stalled bodies end
    /// short of their declared length. The server keeps serving.
    #[allow(dead_code)]
    pub fn release(&self) {
        self.closing.send_replace(true);
    }

    /// Release stalled responses and stop serving.
    pub async fn close(mut self) -> std::io::Result<()> {
        self.closing.send_replace(true);
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        match self.task.take() {
            Some(task) => task.await.map_err(std::io::Error::other)?,
            None => Ok(()),
        }
    }
}

impl Drop for Collector {
    fn drop(&mut self) {
        self.closing.send_replace(true);
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

fn protobuf(status: StatusCode, message: &impl Message) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/x-protobuf")],
        message.encode_to_vec(),
    )
        .into_response()
}

async fn export(State(shared): State<Arc<Shared>>, body: Bytes) -> Response {
    let Ok(request) = ExportMetricsServiceRequest::decode(body) else {
        return (StatusCode::BAD_REQUEST, BODY_MARKER).into_response();
    };
    shared
        .requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(request);
    shared.received.notify_waiters();
    let mut closing = shared.closing.clone();
    match shared.behavior() {
        Behavior::Accept => protobuf(StatusCode::OK, &ExportMetricsServiceResponse::default()),
        Behavior::PartialReject(rejected) => protobuf(
            StatusCode::OK,
            &ExportMetricsServiceResponse {
                partial_success: Some(ExportMetricsPartialSuccess {
                    rejected_data_points: rejected,
                    error_message: BODY_MARKER.to_owned(),
                }),
            },
        ),
        Behavior::Status(code) => (
            StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            BODY_MARKER,
        )
            .into_response(),
        Behavior::Malformed => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/x-protobuf")],
            vec![0xff; 16],
        )
            .into_response(),
        Behavior::WrongContentType => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/plain")],
            BODY_MARKER,
        )
            .into_response(),
        Behavior::DeclaredOversized => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/x-protobuf")],
            vec![0; OVERSIZED],
        )
            .into_response(),
        Behavior::StreamedOversized => {
            let chunks = (0..OVERSIZED / 4096).map(|_| Ok::<_, std::io::Error>(vec![0; 4096]));
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/x-protobuf")],
                Body::from_stream(futures_util::stream::iter(chunks)),
            )
                .into_response()
        }
        Behavior::StallHeaders => {
            let _ = closing.wait_for(|closed| *closed).await;
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
        Behavior::StallBody => {
            let first = futures_util::stream::iter([Ok::<_, std::io::Error>(vec![0x0a])]);
            let stalled = futures_util::stream::unfold(Some(closing), |closing| async move {
                let _ = closing?.wait_for(|closed| *closed).await;
                None::<(Result<Vec<u8>, std::io::Error>, _)>
            });
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "application/x-protobuf"),
                    (header::CONTENT_LENGTH, "64"),
                ],
                Body::from_stream(futures_util::StreamExt::chain(first, stalled)),
            )
                .into_response()
        }
    }
}

/// One exported series flattened from an OTLP request.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    pub name: String,
    pub description: String,
    pub unit: String,
    pub labels: Vec<(String, String)>,
    pub value: Value,
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Sum {
        value: i64,
        monotonic: bool,
    },
    Histogram {
        count: u64,
        bounds: Vec<f64>,
        buckets: Vec<u64>,
    },
    Other,
}

fn text(value: Option<&AnyValue>) -> String {
    match value.and_then(|value| value.value.as_ref()) {
        Some(any_value::Value::StringValue(text)) => text.clone(),
        _ => String::new(),
    }
}

/// Flatten every data point in one request.
#[allow(dead_code)]
pub fn series(request: &ExportMetricsServiceRequest) -> Vec<Series> {
    let mut flattened = Vec::new();
    for resource in &request.resource_metrics {
        for scope in &resource.scope_metrics {
            for metric in &scope.metrics {
                let base = |labels: Vec<(String, String)>, value| Series {
                    name: metric.name.clone(),
                    description: metric.description.clone(),
                    unit: metric.unit.clone(),
                    labels,
                    value,
                };
                let labels = |attributes: &[opentelemetry_proto::tonic::common::v1::KeyValue]| {
                    attributes
                        .iter()
                        .map(|pair| (pair.key.clone(), text(pair.value.as_ref())))
                        .collect::<Vec<_>>()
                };
                match &metric.data {
                    Some(metric::Data::Sum(sum)) => {
                        for point in &sum.data_points {
                            let value = match point.value {
                                Some(number_data_point::Value::AsInt(value)) => value,
                                Some(number_data_point::Value::AsDouble(value)) => value as i64,
                                None => 0,
                            };
                            flattened.push(base(
                                labels(&point.attributes),
                                Value::Sum {
                                    value,
                                    monotonic: sum.is_monotonic,
                                },
                            ));
                        }
                    }
                    Some(metric::Data::Histogram(histogram)) => {
                        for point in &histogram.data_points {
                            flattened.push(base(
                                labels(&point.attributes),
                                Value::Histogram {
                                    count: point.count,
                                    bounds: point.explicit_bounds.clone(),
                                    buckets: point.bucket_counts.clone(),
                                },
                            ));
                        }
                    }
                    _ => flattened.push(base(Vec::new(), Value::Other)),
                }
            }
        }
    }
    flattened
}

/// The single series with `name` whose labels contain every `wanted` pair.
#[allow(dead_code)]
pub fn find<'a>(series: &'a [Series], name: &str, wanted: &[(&str, &str)]) -> Option<&'a Series> {
    series.iter().find(|series| {
        series.name == name
            && wanted.iter().all(|(key, value)| {
                series
                    .labels
                    .iter()
                    .any(|(have_key, have_value)| have_key == key && have_value == value)
            })
    })
}

/// The monotonic counter value of the matching series, if present.
#[allow(dead_code)]
pub fn counter(series: &[Series], name: &str, wanted: &[(&str, &str)]) -> Option<i64> {
    match find(series, name, wanted)?.value {
        Value::Sum {
            value,
            monotonic: true,
        } => Some(value),
        _ => None,
    }
}

/// Resource attributes of the first resource in a request.
#[allow(dead_code)]
pub fn resource(request: &ExportMetricsServiceRequest) -> Vec<(String, String)> {
    request
        .resource_metrics
        .first()
        .and_then(|resource| resource.resource.as_ref())
        .map(|resource| {
            resource
                .attributes
                .iter()
                .map(|pair| (pair.key.clone(), text(pair.value.as_ref())))
                .collect()
        })
        .unwrap_or_default()
}
