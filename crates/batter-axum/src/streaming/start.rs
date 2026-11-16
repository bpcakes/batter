//! One request's owned-body start: the continuation admission records, the
//! request head that decides how a response is framed and the framing of the
//! body it started; and the framing of the message the registered server's
//! transport writes, which a body that transport reaches adopts.

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
use std::{
    cell::Cell,
    sync::{Arc, Mutex, OnceLock},
};

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
/// Admission settles it from the response it returns, after the handler's
/// head at the hand-over. On the registered server the message that actually
/// carries the body decides instead: once the transport writing a response
/// calls into the body, through any layer, the body adopts that message's
/// [`Written`] framing, and nothing settles it again.
#[derive(Clone, Default)]
pub(crate) struct Framing(Arc<Mutex<Settled>>);

#[derive(Clone, Copy, Default)]
struct Settled {
    length: Option<u64>,
    /// Hyper refuses to write the head of the message carrying the body, so
    /// it never polls the body.
    unwritable: bool,
    /// The registered server's transport reached the body: this is the
    /// framing of the message it writes.
    written: bool,
}

/// What decides how Hyper frames one response.
pub(super) struct Head<'a> {
    pub(super) method: &'a Method,
    /// Hyper's effective version for the answer.
    pub(super) version: Version,
    pub(super) status: StatusCode,
    pub(super) headers: &'a HeaderMap,
}

impl Head<'_> {
    fn bodyless(&self) -> bool {
        self.method == Method::HEAD
            || self.method == Method::CONNECT && self.status.is_success()
            || self.status.is_informational()
            || matches!(
                self.status,
                StatusCode::NO_CONTENT | StatusCode::NOT_MODIFIED
            )
    }

    fn length(&self, hint: &SizeHint) -> Option<u64> {
        let chunked =
            self.version != Version::HTTP_10 && self.headers.contains_key(TRANSFER_ENCODING);
        if self.bodyless() || chunked {
            None
        } else {
            declared_length(self.headers).or_else(|| hint.exact())
        }
    }

    /// Whether Hyper refuses to write this head (`role.rs:396-408`,
    /// `706-710`, `756-781`, `806-810`, before its no-body rule at `965`):
    /// a final 1xx status other than 101, which it answers with a 500
    /// instead; a 2xx answer to CONNECT with any length header; a
    /// `Content-Length` beside `Transfer-Encoding`, unless `Transfer-Encoding`
    /// comes first in an answer that cannot be chunked, which drops it; or,
    /// for a body of unknown size, a `Content-Length` it cannot parse or whose
    /// values differ.
    fn refused(&self, hint: &SizeHint) -> bool {
        if self.status.is_informational() && self.status != StatusCode::SWITCHING_PROTOCOLS {
            return true;
        }
        let length = self.headers.contains_key(CONTENT_LENGTH);
        let encoding = self.headers.contains_key(TRANSFER_ENCODING);
        let connect = self.method == Method::CONNECT && self.status.is_success();
        if connect && (length || encoding) {
            return true;
        }
        // Only a head carrying both headers depends on their order, so only
        // that one is scanned.
        let conflicting = length && encoding && {
            let position = |name| self.headers.keys().position(|key| key == name);
            let chunkable = self.version != Version::HTTP_10 && !self.bodyless();
            position(CONTENT_LENGTH) < position(TRANSFER_ENCODING) || chunkable
        };
        let unparsable =
            length && hint.exact().is_none() && declared_length(self.headers).is_none();
        conflicting || unparsable
    }
}

/// The version Hyper writes an answer in: HTTP/1.0 to an HTTP/1.0 request,
/// else the response's own.
fn answered(request: Version, response: Version) -> Version {
    if request == Version::HTTP_10 {
        Version::HTTP_10
    } else {
        response
    }
}

impl Framing {
    /// Take the length `head` frames the body with, unless the transport
    /// writing the body already decided it.
    fn settle(&self, head: &Head<'_>, hint: &SizeHint) {
        let mut settled = lock(&self.0);
        if !settled.written {
            settled.length = head.length(hint);
        }
    }

    /// Adopt the framing of the message whose transport is calling into the
    /// body now, if any. The first such message decides: the body is part of
    /// it from then on.
    pub(super) fn adopt_written(&self) {
        let Some(written) = Written::current() else {
            return;
        };
        let mut settled = lock(&self.0);
        if !settled.written {
            *settled = Settled {
                length: written.length,
                unwritable: written.refused,
                written: true,
            };
        }
    }

