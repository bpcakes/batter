//! Catalog reads shared by managed-object apply and verification.
//!
//! Both phases must reach the same conclusions from the same catalog state, so
//! identity resolution, ledger shape and definition rendering live here once.
//! Identities are resolved by catalog joins rather than `regprocedure` input,
//! so no schema `USAGE` is needed and no search path can redirect a name.

use super::{Identity, MAX_MANAGED_OBJECTS, ManagedObjectKind, ManagedObjectManifest};
use crate::verification::{QualifiedName, RoutineSignature, RoutineType};
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use std::collections::{HashMap, HashSet};

/// Rows one catalog read may return before capacity is exceeded. Routine
/// resolution returns every overload of every declared or retired name, so this
/// is ten times the manifest bound.
const MAX_CATALOG_ROWS: usize = 100_000;

/// A bounded catalog read failed natively or exceeded its row capacity.
#[derive(Debug)]
pub(crate) enum CatalogError {
    Native(sqlx::Error),
    Capacity,
}

impl From<sqlx::Error> for CatalogError {
    fn from(error: sqlx::Error) -> Self {
        Self::Native(error)
    }
}

/// Session settings that change `pg_get_functiondef`/`pg_get_triggerdef` text.
///
/// Type and function names are qualified relative to `search_path`; constants
/// in defaults, bodies and `WHEN` clauses render through date, interval, time
/// zone, float, money and string settings; `quote_all_identifiers` changes
/// quoting. Apply and verification render under these fixed values so session
/// or role defaults cannot turn an unchanged definition into drift. Apply also
/// runs every library-built statement under them, so a session search path
/// cannot redirect a catalog helper; only application sources run under the
/// session's own values.
const RENDER_SETTINGS: [(&str, &str); 9] = [
    ("search_path", "pg_catalog, pg_temp"),
    ("datestyle", "ISO, YMD"),
    ("intervalstyle", "postgres"),
    ("timezone", "UTC"),
    ("extra_float_digits", "1"),
    ("bytea_output", "hex"),
    ("standard_conforming_strings", "on"),
    ("quote_all_identifiers", "off"),
    ("lc_monetary", "C"),
];

/// Transaction-local canonical settings and the session values they replaced.
#[must_use = "restore the saved settings before running application SQL"]
pub(crate) struct RenderSettings {
    saved: Vec<String>,
}

impl RenderSettings {
    /// Save the current values and switch to the canonical ones.
    pub(crate) async fn apply(connection: &mut PgConnection) -> Result<Self, sqlx::Error> {
        let names = RENDER_SETTINGS.map(|(name, _)| name);
        let saved = sqlx::query_scalar::<_, Vec<String>>(
            "SELECT ARRAY(
                 SELECT pg_catalog.current_setting(setting.name)
                 FROM pg_catalog.unnest($1::pg_catalog.text[])
                      WITH ORDINALITY AS setting(name, position)
                 ORDER BY setting.position)",
        )
        .bind(names.as_slice())
        .fetch_one(&mut *connection)
        .await?;
        let settings = Self { saved };
        settings.reapply(connection).await?;
        Ok(settings)
    }

    /// Switch back to the canonical values after [`Self::restore`].
    pub(crate) async fn reapply(&self, connection: &mut PgConnection) -> Result<(), sqlx::Error> {
        set_local(connection, &RENDER_SETTINGS.map(|(_, value)| value)).await
    }

    /// Return to the saved session values, for example to run application SQL.
    pub(crate) async fn restore(&self, connection: &mut PgConnection) -> Result<(), sqlx::Error> {
        set_local(connection, &self.saved).await
    }
}

async fn set_local(
    connection: &mut PgConnection,
    values: &[impl AsRef<str>],
) -> Result<(), sqlx::Error> {
    let names = RENDER_SETTINGS.map(|(name, _)| name);
    let values = values.iter().map(AsRef::as_ref).collect::<Vec<_>>();
    sqlx::query(
        "SELECT pg_catalog.set_config(setting.name, setting.value, true)
         FROM ROWS FROM (pg_catalog.unnest($1::pg_catalog.text[]),
                         pg_catalog.unnest($2::pg_catalog.text[]))
              AS setting(name, value)",
    )
    .bind(names.as_slice())
    .bind(values)
    .execute(connection)
    .await
    .map(|_| ())
}

