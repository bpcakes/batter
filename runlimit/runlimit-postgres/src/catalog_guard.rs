//! Every operational statement resolves catalog objects by their
//! `pg_catalog` name, so a schema earlier on the caller's `search_path` cannot
//! replace a function or type the limiter relies on (a shadowed `set_config`
//! would silently drop the server timeouts; a shadowed clock would move
//! windows).
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
//! typed literal such as `text 'value'`), and string constants only as
//! function arguments or list elements. The guard rejects anything outside it
//! rather than trying to parse it.
//!
//! The set of statements is closed by construction: `crate::statements` owns
//! every [`Statement`](crate::statements::Statement), each family declares
//! its statements through one macro that also registers them, and only a
//! statement can be executed. This guard checks that nothing else in that
//! module constructs a statement and that no other production module builds
//! SQL: no SQL-looking literal, no `sqlx` query constructor, macro or
//! builder, no `include!`, no `include_str!` or `include_bytes!` but a
//! published migration's, and no reference to a published migration beyond
//! its declaration and re-export, since those are the crate's only other SQL
//! strings and an `Executor` accepts a string directly.

use std::{collections::BTreeSet, fs, path::Path};

mod detector;
mod literals;
mod modules;

use detector::unqualified;
use literals::{files, includes, sql_literals};
use modules::production_sources;

use crate::statements;

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

/// `sqlx` items that build or run SQL from a string; only `statements` uses
/// them.
const SQL_BUILDERS: &[&str] = &[
    "QueryBuilder",
    "query_as",
    "query_as_unchecked",
    "query_as_with",
    "query_file",
    "query_file_as",
    "query_file_as_unchecked",
    "query_file_scalar",
    "query_file_scalar_unchecked",
    "query_file_unchecked",
    "query_scalar",
    "query_scalar_unchecked",
    "query_scalar_with",
    "query_unchecked",
    "query_with",
    "raw_sql",
];

fn source_directory() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn is_statements_file(file: &str) -> bool {
    file == "statements.rs" || file.starts_with("statements/")
}

#[test]
fn every_statement_qualifies_its_catalog_functions_and_types() {
    let mut violations = Vec::new();
    for (name, statement) in statements::all() {
        violations.extend(
            unqualified(statement.sql())
                .into_iter()
                .map(|found| format!("{name}: {found}")),
        );
    }
    assert!(
        violations.is_empty(),
        "unqualified catalog objects:\n{violations:#?}"
    );
}

#[test]
fn only_the_registering_macro_constructs_statements() {
    let source = source_directory();
    let root = fs::read_to_string(source.join("statements.rs")).unwrap();
    let (_, root_code) = sql_literals(&root);
    let constructions = root_code.matches("Statement(").count();
    assert_eq!(
        constructions, 2,
        "statements.rs may name Statement( only in its definition and in the macro body"
    );
    let families: BTreeSet<String> = statements::all()
        .iter()
        .filter_map(|(name, _)| name.split_once("::").map(|(family, _)| family.to_owned()))
        .collect();
    let mut files_found = BTreeSet::new();
    for path in files(&source.join("statements"), "rs") {
        let (_, code) = sql_literals(&fs::read_to_string(&path).unwrap());
        assert!(
            !code.contains("Statement("),
            "{} constructs a Statement outside the statements! macro",
            path.display()
        );
        files_found.insert(path.file_stem().unwrap().to_str().unwrap().to_owned());
    }
    assert_eq!(
        files_found, families,
        "statements::all() must include every family file"
    );
}

