//! Caller-controlled Axum admission middleware for Runlimit.
//!
//! [`RateLimitLayer`] evaluates one [`runlimit_core::Check`] before calling the
//! wrapped service. Applications supply both trust-sensitive operations:
//!
//! - a synchronous [`ExtractSubjectKey`] implementation that supplies an
//!   opaque subject key from the request; and
//! - a rejection mapper that converts extraction failures, enforced denials,
//!   and backend failures into an Axum [`Response`].
//!
//! This crate never interprets forwarding headers, connection metadata, or
//! application identities. It also does not select response status codes,
//! bodies, or headers.
//!
//! Every admitted request carries an [`Admissions`] request extension. Each
//! layer the request passed through appends one [`Admission`] naming its
//! policy and the [`runlimit_core::Admitted`] outcome, so stacking a client
//! gate and an identity gate loses neither decision. A shadow denial is
//! admitted and recorded just like an allowance; an enforced denial never
//! reaches the extension because it is handed to the rejection mapper as a
//! [`runlimit_core::Denial`].

use std::{
    any::type_name,
    fmt,
    future::Future,
    marker::PhantomData,
    mem,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use axum::{extract::Request, response::Response};
use runlimit_core::{
    Admitted, Check, Denial, Limiter, PolicyFingerprint, PolicyId, RateLimitPolicy, ScopeId,
    SubjectKey,
};
use tower::{Layer, Service};

/// A synchronous, application-owned request-to-subject-key boundary.
///
/// The extractor receives the complete request and the configured policy.
/// Implementations decide whether request metadata is trusted and how an
/// application identity is normalized. Closures with the matching signature
/// implement this trait automatically.
///
/// Every implementation returns only an already-opaque, secret-keyed
/// [`SubjectKey`]. The layer, not the extractor, binds that key to its
/// configured policy before constructing a check. Consequently an extractor
/// can choose the subject identity but cannot replace the policy evaluated or
/// recorded by the layer.
///
/// A named implementation that derives keys with
/// [`runlimit_core::KeyHasher::hash_for`] must explicitly call
/// [`runlimit_core::PolicySubject::into_unbound_subject_key`]. This deliberate
/// unbinding is safe at this boundary because the layer immediately binds the
/// key to its own configured policy. Raw identities should not cross into a
/// Runlimit backend or generic error and telemetry paths.
pub trait ExtractSubjectKey<P: RateLimitPolicy + ?Sized, B>: Send + Sync {
    /// Application-defined extraction failure.
    type Error;

    /// Derives the opaque subject key for this request and policy.
    ///
    /// # Errors
    ///
    /// Returns an application-defined error when the request does not contain
    /// usable or trusted subject material.
    fn extract_subject_key(
        &self,
        request: &Request<B>,
        policy: &P,
    ) -> Result<SubjectKey, Self::Error>;
}

impl<P: RateLimitPolicy + ?Sized, B, F, E> ExtractSubjectKey<P, B> for F
where
    F: Fn(&Request<B>, &P) -> Result<SubjectKey, E> + Send + Sync,
{
    type Error = E;

    fn extract_subject_key(
        &self,
        request: &Request<B>,
        policy: &P,
    ) -> Result<SubjectKey, Self::Error> {
        self(request, policy)
    }
}

/// A rate-limit rejection handed to the application-owned response mapper.
///
/// This type deliberately does not implement `IntoResponse`: the application
/// controls its status, response body, headers, and operational-failure
/// policy.
///
/// The enum is exhaustive. A mapper names every variant, so a new rejection
/// category is a compile error in every application instead of landing in
/// whatever response a wildcard arm returns. The denied variant carries a
/// [`Denial`] rather than a full decision: an allowed or shadow-denied
/// outcome is never rejected, so the mapper never handles one. The backend
/// variant carries the limiter's single-check error type
/// ([`Limiter::CheckError`]), which never includes batch-only failures.
pub enum RateLimitRejection<KeyError, BackendError> {
    /// The application-owned key extractor rejected the request.
    Key(KeyError),
    /// The backend returned an enforced quota or storage-capacity denial.
    Denied(Denial),
    /// The backend could not complete the admission check.
    Backend(BackendError),
}

impl<KeyError, BackendError> RateLimitRejection<KeyError, BackendError> {
    /// Returns a stable, non-sensitive label for the rejection category.
    pub const fn kind(&self) -> RejectionKind {
        match self {
            Self::Key(_) => RejectionKind::Key,
            Self::Denied(_) => RejectionKind::Denied,
            Self::Backend(_) => RejectionKind::Backend,
        }
    }
}

impl<KeyError, BackendError> fmt::Debug for RateLimitRejection<KeyError, BackendError> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RateLimitRejection")
            .field("kind", &self.kind())
            .field("details", &Redacted)
            .finish()
    }
}

