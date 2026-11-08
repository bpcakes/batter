//! What an operator reads: every key the store holds or a retained catalog
//! declares, and the check every command runs first.

use crate::{
    ErrorClass, PooledStore, REQUIRED_SCHEMA_REVISION, StoreError, StoredState,
    errors::check_primary,
    load::{snapshot_error, version},
};
use batter_flags_core::{FlagKey, Version};
use batter_sqlx::PgReadOnlySnapshot;
use sqlx::{Row, postgres::PgRow};
use tokio::time::Instant;

/// What the store holds for a key.
#[derive(Clone, Debug, PartialEq)]
pub enum Stored {
    /// No row: the key's revision is 0.
    Absent,
    /// An active row and its state.
    Active(StoredState),
    /// An archived row; it loads as absent.
    Archived(StoredState),
}

/// Whether a retained catalog declares a key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Declaration {
    /// No retained catalog declares it.
    Undeclared,
    /// At least one retained catalog does.
    Retained {
        /// Whether any declaring catalog requires coordinated flips.
        coordinated: Coordination,
        /// The earliest expiry a declaring catalog records, as `YYYY-MM-DD`.
        expiry: Expiry,
        /// Whether a process row references a declaring catalog.
        reference: Reference,
    },
}

/// Whether a flip may be rolled out poll by poll.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Coordination {
    /// Every declaring catalog allows a rolling flip.
    Rolling,
    /// A declaring catalog requires every process to flip together.
    Coordinated,
}

/// A declared release flag's expiry on the server's date.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Expiry {
    /// No declaring catalog records an expiry.
    None,
    /// Not yet past.
    On(String),
    /// Past on the server's date.
    Passed(String),
}

/// Whether a declaration is referenced by a process row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Reference {
    /// A process row, of any state, references a declaring catalog.
    Referenced,
    /// Only catalogs no process row references declare the key.
    Orphaned,
}

/// One key as an operator sees it.
#[derive(Clone, Debug)]
pub struct KeyListing {
    /// The key.
    pub key: String,
    /// The stored kind, else the newest declaring catalog's.
    pub kind: String,
    /// The stored value type, else the newest declaring catalog's.
    pub value_type: String,
    /// The row's `updated_version`, 0 for an absent row; the value
    /// `UnchangedSince` takes.
    pub revision: Version,
    /// What the store holds.
    pub stored: Stored,
    /// What the retained catalogs say.
    pub declaration: Declaration,
}

impl PooledStore {
    /// Refuses a standby and a schema older than this build requires, by one
    /// plain read with no explicit lock. An absent schema reads as revision 0.
    pub async fn preflight(&self, deadline: Instant) -> Result<(), StoreError> {
        let deadline = deadline.min(Instant::now() + self.timing.acquire);
        PgReadOnlySnapshot::inspect_until(&self.pool, deadline, async |sql| {
            let [statement, lock, idle] = self.timing.settings(deadline)?;
            sqlx::query("SELECT set_config('statement_timeout',$1,true), set_config('lock_timeout',$2,true), set_config('idle_in_transaction_session_timeout',$3,true)")
                .bind(statement).bind(lock).bind(idle).execute(sql.executor()).await?;
            check_primary(sqlx::query_scalar("SELECT pg_is_in_recovery()").fetch_one(sql.executor()).await?)?;
            let found: i32 = sqlx::query_scalar("SELECT schema_revision FROM flags.flag_control WHERE singleton")
                .fetch_one(sql.executor()).await.map_err(|error| match error.as_database_error().and_then(|e| e.code()) {
                    Some(code) if code == "42P01" || code == "3F000" => StoreError::schema_behind(0),
                    _ => StoreError::from(error),
                })?;
            if found < REQUIRED_SCHEMA_REVISION {
                return Err(StoreError::schema_behind(found));
            }
            Ok(())
        })
        .await
        .map_err(snapshot_error)
    }

    /// Every key the store holds or a retained catalog declares, in key
    /// order, read in one snapshot.
    pub async fn keys(&self, deadline: Instant) -> Result<Vec<KeyListing>, StoreError> {
        self.list(None, deadline).await
    }

    /// One key, which is listed even when neither stored nor declared.
    pub async fn key(&self, key: &FlagKey, deadline: Instant) -> Result<KeyListing, StoreError> {
        let mut listed = self.list(Some(key.as_str()), deadline).await?;
        Ok(listed.pop().unwrap_or_else(|| KeyListing {
            key: key.as_str().to_owned(),
            kind: String::new(),
            value_type: String::new(),
            revision: Version::new(0),
            stored: Stored::Absent,
            declaration: Declaration::Undeclared,
        }))
    }

