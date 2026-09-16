//! Application-owned trusted request metadata.
//!
//! The production server accepts only the direct TCP peer supplied by Axum's
//! native [`axum::extract::ConnectInfo`]. It does not interpret `Forwarded`,
//! `X-Forwarded-For`, `X-Real-IP`, `traceparent`, `tracestate`, or a client
//! request ID. Deployments behind a proxy therefore observe the proxy address;
//! no trusted-proxy mode is implemented.
//!
//! Request metadata is diagnostic/admission input, not application authority or
//! execution lifetime. Handlers extract the authenticated [`crate::delivery::OwnerId`],
//! this metadata, and [`batter::operation::OperationContext`] separately and pass
//! each value explicitly:
//!
//! ```
//! use axum::Extension;
//! use batter::operation::OperationContext;
//! use batter_example_reference_service::{
//!     delivery::OwnerId,
//!     request::TrustedRequestMetadata,
//! };
//!
//! async fn handle(
//!     Extension(owner): Extension<OwnerId>,
//!     Extension(metadata): Extension<TrustedRequestMetadata>,
//!     Extension(context): Extension<OperationContext>,
//! ) {
//!     let _authority = owner.as_uuid();
//!     let _request_id = metadata.correlation_id().as_str();
//!     let _direct_peer = metadata.peer().ip();
//!     let _remaining = context.remaining();
//! }
//! ```

use axum::{
    extract::{ConnectInfo, Request, State},
    middleware::Next,
    response::Response,
};
use batter_axum::{CorrelationId, HttpFailure, render_infrastructure_failure};
use std::net::{IpAddr, SocketAddr};

/// Trusted IP from the accepted TCP socket, never from an HTTP header.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct TrustedPeer(IpAddr);

impl TrustedPeer {
    /// Return the direct peer IP for application-owned admission policy.
    #[must_use]
    pub const fn ip(self) -> IpAddr {
        self.0
    }
}

impl std::fmt::Debug for TrustedPeer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TrustedPeer([REDACTED])")
    }
}

/// Server correlation plus trusted transport metadata for one request.
///
/// This type deliberately contains no owner, principal, tenant, quota grant,
/// cancellation token, or deadline. [`Self::correlation_id`] returns Batter's
/// server-generated value unchanged; it never falls back to a header.
///
/// There is intentionally no public constructor. Production middleware creates
/// this value only after combining the server-owned [`CorrelationId`] with
/// native [`ConnectInfo<SocketAddr>`]. An in-process caller must insert that
/// exact ConnectInfo extension and thereby owns the synthetic transport-trust
/// assertion; extractor-only mock fallback does not construct this type.
#[derive(Clone)]
pub struct TrustedRequestMetadata {
    correlation_id: CorrelationId,
    peer: TrustedPeer,
}

impl TrustedRequestMetadata {
    /// Borrow the shared server-generated correlation value.
    #[must_use]
    pub const fn correlation_id(&self) -> &CorrelationId {
        &self.correlation_id
    }

    /// Return the trusted direct socket peer.
    #[must_use]
    pub const fn peer(&self) -> TrustedPeer {
        self.peer
    }
}

impl std::fmt::Debug for TrustedRequestMetadata {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TrustedRequestMetadata([REDACTED])")
    }
}

/// Explicit direct-peer trust policy used by the production listener.
///
/// The only supported mode trusts the accepted socket's peer IP and ignores all
/// forwarding and trace headers. Constructing this policy does not install
/// middleware or grant application authority. Serving settings select this mode
/// in code and carry it through preparation; it is not parsed from an
/// environment setting and does not imply a planned proxy mode.
#[derive(Clone, Copy, Debug, Default)]
pub struct TrustedPeerPolicy {
    _private: (),
}

impl TrustedPeerPolicy {
    /// Select direct-socket peers without proxy-header interpretation.
    #[must_use]
    pub const fn direct() -> Self {
        Self { _private: () }
    }

    fn metadata(self, correlation_id: CorrelationId, peer: SocketAddr) -> TrustedRequestMetadata {
        TrustedRequestMetadata {
            correlation_id,
            peer: TrustedPeer(peer.ip()),
        }
    }
}

pub(crate) async fn install_trusted_request_metadata(
    State(policy): State<TrustedPeerPolicy>,
    mut request: Request,
    next: Next,
) -> Response {
    request.extensions_mut().remove::<TrustedRequestMetadata>();
    let Some(correlation_id) = request.extensions().get::<CorrelationId>().cloned() else {
        return render_infrastructure_failure(HttpFailure::Internal, None);
    };
    let Some(peer) = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connect_info| connect_info.0)
    else {
        return render_infrastructure_failure(HttpFailure::Internal, Some(&correlation_id));
    };
    request
        .extensions_mut()
        .insert(policy.metadata(correlation_id, peer));
    next.run(request).await
}
