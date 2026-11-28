//! The Clippy gate that keeps SQL text to `Statement::query`.
//!
//! `clippy.toml` disallows every sqlx entry point that accepts SQL text, and
//! Clippy resolves each call, import, rename, function pointer and trait
//! method to those definitions, so this crate cannot send SQL except through
//! the pinned exceptions below. These tests pin that inventory against sqlx
//! 0.9 (re-read sqlx's API for SQL-text parameters, `SqlSafeStr` and
//! `Execute` implementations on every sqlx upgrade) and check that only the
//! pinned sites suppress the lints. `probes` calls each listed entry point
//! under an expectation that the lint fires, so a Clippy run fails when an
//! entry stops resolving. `scripts/check_runlimit_workspace.py` keeps the
//! manifest from lowering the lints, which the packaged manifest cannot show
//! reliably; lint flags passed on the command line belong to
//! `scripts/verify.sh`.
//!
//! Every file is read through `rust_text::normalize`, so a comment, a doc
//! line or a string that quotes an attribute is not mistaken for one, and a
//! comment inside a lint list does not hide its lint.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

/// sqlx functions and methods that accept SQL text or turn text into SQL.
const METHODS: &[&str] = &[
    "sqlx::query",
    "sqlx::query_with",
    "sqlx::__query_with_result",
    "sqlx::query_as",
    "sqlx::query_as_with",
    "sqlx::query_scalar",
    "sqlx::query_scalar_with",
    "sqlx::__query_scalar_with_result",
    "sqlx::raw_sql",
    "sqlx::Executor::execute",
    "sqlx::Executor::execute_many",
    "sqlx::Executor::fetch",
    "sqlx::Executor::fetch_many",
    "sqlx::Executor::fetch_all",
    "sqlx::Executor::fetch_one",
    "sqlx::Executor::fetch_optional",
    "sqlx::Executor::prepare",
    "sqlx::Executor::prepare_with",
    "sqlx::Executor::describe",
    "sqlx::Statement::query",
    "sqlx::Statement::query_with",
    "sqlx::Statement::query_as",
    "sqlx::Statement::query_as_with",
    "sqlx::Statement::query_scalar",
    "sqlx::Statement::query_scalar_with",
    "sqlx::Connection::begin_with",
    "sqlx::Pool::begin_with",
    "sqlx::Pool::try_begin_with",
    "sqlx::postgres::PgConnection::copy_in_raw",
    "sqlx::postgres::PgConnection::copy_out_raw",
    "sqlx::postgres::PgPoolCopyExt::copy_in_raw",
    "sqlx::postgres::PgPoolCopyExt::copy_out_raw",
    "sqlx::SqlSafeStr::into_sql_str",
    "sqlx::Execute::sql",
    "sqlx::Transaction::begin",
    "sqlx::migrate::Migrator::new",
    "sqlx::migrate::Migrator::run",
    "sqlx::migrate::Migrator::run_to",
    "sqlx::migrate::Migrator::run_direct",
    "sqlx::migrate::Migrator::skip",
    "sqlx::migrate::Migrator::undo",
    "sqlx::migrate::Migrator::dangerous_set_table_name",
    "sqlx::migrate::Migrator::create_schema",
    "sqlx::migrate::MigrationSource::resolve",
    "sqlx::migrate::resolve_blocking",
    "sqlx::migrate::resolve_blocking_with_config",
    "sqlx::migrate::Migrate::create_schema_if_not_exists",
    "sqlx::migrate::Migrate::ensure_migrations_table",
    "sqlx::migrate::Migrate::dirty_version",
    "sqlx::migrate::Migrate::list_applied_migrations",
    "sqlx::migrate::Migrate::lock",
    "sqlx::migrate::Migrate::unlock",
    "sqlx::migrate::Migrate::apply",
    "sqlx::migrate::Migrate::revert",
    "sqlx::migrate::Migrate::skip",
    "sqlx::migrate::MigrateDatabase::create_database",
    "sqlx::migrate::MigrateDatabase::database_exists",
    "sqlx::migrate::MigrateDatabase::drop_database",
    "sqlx::migrate::MigrateDatabase::force_drop_database",
    "sqlx_core::any::driver::AnyDriver::get_migrate_database",
    "sqlx_core::any::driver::AnyMigrateDatabase::create_database",
    "sqlx_core::any::driver::AnyMigrateDatabase::database_exists",
    "sqlx_core::any::driver::AnyMigrateDatabase::drop_database",
    "sqlx_core::any::driver::AnyMigrateDatabase::force_drop_database",
    "sqlx::testing::TestFn::run_test",
    "sqlx::testing::TestSupport::test_context",
    "sqlx::testing::TestSupport::cleanup_test",
    "sqlx::testing::TestSupport::cleanup_test_dbs",
    "sqlx::testing::TestSupport::snapshot",
];

