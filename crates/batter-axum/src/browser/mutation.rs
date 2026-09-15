use super::BrowserOrigin;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use std::{error::Error, fmt};

const SEC_FETCH_SITE: HeaderName = HeaderName::from_static("sec-fetch-site");

/// One exact custom browser-request marker.
///
/// The configured value is deliberately absent from accessors and Debug output.
/// An accepted name can be set by browser script and is not CORS-safelisted,
/// but that does not prove the browser received it from script: user agents can
/// attach other non-safelisted request fields themselves. [`MutationPolicy`]
/// therefore always pairs a custom marker with strict same-origin Fetch Metadata.
///
/// ```
/// use axum::http::{HeaderName, HeaderValue};
/// use batter_axum::browser::{MutationPolicy, RequiredHeader};
///
/// let marker = RequiredHeader::new(
///     HeaderName::from_static("x-browser-request"),
///     HeaderValue::from_static("1"),
/// )?;
/// assert_eq!(marker.name(), "x-browser-request");
/// assert!(!format!("{marker:?}").contains('1'));
/// let policy = MutationPolicy::required_header(marker);
/// # let _ = policy;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone)]
pub struct RequiredHeader {
    name: HeaderName,
    value: HeaderValue,
}

impl RequiredHeader {
    /// Construct an exact non-safelisted, script-settable request header.
    ///
    /// CORS-safelisted names and browser-forbidden names cannot serve as a
    /// custom mutation marker. The expected value must use printable ASCII,
    /// contain no tabs, and have no surrounding spaces so Fetch normalization
    /// cannot change it before the request reaches the adapter. Construction
    /// alone does not prove that a browser received the field from script or
    /// performed a preflight; [`MutationPolicy`] supplies the independent
    /// strict same-origin check.
    pub fn new(name: HeaderName, value: HeaderValue) -> Result<Self, RequiredHeaderError> {
        if is_cors_safelisted_name(&name) {
            return Err(RequiredHeaderError::CorsSafelistedName);
        }
        if is_forbidden_request_name(&name) {
            return Err(RequiredHeaderError::BrowserControlledName);
        }
        let bytes = value.as_bytes();
        if bytes.iter().any(|byte| !(b' '..=b'~').contains(byte)) {
            return Err(RequiredHeaderError::NonVisibleAsciiValue);
        }
        if matches!(bytes.first(), Some(b' ')) || matches!(bytes.last(), Some(b' ')) {
            return Err(RequiredHeaderError::SurroundingWhitespaceValue);
        }
        Ok(Self { name, value })
    }

    /// Return the configured header name without exposing its expected value.
    pub const fn name(&self) -> &HeaderName {
        &self.name
    }
}

/// Sanitized invalid custom-marker configuration.
///
/// ```
/// use axum::http::{HeaderValue, header};
/// use batter_axum::browser::{RequiredHeader, RequiredHeaderError};
///
/// assert!(matches!(
///     RequiredHeader::new(header::ACCEPT, HeaderValue::from_static("*/*")),
///     Err(RequiredHeaderError::CorsSafelistedName),
/// ));
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RequiredHeaderError {
    /// This name can be sent by a cross-origin request without a CORS preflight.
    CorsSafelistedName,
    /// Browser script cannot control this request-header name.
    BrowserControlledName,
    /// The exact expected value is outside the visible-ASCII request contract.
    NonVisibleAsciiValue,
    /// Fetch would remove surrounding whitespace before sending this value.
    SurroundingWhitespaceValue,
}

impl fmt::Display for RequiredHeaderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::CorsSafelistedName => {
                "custom browser marker name does not require a CORS preflight"
            }
            Self::BrowserControlledName => {
                "custom browser marker name is controlled by the user agent"
            }
            Self::NonVisibleAsciiValue => "custom browser marker value is outside visible ASCII",
            Self::SurroundingWhitespaceValue => {
                "custom browser marker value has surrounding whitespace"
            }
        })
    }
}

impl Error for RequiredHeaderError {}