/// The exact ledger contract, or the first reason a relation violates it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LedgerShape {
    Exact,
    /// Row-level security could hide or filter ledger rows.
    RowSecurity,
    /// The relation is an inheritance parent or child.
    Inherited,
    /// The relation kind, columns, types, nullability or primary key differ.
    Mismatch,
}

pub(crate) const LEDGER_COLUMNS: [(&str, &str); 5] = [
    ("identity", "text"),
    ("kind", "text"),
    ("source_checksum", "bytea"),
    ("rendered_checksum", "bytea"),
    ("applied_at", "timestamptz"),
];

pub(crate) fn create_ledger_sql(ledger: &QualifiedName) -> String {
    format!(
        "CREATE TABLE {} (
             identity pg_catalog.text PRIMARY KEY,
             kind pg_catalog.text NOT NULL,
             source_checksum pg_catalog.bytea NOT NULL,
             rendered_checksum pg_catalog.bytea NOT NULL,
             applied_at pg_catalog.timestamptz NOT NULL)",
        ledger.quoted()
    )
}

/// Resolve a relation's oid and `relkind` by exact schema and name in the
/// current catalog snapshot: the managed ledger, or a managed trigger's table.
pub(crate) async fn named_relation(
    connection: &mut PgConnection,
    ledger: &QualifiedName,
) -> Result<Option<(i64, String)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT c.oid::bigint, c.relkind::text
         FROM pg_catalog.pg_class AS c
         JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
         WHERE n.nspname = $1 AND c.relname = $2",
    )
    .bind(ledger.schema())
    .bind(ledger.name())
    .fetch_optional(connection)
    .await
}

pub(crate) async fn inspect_ledger_shape(
    connection: &mut PgConnection,
    oid: i64,
) -> Result<LedgerShape, sqlx::Error> {
    let metadata = sqlx::query_as::<_, (String, bool, bool)>(
        "SELECT c.relkind::text, c.relrowsecurity,
                EXISTS (SELECT 1 FROM pg_catalog.pg_inherits AS i
                        WHERE i.inhparent = c.oid OR i.inhrelid = c.oid)
         FROM pg_catalog.pg_class AS c
         WHERE c.oid = $1::bigint::oid",
    )
    .bind(oid)
    .fetch_optional(&mut *connection)
    .await?;
    let Some((kind, row_security, inherited)) = metadata else {
        return Ok(LedgerShape::Mismatch);
    };
    if kind != "r" {
        return Ok(LedgerShape::Mismatch);
    }
    if row_security {
        return Ok(LedgerShape::RowSecurity);
    }
    if inherited {
        return Ok(LedgerShape::Inherited);
    }
    let columns = sqlx::query_as::<_, (String, String, String, bool)>(
        "SELECT a.attname::text, n.nspname::text, t.typname::text, a.attnotnull
         FROM pg_catalog.pg_attribute AS a
         JOIN pg_catalog.pg_type AS t ON t.oid = a.atttypid
         JOIN pg_catalog.pg_namespace AS n ON n.oid = t.typnamespace
         WHERE a.attrelid = $1::bigint::oid AND a.attnum > 0 AND NOT a.attisdropped
         LIMIT 6",
    )
    .bind(oid)
    .fetch_all(&mut *connection)
    .await?;
    let primary_keys = sqlx::query_scalar::<_, Vec<String>>(
        "SELECT ARRAY(
             SELECT a.attname::text
             FROM unnest(c.conkey) WITH ORDINALITY AS key(attnum, position)
             JOIN pg_catalog.pg_attribute AS a
               ON a.attrelid = c.conrelid AND a.attnum = key.attnum
             ORDER BY key.position)
         FROM pg_catalog.pg_constraint AS c
         WHERE c.conrelid = $1::bigint::oid AND c.contype = 'p'
         LIMIT 2",
    )
    .bind(oid)
    .fetch_all(connection)
    .await?;
    Ok(if shape_matches(&columns, &primary_keys) {
        LedgerShape::Exact
    } else {
        LedgerShape::Mismatch
    })
}

