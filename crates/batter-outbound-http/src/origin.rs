//! Validated provider origins.

use std::{fmt, net::SocketAddr};
use url::Url;

/// One provider's scheme, host and port.
///
/// Requests built from a [`crate::Client`] can only target paths below its
/// origin. HTTPS is the only network scheme. Plain HTTP is accepted only for a
/// literal loopback socket address through [`Origin::loopback`], which serves
/// local fixtures and tests and cannot leave the host.
///
/// `Debug` shows the scheme kind only, never the host.
#[derive(Clone, PartialEq, Eq)]
pub struct Origin {
    url: Url,
    scheme: Scheme,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scheme {
    Https,
    LoopbackHttp,
}

impl Origin {
    /// Accept `https://host` or `https://host:port`, with an optional trailing
    /// slash and nothing else.
    ///
    /// ```
    /// use batter_outbound_http::{Origin, OriginError};
    ///
    /// assert!(Origin::https("https://provider.example").is_ok());
    /// assert_eq!(Origin::https("http://provider.example"), Err(OriginError::NotHttps));
    /// assert_eq!(Origin::https("https://provider.example/v1"), Err(OriginError::Path));
    /// assert_eq!(Origin::https("https://user:secret@provider.example"), Err(OriginError::Userinfo));
    /// ```
    pub fn https(origin: &str) -> Result<Self, OriginError> {
        let url = Url::parse(origin).map_err(|_| OriginError::Unparseable)?;
        if url.scheme() != "https" {
            return Err(OriginError::NotHttps);
        }
        validate_components(&url)?;
        Ok(Self {
            url,
            scheme: Scheme::Https,
        })
    }

    /// Accept plain HTTP to a literal loopback address, such as a local fixture.
    ///
    /// ```
    /// use batter_outbound_http::{Origin, OriginError};
    /// use std::net::SocketAddr;
    ///
    /// let local: SocketAddr = "127.0.0.1:8080".parse().unwrap();
    /// assert!(Origin::loopback(local).is_ok());
    /// let remote: SocketAddr = "192.0.2.10:8080".parse().unwrap();
    /// assert_eq!(Origin::loopback(remote), Err(OriginError::NotLoopback));
    /// ```
    pub fn loopback(address: SocketAddr) -> Result<Self, OriginError> {
        if !address.ip().is_loopback() {
            return Err(OriginError::NotLoopback);
        }
        if address.port() == 0 {
            return Err(OriginError::Port);
        }
        let url =
            Url::parse(&format!("http://{address}/")).map_err(|_| OriginError::Unparseable)?;
        validate_components(&url)?;
        Ok(Self {
            url,
            scheme: Scheme::LoopbackHttp,
        })
    }

    pub(crate) fn url(&self) -> &Url {
        &self.url
    }

    pub(crate) fn scheme(&self) -> Scheme {
        self.scheme
    }
}

fn validate_components(url: &Url) -> Result<(), OriginError> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err(OriginError::Userinfo);
    }
    if url.host().is_none() || url.cannot_be_a_base() {
        return Err(OriginError::MissingHost);
    }
    if url.port() == Some(0) {
        return Err(OriginError::Port);
    }
    if url.path() != "/" {
        return Err(OriginError::Path);
    }
    if url.query().is_some() {
        return Err(OriginError::Query);
    }
    if url.fragment().is_some() {
        return Err(OriginError::Fragment);
    }
    // Refuse hosts that cannot form an HTTP request authority.
    if http::Uri::try_from(url.as_str()).is_err() {
        return Err(OriginError::Unparseable);
    }
    Ok(())
}

impl fmt::Debug for Origin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.scheme {
            Scheme::Https => formatter.write_str("Origin(https)"),
            Scheme::LoopbackHttp => formatter.write_str("Origin(loopback http)"),
        }
    }
}

/// Why an origin was refused. The message never includes the input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum OriginError {
    /// The value is not an absolute URL that an HTTP request can target.
    #[error("the origin is not an absolute HTTP URL")]
    Unparseable,
    /// [`Origin::https`] accepts only the `https` scheme.
    #[error("the origin must use https")]
    NotHttps,
    /// [`Origin::loopback`] accepts only loopback addresses.
    #[error("the origin address is not a loopback address")]
    NotLoopback,
    /// Credentials in the URL would be sent and logged with every request.
    #[error("the origin must not contain credentials")]
    Userinfo,
    /// The URL has no host.
    #[error("the origin has no host")]
    MissingHost,
    /// Port zero cannot be dialed.
    #[error("the origin port must not be zero")]
    Port,
    /// Paths belong to each request, not to the origin.
    #[error("the origin must not contain a path")]
    Path,
    /// A query in the origin would be sent with every request.
    #[error("the origin must not contain a query")]
    Query,
    /// Fragments are never sent and usually indicate a mistake.
    #[error("the origin must not contain a fragment")]
    Fragment,
}
