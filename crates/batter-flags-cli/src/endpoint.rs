//! A native parser for the captured URL and the transport policy it must
//! meet: a host off the loopback interface requires `verify-full`.

use std::{fmt, net::IpAddr, path::PathBuf};

use sqlx::postgres::{PgConnectOptions, PgSslMode};

/// Where the command connects. Its formatting never shows the password.
pub(crate) struct Endpoint {
    host: String,
    port: u16,
    user: String,
    password: Option<String>,
    database: String,
    tls: Tls,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Tls {
    Disable,
    Require,
    VerifyFull { root: Option<PathBuf> },
}

/// Why the URL was refused. It never echoes the URL.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EndpointError {
    Malformed,
    /// Only `sslmode=disable`, `require` or `verify-full`, and `sslrootcert`
    /// with an absolute path, are accepted.
    UnsupportedParameter,
    /// A host off the loopback interface requires `sslmode=verify-full`.
    Unverified,
}

impl fmt::Display for EndpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Malformed => "the database URL is malformed",
            Self::UnsupportedParameter => {
                "the database URL takes only sslmode (disable, require or verify-full) and an absolute sslrootcert"
            }
            Self::Unverified => "a database host off the loopback interface requires sslmode=verify-full",
        })
    }
}

impl fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Endpoint([redacted])")
    }
}

impl Endpoint {
    /// Parses `postgres://[user[:password]@]host[:port]/database[?query]`.
    pub(crate) fn parse(url: &str) -> Result<Self, EndpointError> {
        let rest = url
            .strip_prefix("postgres://")
            .or_else(|| url.strip_prefix("postgresql://"))
            .ok_or(EndpointError::Malformed)?;
        let (authority, rest) = rest.split_once('/').ok_or(EndpointError::Malformed)?;
        let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
        let (userinfo, hostport) = match authority.rsplit_once('@') {
            Some((userinfo, hostport)) => (Some(userinfo), hostport),
            None => (None, authority),
        };
        let (user, password) = match userinfo {
            None => (String::new(), None),
            Some(info) => match info.split_once(':') {
                Some((user, password)) => (decode(user)?, Some(decode(password)?)),
                None => (decode(info)?, None),
            },
        };
        let (host, port) = split_host(hostport)?;
        let database = decode(path)?;
        if host.is_empty() || database.is_empty() || user.is_empty() {
            return Err(EndpointError::Malformed);
        }
        let mut mode = None;
        let mut root = None;
        for parameter in query.split('&').filter(|p| !p.is_empty()) {
            match parameter.split_once('=') {
                Some(("sslmode", value)) => mode = Some(value.to_owned()),
                Some(("sslrootcert", value)) => {
                    let path = PathBuf::from(decode(value)?);
                    if !path.is_absolute() {
                        return Err(EndpointError::UnsupportedParameter);
                    }
                    root = Some(path);
                }
                _ => return Err(EndpointError::UnsupportedParameter),
            }
        }
        let tls = match (mode.as_deref().unwrap_or("verify-full"), root) {
            ("verify-full", root) => Tls::VerifyFull { root },
            ("require", None) => Tls::Require,
            ("disable", None) => Tls::Disable,
            _ => return Err(EndpointError::UnsupportedParameter),
        };
        let loopback =
            host == "localhost" || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback());
        if !loopback && !matches!(tls, Tls::VerifyFull { .. }) {
            return Err(EndpointError::Unverified);
        }
        Ok(Self {
            host,
            port,
            user,
            password,
            database,
            tls,
        })
    }

    /// The connection options, built through setters with no pgpass.
    pub(crate) fn options(&self) -> PgConnectOptions {
        let mut options = PgConnectOptions::new_without_pgpass()
            .host(&self.host)
            .port(self.port)
            .username(&self.user)
            .database(&self.database);
        if let Some(password) = &self.password {
            options = options.password(password);
        }
        match &self.tls {
            Tls::Disable => options.ssl_mode(PgSslMode::Disable),
            Tls::Require => options.ssl_mode(PgSslMode::Require),
            Tls::VerifyFull { root } => {
                let options = options.ssl_mode(PgSslMode::VerifyFull);
                match root {
                    Some(root) => options.ssl_root_cert(root),
                    None => options,
                }
            }
        }
    }
}