pub(crate) fn shape_matches(
    columns: &[(String, String, String, bool)],
    primary_keys: &[Vec<String>],
) -> bool {
    columns.len() == LEDGER_COLUMNS.len()
        && LEDGER_COLUMNS.iter().all(|(name, ty)| {
            columns.iter().any(|(column, schema, actual, not_null)| {
                column == name && schema == "pg_catalog" && actual == ty && *not_null
            })
        })
        && primary_keys == [vec!["identity".to_owned()]]
}

/// One ledger row as written by apply.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LedgerRow {
    pub(crate) identity: String,
    pub(crate) kind: String,
    pub(crate) source_checksum: Vec<u8>,
    pub(crate) rendered_checksum: Vec<u8>,
}

/// Read every ledger row, bounded by the manifest capacity plus the set row.
pub(crate) async fn load_ledger_rows(
    connection: &mut PgConnection,
    ledger: &QualifiedName,
) -> Result<Vec<LedgerRow>, CatalogError> {
    let limit = MAX_MANAGED_OBJECTS + 1;
    let rows =
        sqlx::query_as::<_, (String, String, Vec<u8>, Vec<u8>)>(sqlx::AssertSqlSafe(format!(
            "SELECT identity, kind, source_checksum, rendered_checksum
             FROM ONLY {} LIMIT {}",
            ledger.quoted(),
            limit + 1
        )))
        .persistent(false)
        .fetch_all(connection)
        .await?;
    if rows.len() > limit {
        return Err(CatalogError::Capacity);
    }
    Ok(rows
        .into_iter()
        .map(
            |(identity, kind, source_checksum, rendered_checksum)| LedgerRow {
                identity,
                kind,
                source_checksum,
                rendered_checksum,
            },
        )
        .collect())
}

/// One routine or trigger found in the catalog for a declared identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LiveObject {
    pub(crate) oid: i64,
    pub(crate) kind: Option<ManagedObjectKind>,
    /// Written by the current transaction. Always false when not requested.
    pub(crate) touched: bool,
}

/// Whether resolution also reports rows the current transaction wrote.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Touched {
    Report,
    Ignore,
}

type RoutineRow = (
    i64,
    String,
    String,
    String,
    bool,
    Vec<String>,
    Vec<String>,
    Vec<bool>,
);

/// Resolve every routine named by `(schema, name)` and key it by canonical identity.
pub(crate) async fn resolve_routines(
    connection: &mut PgConnection,
    names: &[(&str, &str)],
    touched: Touched,
) -> Result<HashMap<String, LiveObject>, CatalogError> {
    if names.is_empty() {
        return Ok(HashMap::new());
    }
    let schemas = names.iter().map(|(schema, _)| *schema).collect::<Vec<_>>();
    let routines = names.iter().map(|(_, name)| *name).collect::<Vec<_>>();
    let query = match touched {
        Touched::Report => ROUTINES_TOUCHED,
        Touched::Ignore => ROUTINES,
    };
    let rows = sqlx::query_as::<_, RoutineRow>(query)
        .bind(schemas)
        .bind(routines)
        .fetch_all(connection)
        .await?;
    if rows.len() > MAX_CATALOG_ROWS {
        return Err(CatalogError::Capacity);
    }
    let mut resolved = HashMap::with_capacity(rows.len());
    for (oid, schema, name, prokind, touched, arg_schemas, arg_names, arg_arrays) in rows {
        let arguments = arg_schemas
            .into_iter()
            .zip(arg_names)
            .zip(arg_arrays)
            .map(|((schema, name), array)| {
                if array {
                    RoutineType::array(schema, name)
                } else {
                    RoutineType::new(schema, name)
                }
            })
            .collect::<Result<Vec<_>, _>>();
        // Catalog names outside the bounded identifier model cannot equal a
        // declared identity, so they are simply not candidates.
        let Ok(signature) = arguments.and_then(|args| RoutineSignature::new(schema, name, args))
        else {
            continue;
        };
        let kind = match prokind.as_str() {
            "f" => Some(ManagedObjectKind::Function),
            "p" => Some(ManagedObjectKind::Procedure),
            _ => None,
        };
        resolved.insert(
            signature.as_str().to_owned(),
            LiveObject { oid, kind, touched },
        );
    }
    Ok(resolved)
}