impl fmt::Debug for RequiredHeader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RequiredHeader")
            .field("name", &self.name)
            .field("value", &"[REDACTED]")
            .finish()
    }
}

/// Policy for the optional `Sec-Fetch-Site` defense-in-depth signal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FetchSitePolicy {
    /// Reject an explicit `cross-site`; permit absence and valid future tokens.
    RejectCrossSite,
    /// Require exactly one case-sensitive `same-origin` token.
    RequireSameOrigin,
}

/// A non-empty collection of exact browser mutation signals.
///
/// At least an exact configured Origin or an exact custom header is always
/// required. A custom header automatically installs strict same-origin Fetch
/// Metadata and cannot weaken an already configured Fetch Metadata check. Every
/// configured check is ANDed. Passing this policy is neither authentication nor
/// a complete CSRF guarantee, and non-browser clients can forge browser fields.
#[derive(Clone, Debug)]
pub struct MutationPolicy {
    origin: Option<BrowserOrigin>,
    required_header: Option<RequiredHeader>,
    fetch_site: Option<FetchSitePolicy>,
    require_json: bool,
}

impl MutationPolicy {
    /// Start a policy that requires exactly one matching Origin field.
    pub fn exact_origin(origin: BrowserOrigin) -> Self {
        Self {
            origin: Some(origin),
            required_header: None,
            fetch_site: None,
            require_json: false,
        }
    }

    /// Start a policy requiring one matching custom field and strict Fetch Metadata.
    ///
    /// Exactly one `Sec-Fetch-Site: same-origin` field is automatic. A caller
    /// cannot replace it with the weaker [`FetchSitePolicy::RejectCrossSite`].
    /// Missing Fetch Metadata fails closed, so the mutation URL must be
    /// potentially trustworthy and the supported browser must emit the field.
    pub fn required_header(header: RequiredHeader) -> Self {
        Self {
            origin: None,
            required_header: Some(header),
            fetch_site: Some(FetchSitePolicy::RequireSameOrigin),
            require_json: false,
        }
    }

    /// Add an exact Origin requirement, rejecting duplicate configuration.
    pub fn and_exact_origin(mut self, origin: BrowserOrigin) -> Result<Self, MutationPolicyError> {
        if self.origin.replace(origin).is_some() {
            return Err(MutationPolicyError::OriginAlreadyConfigured);
        }
        Ok(self)
    }

    /// Add an exact custom-header requirement and enforce strict Fetch Metadata.
    ///
    /// Duplicate custom-header configuration is rejected. Any existing
    /// [`FetchSitePolicy::RejectCrossSite`] check is strengthened to
    /// [`FetchSitePolicy::RequireSameOrigin`]; a custom marker can never weaken
    /// strict Fetch Metadata or serve as the only cross-site defense. Missing
    /// Fetch Metadata fails closed, including on non-trustworthy URLs and in
    /// clients that do not emit the field.
    pub fn and_required_header(
        mut self,
        header: RequiredHeader,
    ) -> Result<Self, MutationPolicyError> {
        if self.required_header.replace(header).is_some() {
            return Err(MutationPolicyError::RequiredHeaderAlreadyConfigured);
        }
        self.fetch_site = Some(FetchSitePolicy::RequireSameOrigin);
        Ok(self)
    }

    /// Add one Fetch Metadata policy, rejecting duplicate configuration.
    pub fn with_fetch_site(mut self, policy: FetchSitePolicy) -> Result<Self, MutationPolicyError> {
        if self.fetch_site.replace(policy).is_some() {
            return Err(MutationPolicyError::FetchSiteAlreadyConfigured);
        }
        Ok(self)
    }

    /// Require exactly one `application/json` Content-Type field.
    pub fn require_json(mut self) -> Result<Self, MutationPolicyError> {
        if self.require_json {
            return Err(MutationPolicyError::JsonAlreadyRequired);
        }
        self.require_json = true;
        Ok(self)
    }

