use super::{PgSessionProfile, mismatch};
use sqlx::{PgConnection, Row, postgres::PgRow};

// All declared profile facts and atomic continuity come from one statement.
// Arrays preserve declaration order and missing timeout rows remain explicit;
// an empty custom-setting or schema result cannot make a partial check pass.
struct Observation {
    login: String,
    role: String,
    path: String,
    schemas: Vec<bool>,
    timeouts: Vec<Option<i64>>,
    settings: Vec<String>,
    xid: Option<String>,
    isolation: String,
    read_only: String,
}

impl sqlx::FromRow<'_, PgRow> for Observation {
    fn from_row(row: &PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            login: row.try_get("login")?,
            role: row.try_get("role")?,
            path: row.try_get("path")?,
            schemas: row.try_get("schemas")?,
            timeouts: row.try_get("timeouts")?,
            settings: row.try_get("settings")?,
            xid: row.try_get("xid")?,
            isolation: row.try_get("isolation")?,
            read_only: row.try_get("read_only")?,
        })
    }
}

impl PgSessionProfile {
    pub(crate) async fn verify(&self, connection: &mut PgConnection) -> Result<(), sqlx::Error> {
        self.verify_transaction(connection).await.map(|_| ())
    }

    pub(crate) async fn verify_transaction(
        &self,
        connection: &mut PgConnection,
    ) -> Result<(Option<String>, String, String), sqlx::Error> {
        let (timeout_names, timeout_values): (Vec<_>, Vec<_>) = self.timeouts().unzip();
        let (setting_names, setting_values): (Vec<_>, Vec<_>) = [
            ("default_transaction_isolation", "read committed"),
            ("default_transaction_read_only", "off"),
            ("default_transaction_deferrable", "off"),
            ("row_security", "on"),
        ]
        .into_iter()
        .chain(
            self.settings
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
        )
        .unzip();
        let observed: Observation = sqlx::query_as(
            "SELECT session_user::text AS login, current_user::text AS role, \
             pg_catalog.current_setting('search_path') AS path, \
             ARRAY(SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_namespace n \
                 WHERE n.nspname::text = declared.name \
                 AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')) \
                 FROM pg_catalog.unnest($1::text[]) WITH ORDINALITY AS declared(name, position) \
                 ORDER BY declared.position) AS schemas, \
             ARRAY(SELECT actual.setting::bigint \
                 FROM pg_catalog.unnest($2::text[]) WITH ORDINALITY AS declared(name, position) \
                 LEFT JOIN pg_catalog.pg_settings actual ON actual.name = declared.name \
                 ORDER BY declared.position) AS timeouts, \
             ARRAY(SELECT pg_catalog.current_setting(declared.name) \
                 FROM pg_catalog.unnest($3::text[]) WITH ORDINALITY AS declared(name, position) \
                 ORDER BY declared.position) AS settings, \
             pg_catalog.pg_current_xact_id_if_assigned()::text AS xid, \
             pg_catalog.current_setting('transaction_isolation') AS isolation, \
             pg_catalog.current_setting('transaction_read_only') AS read_only",
        )
        .bind(&self.schemas)
        .bind(timeout_names)
        .bind(setting_names)
        .fetch_one(connection)
        .await?;
        if observed.login != self.login_role
            || observed.role != self.effective_role
            || observed.path != self.search_path
            || observed.schemas.len() != self.schemas.len()
            || observed.schemas.iter().any(|usable| !usable)
        {
            return Err(mismatch());
        }
        // Match the old fetch_one failure for an absent declared timeout, rather
        // than silently omitting the setting through an inner join.
        if observed.timeouts.iter().any(Option::is_none) {
            return Err(sqlx::Error::RowNotFound);
        }
        if observed.timeouts.len() != timeout_values.len()
            || observed
                .timeouts
                .iter()
                .zip(timeout_values)
                .any(|(actual, expected)| *actual != Some(i64::from(expected)))
            || observed.settings != setting_values
        {
            return Err(mismatch());
        }
        Ok((observed.xid, observed.isolation, observed.read_only))
    }
}
