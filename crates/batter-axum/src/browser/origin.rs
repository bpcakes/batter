use std::{error::Error, fmt, net::IpAddr};
use url::{Host, Url};

/// A validated, canonical browser origin selected by trusted configuration.
///
/// Request headers never construct this value. Use [`Self::https`] in deployed
/// environments and the visibly weaker [`Self::loopback_http`] only for local
/// development.
///
/// ```
/// use batter_axum::browser::BrowserOrigin;
///
/// let origin = BrowserOrigin::https("https://app.example:443/")?;
/// assert_eq!(origin.as_str(), "https://app.example");
/// assert!(origin.is_secure());
/// # Ok::<(), batter_axum::browser::BrowserOriginError>(())
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrowserOrigin {
    serialized: Box<str>,
    secure: bool,
}

impl BrowserOrigin {
    /// Parse an explicit `https://authority` URL with no credentials,
    /// application path, query, fragment, or reverse solidus.
    pub fn https(value: &str) -> Result<Self, BrowserOriginError> {
        Self::parse(value, OriginKind::Https)
    }

    /// Parse an explicit `http://authority` origin restricted to loopback IPs
    /// or `localhost` names.
    ///
    /// ```
    /// use batter_axum::browser::BrowserOrigin;
    ///
    /// let origin = BrowserOrigin::loopback_http("http://api.localhost:3000")?;
    /// assert_eq!(origin.as_str(), "http://api.localhost:3000");
    /// assert!(!origin.is_secure());
    /// # Ok::<(), batter_axum::browser::BrowserOriginError>(())
    /// ```
    pub fn loopback_http(value: &str) -> Result<Self, BrowserOriginError> {
        Self::parse(value, OriginKind::LoopbackHttp)
    }

    /// Return the canonical ASCII serialization used for exact Origin checks.
    pub fn as_str(&self) -> &str {
        &self.serialized
    }

    /// Return whether this origin uses HTTPS.
    pub const fn is_secure(&self) -> bool {
        self.secure
    }

    fn parse(value: &str, kind: OriginKind) -> Result<Self, BrowserOriginError> {
        if value
            .bytes()
            .any(|byte| byte <= b' ' || byte == b'\\' || byte == 0x7f)
        {
            return Err(BrowserOriginError::InvalidUrl);
        }
        let expected_scheme = match kind {
            OriginKind::Https => "https",
            OriginKind::LoopbackHttp => "http",
        };
        let raw_authority = raw_authority(value, expected_scheme)?;
        if raw_authority
            .split(['/', '?', '#'])
            .next()
            .is_none_or(str::is_empty)
        {
            return Err(BrowserOriginError::MissingHost);
        }
        let url = Url::parse(value).map_err(|_| BrowserOriginError::InvalidUrl)?;
        if url.scheme() != expected_scheme {
            return Err(BrowserOriginError::UnsupportedScheme);
        }
        if url.host().is_none() {
            return Err(BrowserOriginError::MissingHost);
        }
        if has_userinfo(raw_authority) || !url.username().is_empty() || url.password().is_some() {
            return Err(BrowserOriginError::CredentialsNotAllowed);
        }
        if has_non_root_path(raw_authority) || url.path() != "/" {
            return Err(BrowserOriginError::PathNotAllowed);
        }
        if url.query().is_some() {
            return Err(BrowserOriginError::QueryNotAllowed);
        }
        if url.fragment().is_some() {
            return Err(BrowserOriginError::FragmentNotAllowed);
        }
        if kind == OriginKind::LoopbackHttp && !is_loopback_host(&url) {
            return Err(BrowserOriginError::InsecureNonLoopback);
        }
        Ok(Self {
            serialized: url.origin().ascii_serialization().into_boxed_str(),
            secure: kind == OriginKind::Https,
        })
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum OriginKind {
    Https,
    LoopbackHttp,
}

fn raw_authority<'a>(value: &'a str, expected_scheme: &str) -> Result<&'a str, BrowserOriginError> {
    let (scheme, remainder) = value
        .split_once(':')
        .ok_or(BrowserOriginError::InvalidUrl)?;
    if !scheme.eq_ignore_ascii_case(expected_scheme) {
        return Err(BrowserOriginError::UnsupportedScheme);
    }
    remainder
        .strip_prefix("//")
        .ok_or(BrowserOriginError::InvalidUrl)
}

fn has_userinfo(raw_authority: &str) -> bool {
    raw_authority
        .split(['/', '?', '#'])
        .next()
        .is_some_and(|authority| authority.contains('@'))
}

fn has_non_root_path(raw_authority: &str) -> bool {
    let suffix_start = raw_authority
        .find(['?', '#'])
        .unwrap_or(raw_authority.len());
    raw_authority[..suffix_start]
        .find('/')
        .is_some_and(|path_start| &raw_authority[path_start..suffix_start] != "/")
}

fn is_loopback_host(url: &Url) -> bool {
    match url.host() {
        Some(Host::Ipv4(address)) => IpAddr::V4(address).is_loopback(),
        Some(Host::Ipv6(address)) => IpAddr::V6(address).is_loopback(),
        Some(Host::Domain(domain)) => domain == "localhost" || domain.ends_with(".localhost"),
        None => false,
    }
}

/// Sanitized failure to construct a trusted browser origin.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BrowserOriginError {
    /// The input is not an absolute URL accepted by the URL parser.
    InvalidUrl,
    /// The URL has no usable authority host.
    MissingHost,
    /// The constructor and URL scheme do not match.
    UnsupportedScheme,
    /// Plain HTTP named a host outside the loopback development boundary.
    InsecureNonLoopback,
    /// User information appeared in the URL authority.
    CredentialsNotAllowed,
    /// The URL contains an application path instead of only `/`.
    PathNotAllowed,
    /// The URL contains a query.
    QueryNotAllowed,
    /// The URL contains a fragment.
    FragmentNotAllowed,
}

impl fmt::Display for BrowserOriginError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidUrl => "invalid browser origin URL",
            Self::MissingHost => "browser origin host is missing",
            Self::UnsupportedScheme => "browser origin scheme is unsupported",
            Self::InsecureNonLoopback => "insecure browser origin is not loopback",
            Self::CredentialsNotAllowed => "browser origin credentials are not allowed",
            Self::PathNotAllowed => "browser origin path is not allowed",
            Self::QueryNotAllowed => "browser origin query is not allowed",
            Self::FragmentNotAllowed => "browser origin fragment is not allowed",
        })
    }
}

impl Error for BrowserOriginError {}
