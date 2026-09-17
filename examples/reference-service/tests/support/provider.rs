use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Notify, oneshot};

use super::ProbeResult;

pub const TOKEN: &str = "fixture-provider-token";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispatchBehavior {
    Accept,
    AcceptBehindBarrier,
    RateLimited { retry_after_ms: u64 },
    BusinessDenied,
    UnknownText,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LookupBehavior {
    Stored,
    Absent,
    Expired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Counts {
    pub dispatch_requests: usize,
    pub accepted_effects: usize,
    pub reconciliation_requests: usize,
}

#[derive(Clone)]
struct Effect {
    request: Value,
    effect_id: String,
}

struct Shared {
    dispatch: Mutex<DispatchBehavior>,
    lookup: Mutex<LookupBehavior>,
    effects: Mutex<HashMap<String, Effect>>,
    dispatch_requests: AtomicUsize,
    accepted_effects: AtomicUsize,
    reconciliation_requests: AtomicUsize,
    accepted: Notify,
    barrier_generation: AtomicUsize,
    barrier: Notify,
}

impl Shared {
    fn counts(&self) -> Counts {
        Counts {
            dispatch_requests: self.dispatch_requests.load(Ordering::SeqCst),
            accepted_effects: self.accepted_effects.load(Ordering::SeqCst),
            reconciliation_requests: self.reconciliation_requests.load(Ordering::SeqCst),
        }
    }
}

/// Real loopback HTTP provider with an acceptance-before-response barrier.
pub struct ProviderFixture {
    address: std::net::SocketAddr,
    shared: Arc<Shared>,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<std::io::Result<()>>>,
}

impl ProviderFixture {
    pub async fn start(behavior: DispatchBehavior) -> Result<Self, std::io::Error> {
        let shared = Arc::new(Shared {
            dispatch: Mutex::new(behavior),
            lookup: Mutex::new(LookupBehavior::Stored),
            effects: Mutex::new(HashMap::new()),
            dispatch_requests: AtomicUsize::new(0),
            accepted_effects: AtomicUsize::new(0),
            reconciliation_requests: AtomicUsize::new(0),
            accepted: Notify::new(),
            barrier_generation: AtomicUsize::new(0),
            barrier: Notify::new(),
        });
        let app = Router::new()
            .route("/effects", post(dispatch))
            .route("/effects/{key}", get(reconcile))
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
            shutdown: Some(shutdown),
            task: Some(task),
        })
    }

    pub fn base_url(&self) -> String {
        format!("http://{}/", self.address)
    }

    pub fn set_dispatch(&self, behavior: DispatchBehavior) {
        *self.shared.dispatch.lock().unwrap() = behavior;
    }

    pub fn set_lookup(&self, behavior: LookupBehavior) {
        *self.shared.lookup.lock().unwrap() = behavior;
    }

    pub fn counts(&self) -> Counts {
        self.shared.counts()
    }

    pub async fn verify_identity_contract(&self) -> ProbeResult {
        let (key, effect) = self
            .shared
            .effects
            .lock()
            .unwrap()
            .iter()
            .next()
            .map(|(key, effect)| (key.clone(), effect.clone()))
            .ok_or("provider identity check requires one accepted effect")?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()?;
        let repeated = client
            .post(format!("{}effects", self.base_url()))
            .bearer_auth(TOKEN)
            .header("Idempotency-Key", &key)
            .json(&effect.request)
            .send()
            .await?;
        if repeated.status() != StatusCode::OK
            || repeated.json::<Value>().await?["provider_effect_id"] != effect.effect_id
        {
            return Err("same provider key and payload did not return the retained effect".into());
        }

        let mut changed = effect.request.clone();
        changed["payload"] = json!({"changed": true});
        let conflict = client
            .post(format!("{}effects", self.base_url()))
            .bearer_auth(TOKEN)
            .header("Idempotency-Key", &key)
            .json(&changed)
            .send()
            .await?;
        if conflict.status() != StatusCode::CONFLICT
            || conflict.json::<Value>().await?["code"] != "idempotency_conflict"
        {
            return Err("same provider key with a different payload did not conflict".into());
        }
        Ok(())
    }

    pub async fn wait_for_acceptance(&self, limit: Duration) -> Result<(), &'static str> {
        self.wait_for_accepted_count(1, limit).await
    }

    pub async fn wait_for_accepted_count(
        &self,
        expected: usize,
        limit: Duration,
    ) -> Result<(), &'static str> {
        tokio::time::timeout(limit, async {
            loop {
                let notified = self.shared.accepted.notified();
                if self.shared.accepted_effects.load(Ordering::SeqCst) >= expected {
                    return;
                }
                notified.await;
            }
        })
        .await
        .map_err(|_| "provider did not accept an effect before the deadline")
    }

    pub fn release_response(&self) {
        self.shared
            .barrier_generation
            .fetch_add(1, Ordering::SeqCst);
        self.shared.barrier.notify_waiters();
    }

    pub async fn close(mut self) -> ProbeResult {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let mut task = self.task.take().expect("provider task is retained");
        match tokio::time::timeout(Duration::from_secs(5), &mut task).await {
            Ok(result) => {
                result??;
                Ok(())
            }
            Err(_) => {
                task.abort();
                let _ = task.await;
                Err("provider fixture did not stop within its bound".into())
            }
        }
    }
}