fn split_host(hostport: &str) -> Result<(String, u16), EndpointError> {
    let (host, port) = if let Some(rest) = hostport.strip_prefix('[') {
        let (host, rest) = rest.split_once(']').ok_or(EndpointError::Malformed)?;
        // A bracketed host is an IPv6 address, followed by nothing or a port.
        if host.parse::<std::net::Ipv6Addr>().is_err() {
            return Err(EndpointError::Malformed);
        }
        let port = match rest {
            "" => None,
            rest => Some(rest.strip_prefix(':').ok_or(EndpointError::Malformed)?),
        };
        (host, port)
    } else {
        match hostport.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (hostport, None),
        }
    };
    let port = match port {
        None => 5432,
        Some(port) => port.parse().map_err(|_| EndpointError::Malformed)?,
    };
    Ok((host.to_owned(), port))
}

/// Percent-decodes one URL component.
fn decode(text: &str) -> Result<String, EndpointError> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let hex = text
                    .get(index + 1..index + 3)
                    .ok_or(EndpointError::Malformed)?;
                out.push(u8::from_str_radix(hex, 16).map_err(|_| EndpointError::Malformed)?);
                index += 3;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    // A NUL would end the field in the NUL-delimited startup packet and let
    // the rest of the component name another startup parameter.
    if out.contains(&0) {
        return Err(EndpointError::Malformed);
    }
    String::from_utf8(out).map_err(|_| EndpointError::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_may_disable_tls_and_other_hosts_must_verify() {
        assert!(
            Endpoint::parse("postgres://ops:p%40ss@localhost:6543/flags?sslmode=disable").is_ok()
        );
        assert!(Endpoint::parse("postgres://ops@127.0.0.1/flags?sslmode=require").is_ok());
        assert!(Endpoint::parse("postgres://ops@[::1]/flags?sslmode=disable").is_ok());
        assert!(
            Endpoint::parse("postgres://ops@db.internal/flags").is_ok(),
            "verify-full by default"
        );
        assert!(
            Endpoint::parse(
                "postgres://ops@db.internal/flags?sslmode=verify-full&sslrootcert=/etc/ca.pem"
            )
            .is_ok()
        );
        assert_eq!(
            Endpoint::parse("postgres://ops@db.internal/flags?sslmode=require").err(),
            Some(EndpointError::Unverified)
        );
        assert_eq!(
            Endpoint::parse("postgres://ops@db.internal/flags?sslmode=disable").err(),
            Some(EndpointError::Unverified)
        );
    }

    #[test]
    fn anything_else_is_refused_without_echoing_the_url() {
        for url in [
            "mysql://ops@localhost/flags",
            "postgres://localhost/flags",
            "postgres://ops@localhost",
            "postgres://ops@localhost:port/flags",
            "postgres://ops@localhost/flags?sslmode=prefer",
            "postgres://ops@localhost/flags?sslrootcert=relative.pem",
            "postgres://ops@localhost/flags?options=-c",
            "postgres://ops:%zz@localhost/flags",
            "postgres://ops@localhost/flags%00database%00other?sslmode=disable",
            "postgres://ops%00x@localhost/flags?sslmode=disable",
            "postgres://ops@[::1]unexpected/flags?sslmode=disable",
            "postgres://ops@[not-an-address]/flags?sslmode=disable",
        ] {
            let error = Endpoint::parse(url).err();
            assert!(error.is_some(), "{url}");
            assert!(
                !error
                    .map(|e| e.to_string())
                    .unwrap_or_default()
                    .contains("ops")
            );
        }
        let endpoint =
            Endpoint::parse("postgres://ops:SECRET@localhost/flags?sslmode=disable").ok();
        assert!(!format!("{endpoint:?}").contains("SECRET"));
    }
}
