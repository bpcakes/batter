//! Declarative session policy, not evidence about an arbitrary borrowed resource.
mod validation;

use sqlx::{Connection, PgConnection};
use std::{collections::BTreeMap, fmt, time::Duration};

/// Validated set of all four PostgreSQL server session timeouts.
///
/// Each duration must be an exact whole number of milliseconds in
/// `0..=i32::MAX`. Zero explicitly disables that timeout, even over a nonzero
/// startup or role/database default. Use [`Self::new`] to declare all four
/// bounds or [`Self::disabled`] to explicitly disable all four (`0ms`).
///
/// `idle_in_transaction_session_timeout` bounds each idle interval inside a
/// transaction; `transaction_timeout` bounds the entire transaction.
/// PostgreSQL ignores a statement or idle timeout when a nonzero transaction
/// timeout is shorter or equal; relative values are application policy, not
/// parent/child operation deadlines.
///
/// There is deliberately no `Default` implementation, so a caller cannot omit
/// timeouts silently:
///
/// ```compile_fail,E0599
/// use batter_sqlx::PgSessionTimeouts;
///
/// let _ = PgSessionTimeouts::default();
/// ```
///
/// Fields are private so unvalidated durations cannot bypass construction:
///
/// ```compile_fail,E0451
/// use batter_sqlx::PgSessionTimeouts;
///
/// let _ = PgSessionTimeouts {
///     statement_ms: 0,
///     lock_ms: 0,
///     idle_in_transaction_ms: 0,
///     transaction_ms: 0,
/// };
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PgSessionTimeouts {
    statement_ms: u32,
    lock_ms: u32,
    idle_in_transaction_ms: u32,
    transaction_ms: u32,
}

impl PgSessionTimeouts {
    /// Declare all four PostgreSQL server timeouts in exact whole milliseconds
    /// within `0..=i32::MAX`.
    ///
    /// Zero explicitly disables that timeout, even over a nonzero startup
    /// default. All four values are applied after session reset and verified at
    /// scope boundaries.
    pub const fn new(
        statement: Duration,
        lock: Duration,
        idle_in_transaction: Duration,
        transaction: Duration,
    ) -> Result<Self, PgProfileError> {
        let statement_ms = match timeout_ms(statement) {
            Ok(ms) => ms,
            Err(error) => return Err(error),
        };
        let lock_ms = match timeout_ms(lock) {
            Ok(ms) => ms,
            Err(error) => return Err(error),
        };
        let idle_in_transaction_ms = match timeout_ms(idle_in_transaction) {
            Ok(ms) => ms,
            Err(error) => return Err(error),
        };
        let transaction_ms = match timeout_ms(transaction) {
            Ok(ms) => ms,
            Err(error) => return Err(error),
        };
        Ok(Self {
            statement_ms,
            lock_ms,
            idle_in_transaction_ms,
            transaction_ms,
        })
    }

    /// Explicitly disable all four PostgreSQL server timeouts (`0ms`).
    ///
    /// This still applies and verifies `0ms` for all four settings after
    /// session reset, overriding any nonzero startup or role/database defaults.
    /// Use this only when cooperative operation contexts or application policy
    /// bound execution instead of server timeouts.
    #[must_use = "pass the declared timeouts to PgSessionProfile::new"]
    pub const fn disabled() -> Self {
        Self {
            statement_ms: 0,
            lock_ms: 0,
            idle_in_transaction_ms: 0,
            transaction_ms: 0,
        }
    }

    /// Declared `statement_timeout`.
    pub const fn statement(self) -> Duration {
        Duration::from_millis(self.statement_ms as u64)
    }

    /// Declared `lock_timeout`.
    pub const fn lock(self) -> Duration {
        Duration::from_millis(self.lock_ms as u64)
    }

    /// Declared `idle_in_transaction_session_timeout`.
    pub const fn idle_in_transaction(self) -> Duration {
        Duration::from_millis(self.idle_in_transaction_ms as u64)
    }

