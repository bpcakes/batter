use super::BrowserOrigin;
use axum::http::{HeaderMap, HeaderValue, header};
use cookie::{Cookie as NativeCookie, CookieBuilder, SameSite as NativeSameSite};
use std::{error::Error, fmt};

/// A validated, case-sensitive cookie name.
///
/// ```
/// use batter_axum::browser::CookieName;
///
/// let name = CookieName::new("__Host-session")?;
/// assert_eq!(name.as_str(), "__Host-session");
/// # Ok::<(), batter_axum::browser::CookieNameError>(())
/// ```
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CookieName(Box<str>);

impl CookieName {
    /// Validate the HTTP token subset used for cookie names.
    pub fn new(value: impl Into<Box<str>>) -> Result<Self, CookieNameError> {
        let value = value.into();
        if !is_cookie_name(&value) {
            return Err(CookieNameError);
        }
        Ok(Self(value))
    }

    /// Return the original case-sensitive name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Sanitized invalid-cookie-name error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CookieNameError;

impl fmt::Display for CookieNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid cookie name")
    }
}

impl Error for CookieNameError {}

/// A borrowed, validated, unquoted opaque cookie value.
///
/// Values are neither decoded nor normalized. Debug output is always redacted.
///
/// ```
/// use batter_axum::browser::CookieValue;
///
/// let value = CookieValue::new("opaque-token==")?;
/// assert_eq!(value.as_str(), "opaque-token==");
/// assert!(!format!("{value:?}").contains("opaque-token"));
/// # Ok::<(), batter_axum::browser::CookieValueError>(())
/// ```
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct CookieValue<'a>(&'a str);

impl<'a> CookieValue<'a> {
    /// Validate the unquoted `cookie-octet` subset without allocating.
    pub fn new(value: &'a str) -> Result<Self, CookieValueError> {
        if !is_cookie_value(value) {
            return Err(CookieValueError);
        }
        Ok(Self(value))
    }

    /// Return the unchanged opaque value.
    pub const fn as_str(self) -> &'a str {
        self.0
    }
}

impl fmt::Debug for CookieValue<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CookieValue([REDACTED])")
    }
}

/// Sanitized invalid-cookie-value error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CookieValueError;

impl fmt::Display for CookieValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid opaque cookie value")
    }
}

impl Error for CookieValueError {}

/// Handling of malformed cookie pairs unrelated to the requested target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CookieHeaderPolicy {
    /// Reject any non-empty pair outside Batter's unquoted token/value subset.
    ///
    /// This is suitable only when the application controls the whole cookie
    /// jar. It rejects some RFC-valid input, including quoted cookie values;
    /// use [`Self::TargetOnly`] when unrelated writers share the host.
    Strict,
    /// Ignore malformed unrelated pairs while keeping target checks strict.
    TargetOnly,
}

/// Read one exact named cookie across every Cookie field.
///
/// Duplicate target cookies, non-UTF8 fields, and malformed target values are
/// always rejected. `TargetOnly` relaxes only unrelated malformed pairs. An
/// empty target value is returned as present and unchanged; the application
/// credential parser decides whether that value is meaningful.
///
/// ```
/// use axum::http::{HeaderMap, HeaderValue, header};
/// use batter_axum::browser::{CookieHeaderPolicy, CookieName, read_cookie};
///
/// let mut headers = HeaderMap::new();
/// headers.append(header::COOKIE, HeaderValue::from_static("other=x"));
/// headers.append(header::COOKIE, HeaderValue::from_static("session=opaque"));
/// let name = CookieName::new("session")?;
/// let value = read_cookie(&headers, &name, CookieHeaderPolicy::TargetOnly)?;
/// assert_eq!(value.unwrap().as_str(), "opaque");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn read_cookie<'a>(
    headers: &'a HeaderMap,
    name: &CookieName,
    policy: CookieHeaderPolicy,
) -> Result<Option<CookieValue<'a>>, CookieHeaderError> {
    let mut found = None;
    for field in headers.get_all(header::COOKIE) {
        let field =
            std::str::from_utf8(field.as_bytes()).map_err(|_| CookieHeaderError::NonUtf8)?;
        for fragment in field.split(';') {
            let fragment = trim_ows(fragment);
            if fragment.is_empty() {
                continue;
            }
            let Some((pair_name, pair_value)) = fragment.split_once('=') else {
                if policy == CookieHeaderPolicy::Strict || trim_ows(fragment) == name.as_str() {
                    return Err(CookieHeaderError::MalformedPair);
                }
                continue;
            };
            let pair_name = trim_ows(pair_name);
            let pair_value = trim_ows(pair_value);
            if pair_name == name.as_str() {
                let value = CookieValue::new(pair_value)
                    .map_err(|_| CookieHeaderError::InvalidTargetValue)?;
                if found.replace(value).is_some() {
                    return Err(CookieHeaderError::DuplicateTarget);
                }
            } else if policy == CookieHeaderPolicy::Strict
                && (!is_cookie_name(pair_name) || !is_cookie_value(pair_value))
            {
                return Err(CookieHeaderError::MalformedPair);
            }
        }
    }
    Ok(found)
}

