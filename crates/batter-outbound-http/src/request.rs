//! Request preparation: every construction failure happens here.

use crate::{exchange::Exchange, limits::ResponseLimit, send};
use bytes::Bytes;
use http::{
    HeaderMap, HeaderName, HeaderValue, Method,
    header::{
        CONNECTION, CONTENT_LENGTH, CONTENT_TYPE, EXPECT, HOST, PROXY_AUTHORIZATION, TE, TRAILER,
        TRANSFER_ENCODING, UPGRADE,
    },
};
use std::fmt;
use tokio::time::Instant;
use url::Url;

/// Headers whose values the exchange owns. Framing and hop-by-hop headers
/// would change how the request is delimited or routed, and `Content-Type`
/// travels with its body.
fn reserved(name: &HeaderName) -> bool {
    [
        HOST,
        CONTENT_LENGTH,
        TRANSFER_ENCODING,
        CONNECTION,
        UPGRADE,
        TE,
        TRAILER,
        EXPECT,
        PROXY_AUTHORIZATION,
        CONTENT_TYPE,
    ]
    .contains(name)
        || name == "keep-alive"
        || name == "proxy-connection"
}

/// A request being assembled. Nothing is validated or sent until
/// [`RequestDraft::prepare`], which reports the first problem found.
///
/// `Debug` shows nothing about the method, path, query, headers or body.
#[must_use = "a draft sends nothing until it is prepared and sent"]
pub struct RequestDraft {
    http: reqwest::Client,
    url: Url,
    method: Method,
    segments: Vec<String>,
    query: Vec<(String, String)>,
    headers: HeaderMap,
    body: Option<(HeaderValue, Bytes)>,
    error: Option<PrepareError>,
}

impl RequestDraft {
    pub(crate) fn new(
        http: reqwest::Client,
        url: Url,
        method: Method,
        segments: Vec<String>,
    ) -> Self {
        Self {
            http,
            url,
            method,
            segments,
            query: Vec::new(),
            headers: HeaderMap::new(),
            body: None,
            error: None,
        }
    }

    /// Append a header. Its value is always marked sensitive, so it is hidden
    /// from `Debug` output of the request and its headers.
    ///
    /// `Host`, `Content-Length`, `Transfer-Encoding`, `Connection`, `Upgrade`,
    /// `TE`, `Trailer`, `Expect`, `Keep-Alive`, `Proxy-Connection`,
    /// `Proxy-Authorization` and `Content-Type` are refused with
    /// [`PrepareError::ReservedHeader`]. Set the content type with
    /// [`RequestDraft::body`] or [`RequestDraft::json`].
    pub fn header(mut self, name: HeaderName, mut value: HeaderValue) -> Self {
        if reserved(&name) {
            self.fail(PrepareError::ReservedHeader);
        } else {
            value.set_sensitive(true);
            self.headers.append(name, value);
        }
        self
    }

    /// Append a form-encoded query parameter.
    ///
    /// The exchange keeps the query out of every value it returns, but the
    /// provider and any intermediary still see it. Prefer headers or the body
    /// for credentials when the provider allows it.
    pub fn query(mut self, name: &str, value: &str) -> Self {
        self.query.push((name.to_owned(), value.to_owned()));
        self
    }

    /// Set a JSON body with `Content-Type: application/json`.
    ///
    /// A serialization failure or a second body is reported by
    /// [`RequestDraft::prepare`].
    pub fn json<T: serde::Serialize + ?Sized>(self, value: &T) -> Self {
        match serde_json::to_vec(value) {
            Ok(bytes) => self.body(HeaderValue::from_static("application/json"), bytes),
            Err(_) => {
                let mut draft = self;
                draft.fail(PrepareError::Json);
                draft
            }
        }
    }