macro_rules! concat_routines {
    ($touched:literal) => {
        concat!(
            "SELECT p.oid::bigint, n.nspname::text, p.proname::text, p.prokind::text, ",
            $touched,
            ",
                    ARRAY(SELECT COALESCE(en.nspname, tn.nspname)::text
                          FROM unnest(p.proargtypes::pg_catalog.oid[]) WITH ORDINALITY AS a(type_oid, position)
                          JOIN pg_catalog.pg_type AS t ON t.oid = a.type_oid
                          JOIN pg_catalog.pg_namespace AS tn ON tn.oid = t.typnamespace
                          LEFT JOIN pg_catalog.pg_type AS e
                            ON e.oid = t.typelem AND e.typarray = t.oid
                           AND t.typsubscript = 'pg_catalog.array_subscript_handler'::pg_catalog.regproc
                          LEFT JOIN pg_catalog.pg_namespace AS en ON en.oid = e.typnamespace
                          ORDER BY a.position),
                    ARRAY(SELECT COALESCE(e.typname, t.typname)::text
                          FROM unnest(p.proargtypes::pg_catalog.oid[]) WITH ORDINALITY AS a(type_oid, position)
                          JOIN pg_catalog.pg_type AS t ON t.oid = a.type_oid
                          LEFT JOIN pg_catalog.pg_type AS e
                            ON e.oid = t.typelem AND e.typarray = t.oid
                           AND t.typsubscript = 'pg_catalog.array_subscript_handler'::pg_catalog.regproc
                          ORDER BY a.position),
                    ARRAY(SELECT e.oid IS NOT NULL
                          FROM unnest(p.proargtypes::pg_catalog.oid[]) WITH ORDINALITY AS a(type_oid, position)
                          JOIN pg_catalog.pg_type AS t ON t.oid = a.type_oid
                          LEFT JOIN pg_catalog.pg_type AS e
                            ON e.oid = t.typelem AND e.typarray = t.oid
                           AND t.typsubscript = 'pg_catalog.array_subscript_handler'::pg_catalog.regproc
                          ORDER BY a.position)
             FROM pg_catalog.pg_proc AS p
             JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
             WHERE (n.nspname, p.proname) IN (
                       SELECT requested.schema_name, requested.routine_name
                       FROM unnest($1::text[], $2::text[]) AS requested(schema_name, routine_name))
             LIMIT 100001"
        )
    };
}

const ROUTINES: &str = concat_routines!("false");
const ROUTINES_TOUCHED: &str = concat_routines!("p.xmin = pg_catalog.pg_current_xact_id()::xid");

/// Resolve every named user trigger and key it by canonical identity.
///
/// A trigger with a `pg_constraint` row (`tgconstraint <> 0`) is a constraint
/// trigger; internal triggers, such as those enforcing foreign keys, are
/// excluded.
pub(crate) async fn resolve_triggers(
    connection: &mut PgConnection,
    keys: &[(&str, &str, &str)],
    touched: Touched,
) -> Result<HashMap<String, LiveObject>, CatalogError> {
    if keys.is_empty() {
        return Ok(HashMap::new());
    }
    let schemas = keys.iter().map(|key| key.0).collect::<Vec<_>>();
    let tables = keys.iter().map(|key| key.1).collect::<Vec<_>>();
    let names = keys.iter().map(|key| key.2).collect::<Vec<_>>();
    let query = match touched {
        Touched::Report => TRIGGERS_TOUCHED,
        Touched::Ignore => TRIGGERS,
    };
    let rows = sqlx::query_as::<_, (i64, String, String, String, bool, bool)>(query)
        .bind(schemas)
        .bind(tables)
        .bind(names)
        .fetch_all(connection)
        .await?;
    if rows.len() > MAX_CATALOG_ROWS {
        return Err(CatalogError::Capacity);
    }
    let mut resolved = HashMap::with_capacity(rows.len());
    for (oid, schema, table, name, constraint, touched) in rows {
        let (Ok(table), Ok(name)) = (
            QualifiedName::new(schema, table),
            crate::verification::Identifier::new(name),
        ) else {
            continue;
        };
        let identity = Identity::Trigger { table, name }.render();
        let kind = if constraint {
            ManagedObjectKind::ConstraintTrigger
        } else {
            ManagedObjectKind::Trigger
        };
        resolved.insert(
            identity,
            LiveObject {
                oid,
                kind: Some(kind),
                touched,
            },
        );
    }
    Ok(resolved)
}

