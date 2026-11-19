//! One parse of the harness endpoint, shared by every fixture client.
//!
//! The harness provisions with tokio-postgres, which reads its admin URL
//! without consulting the process environment, while SQLx 0.9.1 reads an
//! ambient `PGHOSTADDR` (as libpq does) and parses some URL text differently.
//! Rather than check that two parsers agree on arbitrary text, fixture pools
//! take their endpoint from the harness's own parse of its admin URL, set every
//! field explicitly, and take the database from the name the harness recorded.

use std::{fmt, path::PathBuf, str::FromStr};

use postgres_test_harness::PostgresHarness;
use sqlx::postgres::{PgConnectOptions, PgSslMode};
use tokio_postgres::config::{
    ChannelBinding, Host, LoadBalanceHosts, SslMode, SslNegotiation, TargetSessionAttrs,
};

const DEFAULT_PORT: u16 = 5432;

/// Native options for `database` on the server the harness provisions on.
///
/// Reads the harness admin URL once with tokio-postgres, the parser the harness
/// itself connects with, and builds SQLx options from that parse field by
/// field: host or Unix socket, port (5432 when the URL names none), user,
/// password and TLS mode, plus `database`. Test code that connects to a harness
/// database directly, such as a diagnostic pool beside a fixture, uses this
/// instead of handing `admin_database_url()` or a lease URL to SQLx, whose
/// parse can differ (for example through an ambient `PGHOSTADDR`, a `dbname`
/// parameter or path normalization).
///
/// Refuses with [`UnsupportedHarnessEndpoint`] inside
/// `sqlx::Error::Configuration` when `PGHOSTADDR` is set (SQLx would apply it
/// and no setter clears it) or when the admin URL selects something these
/// options cannot carry faithfully; see [`HarnessEndpointReason`].
///
/// ```no_run
/// # fn example(harness: &postgres_test_harness::PostgresHarness) -> Result<(), sqlx::Error> {
/// let options = batter_sqlx::test_support::harness_connect_options(harness, "fixture_db")?;
/// let pool = sqlx::postgres::PgPoolOptions::new().connect_lazy_with(options);
/// # drop(pool);
/// # Ok(())
/// # }
/// ```
pub fn harness_connect_options(
    harness: &PostgresHarness,
    database: &str,
) -> Result<PgConnectOptions, sqlx::Error> {
    HarnessEndpoint::from_admin_url(harness.admin_database_url())?.options(database)
}

/// Native options for the harness's own administrative database.
///
/// The same single parse as [`harness_connect_options`], for the database the
/// admin URL itself selects.
pub fn harness_admin_connect_options(
    harness: &PostgresHarness,
) -> Result<PgConnectOptions, sqlx::Error> {
    let endpoint = HarnessEndpoint::from_admin_url(harness.admin_database_url())?;
    let database = endpoint.admin_database.clone();
    endpoint.options(&database)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Address {
    Tcp(String),
    Unix(PathBuf),
}

/// The harness's endpoint as tokio-postgres read it. `Debug` omits the
/// password.
#[derive(Clone)]
pub(super) struct HarnessEndpoint {
    address: Address,
    port: u16,
    user: String,
    password: Option<String>,
    ssl_mode: PgSslMode,
    application_name: Option<String>,
    /// tokio-postgres's raw `options`, carried only as `-c key=value` pairs
    /// that SQLx's setter rebuilds byte for byte.
    options: Option<(String, Vec<(String, String)>)>,
    /// Carried to template initializers, which may use tokio-postgres; SQLx
    /// ignores the parameter, as it always has for these pools.
    connect_timeout: Option<std::time::Duration>,
    admin_database: String,
}

impl std::fmt::Debug for HarnessEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HarnessEndpoint")
            .field("address", &self.address)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("admin_database", &self.admin_database)
            .finish_non_exhaustive()
    }
}