    async fn list(
        &self,
        key: Option<&str>,
        deadline: Instant,
    ) -> Result<Vec<KeyListing>, StoreError> {
        let deadline = deadline.min(Instant::now() + self.timing.acquire);
        PgReadOnlySnapshot::inspect_until(&self.pool, deadline, async |sql| {
            let [statement, lock, idle] = self.timing.settings(deadline)?;
            sqlx::query("SELECT set_config('statement_timeout',$1,true), set_config('lock_timeout',$2,true), set_config('idle_in_transaction_session_timeout',$3,true)")
                .bind(statement).bind(lock).bind(idle).execute(sql.executor()).await?;
            sqlx::query("WITH declared AS ( \
                    SELECT e.key, \
                      bool_or(e.flip_discipline = 'coordinated') AS coordinated, \
                      min(e.expiry) AS expiry, \
                      bool_or(EXISTS (SELECT FROM flags.flag_processes p WHERE p.manifest_digest = e.manifest_digest)) AS referenced, \
                      (array_agg(e.kind ORDER BY c.last_referenced_at DESC, e.manifest_digest))[1] AS kind, \
                      (array_agg(e.value_type ORDER BY c.last_referenced_at DESC, e.manifest_digest))[1] AS value_type \
                    FROM flags.flag_catalog_entries e JOIN flags.flag_catalogs c USING (manifest_digest) \
                    WHERE $1::text IS NULL OR e.key = $1 GROUP BY e.key) \
                SELECT coalesce(s.name, d.key) AS key, coalesce(s.kind, d.kind) AS kind, coalesce(s.value_type, d.value_type) AS value_type, \
                  s.state, coalesce(s.updated_version, 0) AS revision, s.archived_version IS NOT NULL AS archived, s.name IS NOT NULL AS stored, \
                  d.key IS NOT NULL AS declared, coalesce(d.coordinated, false) AS coordinated, d.expiry::text AS expiry, \
                  coalesce(d.expiry < current_date, false) AS expired, coalesce(d.referenced, false) AS referenced \
                FROM (SELECT * FROM flags.flag_state WHERE $1::text IS NULL OR name = $1) s FULL OUTER JOIN declared d ON d.key = s.name \
                ORDER BY 1")
                .bind(key).fetch_all(sql.executor()).await?
                .iter().map(listing).collect::<Result<Vec<_>, StoreError>>()
        })
        .await
        .map_err(snapshot_error)
    }
}

#[allow(
    clippy::fn_params_excessive_bools,
    reason = "private decoding of SQL booleans into the listing's enums"
)]
fn listing(row: &PgRow) -> Result<KeyListing, StoreError> {
    let invalid = || StoreError::classified(ErrorClass::InvalidData);
    let stored = match (
        row.try_get::<bool, _>("stored")?,
        row.try_get::<bool, _>("archived")?,
    ) {
        (false, _) => Stored::Absent,
        (true, false) => Stored::Active(StoredState(row.try_get("state")?)),
        (true, true) => Stored::Archived(StoredState(row.try_get("state")?)),
    };
    let declaration = if row.try_get::<bool, _>("declared")? {
        let expiry = match (
            row.try_get::<Option<String>, _>("expiry")?,
            row.try_get::<bool, _>("expired")?,
        ) {
            (None, _) => Expiry::None,
            (Some(date), false) => Expiry::On(date),
            (Some(date), true) => Expiry::Passed(date),
        };
        Declaration::Retained {
            coordinated: if row.try_get("coordinated")? {
                Coordination::Coordinated
            } else {
                Coordination::Rolling
            },
            expiry,
            reference: if row.try_get("referenced")? {
                Reference::Referenced
            } else {
                Reference::Orphaned
            },
        }
    } else {
        Declaration::Undeclared
    };
    Ok(KeyListing {
        key: row
            .try_get::<Option<String>, _>("key")?
            .ok_or_else(invalid)?,
        kind: row
            .try_get::<Option<String>, _>("kind")?
            .unwrap_or_default(),
        value_type: row
            .try_get::<Option<String>, _>("value_type")?
            .unwrap_or_default(),
        revision: version(row.try_get("revision")?)?,
        stored,
        declaration,
    })
}