    /// Declared `transaction_timeout`.
    pub const fn transaction(self) -> Duration {
        Duration::from_millis(self.transaction_ms as u64)
    }

    pub(crate) const fn as_pairs(self) -> [(&'static str, u32); 4] {
        [
            ("statement_timeout", self.statement_ms),
            ("lock_timeout", self.lock_ms),
            (
                "idle_in_transaction_session_timeout",
                self.idle_in_transaction_ms,
            ),
            ("transaction_timeout", self.transaction_ms),
        ]
    }
}

/// Explicit policy re-established after session reset. Names are quoted as
/// identifiers; setting values are bound parameters. No setup callback can run
/// application SQL before the policy is checked.
/// PostgreSQL 18 is the minimum supported server.
///
/// Search-path schemas are a trust declaration: only include schemas whose
/// object creators you trust. `pg_catalog` is implicitly searched first and
/// `pg_temp` is explicitly last. The first supplied schema receives unqualified
/// DDL. All supplied schemas must exist and be usable by the effective role.
///
/// ```no_run
/// # async fn example(pool: &sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
/// use batter_sqlx::{PgSessionProfile, PgSessionTimeouts, run_atomic_profiled};
/// use std::time::Duration;
/// let timeouts = PgSessionTimeouts::new(
///     Duration::from_secs(30),
///     Duration::from_secs(5),
///     Duration::from_secs(10),
///     Duration::from_secs(60),
/// )?;
/// let profile = PgSessionProfile::new(
///     "login",
///     "serving",
///     vec!["application".into()],
///     timeouts,
/// )?
/// .with_setting("app.tenant", "tenant-one")?;
/// let value = run_atomic_profiled(pool, &profile, async |scope| {
///     scope.application(async |sql| {
///         sqlx::query_scalar::<_, i64>("SELECT 1::bigint").fetch_one(sql.executor()).await
///     }).await
/// }).await?;
/// # let _ = value; Ok(()) }
/// ```
#[derive(Clone)]
pub struct PgSessionProfile {
    login_role: String,
    effective_role: String,
    schemas: Vec<String>,
    search_path: String,
    timeouts: PgSessionTimeouts,
    settings: BTreeMap<String, String>,
}

/// Redacted invalid declaration or effective-session mismatch.
#[derive(Debug)]
pub struct PgProfileError(&'static str);

impl fmt::Display for PgProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for PgProfileError {}

impl fmt::Debug for PgSessionProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PgSessionProfile")
    }
}