    /// Set the body and its content type. A second body is reported by
    /// [`RequestDraft::prepare`].
    pub fn body(mut self, content_type: HeaderValue, body: impl Into<Bytes>) -> Self {
        if self.body.is_some() {
            self.fail(PrepareError::BodyAlreadySet);
        } else {
            self.body = Some((content_type, body.into()));
        }
        self
    }

    /// Validate and build the request. No connection is made.
    ///
    /// ```
    /// use batter_outbound_http::{Client, ConnectTimeout, Origin, PrepareError, http::Method};
    /// use std::time::Duration;
    ///
    /// let client = Client::new(
    ///     Origin::https("https://provider.example").unwrap(),
    ///     ConnectTimeout::new(Duration::from_secs(5)).unwrap(),
    /// )
    /// .unwrap();
    /// let refused = client.request(Method::GET, ["v1", ".."]).prepare();
    /// assert_eq!(refused.err(), Some(PrepareError::DotSegment));
    /// ```
    pub fn prepare(self) -> Result<PreparedRequest, PrepareError> {
        if let Some(error) = self.error {
            return Err(error);
        }
        if self.method == Method::CONNECT {
            return Err(PrepareError::ConnectMethod);
        }
        let Self {
            http,
            mut url,
            method,
            segments,
            query,
            mut headers,
            body,
            ..
        } = self;
        for segment in &segments {
            if segment.is_empty() {
                return Err(PrepareError::EmptySegment);
            }
            if segment == "." || segment == ".." {
                return Err(PrepareError::DotSegment);
            }
            // Servers that strip `;` path parameters would read "..;x" as "..".
            match segment.split(';').next() {
                Some("") => return Err(PrepareError::EmptySegment),
                Some("." | "..") => return Err(PrepareError::DotSegment),
                _ => {}
            }
            // URL parsing deletes these before applying its dot-segment rules,
            // so ".\t." would otherwise become ".." and remove a path level.
            if segment.contains(['\t', '\n', '\r']) {
                return Err(PrepareError::ControlCharacter);
            }
        }
        if !segments.is_empty() {
            let mut path = url.path_segments_mut().map_err(|()| PrepareError::Build)?;
            path.pop_if_empty().extend(&segments);
        }
        // Every segment must survive URL normalization as exactly one segment.
        let expected = segments.len().max(1);
        if url.path_segments().map(Iterator::count) != Some(expected) {
            return Err(PrepareError::Build);
        }
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(&query);
        }
        // reqwest converts the URL to an HTTP URI only when the request is
        // executed and refuses it there; refuse it here, before an exchange exists.
        if http::Uri::try_from(url.as_str()).is_err() {
            return Err(PrepareError::InvalidUri);
        }
        let mut builder = http.request(method, url);
        if let Some((content_type, bytes)) = body {
            headers.insert(CONTENT_TYPE, content_type);
            builder = builder.body(bytes);
        }
        let request = builder
            .headers(headers)
            .build()
            .map_err(|_| PrepareError::Build)?;
        Ok(PreparedRequest { http, request })
    }

    fn fail(&mut self, error: PrepareError) {
        self.error.get_or_insert(error);
    }
}

impl fmt::Debug for RequestDraft {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RequestDraft { .. }")
    }
}

/// A validated request that can be sent exactly once.
///
/// It is not `Clone`: a second attempt needs a new preparation, which keeps
/// every send visible in the application's own code.
///
/// ```compile_fail,E0599
/// # use batter_outbound_http::PreparedRequest;
/// fn twice(request: PreparedRequest) -> (PreparedRequest, PreparedRequest) {
///     (request.clone(), request)
/// }
/// ```
///
/// ```compile_fail,E0382
/// # use batter_outbound_http::{PreparedRequest, ResponseLimit};
/// async fn twice(request: PreparedRequest, deadline: tokio::time::Instant, limit: ResponseLimit) {
///     let _first = request.send(deadline, limit).await;
///     let _second = request.send(deadline, limit).await;
/// }
/// ```
///
/// `Debug` shows nothing about the request.
#[must_use = "a prepared request sends nothing until it is sent"]
pub struct PreparedRequest {
    pub(crate) http: reqwest::Client,
    pub(crate) request: reqwest::Request,
}