/// sqlx types that carry SQL text or send sqlx's own: `SqlStr` has no public
/// tuple constructor, so disallowing the type also closes
/// `Transaction::begin(.., Some(text))` and a hand-built `Migration`;
/// `PgListener` builds `LISTEN` statements from channel names and
/// `PgAdvisoryLock` calls `pg_advisory_lock` through `search_path`.
const TYPES: &[&str] = &[
    "sqlx::AssertSqlSafe",
    "sqlx::SqlStr",
    "sqlx::QueryBuilder",
    "sqlx::query_builder::Separated",
    "sqlx::RawSql",
    "sqlx::migrate::Migration",
    "sqlx::postgres::PgListener",
    "sqlx::postgres::PgAdvisoryLock",
    "sqlx::postgres::PgAdvisoryLockGuard",
    "sqlx::testing::TestArgs",
    "sqlx::testing::TestFixture",
    "sqlx::testing::TestContext",
    "sqlx::query::Query",
    "sqlx::query::QueryAs",
    "sqlx::query::QueryScalar",
    "sqlx::query::Map",
];

/// sqlx's query macros.
const MACROS: &[&str] = &[
    "sqlx::query",
    "sqlx::query_unchecked",
    "sqlx::query_file",
    "sqlx::query_file_unchecked",
    "sqlx::query_as",
    "sqlx::query_as_unchecked",
    "sqlx::query_file_as",
    "sqlx::query_file_as_unchecked",
    "sqlx::query_scalar",
    "sqlx::query_scalar_unchecked",
    "sqlx::query_file_scalar",
    "sqlx::query_file_scalar_unchecked",
    "sqlx::sqlx_macros::expand_query",
    "sqlx::test",
];

/// The probe module, relative to `src`.
const PROBES: &str = "catalog_guard/gate/probes.rs";

mod configuration;
#[cfg(runlimit_macro_probes)]
mod macro_probes;
mod probes;
mod rust_text;

use rust_text::normalize;

/// The gate's lints, each also matching its renamed singular spelling.
const GATE_LINTS: &[&str] = &[
    "clippy::disallowed_method",
    "clippy::disallowed_type",
    "clippy::disallowed_macro",
];

/// The groups that hold the gate's lints (`clippy-driver -Whelp`), rustc's
/// `warnings`, and the lint that fails an unmet probe.
const GATE_GROUPS: &[&str] = &[
    "clippy::all",
    "clippy::default",
    "clippy::style",
    "warnings",
    "unfulfilled_lint_expectations",
];

/// Whether one item of a lint list names a gate lint.
fn is_gate_lint(item: &str) -> bool {
    GATE_LINTS.iter().any(|lint| item.starts_with(lint))
}

/// Whether one item of a lint list names the gate, a group holding it, or a
/// lint a macro assembles.
fn names_the_gate(item: &str) -> bool {
    item.contains('$') || is_gate_lint(item) || GATE_GROUPS.contains(&item)
}