    /// Check configured signals in Origin, Fetch Metadata, marker, JSON order.
    ///
    /// ```
    /// use axum::http::{HeaderMap, HeaderValue, header};
    /// use batter_axum::browser::{BrowserOrigin, FetchSitePolicy, MutationPolicy};
    ///
    /// let policy = MutationPolicy::exact_origin(
    ///     BrowserOrigin::https("https://app.example")?,
    /// )
    /// .with_fetch_site(FetchSitePolicy::RejectCrossSite)?
    /// .require_json()?;
    /// let mut headers = HeaderMap::new();
    /// headers.insert(header::ORIGIN, HeaderValue::from_static("https://app.example"));
    /// headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    /// policy.check(&headers)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn check(&self, headers: &HeaderMap) -> Result<(), MutationRejection> {
        if let Some(origin) = &self.origin {
            check_origin(headers, origin)?;
        }
        if let Some(policy) = self.fetch_site {
            check_fetch_site(headers, policy)?;
        }
        if let Some(required) = &self.required_header {
            check_required_header(headers, required)?;
        }
        if self.require_json {
            check_json(headers)?;
        }
        Ok(())
    }
}

/// Duplicate mutation-policy configuration category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MutationPolicyError {
    /// A second exact Origin was supplied.
    OriginAlreadyConfigured,
    /// A second exact custom header was supplied.
    RequiredHeaderAlreadyConfigured,
    /// A second Fetch Metadata policy was supplied.
    FetchSiteAlreadyConfigured,
    /// JSON was required a second time.
    JsonAlreadyRequired,
}

impl fmt::Display for MutationPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::OriginAlreadyConfigured => "browser Origin policy is already configured",
            Self::RequiredHeaderAlreadyConfigured => {
                "browser custom-header policy is already configured"
            }
            Self::FetchSiteAlreadyConfigured => "Fetch Metadata policy is already configured",
            Self::JsonAlreadyRequired => "JSON media-type policy is already configured",
        })
    }
}

impl Error for MutationPolicyError {}

/// Sanitized application-mappable browser mutation rejection.
///
/// Batter selects no response body or error envelope:
///
/// ```
/// use axum::{http::StatusCode, response::{IntoResponse, Response}};
/// use batter_axum::browser::MutationRejection;
///
/// fn render(rejection: MutationRejection) -> Response {
///     (rejection.status(), rejection.code()).into_response()
/// }
/// let response = render(MutationRejection::OriginMissing);
/// assert_eq!(response.status(), StatusCode::FORBIDDEN);
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MutationRejection {
    /// The exact Origin field is absent.
    OriginMissing,
    /// Origin has duplicate or non-UTF8 field values.
    OriginAmbiguous,
    /// The one Origin field does not exactly match configuration.
    OriginMismatch,
    /// Fetch Metadata explicitly reports a cross-site request.
    FetchSiteCrossSite,
    /// Strict Fetch Metadata did not contain exact `same-origin`.
    FetchSiteRequired,
    /// Fetch Metadata is duplicated or malformed.
    FetchSiteAmbiguous,
    /// The exact custom marker is absent.
    RequiredHeaderMissing,
    /// The exact custom marker is duplicated or non-UTF8.
    RequiredHeaderAmbiguous,
    /// The custom marker bytes do not match configuration.
    RequiredHeaderMismatch,
    /// Content-Type is absent.
    JsonContentTypeMissing,
    /// Content-Type is duplicated, or is not accepted JSON and contains
    /// non-UTF8 bytes.
    JsonContentTypeAmbiguous,
    /// Content-Type is not exact `application/json` with optional parameters.
    JsonContentTypeRequired,
}

impl MutationRejection {
    /// Return the default transport status for application rendering.
    pub const fn status(self) -> StatusCode {
        match self {
            Self::JsonContentTypeMissing
            | Self::JsonContentTypeAmbiguous
            | Self::JsonContentTypeRequired => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            _ => StatusCode::FORBIDDEN,
        }
    }

