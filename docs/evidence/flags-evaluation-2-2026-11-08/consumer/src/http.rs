//! The `checkout` route group behind Batter's HTTP boundary.
//!
//! The group is gated by `checkout_halt`: while it is engaged every method and
//! path of the group answers 503 `disabled` without reaching a handler, while
//! the `/live` and `/ready` probes stay outside the gate. Each admitted request
//! evaluates one frozen flags view, so the `express_receipt` decision pinned
//! into the order row is the one this request was admitted under.

use axum::{
    Json,
    extract::{State, rejection::JsonRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use batter::{
    BoxError,
    axum::{
        AdmittedRequest, AssembledHttp, GroupGate, GroupPolicy, GuardedRouter, HttpBoundary,
        HttpFailure, ProbePath, ReadinessPolicy, RequestPolicy, ResponseConstructionBudget,
        RouteGroup, render_infrastructure_failure,
    },
    flags::{
        core::{ReleaseFlag, Toggle},
        http::AdmittedFlags,
    },
    lifecycle::{LifecycleStatus, OperationAdmission},
    operation::OperationError,
    runledger::{PgAtomicError, RunledgerDatabase},
};
use serde::Deserialize;
use serde_json::json;

use crate::orders::{self, Acceptance, NewOrder};

/// What the checkout handlers share.
#[derive(Clone)]
pub struct AppState {
    pub database: RunledgerDatabase,
    pub express_receipt: ReleaseFlag<Toggle>,
}

#[derive(Debug, Deserialize)]
struct CheckoutBody {
    order_ref: String,
    recipient: String,
}

fn invalid(message: &'static str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error": "invalid_request", "message": message})),
    )
        .into_response()
}

fn validate(body: CheckoutBody) -> Result<NewOrder, &'static str> {
    let ref_ok = (1..=64).contains(&body.order_ref.len())
        && body
            .order_ref
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if !ref_ok {
        return Err("order_ref must be 1 to 64 ASCII letters, digits, '-' or '_'");
    }
    let recipient = body.recipient.trim();
    if !(3..=254).contains(&recipient.len()) || !recipient.contains('@') {
        return Err("recipient must be an email address");
    }
    Ok(NewOrder {
        order_ref: body.order_ref,
        recipient: recipient.to_owned(),
    })
}

/// `POST /checkout`: accept an order and enqueue its receipt atomically.
async fn accept(
    State(state): State<AppState>,
    admitted: AdmittedRequest,
    flags: AdmittedFlags,
    body: Result<Json<CheckoutBody>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return invalid("expected a JSON body with order_ref and recipient");
    };
    let order = match validate(body) {
        Ok(order) => order,
        Err(message) => return invalid(message),
    };
    // The decision this request was admitted under, recorded for the receipt.
    let pin = flags.view().pin(&state.express_receipt);
    let express_receipt = pin.variant().unwrap_or("off").to_owned();
    let Ok(stored_pin) = serde_json::to_value(&pin) else {
        return render_infrastructure_failure(
            HttpFailure::Internal,
            Some(admitted.correlation_id()),
        );
    };
    let accepted = admitted
        .context()
        .run("checkout.accept", |_scope| {
            orders::accept(&state.database, &order, &stored_pin)
        })
        .await;
    let id = admitted.correlation_id();
    match accepted {
        Ok(Acceptance::Accepted) => (
            StatusCode::ACCEPTED,
            Json(json!({
                "status": "accepted",
                "order_ref": order.order_ref,
                "express_receipt": express_receipt,
            })),
        )
            .into_response(),
        Ok(Acceptance::AlreadyAccepted) => (
            StatusCode::OK,
            Json(json!({"status": "already_accepted", "order_ref": order.order_ref})),
        )
            .into_response(),
        // Nothing was written: the transaction never began.
        Err(OperationError::Failed(PgAtomicError::Begin(_))) => {
            render_infrastructure_failure(HttpFailure::Unavailable, Some(id))
        }
        // Nothing was written: the rollback was acknowledged.
        Err(OperationError::Failed(PgAtomicError::Rejected(failure))) => {
            tracing::error!(target: "checkout_service", step = %failure, "checkout not accepted");
            render_infrastructure_failure(HttpFailure::Internal, Some(id))
        }
        // The commit may or may not have happened. A retry with the same
        // order_ref is safe: it is answered from the row if it committed.
        Err(OperationError::Failed(PgAtomicError::Uncertain(_))) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": "outcome_unknown",
                "message": "retry with the same order_ref",
                "request_id": id.to_string(),
            })),
        )
            .into_response(),
        // Deadline or drain won; PostgreSQL may still have committed, and the
        // same retry rule applies.
        Err(OperationError::Interrupted(reason)) => {
            admitted.interruption_responder().render(reason)
        }
    }
}

/// `GET /checkout/status`: what a checkout accepted now would get.
async fn status(State(state): State<AppState>, flags: AdmittedFlags) -> Response {
    let decision = flags.view().evaluate(&state.express_receipt);
    Json(json!({"accepting": true, "express_receipt": decision.variant()})).into_response()
}

/// Assembles the boundary: probes outside admission, the gated `checkout`
/// group, and an empty default group whose unmatched paths answer 404.
pub async fn assemble(
    lifecycle: LifecycleStatus,
    admission: OperationAdmission,
    budget: ResponseConstructionBudget,
    gate: GroupGate,
    state: AppState,
) -> Result<AssembledHttp, BoxError> {
    let policy = RequestPolicy::new(admission, budget).with_infrastructure_json();
    let routes = GuardedRouter::new()
        .route("/checkout", post(accept))
        .route("/checkout/status", get(status))
        .with_state(state);
    let assembled = HttpBoundary::new(policy.clone())
        .with_liveness(ProbePath::new("/live")?)?
        .with_readiness(
            ProbePath::new("/ready")?,
            ReadinessPolicy::lifecycle_only(lifecycle),
        )?
        .with_group(RouteGroup::new(
            "checkout",
            GroupPolicy::gated(policy, gate),
            routes,
        ))?
        .assemble(GuardedRouter::new())
        .await?;
    Ok(assembled)
}

#[cfg(test)]
mod tests;
