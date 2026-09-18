//! Adapter-facing retained quota facts for [`crate::operational_http_with_quota`].
//!
//! This is an observation capability, never authentication or quota authority.
//! The operational middleware creates a fresh private record, replacing inbound
//! records. A quota adapter takes the writer before invoking application code.
//! The observer retains its own reader through response construction or drop.
//! No caller callback runs from a destructor and no subject or error text fits
//! this interface. Ordinary [`crate::operational_http`] allocates no quota record.

use axum::extract::Request;
use std::sync::{Arc, Mutex};

/// What a completed native failure establishes about consumption.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuotaConsumption {
    /// The native backend confirms consumption.
    Consumed,
    /// The native backend confirms no consumption.
    NotConsumed,
    /// No definitive consumption result is available.
    Unknown,
}

impl QuotaConsumption {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Consumed => "consumed",
            Self::NotConsumed => "not_consumed",
            Self::Unknown => "unknown",
        }
    }
}

/// Bounded quota facts, independent of the final HTTP status and work outcome.
///
/// Allowed always means consumed, and native denials always mean not consumed.
/// Unresolved means a check started without an observed result; it does not
/// establish that local work still runs after the response future was dropped.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum QuotaFacts {
    /// No backend check was started.
    #[default]
    NotChecked,
    /// A backend check started without a retained result.
    Unresolved,
    /// Native quota consumption was confirmed.
    Allowed,
    /// Native quota denial allowed work under an explicit shadow policy.
    ShadowDenied,
    /// Quota exhaustion prevented work.
    QuotaDenied,
    /// The limiter could not allocate storage for a new key.
    StorageCapacity,
    /// A reason asserted by a manual writer outside the pinned native adapter.
    OtherDenial,
    /// A native failure, with independently retained consumption knowledge.
    BackendFailed(QuotaConsumption),
}

impl QuotaFacts {
    pub(crate) fn outcome(self) -> &'static str {
        match self {
            Self::NotChecked => "not_checked",
            Self::Unresolved => "unresolved",
            Self::Allowed => "allowed",
            Self::ShadowDenied => "shadow_denied",
            Self::QuotaDenied => "quota_denied",
            Self::StorageCapacity => "storage_capacity",
            Self::OtherDenial => "other_denial",
            Self::BackendFailed(_) => "backend_failed",
        }
    }

    pub(crate) fn consumption(self) -> QuotaConsumption {
        match self {
            Self::Allowed => QuotaConsumption::Consumed,
            Self::Unresolved => QuotaConsumption::Unknown,
            Self::BackendFailed(consumption) => consumption,
            _ => QuotaConsumption::NotConsumed,
        }
    }
}

#[derive(Default)]
struct QuotaRecord {
    facts: QuotaFacts,
    writer_taken: bool,
}

#[derive(Clone, Default)]
pub(crate) struct QuotaObservation(Arc<Mutex<QuotaRecord>>);

impl QuotaObservation {
    pub(crate) fn snapshot(&self) -> QuotaFacts {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .facts
    }
}

/// Sole writer taken by a supported quota adapter before application execution.
///
/// No public constructor or Clone implementation exists. The private request
/// extension is removed when taken, and the shared record is permanently claimed.
/// Cloning request metadata cannot duplicate or restore the writer capability.
/// The adapter must finish with native truth after starting the check; this
/// low-level observation seam does not itself enforce
/// quota, supervise work, or validate an application-provided backend.
///
/// ```
/// use axum::extract::Request;
/// use batter_axum::quota_observation::{QuotaRecorder, QuotaTerminalFacts, StartedQuotaRecorder};
/// fn begin_check(request: &mut Request) -> Option<StartedQuotaRecorder> {
///     Some(QuotaRecorder::take(request)?.start()) // Immediately before native check.
/// }
/// fn complete(started: StartedQuotaRecorder) {
///     started.finish(QuotaTerminalFacts::Allowed); // After the native decision.
/// }
/// ```
///
/// ```compile_fail
/// let forged = batter_axum::quota_observation::QuotaRecorder::default();
/// ```
///
/// ```compile_fail
/// use batter_axum::quota_observation::{QuotaTerminalFacts, StartedQuotaRecorder};
/// fn cannot_rewrite(started: StartedQuotaRecorder) {
///     started.finish(QuotaTerminalFacts::Allowed);
///     started.finish(QuotaTerminalFacts::QuotaDenied);
/// }
/// ```
///
/// ```compile_fail
/// use batter_axum::quota_observation::{QuotaFacts, StartedQuotaRecorder};
/// fn cannot_finish_unresolved(started: StartedQuotaRecorder) {
///     started.finish(QuotaFacts::Unresolved);
/// }
/// ```
pub struct QuotaRecorder(QuotaObservation);

