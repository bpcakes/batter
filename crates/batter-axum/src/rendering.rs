//! Output contract for adapter-owned synchronous renderers.

use crate::RenderedInterruption;
use axum::response::{IntoResponse, Response};

/// A rendered response without interruption evidence from a previous response.
///
/// Stored renderer callbacks return this opaque value, so their callers cannot
/// forget cleanup. Ordinary handler responses never pass through this type.
pub(crate) struct PolicyResponse(Response);

impl PolicyResponse {
    pub(crate) fn new(mut response: Response) -> Self {
        response.extensions_mut().remove::<RenderedInterruption>();
        Self(response)
    }
}

impl IntoResponse for PolicyResponse {
    fn into_response(self) -> Response {
        self.0
    }
}