macro_rules! concat_triggers {
    ($touched:literal) => {
        concat!(
            "SELECT t.oid::bigint, n.nspname::text, c.relname::text, t.tgname::text,
                    t.tgconstraint <> 0, ",
            $touched,
            "
             FROM pg_catalog.pg_trigger AS t
             JOIN pg_catalog.pg_class AS c ON c.oid = t.tgrelid
             JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
             WHERE NOT t.tgisinternal
               AND (n.nspname, c.relname, t.tgname) IN (
                       SELECT requested.schema_name, requested.table_name, requested.trigger_name
                       FROM unnest($1::text[], $2::text[], $3::text[])
                            AS requested(schema_name, table_name, trigger_name))
             LIMIT 100001"
        )
    };
}

const TRIGGERS: &str = concat_triggers!("false");
const TRIGGERS_TOUCHED: &str = concat_triggers!("t.xmin = pg_catalog.pg_current_xact_id()::xid");

/// The tables and partitioned tables among `names`, with their `relkind`,
/// ordered so that every partitioned table precedes its partitions: by how
/// many relations `pg_partition_ancestors` lists (a partition with its
/// ancestors, nothing for a table outside any partition tree), then by name.
/// Views and foreign tables, which `LOCK TABLE` would extend to base relations
/// or refuse, are left out.
pub(crate) async fn lockable_tables_in_lock_order(
    connection: &mut PgConnection,
    names: &[(&str, &str)],
) -> Result<Vec<(String, String, String)>, sqlx::Error> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let schemas = names.iter().map(|(schema, _)| *schema).collect::<Vec<_>>();
    let tables = names.iter().map(|(_, table)| *table).collect::<Vec<_>>();
    sqlx::query_as(
        "SELECT n.nspname::text, c.relname::text, c.relkind::text
         FROM pg_catalog.pg_class AS c
         JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
         WHERE c.relkind IN ('r', 'p')
           AND (n.nspname, c.relname) IN (
                   SELECT requested.schema_name, requested.table_name
                   FROM unnest($1::text[], $2::text[]) AS requested(schema_name, table_name))
         ORDER BY (SELECT pg_catalog.count(*)
                   FROM pg_catalog.pg_partition_ancestors(c.oid::pg_catalog.regclass)),
                  n.nspname, c.relname",
    )
    .bind(schemas)
    .bind(tables)
    .fetch_all(connection)
    .await
}

/// Resolve every declared entry and retirement of `manifest`.
pub(crate) async fn resolve_live(
    connection: &mut PgConnection,
    manifest: &ManagedObjectManifest,
) -> Result<HashMap<String, LiveObject>, CatalogError> {
    let identities = manifest
        .objects()
        .iter()
        .map(super::ManagedObject::structural_identity)
        .chain(
            manifest
                .retirements()
                .iter()
                .map(super::Retirement::structural_identity),
        );
    let mut routines = HashSet::new();
    let mut triggers = HashSet::new();
    for identity in identities {
        match identity {
            Identity::Routine(signature) => {
                routines.insert((signature.schema(), signature.name()));
            }
            Identity::Trigger { table, name } => {
                triggers.insert((table.schema(), table.name(), name.as_str()));
            }
        }
    }
    let routines = routines.into_iter().collect::<Vec<_>>();
    let triggers = triggers.into_iter().collect::<Vec<_>>();
    let mut live = resolve_routines(connection, &routines, Touched::Ignore).await?;
    live.extend(resolve_triggers(connection, &triggers, Touched::Ignore).await?);
    Ok(live)
}