/// A started quota check with one remaining terminal publication.
///
/// Dropping this writer leaves `Unresolved` for the response observer. It cannot
/// be cloned, and [`finish`](Self::finish) consumes it so no later update is possible.
pub struct StartedQuotaRecorder(QuotaObservation);

/// Terminal facts established by a completed native quota check.
///
/// Neither `NotChecked` nor `Unresolved` can be submitted as a terminal fact.
/// This type carries the adapter's assertion; it cannot verify native truth.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuotaTerminalFacts {
    /// Native quota consumption was confirmed.
    Allowed,
    /// Native quota denial allowed work under an explicit shadow policy.
    ShadowDenied,
    /// Quota exhaustion prevented work.
    QuotaDenied,
    /// The limiter could not allocate storage for a new key.
    StorageCapacity,
    /// A reason asserted by a manual writer outside the pinned native adapter.
    OtherDenial,
    /// A native failure, with independently retained consumption knowledge.
    BackendFailed(QuotaConsumption),
}

impl From<QuotaTerminalFacts> for QuotaFacts {
    fn from(facts: QuotaTerminalFacts) -> Self {
        match facts {
            QuotaTerminalFacts::Allowed => Self::Allowed,
            QuotaTerminalFacts::ShadowDenied => Self::ShadowDenied,
            QuotaTerminalFacts::QuotaDenied => Self::QuotaDenied,
            QuotaTerminalFacts::StorageCapacity => Self::StorageCapacity,
            QuotaTerminalFacts::OtherDenial => Self::OtherDenial,
            QuotaTerminalFacts::BackendFailed(consumption) => Self::BackendFailed(consumption),
        }
    }
}

impl QuotaRecorder {
    /// Claim the private writer at most once across all clones of request metadata.
    ///
    /// Removes the request's private extension. Returns None without the opt-in
    /// middleware or if any clone already claimed this record, even if that
    /// writer has since been dropped. Cloning metadata grants no new authority.
    pub fn take(request: &mut Request) -> Option<Self> {
        let observation = request.extensions_mut().remove::<QuotaObservation>()?;
        {
            let mut record = observation
                .0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if record.writer_taken {
                return None;
            }
            record.writer_taken = true;
        }
        Some(Self(observation))
    }

    /// Mark the native check as started immediately before invoking it.
    ///
    /// Dropping the returned writer before completion retains `Unresolved`.
    pub fn start(self) -> StartedQuotaRecorder {
        self.0
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .facts = QuotaFacts::Unresolved;
        StartedQuotaRecorder(self.0)
    }
}

impl StartedQuotaRecorder {
    /// Retain a completed native fact even if later HTTP work times out.
    ///
    /// Consumes the only writer. A later nonterminal update cannot be expressed.
    /// This does not prove that the adapter's fact matches the native result.
    pub fn finish(self, facts: QuotaTerminalFacts) {
        self.0
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .facts = facts.into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, http::StatusCode, middleware, routing::get};
    use tower::ServiceExt;

    #[test]
    fn metadata_clones_share_one_irreversible_writer_claim() {
        let observation = QuotaObservation::default();
        let mut request = Request::new(Body::empty());
        request.extensions_mut().insert(observation.clone());
        let (parts, body) = request.into_parts();
        let mut before = Request::from_parts(parts.clone(), Body::empty());
        let mut original = Request::from_parts(parts, body);

        let writer = QuotaRecorder::take(&mut original).unwrap().start();
        writer.finish(QuotaTerminalFacts::Allowed);
        assert!(QuotaRecorder::take(&mut original).is_none());

        // This clone still contains the extension, unlike the original request.
        let mut after = Request::new(Body::empty());
        *after.extensions_mut() = before.extensions().clone();
        assert!(QuotaRecorder::take(&mut before).is_none());
        assert!(QuotaRecorder::take(&mut after).is_none());
        assert_eq!(observation.snapshot(), QuotaFacts::Allowed);
    }

