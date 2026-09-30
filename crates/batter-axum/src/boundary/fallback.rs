//! Metadata-only unmatched-path rendering outside admission.

use super::{BoundaryAssemblyError, HttpBoundary};
use crate::correlation;
use axum::{Router, extract::Request, http::request::Parts, response::Response};
use std::sync::Arc;

type Renderer = dyn Fn(&Parts) -> Response + Send + Sync;

pub(super) struct RenderedFallback(Arc<Renderer>);

impl RenderedFallback {
    pub(super) fn into_router(self) -> Router {
        Router::new().fallback(move |request: Request| {
            let render = self.0.clone();
            async move {
                let (parts, body) = request.into_parts();
                drop(body);
                render(&correlation::renderer_parts(parts))
            }
        })
    }
}

impl HttpBoundary {
    /// Render unmatched paths outside admission, with application status, body and headers.
    ///
    /// The renderer receives metadata, including the generated
    /// [`CorrelationId`](crate::CorrelationId), but no body or private admission,
    /// quota, observation or operational ownership state. It runs synchronously
    /// and must not block or perform business work. There is no request deadline
    /// or streaming-body lifetime guarantee for this renderer, just as for probes.
    ///
    /// Only requests unmatched by all native and declared routes reach it.
    /// Unsupported methods on a matched route remain in that route's group,
    /// and probes retain their reserved paths. Correlation and the single
    /// observer surround the response in every lifecycle state. Group request
    /// and browser policies do not apply: select application response headers
    /// here, using [`PrivateResponsePolicy::apply`](crate::browser::PrivateResponsePolicy::apply)
    /// where appropriate. Prefix selection is application policy.
    ///
    /// A second renderer is rejected. Assembly also rejects a declared root or
    /// nested [`GuardedRouter`](super::GuardedRouter) fallback when this renderer
    /// is configured; existing guarded fallbacks keep their admission semantics.
    /// An admitted router's own fallback remains unreachable.
    ///
    /// ```
    /// use axum::{http::StatusCode, response::IntoResponse, routing::get};
    /// use batter_axum::{GuardedRouter, HttpBoundary, RequestPolicy, browser::PrivateResponsePolicy};
    /// # async fn example(policy: RequestPolicy) -> Result<(), Box<dyn std::error::Error>> {
    /// let assembled = HttpBoundary::new(policy)
    ///     .with_rendered_fallback(|parts| {
    ///         let mut response = (StatusCode::NOT_FOUND, "not found").into_response();
    ///         if parts.uri.path().starts_with("/private/") {
    ///             PrivateResponsePolicy::NoReferrer.apply(response.headers_mut());
    ///         }
    ///         response
    ///     })?
    ///     .assemble(GuardedRouter::new().route("/work", get(|| async { "ok" })))
    ///     .await?;
    /// # let _ = assembled;
    /// # Ok(()) }
    /// ```
    pub fn with_rendered_fallback<F>(mut self, render: F) -> Result<Self, BoundaryAssemblyError>
    where
        F: Fn(&Parts) -> Response + Send + Sync + 'static,
    {
        if self.fallback.is_some() {
            return Err(BoundaryAssemblyError::ConflictingFallback);
        }
        self.fallback = Some(RenderedFallback(Arc::new(render)));
        Ok(self)
    }
}