/// The non-sensitive category of a [`RateLimitRejection`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RejectionKind {
    /// Subject-key extraction failed.
    Key,
    /// The limiter returned an enforced denial.
    Denied,
    /// The limiter backend failed.
    Backend,
}

/// One layer's admitted decision, keyed by the policy it evaluated.
///
/// The policy is identified by its identifier, scope, and storage fingerprint
/// rather than by a clone of the policy value, so admissions from layers with
/// different policy types share one collection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Admission {
    policy_id: PolicyId,
    scope_id: ScopeId,
    fingerprint: PolicyFingerprint,
    decision: Admitted,
}

impl Admission {
    /// Records an admitted decision for the policy that produced it.
    pub fn new<P: RateLimitPolicy + ?Sized>(policy: &P, decision: Admitted) -> Self {
        Self {
            policy_id: policy.id().clone(),
            scope_id: policy.scope().clone(),
            fingerprint: policy.fingerprint(),
            decision,
        }
    }

    /// Returns the evaluated policy's identifier.
    pub const fn policy_id(&self) -> &PolicyId {
        &self.policy_id
    }

    /// Returns the evaluated policy's scope.
    pub const fn scope_id(&self) -> &ScopeId {
        &self.scope_id
    }

    /// Returns the evaluated policy's storage configuration fingerprint.
    pub const fn policy_fingerprint(&self) -> PolicyFingerprint {
        self.fingerprint
    }

    /// Returns the admitted decision.
    pub const fn decision(&self) -> Admitted {
        self.decision
    }

    fn is_for<P: RateLimitPolicy + ?Sized>(&self, policy: &P) -> bool {
        self.policy_id == *policy.id()
            && self.scope_id == *policy.scope()
            && self.fingerprint == policy.fingerprint()
    }
}

/// The admitted decisions of every [`RateLimitLayer`] a request passed.
///
/// Each layer appends its [`Admission`] before calling the wrapped service,
/// so the collection lists layers in evaluation order: the outermost layer's
/// admission first. Handlers read it as an `Extension<Admissions>` extractor
/// or from `Request::extensions`. A rejected request never carries one.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Admissions {
    entries: Vec<Admission>,
}

impl Admissions {
    /// Creates an empty collection.
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Appends the admission of one more layer.
    pub fn push(&mut self, admission: Admission) {
        self.entries.push(admission);
    }

    /// Returns the admission recorded for `policy`, when a layer evaluated it.
    ///
    /// A policy matches by identifier, scope, and storage fingerprint. If
    /// several layers evaluated the same policy, the outermost layer's
    /// admission is returned; iterate the collection to see every entry.
    pub fn get<P: RateLimitPolicy + ?Sized>(&self, policy: &P) -> Option<&Admission> {
        self.entries.iter().find(|entry| entry.is_for(policy))
    }

    /// Iterates admissions in evaluation order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &Admission> + '_ {
        self.entries.iter()
    }

    /// Returns the number of layers that admitted the request.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether no layer has recorded an admission.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl<'a> IntoIterator for &'a Admissions {
    type Item = &'a Admission;
    type IntoIter = std::slice::Iter<'a, Admission>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}

impl From<Admission> for Admissions {
    fn from(admission: Admission) -> Self {
        Self {
            entries: vec![admission],
        }
    }
}

/// A Tower layer that performs one Runlimit admission check per request.
///
/// The layer owns one policy and shares the limiter and callback state across
/// clones. It extracts the key and starts the limiter operation before the
/// wrapped service is called. Neither the layer nor its service reads or polls
/// the request body.
pub struct RateLimitLayer<L: Limiter, K, R> {
    limiter: Arc<L>,
    policy: Arc<L::Policy>,
    key_extractor: Arc<K>,
    rejection_mapper: Arc<R>,
}

impl<L: Limiter, K, R> RateLimitLayer<L, K, R> {
    /// Constructs a layer and moves a limiter into shared ownership.
    pub fn new(limiter: L, policy: L::Policy, key_extractor: K, rejection_mapper: R) -> Self {
        Self::from_shared(Arc::new(limiter), policy, key_extractor, rejection_mapper)
    }

    /// Constructs a layer from an already shared limiter.
    pub fn from_shared(
        limiter: Arc<L>,
        policy: L::Policy,
        key_extractor: K,
        rejection_mapper: R,
    ) -> Self {
        Self {
            limiter,
            policy: Arc::new(policy),
            key_extractor: Arc::new(key_extractor),
            rejection_mapper: Arc::new(rejection_mapper),
        }
    }

    /// Returns the shared limiter used by this layer.
    pub const fn limiter(&self) -> &Arc<L> {
        &self.limiter
    }

    /// Returns the policy evaluated for every request.
    pub fn policy(&self) -> &L::Policy {
        self.policy.as_ref()
    }
}