    /// Return a stable sanitized classification without a wire envelope.
    pub const fn code(self) -> &'static str {
        match self {
            Self::OriginMissing => "origin_missing",
            Self::OriginAmbiguous => "origin_ambiguous",
            Self::OriginMismatch => "origin_mismatch",
            Self::FetchSiteCrossSite => "fetch_site_cross_site",
            Self::FetchSiteRequired => "fetch_site_required",
            Self::FetchSiteAmbiguous => "fetch_site_ambiguous",
            Self::RequiredHeaderMissing => "required_header_missing",
            Self::RequiredHeaderAmbiguous => "required_header_ambiguous",
            Self::RequiredHeaderMismatch => "required_header_mismatch",
            Self::JsonContentTypeMissing => "json_content_type_missing",
            Self::JsonContentTypeAmbiguous => "json_content_type_ambiguous",
            Self::JsonContentTypeRequired => "json_content_type_required",
        }
    }
}

impl fmt::Display for MutationRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for MutationRejection {}

enum HeaderOccurrence<'a> {
    Missing,
    One(&'a HeaderValue),
    Multiple,
}

fn occurrence<'a>(headers: &'a HeaderMap, name: &HeaderName) -> HeaderOccurrence<'a> {
    let mut values = headers.get_all(name).iter();
    match (values.next(), values.next()) {
        (None, _) => HeaderOccurrence::Missing,
        (Some(value), None) => HeaderOccurrence::One(value),
        (Some(_), Some(_)) => HeaderOccurrence::Multiple,
    }
}

fn check_origin(headers: &HeaderMap, origin: &BrowserOrigin) -> Result<(), MutationRejection> {
    match occurrence(headers, &header::ORIGIN) {
        HeaderOccurrence::Missing => Err(MutationRejection::OriginMissing),
        HeaderOccurrence::Multiple => Err(MutationRejection::OriginAmbiguous),
        HeaderOccurrence::One(value) => match std::str::from_utf8(value.as_bytes()) {
            Err(_) => Err(MutationRejection::OriginAmbiguous),
            Ok(value) if value == origin.as_str() => Ok(()),
            Ok(_) => Err(MutationRejection::OriginMismatch),
        },
    }
}

fn check_fetch_site(headers: &HeaderMap, policy: FetchSitePolicy) -> Result<(), MutationRejection> {
    let value = match occurrence(headers, &SEC_FETCH_SITE) {
        HeaderOccurrence::Missing if policy == FetchSitePolicy::RejectCrossSite => return Ok(()),
        HeaderOccurrence::Missing => return Err(MutationRejection::FetchSiteRequired),
        HeaderOccurrence::Multiple => return Err(MutationRejection::FetchSiteAmbiguous),
        HeaderOccurrence::One(value) => std::str::from_utf8(value.as_bytes())
            .map_err(|_| MutationRejection::FetchSiteAmbiguous)?,
    };
    if !is_structured_token(value) {
        return Err(MutationRejection::FetchSiteAmbiguous);
    }
    if value == "cross-site" {
        return Err(MutationRejection::FetchSiteCrossSite);
    }
    if policy == FetchSitePolicy::RequireSameOrigin && value != "same-origin" {
        return Err(MutationRejection::FetchSiteRequired);
    }
    Ok(())
}

fn check_required_header(
    headers: &HeaderMap,
    required: &RequiredHeader,
) -> Result<(), MutationRejection> {
    match occurrence(headers, &required.name) {
        HeaderOccurrence::Missing => Err(MutationRejection::RequiredHeaderMissing),
        HeaderOccurrence::Multiple => Err(MutationRejection::RequiredHeaderAmbiguous),
        HeaderOccurrence::One(value) if std::str::from_utf8(value.as_bytes()).is_err() => {
            Err(MutationRejection::RequiredHeaderAmbiguous)
        }
        HeaderOccurrence::One(value) if value == required.value => Ok(()),
        HeaderOccurrence::One(_) => Err(MutationRejection::RequiredHeaderMismatch),
    }
}

