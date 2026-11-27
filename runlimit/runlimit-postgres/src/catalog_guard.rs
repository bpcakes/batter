//! Every production statement resolves catalog objects by their `pg_catalog`
//! name, so a schema earlier on the caller's `search_path` cannot replace a
//! function or type the limiter relies on (a shadowed `set_config` would
//! silently drop the server timeouts; a shadowed clock would move windows).
//!
//! The rule covers function calls and casts. SQL syntax that the PostgreSQL
//! grammar maps to `pg_catalog` itself (`EXTRACT`, `COALESCE`, `GREATEST`,
//! `LEAST`, and keyword types such as `bigint`) needs no prefix. Operators
//! and the limiter's own tables still resolve through `search_path`, which
//! the application chooses. Published migrations are immutable, are not
//! qualified and are not checked here; they run under the caller's
//! `search_path`, which must be trusted while migrating.
//!
//! Operational SQL keeps to a dialect this guard reads completely: no dollar
//! quoting, no quoted identifiers, casts only as `::` (never `CAST(...)` or a
//! typed literal such as `text 'value'`), string constants only as function
//! arguments or list elements, and no `sqlx` query macros. The guard rejects
//! anything outside it rather than trying to parse it.
//!
//! Completeness does not depend on remembering to register a statement: every
//! SQL-looking string literal in production source must initialize a
//! registered constant, and every `.sql` file in `src/` must be registered.

use std::{collections::BTreeSet, fs, path::Path};

mod detector;
mod literals;

use detector::unqualified;
use literals::{files, includes, production_sources, query_arguments, sql_literals};

use crate::{
    attempts::{row::StoredAttempt, sql as attempt},
    gcra, protocol,
};

/// `(source file, constant, statement)` for every statement the crate sends.
const STATEMENTS: &[(&str, &str, &str)] = &[
    (
        "protocol.rs",
        "SET_LOCAL_TIMEOUTS_SQL",
        protocol::SET_LOCAL_TIMEOUTS_SQL,
    ),
    (
        "protocol.rs",
        "BATCH_ADVISORY_LOCK_SQL",
        protocol::BATCH_ADVISORY_LOCK_SQL,
    ),
    (
        "protocol.rs",
        "BATCH_ROW_LOCK_SQL",
        protocol::BATCH_ROW_LOCK_SQL,
    ),
    (
        "protocol.rs",
        "BATCH_CAPACITY_LOCK_SQL",
        protocol::BATCH_CAPACITY_LOCK_SQL,
    ),
    (
        "protocol.rs",
        "BATCH_PREFLIGHT_SQL",
        protocol::BATCH_PREFLIGHT_SQL,
    ),
    (
        "protocol.rs",
        "BATCH_UPSERT_SQL",
        protocol::BATCH_UPSERT_SQL,
    ),
    ("protocol.rs", "CLEANUP_SQL", protocol::CLEANUP_SQL),
    ("gcra.rs", "LOCK_SHARDS", gcra::LOCK_SHARDS),
    ("gcra.rs", "SAMPLE_TIME", gcra::SAMPLE_TIME),
    ("gcra.rs", "READ_COUNTERS", gcra::READ_COUNTERS),
    ("gcra.rs", "UPSERT", gcra::UPSERT),
    ("gcra.rs", "UPDATE_WATERMARKS", gcra::UPDATE_WATERMARKS),
    ("gcra.rs", "CLEANUP_LOCKS", gcra::CLEANUP_LOCKS),
    (
        "gcra.rs",
        "CLEANUP_ADVANCE_TIME",
        gcra::CLEANUP_ADVANCE_TIME,
    ),
    ("gcra.rs", "CLEANUP_DELETE", gcra::CLEANUP_DELETE),
    ("attempts/row.rs", "LOCK_SQL", StoredAttempt::LOCK_SQL),
    ("attempts/sql.rs", "SHARD_LOCK_SQL", attempt::SHARD_LOCK_SQL),
    ("attempts/sql.rs", "CLEANUP_SQL", attempt::CLEANUP_SQL),
    (
        "attempts/sql.rs",
        "SAMPLE_TIME_SQL",
        attempt::SAMPLE_TIME_SQL,
    ),
    ("attempts/sql.rs", "OCCUPANCY_SQL", attempt::OCCUPANCY_SQL),
    ("attempts/sql.rs", "FREE_SLOT_SQL", attempt::FREE_SLOT_SQL),
    ("attempts/sql.rs", "INSERT_SQL", attempt::INSERT_SQL),
    ("attempts/sql.rs", "LEASE_SQL", attempt::LEASE_SQL),
    ("attempts/sql.rs", "CLAIM_SQL", attempt::CLAIM_SQL),
    ("attempts/sql.rs", "COMPLETE_SQL", attempt::COMPLETE_SQL),
];

