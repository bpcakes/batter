//! One parse of the harness endpoint, shared by every fixture client.
//!
//! The harness provisions with tokio-postgres, which reads its admin URL
//! without consulting the process environment, while SQLx 0.9.1 fills unset
//! settings from `PG*` variables (as libpq does) and parses some URL text
//! differently. Fixture endpoints therefore refuse every `PG*` variable a
//! PostgreSQL client could read and take all settings from the admin URL.
//! Rather than check that two parsers agree on arbitrary text, fixture pools
//! take their endpoint from the harness's own parse of its admin URL, set every
//! field explicitly, and take the database from the name the harness recorded.

use std::{path::PathBuf, str::FromStr, sync::Arc};

use postgres_test_harness::{DatabaseLease, PostgresHarness};
use sqlx::postgres::{PgConnectOptions, PgSslMode};
use tokio_postgres::config::{
    ChannelBinding, Host, LoadBalanceHosts, SslMode, SslNegotiation, TargetSessionAttrs,
};

const DEFAULT_PORT: u16 = 5432;

/// Native options for `lease`'s database on the server the harness
/// provisioned it on.
///
/// Each call parses the harness admin URL with tokio-postgres, the parser the
/// harness itself connects with (fixture suites and scopes keep one parse
/// instead), and builds SQLx options from that parse field by
/// field: host or Unix socket, port (5432 when the URL names none), user,
/// password (none when the URL omits it; no passfile is read) and the harness
/// connector’s plaintext transport, plus the lease's recorded database. The lease, not a
/// free-form name, selects the database, so these options cannot open the
/// admin database or a database no lease names; a lease whose URL does not
/// name its database is refused with [`super::LeaseDatabaseMismatch`], and
/// one that reaches another server, login or session settings than this
/// harness's admin URL with [`HarnessEndpointReason::LeaseEndpointMismatch`],
/// and one naming this harness's admin database with
/// [`HarnessEndpointReason::LeaseNamesAdminDatabase`].
/// That is an endpoint match, not proof of ownership: the harness gives a
/// lease no issuer identity, so a lease from a second harness on the same
/// server and login is accepted, and the caller remains responsible for
/// passing a lease it holds. The guarantees describe the options as returned:
/// `PgConnectOptions` keeps its native setters, and a caller that changes the
/// database or host afterwards must preserve them itself.
/// Test code that connects to a leased database directly, such as a
/// diagnostic pool beside a fixture, uses this
/// instead of handing `admin_database_url()` or a lease URL to SQLx, whose
/// parse can differ (for example through an ambient `PG*` variable, a `dbname`
/// parameter or path normalization).
///
/// Refuses with [`UnsupportedHarnessEndpoint`] inside
/// `sqlx::Error::Configuration` when a client `PG*` environment variable is
/// set (SQLx would read some, and no setter clears them) or when the admin URL
/// selects something these
/// options cannot carry faithfully; see [`HarnessEndpointReason`].
///
/// ```no_run
/// # fn example(
/// #     harness: &postgres_test_harness::PostgresHarness,
/// #     lease: &postgres_test_harness::DatabaseLease,
/// # ) -> Result<(), sqlx::Error> {
/// let options = batter_sqlx::test_support::harness_connect_options(harness, lease)?;
/// let pool = sqlx::postgres::PgPoolOptions::new().connect_lazy_with(options);
/// # drop(pool);
/// # Ok(())
/// # }
/// ```
pub fn harness_connect_options(
    harness: &PostgresHarness,
    lease: &DatabaseLease,
) -> Result<PgConnectOptions, sqlx::Error> {
    lease_options(
        harness.admin_database_url(),
        lease.database_url(),
        lease.database_name(),
        Environment::process(),
    )
}

fn lease_options(
    admin_url: &str,
    lease_url: &str,
    name: &str,
    environment: Environment,
) -> Result<PgConnectOptions, sqlx::Error> {
    let endpoint = HarnessEndpoint::from_admin_url(admin_url, environment)?;
    endpoint.bind(lease_url, name)?;
    endpoint.options(name)
}