fn check_json(headers: &HeaderMap) -> Result<(), MutationRejection> {
    let value = match occurrence(headers, &header::CONTENT_TYPE) {
        HeaderOccurrence::Missing => return Err(MutationRejection::JsonContentTypeMissing),
        HeaderOccurrence::Multiple => return Err(MutationRejection::JsonContentTypeAmbiguous),
        HeaderOccurrence::One(value) => value.as_bytes(),
    };
    if is_json_media_type(value) {
        Ok(())
    } else if std::str::from_utf8(value).is_err() {
        Err(MutationRejection::JsonContentTypeAmbiguous)
    } else {
        Err(MutationRejection::JsonContentTypeRequired)
    }
}

fn is_json_media_type(bytes: &[u8]) -> bool {
    let mut cursor = 0;
    skip_ows(bytes, &mut cursor);

    let type_start = cursor;
    consume_token(bytes, &mut cursor);
    let media_type = &bytes[type_start..cursor];
    if bytes.get(cursor) != Some(&b'/') {
        return false;
    }
    cursor += 1;
    let subtype_start = cursor;
    consume_token(bytes, &mut cursor);
    let subtype = &bytes[subtype_start..cursor];
    if !media_type.eq_ignore_ascii_case(b"application") || !subtype.eq_ignore_ascii_case(b"json") {
        return false;
    }

    loop {
        skip_ows(bytes, &mut cursor);
        if cursor == bytes.len() {
            return true;
        }
        if bytes[cursor] != b';' {
            return false;
        }
        cursor += 1;
        skip_ows(bytes, &mut cursor);

        if cursor == bytes.len() || bytes[cursor] == b';' {
            continue;
        }

        let name_start = cursor;
        consume_token(bytes, &mut cursor);
        if cursor == name_start {
            return false;
        }
        if bytes.get(cursor) != Some(&b'=') {
            return false;
        }
        cursor += 1;
        if !consume_parameter_value(bytes, &mut cursor) {
            return false;
        }
    }
}

fn consume_parameter_value(bytes: &[u8], cursor: &mut usize) -> bool {
    if bytes.get(*cursor) != Some(&b'"') {
        let start = *cursor;
        consume_token(bytes, cursor);
        return *cursor != start;
    }

    *cursor += 1;
    while let Some(&byte) = bytes.get(*cursor) {
        match byte {
            b'"' => {
                *cursor += 1;
                return true;
            }
            b'\\' => {
                *cursor += 1;
                let Some(&escaped) = bytes.get(*cursor) else {
                    return false;
                };
                if escaped != b'\t' && !(b' '..=b'~').contains(&escaped) && escaped < 0x80 {
                    return false;
                }
                *cursor += 1;
            }
            b'\t' | b' ' | b'!' | b'#'..=b'[' | b']'..=b'~' => *cursor += 1,
            0x80..=0xff => *cursor += 1,
            _ => return false,
        }
    }
    false
}

fn skip_ows(bytes: &[u8], cursor: &mut usize) {
    while matches!(bytes.get(*cursor), Some(b' ' | b'\t')) {
        *cursor += 1;
    }
}

fn consume_token(bytes: &[u8], cursor: &mut usize) {
    while bytes.get(*cursor).is_some_and(|byte| is_token_byte(*byte)) {
        *cursor += 1;
    }
}

fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

fn is_cors_safelisted_name(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "accept" | "accept-language" | "content-language" | "content-type" | "range"
    )
}

fn is_forbidden_request_name(name: &HeaderName) -> bool {
    let name = name.as_str();
    name.starts_with("proxy-")
        || name.starts_with("sec-")
        || matches!(
            name,
            "accept-charset"
                | "accept-encoding"
                | "access-control-request-headers"
                | "access-control-request-method"
                | "connection"
                | "content-length"
                | "cookie"
                | "cookie2"
                | "date"
                | "dnt"
                | "expect"
                | "host"
                | "keep-alive"
                | "origin"
                | "referer"
                | "set-cookie"
                | "te"
                | "trailer"
                | "transfer-encoding"
                | "upgrade"
                | "via"
                | "x-http-method"
                | "x-http-method-override"
                | "x-method-override"
        )
}

fn is_structured_token(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == b'*')
        && bytes.all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                        | b':'
                        | b'/'
                )
        })
}