/// Sanitized Cookie-field failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CookieHeaderError {
    /// A Cookie field was not UTF-8 and could conceal another target.
    NonUtf8,
    /// A required pair, or any pair under strict policy, was malformed.
    MalformedPair,
    /// The exact target appeared more than once.
    DuplicateTarget,
    /// The target value was outside the supported opaque syntax.
    InvalidTargetValue,
}

impl fmt::Display for CookieHeaderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonUtf8 => "cookie header is not UTF-8",
            Self::MalformedPair => "cookie header contains a malformed pair",
            Self::DuplicateTarget => "target cookie is duplicated",
            Self::InvalidTargetValue => "target cookie value is invalid",
        })
    }
}

impl Error for CookieHeaderError {}

/// Explicit SameSite attribute for an emitted browser cookie.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SameSite {
    /// Send only in same-site contexts permitted by Strict semantics.
    Strict,
    /// Permit the navigation cases defined by Lax semantics.
    Lax,
    /// Permit cross-site contexts; valid only with HTTPS/Secure.
    None,
}

/// Whether browser script may read the emitted cookie.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CookieVisibility {
    /// Emit the HttpOnly attribute.
    HttpOnly,
    /// Deliberately omit HttpOnly for an application-owned protocol.
    ScriptReadable,
}

/// Browser-side lifetime metadata for an emitted cookie.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CookieLifetime {
    /// Omit Max-Age and Expires.
    Session,
    /// Emit a positive whole-second Max-Age.
    Persistent(CookieMaxAge),
}

/// A positive Max-Age representable by the cookie serializer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CookieMaxAge(u64);

impl CookieMaxAge {
    /// Validate a positive whole-second lifetime.
    pub fn seconds(value: u64) -> Result<Self, CookiePolicyError> {
        if value == 0 || value > i64::MAX as u64 {
            return Err(CookiePolicyError::InvalidMaxAge);
        }
        Ok(Self(value))
    }

    /// Return the validated whole-second value.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A fixed host-only, root-path browser cookie policy.
///
/// HTTPS origins require a case-sensitive `__Host-` name. Explicit loopback
/// HTTP origins reject browser-reserved secure prefixes and SameSite None.
///
/// ```
/// use batter_axum::browser::{
///     BrowserCookie, BrowserOrigin, CookieName, CookieVisibility, SameSite,
/// };
///
/// let origin = BrowserOrigin::https("https://app.example")?;
/// let cookie = BrowserCookie::new(
///     &origin,
///     CookieName::new("__Host-session")?,
///     SameSite::Lax,
///     CookieVisibility::HttpOnly,
/// )?;
/// assert!(cookie.is_secure());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrowserCookie {
    name: CookieName,
    same_site: SameSite,
    visibility: CookieVisibility,
    secure: bool,
}

impl BrowserCookie {
    /// Validate the name, origin mode, SameSite, and Secure relationship.
    pub fn new(
        origin: &BrowserOrigin,
        name: CookieName,
        same_site: SameSite,
        visibility: CookieVisibility,
    ) -> Result<Self, CookiePolicyError> {
        let secure = origin.is_secure();
        let name_str = name.as_str();
        let host_prefix = name_str.starts_with("__Host-");
        if secure && !host_prefix {
            return Err(CookiePolicyError::SecureNameRequired);
        }
        if !secure && has_reserved_secure_prefix(name_str) {
            return Err(CookiePolicyError::SecureNameNotAllowed);
        }
        if secure
            && starts_with_ignore_ascii_case(name_str, "__Host-Http-")
            && visibility != CookieVisibility::HttpOnly
        {
            return Err(CookiePolicyError::HttpOnlyRequired);
        }
        if !secure && same_site == SameSite::None {
            return Err(CookiePolicyError::SameSiteNoneRequiresSecure);
        }
        Ok(Self {
            name,
            same_site,
            visibility,
            secure,
        })
    }