/// Whether SQLx's `options` setter, applied to `base`, writes exactly
/// `prefix raw`: the setter appends to options SQLx already took from an
/// ambient `PGOPTIONS`, joining them with a space, and escapes each value.
fn setter_rebuilds(base: PgConnectOptions, raw: &str, pairs: &[(String, String)]) -> bool {
    let expected = match base.get_options() {
        Some(ambient) if !ambient.is_empty() => format!("{ambient} {raw}"),
        _ => raw.to_owned(),
    };
    base.options(pairs.iter().cloned()).get_options() == Some(expected.as_str())
}

impl HarnessEndpoint {
    pub(super) fn from_admin_url(admin_url: &str) -> Result<Self, sqlx::Error> {
        Self::from_admin_url_with(admin_url, std::env::var_os("PGHOSTADDR").as_deref())
    }

    fn from_admin_url_with(
        admin_url: &str,
        ambient_host_address: Option<&std::ffi::OsStr>,
    ) -> Result<Self, sqlx::Error> {
        use HarnessEndpointReason::*;
        if ambient_host_address.is_some() {
            return Err(refusal(AmbientHostAddress));
        }
        let config =
            tokio_postgres::Config::from_str(admin_url).map_err(|_| refusal(Unparseable))?;
        if !config.get_hostaddrs().is_empty() {
            return Err(refusal(HostAddress));
        }
        let (address, port) = match (config.get_hosts(), config.get_ports()) {
            ([host], [] | [_]) => {
                let address = match host {
                    // SQLx trims edge brackets before resolving, even unmatched
                    // ones, and its URL rendering panics on an interior one;
                    // tokio-postgres already removed an IPv6 literal's.
                    Host::Tcp(name) if name.contains(['[', ']']) => {
                        return Err(refusal(UnsupportedHostName));
                    }
                    Host::Tcp(name) => Address::Tcp(name.clone()),
                    // SQLx formats the socket path as text, replacing bytes that
                    // are not UTF-8, so it could open a different directory.
                    Host::Unix(path) if path.to_str().is_some() => Address::Unix(path.clone()),
                    Host::Unix(_) => return Err(refusal(NonUtf8SocketPath)),
                };
                (
                    address,
                    config.get_ports().first().copied().unwrap_or(DEFAULT_PORT),
                )
            }
            _ => return Err(refusal(MultipleHosts)),
        };
        // tokio-postgres can shuffle the addresses one host name resolves to;
        // SQLx takes them in order, so the two could reach different servers.
        if config.get_target_session_attrs() != TargetSessionAttrs::Any
            || config.get_ssl_negotiation() != SslNegotiation::Postgres
            || config.get_channel_binding() == ChannelBinding::Require
            || config.get_load_balance_hosts() != LoadBalanceHosts::Disable
            // Connection tuning the rebuilt initializer URL does not carry.
            || config.get_tcp_user_timeout().is_some()
            || !config.get_keepalives()
            || config.get_keepalives_idle() != tokio_postgres::Config::new().get_keepalives_idle()
            || config.get_keepalives_interval().is_some()
            || config.get_keepalives_retries().is_some()
        {
            return Err(refusal(UnsupportedSetting));
        }
        let options = match config.get_options() {
            None => None,
            Some(raw) => {
                let pairs = option_pairs(raw).ok_or_else(|| refusal(UnsupportedSetting))?;
                if !setter_rebuilds(PgConnectOptions::new_without_pgpass(), raw, &pairs) {
                    return Err(refusal(UnsupportedSetting));
                }
                Some((raw.to_owned(), pairs))
            }
        };
        let ssl_mode = match config.get_ssl_mode() {
            SslMode::Disable => PgSslMode::Disable,
            SslMode::Prefer => PgSslMode::Prefer,
            SslMode::Require => PgSslMode::Require,
            _ => return Err(refusal(UnsupportedSetting)),
        };
        let user = config
            .get_user()
            .ok_or_else(|| refusal(MissingUser))?
            .to_owned();
        let password = match config.get_password() {
            None => None,
            Some(bytes) => {
                Some(String::from_utf8(bytes.to_vec()).map_err(|_| refusal(NonUtf8Password))?)
            }
        };
        let admin_database = config.get_dbname().unwrap_or(&user).to_owned();
        Ok(Self {
            address,
            port,
            user,
            password,
            ssl_mode,
            application_name: config.get_application_name().map(str::to_owned),
            options,
            connect_timeout: config.get_connect_timeout().copied(),
            admin_database,
        })
    }

