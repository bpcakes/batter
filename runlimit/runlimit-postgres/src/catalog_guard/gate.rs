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
    "sqlx::RawSql",
    "sqlx::migrate::Migration",
    "sqlx::postgres::PgListener",
    "sqlx::postgres::PgAdvisoryLock",
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
];

/// The probe module, relative to `src`.
const PROBES: &str = "catalog_guard/gate/probes.rs";

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
        (
            "statements.rs",
            "#[expect(clippy::disallowed_methods, reason = \"the one place SQL text reaches sqlx\")]
             pub(crate) fn query<'q>(self) -> Query<'q, Postgres, PgArguments> {
                 sqlx::query(self.sql())
             }"
            .to_owned(),
        ),
        // A fresh migrator over the bundled migrations, so neither a table
        // name nor a schema written on `bundled` reaches SQL.
        (
            "base.rs",
            "#[expect(clippy::disallowed_methods, reason = \"the one place the bundled migrations run\")]
             pub(crate) async fn migrate(&self, bundled: &Migrator) -> Result<(), MigrateError> {
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
/// `{ path = "...", reason = "..." },` entry; anything else fails, so a
/// commented-out entry is never counted as one Clippy loaded.
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
            .filter(|(_, rest)| rest.starts_with(", reason = \"") && rest.ends_with("\" },"))
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

/// Each `allow(...)` or `expect(...)` lint list in `code`, whose whitespace
/// is removed, with whether it is an expectation and its items, skipping
/// `.expect(` and `::expect(` calls and a list's `reason`.
fn lint_lists(code: &str) -> Vec<(bool, Vec<&str>)> {
    let mut lists = Vec::new();
    for (expect, opener) in [(false, "allow("), (true, "expect(")] {
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
            lists.push((expect, items));
        }
    }
    lists
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
        for (expect, items) in lint_lists(&code) {
            // A probe expects a gate lint to fire; nothing may allow one.
            let probe = file == PROBES && expect;
            for item in items {
                if names_the_gate(item) && !(probe && is_gate_lint(item)) {
                    problems.push(format!("{file}: suppresses `{item}`"));
                }
            }
        }
        // Code from outside `src` would escape this scan.
        for include in ["#[path", "include!("] {
            if code.contains(include) {
                problems.push(format!("{file}: uses {include}"));
            }
        }
    }
    // The probes are test code: catalog_guard compiles only under test.
    let root = normalize(&fs::read_to_string(source.join("lib.rs")).expect("lib.rs"));
    if !root.contains("#[cfg(test)]modcatalog_guard;") {
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
         Option::expect(value, \"$1\");",
    );
    let lists = lint_lists(&code);
    assert_eq!(
        lists,
        [
            (false, vec!["dead_code", "warnings"]),
            (true, vec!["$lint"])
        ]
    );
    let flagged: Vec<_> = lists
        .iter()
        .flat_map(|(_, items)| items.iter().copied().filter(|item| names_the_gate(item)))
        .collect();
    assert_eq!(flagged, ["warnings", "$lint"]);
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
    for unreadable in [
        "list = [\n    { path = \"a\", reason = \"r\" }, # trailing\n]\n",
        "list = [\n    { path = \"a\", reason = \"r\" },\n",
    ] {
        assert!(std::panic::catch_unwind(|| listed(unreadable, "list")).is_err());
    }
}