#[test]
fn no_production_module_builds_sql_outside_statements() {
    let source = source_directory();
    let mut violations = Vec::new();
    let mut included_files = BTreeSet::new();
    for (file, production) in production_sources(&source) {
        let (literals, code) = sql_literals(&production);
        let statements = is_statements_file(&file);
        for (start, _) in code.match_indices("include!") {
            if start == 0 || !code.as_bytes()[start - 1].is_ascii_alphanumeric() {
                violations.push(format!("{file}: include!"));
            }
        }
        for (owner, path) in includes(&production, &code) {
            let resolved = Path::new(&file)
                .parent()
                .and_then(|parent| source.join(parent).join(&path).canonicalize().ok());
            let migration = owner
                .as_deref()
                .is_some_and(|name| MIGRATIONS.contains(&(file.as_str(), name)));
            match resolved {
                Some(resolved) if statements => {
                    included_files.insert(resolved);
                }
                Some(_) if migration => {}
                _ => violations.push(format!("{file}: {owner:?} includes {path}")),
            }
        }
        if statements {
            continue;
        }
        for (_, name) in MIGRATIONS {
            for (start, _) in code.match_indices(name) {
                let item_start = code[..start]
                    .rfind([';', '}'])
                    .map_or(0, |end| end + 1);
                let item = code[item_start..start].trim_start();
                let declaration = item.contains("const")
                    && code[start + name.len()..].trim_start().starts_with(':');
                if !declaration && !item.starts_with("pub use") {
                    violations.push(format!("{file}: refers to the migration {name}"));
                }
            }
        }
        for (_, literal) in literals {
            violations.push(format!("{file}: SQL literal {literal:.60}"));
        }
        for word in identifiers(&code) {
            if SQL_BUILDERS.contains(&word.text) {
                violations.push(format!("{file}: sqlx {}", word.text));
            }
            // `sqlx::query::Query` names the module; `sqlx::query::<_>(..)`
            // is the function with a turbofish.
            let module_path = word
                .after
                .strip_prefix("::")
                .is_some_and(|rest| !rest.trim_start().starts_with('<'));
            let function = word.qualified_by_sqlx && !module_path;
            if word.text == "query" && (word.after.starts_with('!') || function) {
                violations.push(format!("{file}: sqlx query{}", &word.after[..1]));
            }
        }
    }
    for path in files(&source, "sql") {
        if !included_files.contains(&path.canonicalize().unwrap()) {
            violations.push(format!("{} is not included by a statement", path.display()));
        }
    }
    assert!(
        violations.is_empty(),
        "SQL outside crate::statements:\n{violations:#?}"
    );
}

/// One identifier in blanked Rust code: its text, what follows it and
/// whether it is written as `sqlx::name` or imported in a `use sqlx` item.
struct Identifier<'a> {
    text: &'a str,
    after: &'a str,
    qualified_by_sqlx: bool,
}

fn identifiers(code: &str) -> Vec<Identifier<'_>> {
    let bytes = code.as_bytes();
    let mut found = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if !(byte.is_ascii_alphabetic() || byte == b'_')
            || index > 0 && (bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'_')
        {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
        {
            index += 1;
        }
        let before = code[..start].trim_end();
        let statement_start = before.rfind(';').map_or(0, |end| end + 1);
        let in_sqlx_use = code[statement_start..start]
            .trim_start()
            .starts_with("use sqlx");
        found.push(Identifier {
            text: &code[start..index],
            after: code[index..].trim_start(),
            qualified_by_sqlx: before.ends_with("sqlx::") || in_sqlx_use,
        });
    }
    found
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
        "SELECT pg_catalog.length('can''t'), floor(1)",
        "SELECT /* outer /* inner */ -- */ 1, floor(1)",
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
        "SELECT pg_catalog.length('can''t'), pg_catalog.length('x'''), pg_catalog.floor(1)",
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
fn the_scanner_finds_every_statement_literal() {
    let source = r##"
        const NAMED_SQL: &str = "-- leading note\nSELECT 1";
        const CONTINUED_SQL: &str = "SELECT \
            2";
        fn inline() {
            let _ = sqlx::query("/* a */ VALUES (1)");
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
    let words: Vec<_> = identifiers(&code)
        .into_iter()
        .filter(|word| word.text == "query" || SQL_BUILDERS.contains(&word.text))
        .map(|word| (word.text, word.qualified_by_sqlx))
        .collect();
    assert_eq!(
        words,
        [("query", true), ("query", false), ("raw_sql", true)]
    );
    let imported = identifiers(
        "use sqlx::{query, query::Query};\nfn f() { statement.query(); sqlx::query::<P>(Q); }",
    );
    let imported: Vec<_> = imported
        .iter()
        .filter(|word| word.text == "query")
        .map(|word| (word.qualified_by_sqlx, word.after.starts_with("::")))
        .collect();
    assert_eq!(
        imported,
        [(true, false), (true, true), (false, false), (true, true)],
        "the import, the module path and the turbofish call are sqlx's; the method is a statement's"
    );
}