    /// Return the exact name used for both reading and response construction.
    pub const fn name(&self) -> &CookieName {
        &self.name
    }

    /// Return the configured SameSite behavior.
    pub const fn same_site(&self) -> SameSite {
        self.same_site
    }

    /// Return whether HttpOnly is emitted.
    pub const fn visibility(&self) -> CookieVisibility {
        self.visibility
    }

    /// Return whether Secure is emitted.
    pub const fn is_secure(&self) -> bool {
        self.secure
    }

    /// Append a Set-Cookie field without replacing existing response cookies.
    ///
    /// This returns [`SetCookieError::DuplicateName`] without changing
    /// `headers` when a Set-Cookie field already uses the same case-sensitive
    /// cookie name. Independent cookie names remain appendable.
    ///
    /// ```
    /// use axum::http::{HeaderMap, HeaderValue, header};
    /// use batter_axum::browser::{
    ///     BrowserCookie, BrowserOrigin, CookieLifetime, CookieMaxAge, CookieName,
    ///     CookieValue, CookieVisibility, SameSite, SetCookieError,
    /// };
    ///
    /// let origin = BrowserOrigin::https("https://app.example")?;
    /// let cookie = BrowserCookie::new(
    ///     &origin,
    ///     CookieName::new("__Host-session")?,
    ///     SameSite::Lax,
    ///     CookieVisibility::HttpOnly,
    /// )?;
    /// let mut headers = HeaderMap::new();
    /// headers.append(header::SET_COOKIE, HeaderValue::from_static("other=1"));
    /// cookie.append(
    ///     &mut headers,
    ///     CookieValue::new("opaque")?,
    ///     CookieLifetime::Persistent(CookieMaxAge::seconds(3_600)?),
    /// )?;
    /// assert_eq!(headers.get_all(header::SET_COOKIE).iter().count(), 2);
    /// assert_eq!(
    ///     cookie.append(
    ///         &mut headers,
    ///         CookieValue::new("replacement")?,
    ///         CookieLifetime::Session,
    ///     ),
    ///     Err(SetCookieError::DuplicateName),
    /// );
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn append(
        &self,
        headers: &mut HeaderMap,
        value: CookieValue<'_>,
        lifetime: CookieLifetime,
    ) -> Result<(), SetCookieError> {
        let mut builder = self.builder(value.as_str());
        if let CookieLifetime::Persistent(max_age) = lifetime {
            builder = builder.max_age(cookie::time::Duration::seconds(max_age.0 as i64));
        }
        append_serialized(headers, builder.build())
    }

    /// Append a matching removal field with zero Max-Age and a past Expires.
    ///
    /// Clearing a browser cookie does not revoke application server state. As
    /// with [`Self::append`], an existing same-name Set-Cookie field is rejected
    /// without changing `headers`.
    ///
    /// ```
    /// use axum::http::{HeaderMap, header};
    /// use batter_axum::browser::{
    ///     BrowserCookie, BrowserOrigin, CookieName, CookieVisibility, SameSite,
    /// };
    ///
    /// let origin = BrowserOrigin::loopback_http("http://localhost:3000")?;
    /// let cookie = BrowserCookie::new(
    ///     &origin,
    ///     CookieName::new("local_session")?,
    ///     SameSite::Lax,
    ///     CookieVisibility::HttpOnly,
    /// )?;
    /// let mut headers = HeaderMap::new();
    /// cookie.append_removal(&mut headers)?;
    /// assert!(headers[header::SET_COOKIE].to_str()?.contains("Max-Age=0"));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn append_removal(&self, headers: &mut HeaderMap) -> Result<(), SetCookieError> {
        let cookie = self
            .builder("")
            .max_age(cookie::time::Duration::ZERO)
            .expires(cookie::time::OffsetDateTime::UNIX_EPOCH)
            .build();
        append_serialized(headers, cookie)
    }

    fn builder<'a>(&'a self, value: &'a str) -> CookieBuilder<'a> {
        let mut builder = NativeCookie::build((self.name.as_str(), value))
            .path("/")
            .same_site(match self.same_site {
                SameSite::Strict => NativeSameSite::Strict,
                SameSite::Lax => NativeSameSite::Lax,
                SameSite::None => NativeSameSite::None,
            });
        if self.secure {
            builder = builder.secure(true);
        }
        if self.visibility == CookieVisibility::HttpOnly {
            builder = builder.http_only(true);
        }
        builder
    }
}