    #[test]
    fn concurrent_metadata_claims_have_exactly_one_winner_even_after_drop() {
        const CLAIMANTS: usize = 8;
        let observation = QuotaObservation::default();
        let mut request = Request::new(Body::empty());
        request.extensions_mut().insert(observation.clone());
        let barrier = Arc::new(std::sync::Barrier::new(CLAIMANTS));
        let claims: Vec<_> = (0..CLAIMANTS)
            .map(|_| {
                let extensions = request.extensions().clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let mut request = Request::new(Body::empty());
                    *request.extensions_mut() = extensions;
                    barrier.wait();
                    let Some(writer) = QuotaRecorder::take(&mut request) else {
                        return false;
                    };
                    writer.start().finish(QuotaTerminalFacts::Allowed);
                    // A later claimant must not be allowed to replace a dropped writer.
                    true
                })
            })
            .collect();
        let winners = claims
            .into_iter()
            .map(|claim| usize::from(claim.join().unwrap()))
            .sum::<usize>();
        assert_eq!(winners, 1);
        assert!(QuotaRecorder::take(&mut request).is_none());
        assert_eq!(observation.snapshot(), QuotaFacts::Allowed);
    }

    #[tokio::test]
    async fn opt_in_replaces_inbound_record_and_writer_is_single_take() {
        let previous = QuotaObservation::default();
        QuotaRecorder(previous.clone())
            .start()
            .finish(QuotaTerminalFacts::Allowed);
        let router = Router::new()
            .route(
                "/",
                get(|mut request: Request| async move {
                    assert_eq!(
                        request
                            .extensions()
                            .get::<QuotaObservation>()
                            .unwrap()
                            .snapshot(),
                        QuotaFacts::NotChecked
                    );
                    let writer = QuotaRecorder::take(&mut request).unwrap();
                    assert!(QuotaRecorder::take(&mut request).is_none());
                    writer.start().finish(QuotaTerminalFacts::ShadowDenied);
                    StatusCode::OK
                }),
            )
            .layer(middleware::from_fn(crate::operational_http_with_quota));
        let mut request = Request::new(Body::empty());
        request.extensions_mut().insert(previous.clone());
        assert_eq!(
            router.oneshot(request).await.unwrap().status(),
            StatusCode::OK
        );
        assert_eq!(previous.snapshot(), QuotaFacts::Allowed);
    }

    #[tokio::test]
    async fn ordinary_operational_http_removes_inbound_record_without_replacing_it() {
        let router = Router::new()
            .route(
                "/",
                get(|mut request: Request| async move {
                    assert!(QuotaRecorder::take(&mut request).is_none());
                    StatusCode::OK
                }),
            )
            .layer(middleware::from_fn(crate::operational_http));
        let mut request = Request::new(Body::empty());
        request.extensions_mut().insert(QuotaObservation::default());
        assert_eq!(
            router.oneshot(request).await.unwrap().status(),
            StatusCode::OK
        );
    }

    #[test]
    fn writer_transitions_retain_the_last_observed_phase() {
        let unstarted = QuotaObservation::default();
        drop(QuotaRecorder(unstarted.clone()));
        assert_eq!(unstarted.snapshot(), QuotaFacts::NotChecked);

        let interrupted = QuotaObservation::default();
        drop(QuotaRecorder(interrupted.clone()).start());
        assert_eq!(interrupted.snapshot(), QuotaFacts::Unresolved);

        let completed = QuotaObservation::default();
        QuotaRecorder(completed.clone())
            .start()
            .finish(QuotaTerminalFacts::BackendFailed(
                QuotaConsumption::Consumed,
            ));
        assert_eq!(
            completed.snapshot(),
            QuotaFacts::BackendFailed(QuotaConsumption::Consumed)
        );

        let future_denial = QuotaObservation::default();
        QuotaRecorder(future_denial.clone())
            .start()
            .finish(QuotaTerminalFacts::OtherDenial);
        assert_eq!(future_denial.snapshot().outcome(), "other_denial");
        assert_eq!(
            future_denial.snapshot().consumption(),
            QuotaConsumption::NotConsumed
        );
    }
}