    /// SQLx options for `database`, every endpoint field set explicitly.
    pub(super) fn options(&self, database: &str) -> Result<PgConnectOptions, sqlx::Error> {
        // `from_admin_url` already refused an ambient `PGHOSTADDR`, the one
        // environment value no setter below overrides.
        let options = PgConnectOptions::new_without_pgpass();
        let options = match &self.address {
            Address::Tcp(name) => options.host(name),
            Address::Unix(path) => options.socket(path),
        };
        let mut options = options
            .port(self.port)
            .username(&self.user)
            .database(database)
            .ssl_mode(self.ssl_mode);
        if let Some(password) = &self.password {
            options = options.password(password);
        }
        if let Some(name) = &self.application_name {
            options = options.application_name(name);
        }
        if let Some((_, pairs)) = &self.options {
            options = options.options(pairs.iter().cloned());
        }
        Ok(options)
    }

    /// A minimal URL for `database` that tokio-postgres and SQLx both read back
    /// as this endpoint, for callers such as template initializers that take a
    /// URL. Refuses rather than hand out a URL either parser reads differently.
    pub(super) fn url(&self, database: &str) -> Result<String, sqlx::Error> {
        let host = match &self.address {
            Address::Tcp(name) if name.contains(':') => format!("[{name}]"),
            Address::Tcp(name)
                if name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-._".contains(&b)) =>
            {
                name.clone()
            }
            Address::Tcp(_) => return Err(refusal(HarnessEndpointReason::RoundTrip)),
            Address::Unix(path) => strict_encode(
                path.to_str()
                    .ok_or_else(|| refusal(HarnessEndpointReason::RoundTrip))?,
            ),
        };
        let mut url = format!("postgres://{}", strict_encode(&self.user));
        // SQLx reads an empty userinfo password as none and may then consult a
        // passfile, so an explicit empty password travels as a parameter.
        match self.password.as_deref() {
            Some("") | None => {}
            Some(password) => {
                url.push(':');
                url.push_str(&strict_encode(password));
            }
        }
        url.push_str(&format!(
            "@{host}:{}/{}",
            self.port,
            strict_encode(database)
        ));
        url.push_str(&format!("?sslmode={}", mode_name(self.ssl_mode)));
        if let Some(name) = &self.application_name {
            url.push_str(&format!("&application_name={}", strict_encode(name)));
        }
        if let Some((raw, _)) = &self.options {
            url.push_str(&format!("&options={}", strict_encode(raw)));
        }
        if let Some(timeout) = self.connect_timeout {
            // tokio-postgres parsed whole seconds, so this is exact.
            url.push_str(&format!("&connect_timeout={}", timeout.as_secs()));
        }
        if self.password.as_deref() == Some("") {
            url.push_str("&password=");
        }
        self.check_round_trip(&url, database)?;
        Ok(url)
    }