    pub(super) fn length(&self) -> Option<u64> {
        lock(&self.0).length
    }

    pub(super) fn unwritable(&self) -> bool {
        lock(&self.0).unwritable
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

thread_local! {
    /// The framing of the message whose transport is calling into its
    /// response body on this thread, if any.
    static WRITING: Cell<Option<Written>> = const { Cell::new(None) };
}

/// How the registered server's transport writes one response: the length
/// Hyper frames the message with and whether it refuses the head. Computed
/// once from the request head and the response handed to Hyper, after every
/// layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Written {
    length: Option<u64>,
    refused: bool,
}

impl Written {
    /// The framing of `response`, answering a `method` request in `version`.
    /// Asks the response body its size, which can run application code.
    pub(crate) fn of(method: &Method, version: Version, response: &Response) -> Self {
        let hint = response.body().size_hint();
        let head = Head {
            method,
            version: answered(version, response.version()),
            status: response.status(),
            headers: response.headers(),
        };
        Self {
            length: head.length(&hint),
            refused: head.refused(&hint),
        }
    }

    /// Run `call`, one call the transport makes into the response body it
    /// writes, with this framing current, so an owned body that call reaches,
    /// through whatever layers wrap it, adopts it. Calls are synchronous, so
    /// a body polled on another task is not reached; the previous value is
    /// restored even if `call` unwinds.
    pub(crate) fn writing<T>(self, call: impl FnOnce() -> T) -> T {
        struct Restore(Option<Written>);
        impl Drop for Restore {
            fn drop(&mut self) {
                let _ = WRITING.try_with(|current| current.set(self.0));
            }
        }
        let previous = WRITING.try_with(|current| current.replace(Some(self)));
        let _restore = Restore(previous.ok().flatten());
        call()
    }

    fn current() -> Option<Self> {
        WRITING.try_with(Cell::get).ok().flatten()
    }
}

/// One request's owned-body start: a single allocation per admitted request,
/// shared by every clone of its admission record.
#[derive(Clone)]
pub(crate) struct BodyStart(Arc<Shared>);

struct Shared {
    method: Method,
    version: Version,
    continuation: Mutex<Option<OperationContinuation>>,
    /// The framing of the body the continuation started, handed over once.
    framing: OnceLock<Framing>,
}

impl BodyStart {
    /// Admission's record for `request`.
    pub(crate) fn new(request: &Request) -> Self {
        Self(Arc::new(Shared {
            method: request.method().clone(),
            version: request.version(),
            continuation: Mutex::new(None),
            framing: OnceLock::new(),
        }))
    }

    pub(crate) fn admit(&self, continuation: OperationContinuation) {
        *lock(&self.0.continuation) = Some(continuation);
    }

    pub(crate) fn take_continuation(&self) -> Option<OperationContinuation> {
        lock(&self.0.continuation).take()
    }

    fn head<'a>(
        &'a self,
        version: Version,
        status: StatusCode,
        headers: &'a HeaderMap,
    ) -> Head<'a> {
        Head {
            method: &self.0.method,
            version: answered(self.0.version, version),
            status,
            headers,
        }
    }

    /// Settle `framing` provisionally from the response the handler built,
    /// for this request's head.
    pub(super) fn frame(&self, framing: &Framing, parts: &Parts, hint: &SizeHint) {
        framing.settle(
            &self.head(parts.version, parts.status, &parts.headers),
            hint,
        );
    }

    /// Keep the started body's framing, so the response admission returns
    /// settles it again.
    pub(super) fn hand_over(&self, framing: &Framing) {
        // The continuation is taken once, so only one body hands over.
        let _ = self.0.framing.set(framing.clone());
    }

    /// Settle the started body's framing from the response request admission
    /// returns. Most responses start no body and do nothing here.
    pub(crate) fn settle(&self, response: &Response) {
        let Some(framing) = self.0.framing.get() else {
            return;
        };
        // The body's answer runs application code, so no lock is held.
        let hint = response.body().size_hint();
        let head = self.head(response.version(), response.status(), response.headers());
        framing.settle(&head, &hint);
    }
}

#[cfg(test)]
#[path = "start_tests.rs"]
mod tests;