/// The only suppressions of the gate, each pinned to its whole item, by the
/// file under `src` that holds it, compared after `normalize`.
fn exceptions() -> Vec<(&'static str, String)> {
    // `sqlx::migrate!` names `sqlx::SqlStr` in its expansion, so each bundled
    // migrator carries the one type exception.
    let migrator = |name: &str, directory: &str| {
        format!(
            "#[expect(clippy::disallowed_types, \
             reason = \"sqlx::migrate! embeds each migration as a sqlx::SqlStr\")]
             pub static {name}: Migrator = sqlx::migrate!(\"{directory}\");"
        )
    };
    vec![
        // The live check asks PostgreSQL to prepare each closed-set
        // statement in a schema it creates and drops for each family.
        (
            "catalog_guard/live.rs",
            "#[expect(clippy::disallowed_methods, reason = \"PostgreSQL prepares each closed-set statement\")]
             async fn prepare(pool: &PgPool, statement: Statement) -> Result<(), sqlx::Error> {
                 let mut connection = pool.acquire().await?;
                 connection
                     .prepare(SqlSafeStr::into_sql_str(statement.sql()))
                     .await
                     .map(drop)
             }"
            .to_owned(),
        ),
        (
            "catalog_guard/live.rs",
            "#[expect(clippy::disallowed_methods, reason = \"the live check administers its own schema\")]
             async fn administer(admin: &PgPool, sql: String) {
                 sqlx::query(AssertSqlSafe(sql))
                     .execute(admin)
                     .await
                     .expect(\"administer the check's schema\");
             }"
            .to_owned(),
        ),
        (
            "catalog_guard/live.rs",
            "#[expect(clippy::disallowed_methods, reason = \"the live check verifies its search path\")]
             async fn current_schemas(pool: &PgPool) -> Result<Vec<String>, sqlx::Error> {
                 sqlx::query_scalar(\"SELECT pg_catalog.current_schemas(false)\")
                     .fetch_one(pool)
                     .await
             }"
            .to_owned(),
        ),
        (
            "statements.rs",
            "#[expect(clippy::disallowed_methods, reason = \"the one place SQL text reaches sqlx\")]
             pub(crate) fn query<'q>(self) -> Bound<'q> {
                 Bound(sqlx::query(self.sql()))
             }"
            .to_owned(),
        ),
        // The one sqlx query value: every statement runs as a `Bound`.
        (
            "statements.rs",
            "pub(crate) struct Bound<'q>(
                 #[expect(clippy::disallowed_types, reason = \"the one place a sqlx query value is held\")]
                 sqlx::query::Query<'q, Postgres, PgArguments>,
             );"
            .to_owned(),
        ),
        // The one migrator run: it selects a bundled history itself and runs a
        // fresh migrator over it, so only published SQL runs, under the
        // default table and no schema.
        (
            "base.rs",
            "#[expect(clippy::disallowed_methods, reason = \"the one place the bundled migrations run\")]
             pub(crate) async fn migrate(&self, history: BundledHistory) -> Result<(), MigrateError> {
                 let bundled = match history {
                     BundledHistory::FixedWindow => &crate::MIGRATOR,
                     BundledHistory::Gcra => &crate::GCRA_MIGRATOR,
                     BundledHistory::Attempts => &crate::attempts::ATTEMPTS_MIGRATOR,
                 };
                 let mut connection = ConnectionCancellationGuard::new(self.pool.acquire().await?);
                 let mut migrator = Migrator::with_migrations(bundled.iter().cloned().collect());
                 migrator.set_ignore_missing(true);
                 let result = migrator
                     .run_direct(None, &mut **connection.connection(), false)
                     .await;
                 connection.release(match result {
                     Ok(()) => ConnectionOutcome::Reusable(Ok(())),
                     Err(error) => ConnectionOutcome::MustClose(Err(error)),
                 })
             }"
            .to_owned(),
        ),
        ("lib.rs", migrator("MIGRATOR", "./migrations")),
        ("gcra.rs", migrator("GCRA_MIGRATOR", "./gcra-migrations")),
        (
            "attempts.rs",
            migrator("ATTEMPTS_MIGRATOR", "./attempts-migrations"),
        ),
    ]
}

fn manifest_directory() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Every `.rs` file under `directory`, by its path relative to it.
fn rust_files(directory: &Path) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current).expect("source directory is readable") {
            let path = entry.expect("directory entry is readable").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let relative = path
                    .strip_prefix(directory)
                    .expect("found under the source directory")
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                found.push((relative, path));
            }
        }
    }
    found.sort();
    found
}

/// The paths of the active entries in one `clippy.toml` list. Each line of
/// the list is blank, a comment, or exactly one
/// `{ path = "...", reason = "..." },` entry, optionally ending
/// `, allow-invalid = true },` for an item only some builds compile (the
/// probes, which run where it exists, show it still resolves); anything else
/// fails, so a commented-out entry is never counted as one Clippy loaded.
fn listed(config: &str, key: &str) -> BTreeSet<String> {
    let opener = format!("{key} = [");
    let mut lines = config.lines().skip_while(|line| *line != opener);
    assert!(lines.next().is_some(), "clippy.toml has no {key} list");
    let mut paths = BTreeSet::new();
    for line in lines.map(str::trim) {
        if line == "]" {
            return paths;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((path, _)) = line
            .strip_prefix("{ path = \"")
            .and_then(|rest| rest.split_once('"'))
            .filter(|(_, rest)| {
                rest.starts_with(", reason = \"")
                    && (rest.ends_with("\" },") || rest.ends_with("\", allow-invalid = true },"))
            })
        else {
            panic!("clippy.toml {key}: unreadable line {line}");
        };
        assert!(
            paths.insert(path.to_owned()),
            "clippy.toml {key} repeats {path}"
        );
    }
    panic!("clippy.toml {key} is not closed");
}

/// The level a lint list sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Level {
    Allow,
    Expect,
    /// `warn` cannot silence a lint under `-D warnings`, except for
    /// `warnings` itself, which it turns back into warnings.
    Warn,
}