    fn check_round_trip(&self, url: &str, database: &str) -> Result<(), sqlx::Error> {
        let mismatch = || refusal(HarnessEndpointReason::RoundTrip);
        // tokio-postgres's parse, as the harness and many initializers read it.
        let again = Self::from_admin_url_with(url, None).map_err(|_| mismatch())?;
        if again.address != self.address
            || again.port != self.port
            || again.user != self.user
            || again.password != self.password
            || mode_name(again.ssl_mode) != mode_name(self.ssl_mode)
            || again.application_name != self.application_name
            || again.options != self.options
            || again.connect_timeout != self.connect_timeout
            || again.admin_database != database
        {
            return Err(mismatch());
        }
        // SQLx's own parse; an ambient `PGHOSTADDR` was refused already.
        let native: PgConnectOptions = url.parse().map_err(|_| mismatch())?;
        // Session settings the URL omits come from SQLx's environment in both
        // the URL parse and the setter-built options, so compare those two:
        // the harness does not send them, and they select no server.
        let built = self.options(database)?;
        let same_address = match &self.address {
            // SQLx keeps the URL brackets around an IPv6 host.
            Address::Tcp(name) => {
                native.get_socket().is_none()
                    && native
                        .get_host()
                        .trim_start_matches('[')
                        .trim_end_matches(']')
                        == name
            }
            Address::Unix(path) => native.get_socket() == Some(path),
        };
        if !same_address
            || native.get_port() != self.port
            || native.get_username() != self.user
            || native.get_database() != Some(database)
            || mode_name(native.get_ssl_mode()) != mode_name(self.ssl_mode)
            || native.get_application_name() != built.get_application_name()
            || native.get_options() != built.get_options()
        {
            return Err(mismatch());
        }
        Ok(())
    }

    /// The database a harness-derived URL was made for: the harness writes the
    /// database name, from its own restricted alphabet, as the URL path. Any
    /// other path is not one the harness wrote.
    pub(super) fn recorded_database(url: &str) -> Option<String> {
        let parsed = url::Url::parse(url).ok()?;
        let name = parsed.path().strip_prefix('/')?;
        (!name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'))
        .then(|| name.to_owned())
    }

    /// Whether `url` (a harness-derived lease or template URL) names `database`
    /// as tokio-postgres reads it. The harness derives such URLs from its admin
    /// URL, keeping any `dbname` parameter that would override the path.
    pub(super) fn names_database(url: &str, database: &str) -> bool {
        tokio_postgres::Config::from_str(url)
            .is_ok_and(|config| config.get_dbname() == Some(database))
    }
}

/// Split a raw startup `options` string of the form `-c key=value ...`, with
/// backslash-escaped spaces and backslashes, into its pairs. Anything else is
/// not a form SQLx's setter can express.
fn option_pairs(raw: &str) -> Option<Vec<(String, String)>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => word.push(chars.next()?),
            ' ' if !word.is_empty() => words.push(std::mem::take(&mut word)),
            ' ' => {}
            _ => word.push(c),
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    if words.is_empty() || words.len() % 2 != 0 {
        return None;
    }
    words
        .chunks(2)
        .map(|pair| match pair {
            [flag, setting] if flag == "-c" => {
                let (key, value) = setting.split_once('=')?;
                (!key.is_empty()).then(|| (key.to_owned(), value.to_owned()))
            }
            _ => None,
        })
        .collect()
}

/// The URL spelling of the TLS modes a harness endpoint can carry.
fn mode_name(mode: PgSslMode) -> &'static str {
    match mode {
        PgSslMode::Disable => "disable",
        PgSslMode::Require => "require",
        PgSslMode::Prefer => "prefer",
        PgSslMode::Allow => "allow",
        PgSslMode::VerifyCa => "verify-ca",
        PgSslMode::VerifyFull => "verify-full",
    }
}

/// The URL a template initializer receives in place of `harness_url`: rebuilt
/// from the harness's parse of `admin_url` for the database the harness
/// recorded in `harness_url`'s path.
pub(super) fn initializer_url(admin_url: &str, harness_url: &str) -> Result<String, sqlx::Error> {
    let endpoint = HarnessEndpoint::from_admin_url(admin_url)?;
    let database = HarnessEndpoint::recorded_database(harness_url)
        .ok_or_else(|| refusal(HarnessEndpointReason::RoundTrip))?;
    if !HarnessEndpoint::names_database(harness_url, &database) {
        return Err(sqlx::Error::Configuration(Box::new(
            super::LeaseDatabaseMismatch,
        )));
    }
    endpoint.url(&database)
}