impl PreparedRequest {
    /// Send the request once and classify what happened before `deadline`.
    ///
    /// The exchange returns no later than [`crate::RESERVE`] before `deadline`,
    /// apart from scheduling delay and any poll that blocks (see the crate's
    /// Limits). It does not start when less than `RESERVE + MINIMUM_ATTEMPT`
    /// remains. `RESERVE` only lets the classification arrive before the
    /// deadline; recording a durable outcome needs its own budget, such as a
    /// reserved finalization phase. Dropping this future after the request was
    /// handed to the client produces no outcome; treat that as
    /// [`Exchange::Unknown`]. Must run inside a Tokio runtime with its time
    /// driver enabled.
    pub async fn send(self, deadline: Instant, limit: ResponseLimit) -> Exchange {
        send::run(
            self.http,
            self.request,
            deadline,
            limit,
            std::future::pending(),
        )
        .await
    }
}

impl fmt::Debug for PreparedRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PreparedRequest { .. }")
    }
}

/// Why a request could not be prepared. The message carries no request data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PrepareError {
    /// An empty path segment, or one whose part before a `;` is empty, would
    /// change the target path.
    #[error("a path segment is empty")]
    EmptySegment,
    /// URL normalization would silently drop a `.` or `..` segment, and servers
    /// that strip `;` path parameters would treat `..;x` the same way.
    #[error("a path segment is a dot segment")]
    DotSegment,
    /// URL parsing deletes tab, line feed and carriage return from a segment,
    /// which could turn it into a dot segment or an empty one.
    #[error("a path segment contains a tab, line feed or carriage return")]
    ControlCharacter,
    /// The URL is not a valid HTTP request target, for example because it is
    /// longer than the `http` crate accepts.
    #[error("the request URL is not a valid HTTP URI")]
    InvalidUri,
    /// The header is managed by the exchange.
    #[error("the header is managed by the exchange and cannot be set")]
    ReservedHeader,
    /// Only one body can be set.
    #[error("the request body was set more than once")]
    BodyAlreadySet,
    /// The value could not be serialized as JSON.
    #[error("the JSON body could not be serialized")]
    Json,
    /// `CONNECT` opens a tunnel, which is not a provider request.
    #[error("the CONNECT method cannot be sent through an exchange")]
    ConnectMethod,
    /// The HTTP request could not be built, or a segment did not survive URL
    /// normalization as exactly one segment. Neither is reachable with the
    /// checks above and url 2.5.8; both stay as defence in depth.
    #[error("the HTTP request could not be built")]
    Build,
}

#[cfg(test)]
mod tests {
    use crate::{Client, ConnectTimeout, Origin};
    use http::{HeaderName, HeaderValue, Method};
    use std::time::Duration;

    #[test]
    fn every_caller_header_value_is_marked_sensitive() {
        let client = Client::new(
            Origin::https("https://provider.example").unwrap(),
            ConnectTimeout::new(Duration::from_secs(5)).unwrap(),
        )
        .unwrap();
        let prepared = client
            .request(Method::POST, ["v1"])
            .header(
                HeaderName::from_static("authorization"),
                HeaderValue::from_static("credential"),
            )
            .header(
                HeaderName::from_static("x-api-key"),
                HeaderValue::from_static("key"),
            )
            .json(&1)
            .prepare()
            .unwrap();
        let headers = prepared.request.headers();
        for name in ["authorization", "x-api-key", "content-type"] {
            let value = headers.get(name).expect("header present");
            assert_eq!(value.is_sensitive(), name != "content-type", "{name}");
        }
        assert!(!format!("{headers:?}").contains("credential"));
    }
}