/// SHA-256 of each object's rendered definition, under [`RenderSettings`].
pub(crate) async fn render_hashes(
    connection: &mut PgConnection,
    objects: impl IntoIterator<Item = (i64, ManagedObjectKind)>,
) -> Result<HashMap<i64, [u8; 32]>, CatalogError> {
    let (mut routines, mut triggers) = (Vec::new(), Vec::new());
    for (oid, kind) in objects {
        match kind {
            ManagedObjectKind::Function | ManagedObjectKind::Procedure => routines.push(oid),
            ManagedObjectKind::Trigger | ManagedObjectKind::ConstraintTrigger => triggers.push(oid),
        }
    }
    let mut hashes = HashMap::with_capacity(routines.len() + triggers.len());
    for (oids, query) in [
        (
            routines,
            "SELECT r.oid, pg_catalog.sha256(pg_catalog.convert_to(
                 pg_catalog.pg_get_functiondef(r.oid::pg_catalog.oid), 'UTF8'))
             FROM unnest($1::bigint[]) AS r(oid)",
        ),
        (
            triggers,
            // pg_get_triggerdef omits whether the trigger fires; apply's
            // CREATE OR REPLACE TRIGGER and CREATE CONSTRAINT TRIGGER always
            // leave it enabled ('O'). Constraint triggers render their
            // deferrability, FROM table and WHEN clause in the definition.
            "SELECT r.oid, pg_catalog.sha256(pg_catalog.convert_to(
                 pg_catalog.pg_get_triggerdef(t.oid) || E'\\nenabled=' || t.tgenabled::pg_catalog.text,
                 'UTF8'))
             FROM unnest($1::bigint[]) AS r(oid)
             JOIN pg_catalog.pg_trigger AS t ON t.oid = r.oid::pg_catalog.oid",
        ),
    ] {
        if oids.is_empty() {
            continue;
        }
        let rows = sqlx::query_as::<_, (i64, Option<Vec<u8>>)>(query)
            .bind(oids)
            .fetch_all(&mut *connection)
            .await?;
        for (oid, hash) in rows {
            if let Some(hash) = hash.and_then(|hash| <[u8; 32]>::try_from(hash).ok()) {
                hashes.insert(oid, hash);
            }
        }
    }
    Ok(hashes)
}

/// Routines among `oids` on which `PUBLIC` holds `EXECUTE`, including by default.
pub(crate) async fn public_execute_holders(
    connection: &mut PgConnection,
    oids: &[i64],
) -> Result<HashSet<i64>, sqlx::Error> {
    if oids.is_empty() {
        return Ok(HashSet::new());
    }
    let rows = sqlx::query_scalar::<_, i64>(
        "SELECT DISTINCT p.oid::bigint
         FROM pg_catalog.pg_proc AS p
         CROSS JOIN LATERAL pg_catalog.aclexplode(
             COALESCE(p.proacl, pg_catalog.acldefault('f'::\"char\", p.proowner))) AS acl
         WHERE p.oid = ANY($1::bigint[]::pg_catalog.oid[])
           AND acl.grantee = 0 AND acl.privilege_type = 'EXECUTE'",
    )
    .bind(oids)
    .fetch_all(connection)
    .await?;
    Ok(rows.into_iter().collect())
}

/// Digest of every entry's rendered definition hash, in sorted identity order.
pub(crate) fn rendered_set_digest<'a>(
    hashes: impl IntoIterator<Item = (&'a str, &'a [u8])>,
) -> [u8; 32] {
    let mut sorted = hashes.into_iter().collect::<Vec<_>>();
    sorted.sort_unstable();
    let mut hasher = Sha256::new();
    hasher.update(b"batter.managed-objects.rendered.v1\0");
    hasher.update((sorted.len() as u64).to_be_bytes());
    for (identity, hash) in sorted {
        hasher.update((identity.len() as u64).to_be_bytes());
        hasher.update(identity.as_bytes());
        hasher.update((hash.len() as u64).to_be_bytes());
        hasher.update(hash);
    }
    hasher.finalize().into()
}