/// Native options for the harness's own administrative database.
///
/// The same per-call parse as [`harness_connect_options`], for the database
/// the admin URL itself selects, with the same refusals: a client `PG*`
/// variable, or an admin URL these options cannot carry (including a `dbname`
/// parameter, which would retarget every lease; see
/// [`HarnessEndpointReason::DatabaseParameter`]). Use it for observer pools
/// that inspect the catalog beside a fixture.
///
/// ```no_run
/// # async fn example(
/// #     harness: &postgres_test_harness::PostgresHarness,
/// # ) -> Result<(), sqlx::Error> {
/// let options = batter_sqlx::test_support::harness_admin_connect_options(harness)?;
/// let observer = sqlx::postgres::PgPoolOptions::new()
///     .max_connections(1)
///     .connect_with(options)
///     .await?;
/// let databases: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_database")
///     .fetch_one(&observer)
///     .await?;
/// # let _ = databases;
/// observer.close().await;
/// # Ok(())
/// # }
/// ```
pub fn harness_admin_connect_options(
    harness: &PostgresHarness,
) -> Result<PgConnectOptions, sqlx::Error> {
    let endpoint =
        HarnessEndpoint::from_admin_url(harness.admin_database_url(), Environment::process())?;
    endpoint.options(&endpoint.admin_database)
}

/// A suite's harness endpoint, parsed once when the suite adopts its harness.
///
/// Every acquisition and template call checks the process environment again
/// and then uses this parse, before the harness provisions anything. The
/// environment is checked once more after provisioning, before pools are built
/// or a template initializer runs, since provisioning can take a while.
#[derive(Clone)]
pub(super) struct SuiteEndpoint(Result<Arc<HarnessEndpoint>, UnsupportedHarnessEndpoint>);

impl SuiteEndpoint {
    pub(super) fn new(harness: &PostgresHarness) -> Self {
        Self(HarnessEndpoint::parse(harness.admin_database_url()).map(Arc::new))
    }

    /// The parsed endpoint, or why fixture clients cannot use it. A refused
    /// admin URL is reported before the environment, since unsetting a
    /// variable would not make it usable.
    pub(super) fn checked(&self) -> Result<Arc<HarnessEndpoint>, super::FixtureError> {
        let endpoint = self.0.clone().map_err(super::FixtureError::Endpoint)?;
        Environment::process()
            .check()
            .map_err(super::FixtureError::Endpoint)?;
        Ok(endpoint)
    }
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
    /// Empty when the URL gives none. Where the server asks for a password,
    /// SQLx then sends an empty one and tokio-postgres refuses locally; both
    /// fail, since PostgreSQL does not store an empty password.
    password: String,
    application_name: Option<String>,
    /// tokio-postgres's raw `options` as `-c key=value` pairs, from which
    /// SQLx's setter rebuilds the raw text byte for byte (checked at parse).
    options: Option<Vec<(String, String)>>,
    /// Compared when binding a lease, whose URL the harness derives from the
    /// admin URL; SQLx options carry neither this nor the next field.
    connect_timeout: Option<std::time::Duration>,
    /// `channel_binding=disable`; SQLx does not implement channel binding.
    channel_binding_disabled: bool,
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

/// Whether the process environment holds a `PG*` variable a PostgreSQL client
/// could read. SQLx fills unset settings from several (`PGHOSTADDR`,
/// `PGOPTIONS`, `PGAPPNAME`, `PGPASSWORD` and others) while the harness reads
/// none, so fixture endpoints refuse every one rather than translate between
/// the two. See [`is_client_variable`] for the names that are not refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Environment {
    pg_variables: bool,
}

/// SQLx's constructors and URL parser read `PG*` variables in-process, and
/// tests may not change the process environment, so offline endpoint tests
/// require a shell without them. One clear failure replaces many confusing
/// assertion failures.
#[cfg(test)]
pub(super) fn require_clean_environment() {
    assert_eq!(
        Environment::process(),
        Environment::CLEAN,
        "unset every PG* client environment variable to run the fixture endpoint tests: SQLx reads them"
    );
}

/// Whether a `PG*` environment variable name is one a PostgreSQL client may
/// read. libpq documents only `PG` followed by a letter (`PGHOST`,
/// `PGCONNECT_TIMEOUT`, ...) and SQLx reads a subset of those. Two shapes no
/// client reads are allowed: the server's `PGDATA` and `PG_`-prefixed names
/// such as the client tools' `PG_COLOR`. Any other `PG` name is refused, so a
/// variable a later SQLx release starts reading is refused already.
pub(super) fn is_client_variable(name: &[u8]) -> bool {
    name.starts_with(b"PG") && name != b"PGDATA" && !name.starts_with(b"PG_")
}