impl Drop for ProviderFixture {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn dispatch(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    shared.dispatch_requests.fetch_add(1, Ordering::SeqCst);
    let Some(key) = authenticated_key(&headers) else {
        return error(StatusCode::UNAUTHORIZED, "authentication_required");
    };
    let behavior = *shared.dispatch.lock().unwrap();
    let barrier_generation = shared.barrier_generation.load(Ordering::SeqCst);
    match behavior {
        DispatchBehavior::RateLimited { retry_after_ms } => {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"code": "rate_limited", "retry_after_ms": retry_after_ms})),
            )
                .into_response();
        }
        DispatchBehavior::BusinessDenied => {
            return error(StatusCode::UNPROCESSABLE_ENTITY, "business_denied");
        }
        DispatchBehavior::UnknownText => {
            return (StatusCode::SERVICE_UNAVAILABLE, "please retry later").into_response();
        }
        DispatchBehavior::Accept | DispatchBehavior::AcceptBehindBarrier => {}
    }

    let effect_id = match request.get("effect_id").and_then(Value::as_str) {
        Some(effect_id) => effect_id.to_owned(),
        None => return error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    let provider_effect_id = format!("provider:{effect_id}");
    let (effect, inserted) = {
        let mut effects = shared.effects.lock().unwrap();
        match effects.get(&key) {
            Some(effect) if effect.request == request => (effect.clone(), false),
            Some(_) => return error(StatusCode::CONFLICT, "idempotency_conflict"),
            None => {
                let effect = Effect {
                    request: request.clone(),
                    effect_id: provider_effect_id,
                };
                effects.insert(key, effect.clone());
                (effect, true)
            }
        }
    };
    if inserted {
        shared.accepted_effects.fetch_add(1, Ordering::SeqCst);
        shared.accepted.notify_waiters();
    }
    if behavior == DispatchBehavior::AcceptBehindBarrier {
        loop {
            let released = shared.barrier.notified();
            if shared.barrier_generation.load(Ordering::SeqCst) != barrier_generation {
                break;
            }
            released.await;
        }
    }
    accepted(&effect)
}

async fn reconcile(
    State(shared): State<Arc<Shared>>,
    Path(key): Path<String>,
    headers: HeaderMap,
) -> Response {
    shared
        .reconciliation_requests
        .fetch_add(1, Ordering::SeqCst);
    if !authenticated(&headers) {
        return error(StatusCode::UNAUTHORIZED, "authentication_required");
    }
    match *shared.lookup.lock().unwrap() {
        LookupBehavior::Absent => return not_found(),
        LookupBehavior::Expired => {
            return error(StatusCode::GONE, "retention_expired");
        }
        LookupBehavior::Stored => {}
    }
    match shared.effects.lock().unwrap().get(&key).cloned() {
        Some(effect) => accepted(&effect),
        None => not_found(),
    }
}

fn authenticated_key(headers: &HeaderMap) -> Option<String> {
    if !authenticated(headers) {
        return None;
    }
    headers
        .get("idempotency-key")?
        .to_str()
        .ok()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn authenticated(headers: &HeaderMap) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        == Some("Bearer fixture-provider-token")
}

fn accepted(effect: &Effect) -> Response {
    (
        StatusCode::OK,
        Json(json!({
            "effect_id": effect.request["effect_id"],
            "provider_effect_id": effect.effect_id,
            "request": effect.request.clone(),
        })),
    )
        .into_response()
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({"code": "not_found", "retention_active": true})),
    )
        .into_response()
}

fn error(status: StatusCode, code: &'static str) -> Response {
    (status, Json(json!({"code": code}))).into_response()
}