#[test]
fn every_statement_qualifies_its_catalog_functions_and_types() {
    let mut violations = Vec::new();
    for (file, name, sql) in STATEMENTS {
        violations.extend(
            unqualified(sql)
                .into_iter()
                .map(|found| format!("{file} {name}: {found}")),
        );
    }
    assert!(
        violations.is_empty(),
        "unqualified catalog objects:\n{violations:#?}"
    );
}

/// The published migration constants: immutable, executed only through the
/// migrators, and exempt from this guard.
const MIGRATIONS: &[(&str, &str)] = &[
    ("lib.rs", "CREATE_RUNLIMIT_FIXED_WINDOWS_SQL"),
    ("lib.rs", "SET_RUNLIMIT_FIXED_WINDOWS_FILLFACTOR_SQL"),
    ("lib.rs", "BOUND_RUNLIMIT_FIXED_WINDOW_CARDINALITY_SQL"),
    ("gcra.rs", "CREATE_RUNLIMIT_GCRA_SQL"),
    ("gcra.rs", "INDEX_RUNLIMIT_GCRA_SHARD_EXPIRY_SQL"),
    ("attempts.rs", "CREATE_RUNLIMIT_ATTEMPTS_SQL"),
];

/// Whether `(file, name)` is registered with exactly `sql`.
fn is_registered(file: &str, name: &str, sql: &str) -> bool {
    STATEMENTS
        .iter()
        .any(|entry| entry.0 == file && entry.1 == name && entry.2 == sql)
}

#[test]
fn every_production_statement_is_registered() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let names: BTreeSet<&str> = STATEMENTS.iter().map(|(_, name, _)| *name).collect();
    let mut unregistered = Vec::new();
    let mut included_files = BTreeSet::new();
    for (file, production) in production_sources(&source) {
        let (literals, code) = sql_literals(&production);
        for (start, _) in code.match_indices("query") {
            let rest = &code[start..];
            let name_end = rest
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .unwrap_or(rest.len());
            let boundary = start == 0 || !code.as_bytes()[start - 1].is_ascii_alphanumeric();
            if boundary && rest[name_end..].starts_with('!') {
                unregistered.push(format!("{file}: sqlx macro {}!", &rest[..name_end]));
            }
        }
        for (owner, value) in literals {
            if !owner
                .as_deref()
                .is_some_and(|name| is_registered(&file, name, &value))
            {
                unregistered.push(format!("{file}: {owner:?} {value:.60}"));
            }
        }
        for (owner, path) in includes(&production, &code) {
            let migration = owner
                .as_deref()
                .is_some_and(|name| MIGRATIONS.contains(&(file.as_str(), name)));
            let included = Path::new(&file)
                .parent()
                .and_then(|parent| source.join(parent).join(&path).canonicalize().ok());
            let registered = owner
                .as_deref()
                .zip(included)
                .is_some_and(|(name, included)| {
                    let matches = fs::read_to_string(&included)
                        .is_ok_and(|sql| is_registered(&file, name, &sql));
                    if matches {
                        included_files.insert(included);
                    }
                    matches
                });
            if !migration && !registered {
                unregistered.push(format!("{file}: {owner:?} includes {path}"));
            }
        }
        for argument in query_arguments(&code) {
            let constant = argument.rsplit("::").next().unwrap_or_default();
            let is_path = argument.split("::").all(|part| {
                !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            });
            if !is_path || !names.contains(constant) {
                unregistered.push(format!("{file}: query constructor argument `{argument}`"));
            }
        }
    }
    for path in files(&source, "sql") {
        if !included_files.contains(&path.canonicalize().unwrap()) {
            unregistered.push(format!(
                "{} is not included by a registered statement",
                path.display()
            ));
        }
    }
    assert!(
        unregistered.is_empty(),
        "unregistered SQL:\n{unregistered:#?}"
    );
}