impl Environment {
    /// An environment without client `PG*` variables, for tests.
    #[cfg(test)]
    pub(super) const CLEAN: Self = Self {
        pg_variables: false,
    };

    /// An environment holding a client `PG*` variable, for refusal tests.
    #[cfg(test)]
    pub(super) const AMBIENT: Self = Self { pg_variables: true };

    pub(super) fn validate(self) -> Result<(), sqlx::Error> {
        self.check().map_err(configuration)
    }

    fn check(self) -> Result<(), UnsupportedHarnessEndpoint> {
        if self.pg_variables {
            return Err(UnsupportedHarnessEndpoint {
                reason: HarnessEndpointReason::AmbientEnvironment,
            });
        }
        Ok(())
    }

    /// The current process environment.
    pub(super) fn process() -> Self {
        Self {
            pg_variables: std::env::vars_os()
                .any(|(key, _)| is_client_variable(key.as_encoded_bytes())),
        }
    }
}

impl HarnessEndpoint {
    pub(super) fn from_admin_url(
        admin_url: &str,
        environment: Environment,
    ) -> Result<Self, sqlx::Error> {
        environment.validate()?;
        Self::parse(admin_url).map_err(configuration)
    }

    /// Parse `admin_url` alone; the caller checks the environment.
    fn parse(admin_url: &str) -> Result<Self, UnsupportedHarnessEndpoint> {
        use HarnessEndpointReason::*;
        let refusal = |reason| UnsupportedHarnessEndpoint { reason };
        let config =
            tokio_postgres::Config::from_str(admin_url).map_err(|_| refusal(Unparseable))?;
        if !config.get_hostaddrs().is_empty() {
            return Err(refusal(HostAddress));
        }
        // The harness derives every lease and template URL by replacing only
        // the path of this URL as `url::Url` reads it, keeping the query, so a
        // `dbname` key there would override every derived database. Reading
        // the query with the harness's own derivation parser, not checking two
        // parsers agree, refuses it before anything is provisioned.
        if url::Url::parse(admin_url)
            .is_ok_and(|url| url.query_pairs().any(|(key, _)| key == "dbname"))
        {
            return Err(refusal(DatabaseParameter));
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
            ([], _) => return Err(refusal(MissingHost)),
            ([_], _) => return Err(refusal(MultiplePorts)),
            _ => return Err(refusal(MultipleHosts)),
        };
        // tokio-postgres can shuffle the addresses one host name resolves to;
        // SQLx takes them in order, so the two could reach different servers.
        if config.get_target_session_attrs() != TargetSessionAttrs::Any
            || config.get_ssl_negotiation() != SslNegotiation::Postgres
            || config.get_channel_binding() == ChannelBinding::Require
            || config.get_load_balance_hosts() != LoadBalanceHosts::Disable
            // Connection tuning SQLx options do not carry.
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
                if raw_options(&pairs).as_deref() != Some(raw) {
                    return Err(refusal(UnsupportedSetting));
                }
                Some(pairs)
            }
        };
        // postgres-test-harness connects with NoTls. Disable and Prefer both
        // use plaintext; a TLS requirement cannot be honored by provisioning.
        if !matches!(config.get_ssl_mode(), SslMode::Disable | SslMode::Prefer) {
            return Err(refusal(UnsupportedSetting));
        }
        let user = config
            .get_user()
            .ok_or_else(|| refusal(MissingUser))?
            .to_owned();
        let password = match config.get_password() {
            None => String::new(),
            Some(bytes) => {
                String::from_utf8(bytes.to_vec()).map_err(|_| refusal(NonUtf8Password))?
            }
        };
        let admin_database = config.get_dbname().unwrap_or(&user).to_owned();
        Ok(Self {
            address,
            port,
            user,
            password,
            application_name: config.get_application_name().map(str::to_owned),
            options,
            connect_timeout: config.get_connect_timeout().copied(),
            channel_binding_disabled: config.get_channel_binding() == ChannelBinding::Disable,
            admin_database,
        })
    }

    /// SQLx options for `database`, every endpoint field set explicitly.
    pub(super) fn options(&self, database: &str) -> Result<PgConnectOptions, sqlx::Error> {
        self.options_for_login(database, &self.user, &self.password)
    }

    /// Check that `url`, a harness-derived lease or template URL, selects
    /// `name` on this endpoint. The harness builds such a URL from its admin
    /// URL, replacing only the path and keeping the query (which `parse`
    /// refuses to contain `dbname`); any other difference reaches another
    /// endpoint.
    pub(super) fn bind(&self, url: &str, name: &str) -> Result<(), sqlx::Error> {
        if !Self::names_database(url, name) {
            return Err(sqlx::Error::Configuration(Box::new(
                super::LeaseDatabaseMismatch,
            )));
        }
        let issuer =
            Self::parse(url).map_err(|_| refusal(HarnessEndpointReason::LeaseEndpointMismatch))?;
        if !self.same_server_and_login(&issuer) {
            return Err(refusal(HarnessEndpointReason::LeaseEndpointMismatch));
        }
        // A matching endpoint cannot prove the issuer, so a lease from another
        // harness on the same server could name this harness's admin database.
        if name == self.admin_database {
            return Err(refusal(HarnessEndpointReason::LeaseNamesAdminDatabase));
        }
        Ok(())
    }

    /// The only place fixture code runs SQLx's option constructor, which
    /// fills unset settings from `PG*` variables. A fresh process scan runs
    /// immediately before it, whatever environment the caller injected, so a
    /// variable set since an earlier check is refused rather than read; only
    /// a concurrent change in the instant between the scan and the
    /// constructor can still be read. Nothing built here is cached. The
    /// constructor's socket-directory probe and user lookup run per call,
    /// as they did when fixture pools parsed lease URLs with SQLx.
    pub(super) fn options_for_login(
        &self,
        database: &str,
        user: &str,
        password: &str,
    ) -> Result<PgConnectOptions, sqlx::Error> {
        Environment::process().validate()?;
        let options = PgConnectOptions::new_without_pgpass();
        let options = match &self.address {
            Address::Tcp(name) => options.host(name),
            Address::Unix(path) => options.socket(path),
        };
        let mut options = options
            .port(self.port)
            .username(user)
            .password(password)
            .database(database)
            .ssl_mode(PgSslMode::Disable);
        if let Some(name) = &self.application_name {
            options = options.application_name(name);
        }
        if let Some(pairs) = &self.options {
            options = options.options(pairs.iter().cloned());
        }
        Ok(options)
    }

    /// Whether `other` reaches the same server with the same login and session
    /// settings, whatever database it names.
    fn same_server_and_login(&self, other: &Self) -> bool {
        self.address == other.address
            && self.port == other.port
            && self.user == other.user
            && self.password == other.password
            && self.application_name == other.application_name
            && self.options == other.options
            && self.connect_timeout == other.connect_timeout
            && self.channel_binding_disabled == other.channel_binding_disabled
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
    /// URL, which `parse` refuses to carry `dbname`, so a mismatch means the
    /// URL was not derived from this harness's admin URL.
    pub(super) fn names_database(url: &str, database: &str) -> bool {
        tokio_postgres::Config::from_str(url)
            .is_ok_and(|config| config.get_dbname() == Some(database))
    }
}

/// The raw startup `options` text SQLx 0.9.1's `PgConnectOptions::options`
/// setter writes for `pairs` (`-c key=value`, space-separated, with spaces
/// and backslashes backslash-escaped), rendered without a SQLx constructor:
/// those prepend an ambient `PGOPTIONS`, and a suite parses its admin URL
/// before any environment check, so the cached parse must depend only on the
/// URL. A unit test pins this rendering to the setter's.
fn raw_options(pairs: &[(String, String)]) -> Option<String> {
    if pairs.is_empty() {
        return None;
    }
    let mut raw = String::new();
    for (key, value) in pairs {
        if !raw.is_empty() {
            raw.push(' ');
        }
        raw.push_str("-c ");
        for character in format!("{key}={value}").chars() {
            if matches!(character, ' ' | '\\') {
                raw.push('\\');
            }
            raw.push(character);
        }
    }
    Some(raw)
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

mod refusal;
#[cfg(test)]
mod tests;

pub use refusal::{HarnessEndpointReason, UnsupportedHarnessEndpoint};
use refusal::{configuration, refusal};