/// Each `allow(...)` or `expect(...)` lint list in `code`, whose whitespace
/// is removed, with whether it is an expectation and its items, skipping
/// `.expect(` and `::expect(` calls and a list's `reason`.
fn lint_lists(code: &str) -> Vec<(Level, Vec<&str>)> {
    let mut lists = Vec::new();
    for (level, opener) in [
        (Level::Allow, "allow("),
        (Level::Expect, "expect("),
        (Level::Warn, "warn("),
    ] {
        for (start, _) in code.match_indices(opener) {
            let before = code[..start].chars().next_back();
            if before.is_some_and(|c| c == '.' || c == ':' || c == '_' || c.is_alphanumeric()) {
                continue;
            }
            let open = start + opener.len();
            let mut depth = 1;
            let mut items = Vec::new();
            let mut item = open;
            let mut end = code.len();
            for (offset, c) in code[open..].char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    ',' if depth == 1 => {
                        items.push(&code[item..open + offset]);
                        item = open + offset + 1;
                    }
                    _ => {}
                }
                if depth == 0 {
                    end = open + offset;
                    break;
                }
            }
            items.push(&code[item..end]);
            items.retain(|item| !item.is_empty() && !item.starts_with("reason="));
            lists.push((level, items));
        }
    }
    lists
}