impl PgSessionProfile {
    /// Declare roles, an ordered trusted schema path, and all four validated
    /// server timeouts.
    ///
    /// All four timeout values in `timeouts` are applied after reset and
    /// checked at scope boundaries, including explicit zeros from
    /// [`PgSessionTimeouts::disabled`] or [`PgSessionTimeouts::new`]. No policy
    /// is inferred from SQLx hooks or the authenticated user's privileges.
    ///
    /// The minimum supported server is PostgreSQL 18. Unsupported settings fail
    /// setup before application access, including when set to zero.
    ///
    /// Server termination does not cancel Rust work or release a held pool lease.
    /// Use [`crate::run_atomic_profiled_in`] for a cooperative operation budget.
    /// Arbitrary SQL can alter settings before revalidation; this is not a sandbox.
    ///
    /// Migration: `PgSessionProfile::with_timeouts` has been removed and
    /// `PgSessionProfile::new` now takes a validated [`PgSessionTimeouts`] as
    /// its fourth argument instead of raw timeout durations. Replace
    /// `PgSessionProfile::with_timeouts(..., statement, lock, idle, transaction)`
    /// with `PgSessionProfile::new(..., PgSessionTimeouts::new(statement, lock, idle, transaction)?)`,
    /// and replace former five-argument `PgSessionProfile::new(..., statement, lock)`
    /// calls with explicit [`PgSessionTimeouts::new`] bounds or
    /// [`PgSessionTimeouts::disabled`] when all four timeouts are intentionally
    /// disabled.
    ///
    /// Passing raw statement and lock timeouts to `new` does not compile:
    ///
    /// ```compile_fail,E0061
    /// use batter_sqlx::PgSessionProfile;
    /// use std::time::Duration;
    ///
    /// let _ = PgSessionProfile::new(
    ///     "login",
    ///     "serving",
    ///     vec!["application".into()],
    ///     Duration::from_secs(30),
    ///     Duration::from_secs(5),
    /// );
    /// ```
    ///
    /// The former `with_timeouts` spelling has been removed:
    ///
    /// ```compile_fail,E0599
    /// use batter_sqlx::PgSessionProfile;
    /// use std::time::Duration;
    ///
    /// let _ = PgSessionProfile::with_timeouts(
    ///     "login",
    ///     "serving",
    ///     vec!["application".into()],
    ///     Duration::from_secs(30),
    ///     Duration::from_secs(5),
    ///     Duration::from_secs(10),
    ///     Duration::from_secs(60),
    /// );
    /// ```
    pub fn new(
        login_role: impl Into<String>,
        effective_role: impl Into<String>,
        schemas: Vec<String>,
        timeouts: PgSessionTimeouts,
    ) -> Result<Self, PgProfileError> {
        let login_role = login_role.into();
        let effective_role = effective_role.into();
        for name in [&login_role, &effective_role]
            .into_iter()
            .chain(schemas.iter())
        {
            if name.is_empty() || name.len() > 63 || name.contains('\0') {
                return Err(PgProfileError("invalid PostgreSQL profile identifier"));
            }
        }
        if schemas.is_empty()
            || schemas
                .iter()
                .any(|s| s.starts_with("pg_") || s == "information_schema" || s == "$user")
        {
            return Err(PgProfileError("profile requires ordinary trusted schemas"));
        }
        let search_path = schemas
            .iter()
            .map(|s| quote(s))
            .chain(std::iter::once("pg_temp".into()))
            .collect::<Vec<_>>()
            .join(", ");
        Ok(Self {
            login_role,
            effective_role,
            schemas,
            search_path,
            timeouts,
            settings: BTreeMap::new(),
        })
    }

