//! One request's owned-body start: the continuation admission records, the
//! request head that decides how the transport frames a response, and the
//! framing a started body hands over so the response actually written settles
//! it.

use crate::lock;
use axum::{
    body::HttpBody,
    extract::Request,
    http::{
        HeaderMap, Method, StatusCode, Version,
        header::{CONTENT_LENGTH, TRANSFER_ENCODING},
        response::Parts,
    },
    response::Response,
};
use batter_core::lifecycle::low_level::OperationContinuation;
use http_body::SizeHint;
use std::sync::{Arc, Mutex};

/// The message length the transport frames an owned body with, after which it
/// stops polling without asking for the end. It follows Hyper 1.11.1's server
/// encoding (`proto/h1/role.rs`): a response written without a body, by
/// `can_have_body` for the request method and response status, frames none;
/// a `Transfer-Encoding` header chunk-encodes the message, except in an HTTP/1.0
/// answer, which drops that header; otherwise a `Content-Length` of plain
/// digits, repeated only with the same value, frames it, else the body's exact
/// size does. The answer's version is Hyper's effective one: HTTP/1.0 for an
/// HTTP/1.0 request, else the response's own (`conn.rs` `enforce_version`).
///
/// A declared length that differs from the body's exact size is an
/// application error Hyper does not support: it frames by the exact size and
/// writes the header unchanged, and debug builds assert they match. The owned
/// body judges by the declared length, which is what the peer reads, so a body
/// shorter than its declaration stays `failed`.
///
/// It is settled again from the response that is finally written: on the
/// registered server from the response handed to Hyper, so layers outside
/// admission count; elsewhere from the response request admission returns.
#[derive(Clone, Default)]
pub(crate) struct Framing(Arc<Mutex<Option<u64>>>);

impl Framing {
    pub(super) fn settle(
        &self,
        method: &Method,
        version: Version,
        status: StatusCode,
        headers: &HeaderMap,
        hint: &SizeHint,
    ) {
        let bodyless = method == Method::HEAD
            || method == Method::CONNECT && status.is_success()
            || status.is_informational()
            || matches!(status, StatusCode::NO_CONTENT | StatusCode::NOT_MODIFIED);
        let chunked = version != Version::HTTP_10 && headers.contains_key(TRANSFER_ENCODING);
        let length = if bodyless || chunked {
            None
        } else {
            declared_length(headers).or_else(|| hint.exact())
        };
        *lock(&self.0) = length;
    }

    pub(super) fn length(&self) -> Option<u64> {
        *lock(&self.0)
    }
}

/// A `Content-Length` Hyper frames by: every value plain digits, and all
/// equal. Hyper refuses to write any other unless the body knows its exact
/// size, which then frames the message instead.
fn declared_length(headers: &HeaderMap) -> Option<u64> {
    let mut values = headers
        .get_all(CONTENT_LENGTH)
        .iter()
        .map(|value| digits(value.as_bytes()));
    let first = values.next()??;
    values.all(|value| value == Some(first)).then_some(first)
}

/// Hyper's `from_digits`: no sign, no whitespace, no overflow.
fn digits(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() {
        return None;
    }
    bytes.iter().try_fold(0_u64, |length, &byte| {
        byte.is_ascii_digit().then_some(())?;
        length.checked_mul(10)?.checked_add(u64::from(byte - b'0'))
    })
}

/// One request's owned-body start: a single allocation per request, shared
/// by every clone of the admission record and, on the registered server, by
/// the connection's future for that response.
#[derive(Clone)]
pub(crate) struct BodyStart(Arc<Mutex<Start>>);

struct Start {
    method: Method,
    version: Version,
    /// Whether an admission took this record. A copy carried into another
    /// request reaches that request's admission already claimed, so it gets
    /// a record of its own instead of sharing this one.
    claimed: bool,
    continuation: Option<OperationContinuation>,
    framing: Option<Framing>,
}

impl Start {
    /// The version Hyper writes a `response` version answer in.
    fn answered(&self, response: Version) -> Version {
        if self.version == Version::HTTP_10 {
            Version::HTTP_10
        } else {
            response
        }
    }

    fn new(method: &Method, version: Version) -> Self {
        Self {
            method: method.clone(),
            version,
            claimed: false,
            continuation: None,
            framing: None,
        }
    }
}

impl BodyStart {
    /// The registered server's record for `request`, which admission claims
    /// from its extensions while the server keeps a copy for the response.
    pub(crate) fn served(request: &Request) -> Self {
        Self(Arc::new(Mutex::new(Start::new(
            request.method(),
            request.version(),
        ))))
    }

    /// Admission's record for `request`: the one the registered server
    /// created, unless another admission claimed it first, else a new one.
    pub(crate) fn claim(request: &mut Request) -> Self {
        let served = request
            .extensions_mut()
            .remove::<Self>()
            .filter(|start| !std::mem::replace(&mut lock(&start.0).claimed, true));
        served.unwrap_or_else(|| {
            Self(Arc::new(Mutex::new(Start {
                claimed: true,
                ..Start::new(request.method(), request.version())
            })))
        })
    }

    pub(crate) fn admit(&self, continuation: OperationContinuation) {
        lock(&self.0).continuation = Some(continuation);
    }

    pub(crate) fn take_continuation(&self) -> Option<OperationContinuation> {
        lock(&self.0).continuation.take()
    }

    /// Settle `framing` from the response the handler built, for this
    /// request's head.
    pub(super) fn frame(&self, framing: &Framing, parts: &Parts, hint: &SizeHint) {
        let start = lock(&self.0);
        let version = start.answered(parts.version);
        framing.settle(&start.method, version, parts.status, &parts.headers, hint);
    }

    /// Keep a started body's framing, so the response finally written settles
    /// it again.
    pub(super) fn hand_over(&self, framing: &Framing) {
        lock(&self.0).framing = Some(framing.clone());
    }

    /// Settle the started body's framing, if a body started, from `response`.
    pub(crate) fn settle(&self, response: &Response) {
        let Some(framing) = lock(&self.0).framing.clone() else {
            return;
        };
        // Asked before locking: the body's answer runs application code.
        let hint = response.body().size_hint();
        let start = lock(&self.0);
        framing.settle(
            &start.method,
            start.answered(response.version()),
            response.status(),
            response.headers(),
            &hint,
        );
    }
}

#[cfg(test)]
#[path = "start_tests.rs"]
mod tests;