impl<L: Limiter, K, R> Clone for RateLimitLayer<L, K, R> {
    fn clone(&self) -> Self {
        Self {
            limiter: Arc::clone(&self.limiter),
            policy: Arc::clone(&self.policy),
            key_extractor: Arc::clone(&self.key_extractor),
            rejection_mapper: Arc::clone(&self.rejection_mapper),
        }
    }
}

impl<L: Limiter, K, R> fmt::Debug for RateLimitLayer<L, K, R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RateLimitLayer")
            .field("limiter_type", &type_name::<L>())
            .field("limiter", &Redacted)
            .field("policy", &self.policy)
            .field("key_extractor", &Redacted)
            .field("rejection_mapper", &Redacted)
            .finish()
    }
}

impl<S, L: Limiter, K, R> Layer<S> for RateLimitLayer<L, K, R> {
    type Service = RateLimitService<S, L, K, R>;

    fn layer(&self, inner: S) -> Self::Service {
        RateLimitService {
            inner,
            limiter: Arc::clone(&self.limiter),
            policy: Arc::clone(&self.policy),
            key_extractor: Arc::clone(&self.key_extractor),
            rejection_mapper: Arc::clone(&self.rejection_mapper),
        }
    }
}

/// The service produced by [`RateLimitLayer`].
pub struct RateLimitService<S, L: Limiter, K, R> {
    inner: S,
    limiter: Arc<L>,
    policy: Arc<L::Policy>,
    key_extractor: Arc<K>,
    rejection_mapper: Arc<R>,
}

impl<S: Clone, L: Limiter, K, R> Clone for RateLimitService<S, L, K, R> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            limiter: Arc::clone(&self.limiter),
            policy: Arc::clone(&self.policy),
            key_extractor: Arc::clone(&self.key_extractor),
            rejection_mapper: Arc::clone(&self.rejection_mapper),
        }
    }
}

impl<S, L: Limiter, K, R> fmt::Debug for RateLimitService<S, L, K, R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RateLimitService")
            .field("inner_type", &type_name::<S>())
            .field("inner", &Redacted)
            .field("limiter_type", &type_name::<L>())
            .field("limiter", &Redacted)
            .field("policy", &self.policy)
            .field("key_extractor", &Redacted)
            .field("rejection_mapper", &Redacted)
            .finish()
    }
}

/// The boxed, sendable response future returned by [`RateLimitService`].
pub struct ResponseFuture<F> {
    inner: Pin<Box<dyn Future<Output = F> + Send + 'static>>,
}

impl<F> Future for ResponseFuture<F> {
    type Output = F;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        self.inner.as_mut().poll(context)
    }
}

impl<F> fmt::Debug for ResponseFuture<F> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResponseFuture")
            .field("state", &Redacted)
            .field("_output", &PhantomData::<fn() -> F>)
            .finish()
    }
}

impl<S, L, K, R, B> Service<Request<B>> for RateLimitService<S, L, K, R>
where
    S: Service<Request<B>, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
    L: Limiter + 'static,
    L::Policy: 'static,
    K: ExtractSubjectKey<L::Policy, B> + 'static,
    K::Error: Send + 'static,
    R: Fn(RateLimitRejection<K::Error, L::CheckError>) -> Response + Send + Sync + 'static,
    B: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = ResponseFuture<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(context)
    }

    fn call(&mut self, mut request: Request<B>) -> Self::Future {
        let limiter = Arc::clone(&self.limiter);
        let policy = Arc::clone(&self.policy);
        let key_extractor = Arc::clone(&self.key_extractor);
        let rejection_mapper = Arc::clone(&self.rejection_mapper);

        let inner_clone = self.inner.clone();
        let mut ready_inner = mem::replace(&mut self.inner, inner_clone);

        ResponseFuture {
            inner: Box::pin(async move {
                let subject = match key_extractor.extract_subject_key(&request, policy.as_ref()) {
                    Ok(subject) => subject,
                    Err(error) => {
                        return Ok(rejection_mapper(RateLimitRejection::Key(error)));
                    }
                };

                let check = Check::new(subject.bind(policy.as_ref()));
                let decision = match limiter.check(&check).await {
                    Ok(decision) => decision,
                    Err(error) => {
                        return Ok(rejection_mapper(RateLimitRejection::Backend(error)));
                    }
                };

                let admitted = match decision.admit() {
                    Ok(admitted) => admitted,
                    Err(denial) => {
                        return Ok(rejection_mapper(RateLimitRejection::Denied(denial)));
                    }
                };

                let admission = Admission::new(policy.as_ref(), admitted);
                match request.extensions_mut().get_mut::<Admissions>() {
                    Some(admissions) => admissions.push(admission),
                    None => {
                        request.extensions_mut().insert(Admissions::from(admission));
                    }
                }
                ready_inner.call(request).await
            }),
        }
    }
}

struct Redacted;

impl fmt::Debug for Redacted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}