    /// Declare a custom dotted setting (for example an application tenant GUC),
    /// application_name, or timezone. Authority, path, transaction defaults and
    /// timeouts cannot be overridden through this extension point. Values must
    /// round-trip through current_setting exactly. Names are ASCII-lowercased
    /// before validation; case-variant duplicates are rejected. Values are not
    /// normalized. See [`Self::reset_and_apply`] for setup-error logging limits.
    pub fn with_setting(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, PgProfileError> {
        let name = name.into().to_ascii_lowercase();
        let value = value.into();
        let custom = name.contains('.')
            && name.split('.').all(|part| {
                !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            });
        if !(custom || matches!(name.as_str(), "application_name" | "timezone"))
            || value.contains('\0')
        {
            return Err(PgProfileError("unsupported PostgreSQL profile setting"));
        }
        if self.settings.insert(name, value).is_some() {
            return Err(PgProfileError("duplicate PostgreSQL profile setting"));
        }
        Ok(self)
    }

    /// First trusted schema, used for unqualified DDL placement.
    pub fn schema(&self) -> &str {
        &self.schemas[0]
    }

    /// Ordered trusted schemas, excluding the implicit catalog and final temp schema.
    pub fn trusted_schemas(&self) -> &[String] {
        &self.schemas
    }

    /// Declared PostgreSQL server timeouts.
    pub const fn timeouts(&self) -> PgSessionTimeouts {
        self.timeouts
    }

    /// Reset an exclusively owned idle session, then establish and verify policy.
    /// This performs no application work and returns no lasting validity witness.
    /// The resource owner must retire the connection on failure/cancellation.
    ///
    /// Every failure is returned as [`sqlx::Error::Configuration`] containing a
    /// [`crate::SqlxFailure`]. Default Debug/Display, including SQLx pool-hook
    /// error logs, omit native contents. Trusted callers can downcast that
    /// payload and inspect `SqlxFailure::native()` or its error source. This does
    /// not redact error-chain reporters, independent SQLx query/notice logging,
    /// or PostgreSQL server logs; configure those separately.
    pub async fn reset_and_apply(&self, connection: &mut PgConnection) -> Result<(), sqlx::Error> {
        async {
            sqlx::raw_sql("ROLLBACK").execute(&mut *connection).await?;
            connection.clear_cached_statements().await?;
            sqlx::raw_sql("DISCARD ALL")
                .execute(&mut *connection)
                .await?;
            self.apply(connection).await
        }
        .await
        .map_err(|error| sqlx::Error::Configuration(Box::new(crate::SqlxFailure::from(error))))
    }

    /// Verify an idle session against the declared policy without resetting
    /// it, with the same redacted failure shape as [`Self::reset_and_apply`].
    pub(crate) async fn verify_idle(
        &self,
        connection: &mut PgConnection,
    ) -> Result<(), sqlx::Error> {
        async {
            if !self.settings.is_empty() {
                // Reads alone cannot prove assignment authority for privileged
                // extension settings. Reassign current values using native
                // permission/check hooks; using declared values could hide drift.
                // Complete the batch before verifying every policy value, since
                // extension assignment hooks can affect other session settings.
                sqlx::query(include_str!("profile/revalidate_settings.sql"))
                    .bind(self.settings.keys().collect::<Vec<_>>())
                    .execute(&mut *connection)
                    .await?;
            }
            self.verify(connection).await
        }
        .await
        .map_err(|error| sqlx::Error::Configuration(Box::new(crate::SqlxFailure::from(error))))
    }

    pub(crate) async fn apply(&self, connection: &mut PgConnection) -> Result<(), sqlx::Error> {
        // Function EXECUTE permissions are checked before a SELECT evaluates
        // any rows. Switch natively first so both set_config execution and GUC
        // assignment permissions belong to the effective role, even when the
        // login can SET that role without inheriting its privileges.
        let role = format!("SET ROLE {}", quote(&self.effective_role));
        sqlx::raw_sql(sqlx::AssertSqlSafe(role))
            .execute(&mut *connection)
            .await?;
        // The remaining policy uses one fixed-width, array-bound statement.
        // Verification checks the complete session, login included, before work.
        let mut assignments: Vec<(&str, String)> = Vec::with_capacity(9 + self.settings.len());
        assignments.push(("search_path", self.search_path.clone()));
        for (key, value) in self.timeouts.as_pairs() {
            assignments.push((key, format!("{value}ms")));
        }
        for (key, value) in [
            ("default_transaction_isolation", "read committed"),
            ("default_transaction_read_only", "off"),
            ("default_transaction_deferrable", "off"),
            ("row_security", "on"),
        ] {
            assignments.push((key, value.to_owned()));
        }
        for (key, value) in &self.settings {
            assignments.push((key.as_str(), value.clone()));
        }
        let (names, values): (Vec<_>, Vec<_>) = assignments.into_iter().unzip();
        sqlx::query(include_str!("profile/apply.sql"))
            .bind(names)
            .bind(values)
            .execute(&mut *connection)
            .await?;
        self.verify(connection).await
    }
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
fn mismatch() -> sqlx::Error {
    sqlx::Error::Configuration(Box::new(PgProfileError(
        "PostgreSQL session does not match declared profile",
    )))
}
const fn timeout_ms(value: Duration) -> Result<u32, PgProfileError> {
    let ms = value.as_millis();
    if ms > i32::MAX as u128 || value.as_nanos() != ms * 1_000_000 {
        return Err(PgProfileError(
            "profile timeout must be whole milliseconds within PostgreSQL range",
        ));
    }
    Ok(ms as u32)
}
