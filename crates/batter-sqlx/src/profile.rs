//! Declarative session policy, not evidence about an arbitrary borrowed resource.
mod validation;

use sqlx::{Connection, PgConnection};
use std::{collections::BTreeMap, fmt, time::Duration};

/// Explicit policy re-established after session reset. Names are quoted as
/// identifiers; setting values are bound parameters. No setup callback can run
/// application SQL before the policy is checked.
///
/// Search-path schemas are a trust declaration: only include schemas whose
/// object creators you trust. `pg_catalog` is implicitly searched first and
/// `pg_temp` is explicitly last. The first supplied schema receives unqualified
/// DDL. All supplied schemas must exist and be usable by the effective role.
///
/// ```no_run
/// # async fn example(pool: &sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
/// use batter_sqlx::{PgSessionProfile, run_atomic_profiled};
/// use std::time::Duration;
/// let profile = PgSessionProfile::with_timeouts("login", "serving", vec!["application".into()],
///     Duration::from_secs(30), Duration::from_secs(5),
///     Duration::from_secs(10), Duration::from_secs(60))?
///     .with_setting("app.tenant", "tenant-one")?;
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
    statement_timeout_ms: u32,
    lock_timeout_ms: u32,
    transaction_timeouts_ms: Option<(u32, u32)>,
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
    /// Declare all four server timeouts, roles and an ordered trusted schema path.
    /// Each timeout must be an exact number of milliseconds in `0..=i32::MAX`.
    /// Zero explicitly disables that timeout, even over a nonzero startup default.
    /// All four values are applied after reset and checked at scope boundaries.
    /// See the type's example for the canonical construction path.
    ///
    /// `idle_in_transaction_session_timeout` bounds each idle interval inside a
    /// transaction; `transaction_timeout` bounds the entire transaction. The
    /// latter requires PostgreSQL 17 or later, including when set to zero. An
    /// unsupported setting fails setup before application access. Native tests
    /// use PostgreSQL 18. PostgreSQL ignores a statement or idle timeout when a
    /// nonzero transaction timeout is shorter or equal; relative values are
    /// application policy, not parent/child operation deadlines.
    ///
    /// Server termination does not cancel Rust work or release a held pool lease.
    /// Use [`crate::run_atomic_profiled_in`] for a cooperative operation budget.
    /// Arbitrary SQL can alter settings before revalidation; this is not a sandbox.
    pub fn with_timeouts(
        login_role: impl Into<String>,
        effective_role: impl Into<String>,
        schemas: Vec<String>,
        statement_timeout: Duration,
        lock_timeout: Duration,
        idle_in_transaction_session_timeout: Duration,
        transaction_timeout: Duration,
    ) -> Result<Self, PgProfileError> {
        let mut profile = Self::new(
            login_role,
            effective_role,
            schemas,
            statement_timeout,
            lock_timeout,
        )?;
        profile.transaction_timeouts_ms = Some((
            timeout_ms(idle_in_transaction_session_timeout)?,
            timeout_ms(transaction_timeout)?,
        ));
        Ok(profile)
    }

    /// Compatibility constructor declaring only statement and lock timeouts.
    /// Prefer [`Self::with_timeouts`] to declare and verify all four timeouts.
    ///
    /// This weaker path leaves both transaction timeouts at their reset defaults
    /// and never verifies them. Startup connection options and role/database
    /// defaults survive reset; later `SET` values and pool hooks are not retained.
    /// It does not require PostgreSQL's `transaction_timeout` parameter to exist.
    /// A zero statement or lock timeout explicitly disables that timeout. No
    /// policy is inferred from SQLx hooks or the authenticated user's privileges.
    pub fn new(
        login_role: impl Into<String>,
        effective_role: impl Into<String>,
        schemas: Vec<String>,
        statement_timeout: Duration,
        lock_timeout: Duration,
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
            statement_timeout_ms: timeout_ms(statement_timeout)?,
            lock_timeout_ms: timeout_ms(lock_timeout)?,
            transaction_timeouts_ms: None,
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

    pub(crate) async fn apply(&self, connection: &mut PgConnection) -> Result<(), sqlx::Error> {
        let login: String = sqlx::query_scalar("SELECT session_user::text")
            .fetch_one(&mut *connection)
            .await?;
        if login != self.login_role {
            return Err(mismatch());
        }
        let role = format!("SET ROLE {}", quote(&self.effective_role));
        sqlx::raw_sql(sqlx::AssertSqlSafe(role))
            .execute(&mut *connection)
            .await?;
        set(connection, "search_path", &self.search_path).await?;
        for (key, value) in self.timeouts() {
            set(connection, key, &format!("{value}ms")).await?;
        }
        for (key, value) in [
            ("default_transaction_isolation", "read committed"),
            ("default_transaction_read_only", "off"),
            ("default_transaction_deferrable", "off"),
            ("row_security", "on"),
        ] {
            set(connection, key, value).await?;
        }
        for (key, value) in &self.settings {
            set(connection, key, value).await?;
        }
        self.verify(connection).await
    }

    fn timeouts(&self) -> impl Iterator<Item = (&'static str, u32)> {
        [
            ("statement_timeout", self.statement_timeout_ms),
            ("lock_timeout", self.lock_timeout_ms),
        ]
        .into_iter()
        .chain(
            self.transaction_timeouts_ms
                .into_iter()
                .flat_map(|(idle, total)| {
                    [
                        ("idle_in_transaction_session_timeout", idle),
                        ("transaction_timeout", total),
                    ]
                }),
        )
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
fn timeout_ms(value: Duration) -> Result<u32, PgProfileError> {
    let ms = value.as_millis();
    if ms > i32::MAX as u128 || Duration::from_millis(ms as u64) != value {
        return Err(PgProfileError(
            "profile timeout must be whole milliseconds within PostgreSQL range",
        ));
    }
    Ok(ms as u32)
}

async fn set(connection: &mut PgConnection, key: &str, value: &str) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_catalog.set_config($1, $2, false)")
        .bind(key)
        .bind(value)
        .execute(connection)
        .await?;
    Ok(())
}
