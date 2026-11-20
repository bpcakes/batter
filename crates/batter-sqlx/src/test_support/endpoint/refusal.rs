//! Why a harness endpoint is refused, and how refusals travel inside SQLx errors.

use std::fmt;

pub(super) fn refusal(reason: HarnessEndpointReason) -> sqlx::Error {
    configuration(UnsupportedHarnessEndpoint { reason })
}

pub(super) fn configuration(refusal: UnsupportedHarnessEndpoint) -> sqlx::Error {
    sqlx::Error::Configuration(Box::new(refusal))
}

/// The harness endpoint cannot be carried faithfully into SQLx options.
///
/// Returned as [`super::FixtureError::Endpoint`] when a fixture suite or scope
/// refuses its endpoint before provisioning a database or looking up a
/// template, and carried inside `sqlx::Error::Configuration` by later checks:
/// pool resolution, the URL handed to a template initializer,
/// [`harness_connect_options`] and [`harness_admin_connect_options`]. Every
/// refusal happens before any pool or caller code exists. Formatting names no
/// URL part.
///
/// ```
/// # fn example(error: &sqlx::Error) -> bool {
/// use batter_sqlx::test_support::{HarnessEndpointReason, UnsupportedHarnessEndpoint};
/// matches!(error, sqlx::Error::Configuration(source)
///     if source.downcast_ref::<UnsupportedHarnessEndpoint>().map(|e| e.reason())
///         == Some(HarnessEndpointReason::AmbientEnvironment))
/// # }
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnsupportedHarnessEndpoint {
    pub(super) reason: HarnessEndpointReason,
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
    /// A `PG*` environment variable a PostgreSQL client could read is set
    /// (every `PG` name except `PGDATA` and `PG_`-prefixed ones). SQLx fills
    /// unset settings from several, as libpq does, while the harness's
    /// tokio-postgres client reads none; put every setting in the harness
    /// admin URL instead.
    AmbientEnvironment,
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
    /// required channel binding or TLS, or connection tuning
    /// (`tcp_user_timeout`, `keepalives*`).
    UnsupportedSetting,
    /// The admin URL names no user.
    MissingUser,
    /// The admin URL's TCP host name contains a bracket. SQLx trims leading
    /// and trailing brackets before resolving, so it would look up a different
    /// name, and its URL rendering panics on such a host.
    UnsupportedHostName,
    /// The lease's URL reaches another server, login or session settings than
    /// this harness's admin URL. A matching endpoint does not prove the lease
    /// came from this harness.
    LeaseEndpointMismatch,
    /// The lease names this harness's own administrative database, which a
    /// lease from another harness on the same server could do.
    LeaseNamesAdminDatabase,
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
            HarnessEndpointReason::AmbientEnvironment => {
                "a PG* client environment variable is set; put every fixture setting in the admin URL"
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
            HarnessEndpointReason::LeaseNamesAdminDatabase => {
                "the lease names this fixture harness's admin database"
            }
            HarnessEndpointReason::LeaseEndpointMismatch => {
                "the lease reaches another endpoint than this fixture harness"
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