fn is_identifier(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// Whether normalized `code` holds `word` as a whole identifier.
fn has_token(code: &str, word: &str) -> bool {
    code.match_indices(word).any(|(at, _)| {
        !code[..at].chars().next_back().is_some_and(is_identifier)
            && !code[at + word.len()..]
                .chars()
                .next()
                .is_some_and(is_identifier)
    })
}

/// The contents of each `#[...]` and `#![...]` attribute in normalized `code`.
fn attributes(code: &str) -> impl Iterator<Item = &str> {
    code.match_indices('#').filter_map(move |(at, _)| {
        let rest = &code[at + 1..];
        let rest = rest.strip_prefix('!').unwrap_or(rest).strip_prefix('[')?;
        let mut depth = 1_usize;
        let end = rest.char_indices().find_map(|(offset, c)| {
            match c {
                '[' => depth += 1,
                ']' => depth -= 1,
                _ => {}
            }
            (depth == 0).then_some(offset)
        })?;
        Some(&rest[..end])
    })
}

/// Whether an attribute's contents assign `key`, at any nesting.
fn sets(attribute: &str, key: &str) -> bool {
    attribute.match_indices(key).any(|(at, _)| {
        !attribute[..at]
            .chars()
            .next_back()
            .is_some_and(is_identifier)
            && attribute[at + key.len()..].starts_with('=')
    })
}

#[test]
fn the_scan_sees_every_way_to_load_code_from_elsewhere() {
    for loads in [
        "#[cfg_attr(unix, path = \"../x.rs\")] mod x;",
        "#[path = \"x.rs\"] mod x;",
        "include! { \"x.rs\" }",
        "use core::include as load;",
        "core::include![\"x.rs\"];",
    ] {
        let code = normalize(loads);
        assert!(
            has_token(&code, "include") || attributes(&code).any(|a| sets(a, "path")),
            "{loads}"
        );
    }
    for harmless in [
        "let path = 1; let _ = include_str![\"x.sql\"];",
        "#[doc = \"path = include\"] fn included() {}",
    ] {
        let code = normalize(harmless);
        assert!(
            !has_token(&code, "include") && !attributes(&code).any(|a| sets(a, "path")),
            "{harmless}"
        );
    }
}

#[test]
fn clippy_disallows_every_sqlx_entry_point_that_accepts_sql() {
    let config = fs::read_to_string(manifest_directory().join("clippy.toml"))
        .expect("runlimit-postgres has a clippy.toml");
    for (key, expected) in [
        ("disallowed-methods", METHODS),
        ("disallowed-types", TYPES),
        ("disallowed-macros", MACROS),
    ] {
        let expected: BTreeSet<String> = expected.iter().map(|path| (*path).to_owned()).collect();
        assert_eq!(listed(&config, key), expected, "clippy.toml {key}");
    }
}

#[test]
fn only_the_pinned_items_suppress_the_gate() {
    let source = manifest_directory().join("src");
    let mut problems = Vec::new();
    let mut excepted = BTreeSet::new();
    let exceptions = exceptions();
    for (file, path) in rust_files(&source) {
        let mut code = normalize(&fs::read_to_string(&path).expect("source is readable"));
        for (_, pinned) in exceptions.iter().filter(|(owner, _)| *owner == file) {
            let pinned = normalize(pinned);
            match code.matches(&pinned).count() {
                1 => {
                    code = code.replacen(&pinned, "", 1);
                    excepted.insert(file.clone());
                }
                count => problems.push(format!(
                    "{file}: its pinned exception appears {count} times"
                )),
            }
        }
        for (level, items) in lint_lists(&code) {
            // A probe expects a gate lint to fire; nothing may allow one, and
            // nothing may turn `warnings` back from the `-D warnings` of the
            // Clippy runs.
            let probe = file == PROBES && level == Level::Expect;
            for item in items {
                let suppresses = match level {
                    Level::Warn => item == "warnings" || item.contains('$'),
                    Level::Allow | Level::Expect => {
                        names_the_gate(item) && !(probe && is_gate_lint(item))
                    }
                };
                if suppresses {
                    problems.push(format!("{file}: suppresses `{item}`"));
                }
            }
        }
        // Code from outside `src` would escape this scan: no `include`
        // token in any form or renaming, and no attribute that sets `path`.
        if has_token(&code, "include") {
            problems.push(format!("{file}: uses include"));
        }
        if attributes(&code).any(|attribute| sets(attribute, "path")) {
            problems.push(format!("{file}: sets a module path"));
        }
    }
    // The probes are test code: catalog_guard compiles only under test.
    let root = normalize(&fs::read_to_string(source.join("lib.rs")).expect("lib.rs"));
    if !root.contains("#[cfg(test)]mod catalog_guard;") {
        problems.push("lib.rs: catalog_guard is not test-only".to_owned());
    }
    for (file, _) in &exceptions {
        if !excepted.contains(*file) {
            problems.push(format!(
                "{file}: its pinned exception is missing or changed"
            ));
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn the_suppression_scan_reads_each_lint_list() {
    let code = normalize(
        "/// Quoting #[allow(clippy::all)] here is documentation.
         #[allow(dead_code, /* a note */ warnings)] fn a() { x.expect(\"warnings\"); }
         #[cfg_attr(test, expect($lint, reason = \"r\"))] fn b() {}
         #[deny(clippy::disallowed_methods)] fn c() {}
         #[warn(warnings)] fn d() {}
         Option::expect(value, \"$1\");",
    );
    let lists = lint_lists(&code);
    assert_eq!(
        lists,
        [
            (Level::Allow, vec!["dead_code", "warnings"]),
            (Level::Expect, vec!["$lint"]),
            (Level::Warn, vec!["warnings"])
        ]
    );
    let flagged: Vec<_> = lists
        .iter()
        .flat_map(|(_, items)| items.iter().copied().filter(|item| names_the_gate(item)))
        .collect();
    assert_eq!(flagged, ["warnings", "$lint", "warnings"]);
    assert!(names_the_gate("clippy::disallowed_method"));
    assert!(!names_the_gate("clippy::pedantic"));
}

#[test]
fn the_inventory_reader_counts_only_active_entries() {
    let config = concat!(
        "x = 1\n",
        "list = [\n",
        "    { path = \"a\", reason = \"r\" },\n",
        "    # { path = \"b\", reason = \"r\" },\n",
        "]\n",
    );
    assert_eq!(listed(config, "list"), BTreeSet::from(["a".to_owned()]));
    let optional = "list = [\n    { path = \"a\", reason = \"r\", allow-invalid = true },\n]\n";
    assert_eq!(listed(optional, "list"), BTreeSet::from(["a".to_owned()]));
    for unreadable in [
        "list = [\n    { path = \"a\", reason = \"r\", allow-invalid = false },\n]\n",
        "list = [\n    { path = \"a\", reason = \"r\" }, # trailing\n]\n",
        "list = [\n    { path = \"a\", reason = \"r\" },\n",
    ] {
        assert!(std::panic::catch_unwind(|| listed(unreadable, "list")).is_err());
    }
}
