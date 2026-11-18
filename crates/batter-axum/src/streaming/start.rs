//! One request's owned-body start: the continuation admission records and the
//! request method, with the rule for the length a built response declares.

use crate::lock;
use axum::{
    extract::Request,
    http::{HeaderMap, Method, StatusCode, header::CONTENT_LENGTH, response::Parts},
};
use batter_core::lifecycle::low_level::OperationContinuation;
use std::sync::{Arc, Mutex};

/// One request's owned-body start: a single allocation per admitted request,
/// shared by every clone of its admission record.
#[derive(Clone)]
pub(crate) struct BodyStart(Arc<Shared>);

struct Shared {
    method: Method,
    continuation: Mutex<Option<OperationContinuation>>,
}

impl BodyStart {
    /// Admission's record for `request`.
    pub(crate) fn new(request: &Request) -> Self {
        Self(Arc::new(Shared {
            method: request.method().clone(),
            continuation: Mutex::new(None),
        }))
    }

    pub(crate) fn admit(&self, continuation: OperationContinuation) {
        *lock(&self.0.continuation) = Some(continuation);
    }

    pub(crate) fn take_continuation(&self) -> Option<OperationContinuation> {
        lock(&self.0.continuation).take()
    }

    /// The length the response the builder returned declares for its body, by
    /// which the owned body's outcome is judged: its `Content-Length`, else
    /// the exact size its body reports, which `exact` asks for. Asking is
    /// application code, so it happens only where nothing else decides.
    ///
    /// These are facts of the application's own response at the hand-over,
    /// read once. Nothing later changes the answer: not a header a handler's
    /// tuple or a layer adds afterwards, and not how a transport frames the
    /// message, which the body cannot observe.
    ///
    /// A response that carries no content by HTTP's own rules declares none,
    /// whatever its headers say (RFC 9110, section 6.4.1): one answering
    /// HEAD, a 2xx answer to CONNECT, and any 1xx, 204 or 304 status. Its
    /// `Content-Length`, where one is allowed, describes another response.
    pub(super) fn declared(
        &self,
        parts: &Parts,
        exact: impl FnOnce() -> Option<u64>,
    ) -> Option<u64> {
        if carries_no_content(&self.0.method, parts.status) {
            return None;
        }
        content_length(&parts.headers).or_else(exact)
    }
}

fn carries_no_content(method: &Method, status: StatusCode) -> bool {
    method == Method::HEAD
        || method == Method::CONNECT && status.is_success()
        || status.is_informational()
        || matches!(status, StatusCode::NO_CONTENT | StatusCode::NOT_MODIFIED)
}

/// A `Content-Length` that states one length: every value plain decimal
/// digits (RFC 9110, section 8.6), and all values equal. Any other header
/// declares nothing here.
fn content_length(headers: &HeaderMap) -> Option<u64> {
    let mut values = headers
        .get_all(CONTENT_LENGTH)
        .iter()
        .map(|value| digits(value.as_bytes()));
    let first = values.next()??;
    values.all(|value| value == Some(first)).then_some(first)
}

/// No sign, no whitespace, no overflow.
fn digits(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() {
        return None;
    }
    bytes.iter().try_fold(0_u64, |length, &byte| {
        byte.is_ascii_digit().then_some(())?;
        length.checked_mul(10)?.checked_add(u64::from(byte - b'0'))
    })
}

#[cfg(test)]
#[path = "start_tests.rs"]
mod tests;