/// Invalid relationship between cookie configuration values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CookiePolicyError {
    /// HTTPS cookies require an exact `__Host-` prefix.
    SecureNameRequired,
    /// Loopback HTTP cannot establish a browser-reserved secure-prefix cookie.
    SecureNameNotAllowed,
    /// A browser-reserved HTTP-only prefix requires HttpOnly visibility.
    HttpOnlyRequired,
    /// SameSite None requires HTTPS/Secure.
    SameSiteNoneRequiresSecure,
    /// Max-Age was zero or outside the serializer's whole-second range.
    InvalidMaxAge,
}

impl fmt::Display for CookiePolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::SecureNameRequired => "secure browser cookie requires a __Host- name",
            Self::SecureNameNotAllowed => {
                "loopback browser cookie cannot use a reserved secure prefix"
            }
            Self::HttpOnlyRequired => "browser cookie prefix requires HttpOnly visibility",
            Self::SameSiteNoneRequiresSecure => "SameSite None requires a secure browser origin",
            Self::InvalidMaxAge => "cookie Max-Age is outside the supported positive range",
        })
    }
}

impl Error for CookiePolicyError {}

fn has_reserved_secure_prefix(name: &str) -> bool {
    ["__Secure-", "__Host-", "__Http-"]
        .into_iter()
        .any(|prefix| starts_with_ignore_ascii_case(name, prefix))
}

fn starts_with_ignore_ascii_case(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix))
}

/// Sanitized failure to serialize a validated Set-Cookie field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SetCookieError {
    /// The response already contains a Set-Cookie field with this cookie name.
    DuplicateName,
    /// The native serializer unexpectedly produced an invalid header value.
    InvalidHeaderValue,
}

impl fmt::Display for SetCookieError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::DuplicateName => "response already contains the browser cookie name",
            Self::InvalidHeaderValue => "could not serialize validated browser cookie",
        })
    }
}

impl Error for SetCookieError {}

fn append_serialized(
    headers: &mut HeaderMap,
    cookie: NativeCookie<'_>,
) -> Result<(), SetCookieError> {
    if has_set_cookie_name(headers, cookie.name().as_bytes()) {
        return Err(SetCookieError::DuplicateName);
    }
    let value = HeaderValue::from_str(&cookie.to_string())
        .map_err(|_| SetCookieError::InvalidHeaderValue)?;
    headers.append(header::SET_COOKIE, value);
    Ok(())
}

fn has_set_cookie_name(headers: &HeaderMap, expected: &[u8]) -> bool {
    headers.get_all(header::SET_COOKIE).iter().any(|value| {
        let field = trim_ows_bytes(value.as_bytes());
        field
            .iter()
            .position(|byte| *byte == b'=')
            .is_some_and(|equals| trim_ows_bytes(&field[..equals]) == expected)
    })
}

fn trim_ows_bytes(mut value: &[u8]) -> &[u8] {
    while value
        .first()
        .is_some_and(|byte| matches!(*byte, b' ' | b'\t'))
    {
        value = &value[1..];
    }
    while value
        .last()
        .is_some_and(|byte| matches!(*byte, b' ' | b'\t'))
    {
        value = &value[..value.len() - 1];
    }
    value
}

fn trim_ows(value: &str) -> &str {
    value.trim_matches([' ', '\t'])
}

fn is_cookie_name(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            (0x21..=0x7e).contains(&byte)
                && !matches!(
                    byte,
                    b'(' | b')'
                        | b'<'
                        | b'>'
                        | b'@'
                        | b','
                        | b';'
                        | b':'
                        | b'\\'
                        | b'"'
                        | b'/'
                        | b'['
                        | b']'
                        | b'?'
                        | b'='
                        | b'{'
                        | b'}'
                )
        })
}

fn is_cookie_value(value: &str) -> bool {
    value.bytes().all(|byte| {
        byte == b'!'
            || (b'#'..=b'+').contains(&byte)
            || (b'-'..=b':').contains(&byte)
            || (b'<'..=b'[').contains(&byte)
            || (b']'..=b'~').contains(&byte)
    })
}