/// Percent-encode everything but RFC 3986 unreserved bytes.
fn strict_encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(byte).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn refusal(reason: HarnessEndpointReason) -> sqlx::Error {
    sqlx::Error::Configuration(Box::new(UnsupportedHarnessEndpoint { reason }))
}

/// The harness endpoint cannot be carried faithfully into SQLx options.
///
/// Carried inside `sqlx::Error::Configuration` by fixture resolution, template
/// preparation, [`harness_connect_options`] and
/// [`harness_admin_connect_options`], before any pool or caller code exists.
/// Formatting names no URL part.
///
/// ```
/// # fn example(error: &sqlx::Error) -> bool {
/// use batter_sqlx::test_support::{HarnessEndpointReason, UnsupportedHarnessEndpoint};
/// matches!(error, sqlx::Error::Configuration(source)
///     if source.downcast_ref::<UnsupportedHarnessEndpoint>().map(|e| e.reason())
///         == Some(HarnessEndpointReason::AmbientHostAddress))
/// # }
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnsupportedHarnessEndpoint {
    reason: HarnessEndpointReason,
}

impl UnsupportedHarnessEndpoint {
    /// Why the endpoint was refused.
    #[must_use]
    pub const fn reason(&self) -> HarnessEndpointReason {
        self.reason
    }
}

/// Why a harness endpoint was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HarnessEndpointReason {
    /// `PGHOSTADDR` is set. SQLx would connect to it, as libpq does, while the
    /// harness's tokio-postgres client ignores it; no SQLx setter clears it.
    AmbientHostAddress,
    /// tokio-postgres could not parse the admin URL.
    Unparseable,
    /// The admin URL names several hosts.
    MultipleHosts,
    /// The admin URL declares `hostaddr`, which SQLx can replace with a Unix
    /// socket.
    HostAddress,
    /// The admin URL selects a setting SQLx options cannot carry faithfully:
    /// `options` other than `-c key=value` pairs, a non-default
    /// `target_session_attrs`, `sslnegotiation` or `load_balance_hosts`,
    /// required channel binding, or an unknown TLS mode.
    UnsupportedSetting,
    /// The admin URL names no user.
    MissingUser,
    /// The admin URL's TCP host name contains a bracket. SQLx trims leading
    /// and trailing brackets before resolving, so it would look up a different
    /// name, and its URL rendering panics on such a host.
    UnsupportedHostName,
    /// The admin URL's password is not UTF-8.
    NonUtf8Password,
    /// The admin URL's Unix socket directory is not UTF-8, which SQLx would
    /// rewrite when it formats the socket path.
    NonUtf8SocketPath,
    /// A rebuilt URL would not read back as the same endpoint in both parsers.
    RoundTrip,
}

impl fmt::Display for UnsupportedHarnessEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.reason {
            HarnessEndpointReason::AmbientHostAddress => {
                "PGHOSTADDR is set; SQLx would connect to it while the fixture harness does not"
            }
            HarnessEndpointReason::Unparseable => "the fixture admin URL could not be parsed",
            HarnessEndpointReason::MultipleHosts => "the fixture admin URL names several hosts",
            HarnessEndpointReason::HostAddress => "the fixture admin URL declares hostaddr",
            HarnessEndpointReason::UnsupportedSetting => {
                "the fixture admin URL selects a setting fixture pools cannot carry"
            }
            HarnessEndpointReason::MissingUser => "the fixture admin URL names no user",
            HarnessEndpointReason::UnsupportedHostName => {
                "the fixture admin URL host name contains a bracket"
            }
            HarnessEndpointReason::NonUtf8Password => "the fixture admin URL password is not UTF-8",
            HarnessEndpointReason::NonUtf8SocketPath => {
                "the fixture admin URL socket directory is not UTF-8"
            }
            HarnessEndpointReason::RoundTrip => {
                "a rebuilt fixture URL would not select the same endpoint"
            }
        })
    }
}

impl std::error::Error for UnsupportedHarnessEndpoint {}

#[cfg(test)]
mod tests;