#[test]
fn the_detector_rejects_each_unqualified_shape_and_accepts_syntax() {
    for sql in [
        "SELECT floor(1)",
        "SELECT count (*) FROM t",
        "SELECT x::text",
        "SELECT $1::BYTEA[]",
        "SELECT set_config('a', $1, true)",
        "WITH t AS (SELECT clock_timestamp()) SELECT 1",
        "SELECT 1 FROM unnest($1::BIGINT[]) AS input(value)",
        "SELECT 1 FROM pg_catalog.unnest($1::pg_catalog.bytea[], $2::BIGINT[]) AS input(a, b)",
        "SELECT public.clock_timestamp()",
        "SELECT \"clock_timestamp\"()",
        "SELECT /* note */ floor(1)",
        "SELECT $1::public.text",
        "SELECT \"PG_CATALOG\".floor(1)",
        "SELECT $1::\"char\"",
        "SELECT * FROM jsonb_to_record($1) AS (value integer)",
        "SELECT * FROM pg_catalog.unnest($1::BIGINT[]), jsonb_to_record($2) AS (v integer)",
        "SELECT $$it's ready$$, pg_catalog.floor(1)",
        "SELECT $tag$x$tag$",
        "SELECT text 'ready'",
        "SELECT CAST($1 AS pg_catalog.text)",
        "SELECT pg_catalog.\"unnest\"($1::integer[], $2::integer[])",
    ] {
        assert!(!unqualified(sql).is_empty(), "{sql} must be rejected");
    }
    for sql in [
        "SELECT pg_catalog.floor(extract(epoch FROM pg_catalog.clock_timestamp()))::bigint",
        "SELECT pg_catalog.count(*) FROM t WHERE x = ANY($1) AND y IN (1)",
        "WITH RECURSIVE acquired(position) AS (SELECT 1), b AS MATERIALIZED (SELECT 2) \
         SELECT COALESCE(GREATEST(1, 2), LEAST(3, 4))",
        "INSERT INTO t (a, b) VALUES (1, $1::pg_catalog.text) ON CONFLICT (a) DO NOTHING",
        "SELECT 1 FROM pg_catalog.unnest($1::pg_catalog.bytea[]) AS input(value)",
        "SELECT 1 FROM ROWS FROM (pg_catalog.unnest($1::pg_catalog.bytea[]), \
         pg_catalog.unnest(ARRAY[1, 2]::BIGINT[])) WITH ORDINALITY AS input(a, b, position)",
        "SELECT pg_catalog.length('floor(1)::text') -- count(*)\n",
        "SELECT $1::integer, $2::SMALLINT, $3::double precision",
        "SELECT pg_catalog.set_config('statement_timeout', $1, true), $2::BIGINT",
        "WITH a(x) AS NOT MATERIALIZED (VALUES (1)), b AS ( SELECT 2) TABLE a",
    ] {
        assert_eq!(
            unqualified(sql),
            Vec::<String>::new(),
            "{sql} must be accepted"
        );
    }
}

#[test]
fn the_scanner_finds_every_statement_literal_and_constructor_argument() {
    let source = r##"
        const NAMED_SQL: &str = "-- leading note\nSELECT 1";
        const CONTINUED_SQL: &str = "SELECT \
            2";
        fn inline() {
            let _ = sqlx::query("/* a */ VALUES (1)");
            let _ = sqlx::query_scalar::<_, i64>(NAMED_SQL);
            let _ = query(&format!("(SELECT {}", 1));
            let _ = sqlx::raw_sql(r#"TABLE t"#);
            // sqlx::query("SELECT 'a comment'")
            let lifetime: &'static str = "not sql";
        }
    "##;
    let (literals, code) = sql_literals(source);
    let statements: Vec<_> = literals
        .iter()
        .map(|(owner, text)| (owner.as_deref(), text.as_str()))
        .collect();
    assert_eq!(
        statements,
        [
            (Some("NAMED_SQL"), "-- leading note\nSELECT 1"),
            (Some("CONTINUED_SQL"), "SELECT 2"),
            (None, "/* a */ VALUES (1)"),
            (None, "(SELECT {}"),
            (None, "TABLE t"),
        ]
    );
    let arguments = query_arguments(&code);
    assert_eq!(arguments.len(), 4, "{arguments:?}");
    assert!(
        arguments[0].is_empty(),
        "a literal argument is blanked: {arguments:?}"
    );
    assert_eq!(arguments[1], "NAMED_SQL");
    assert!(arguments[2].starts_with("&format!"), "{arguments:?}");
    assert!(arguments[3].is_empty(), "{arguments:?}");
}
