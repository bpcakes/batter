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
//! The lint names this file looks for are assembled with `concat!`, so its
//! own source names none of them.

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
    "sqlx::migrate::MigrationSource::resolve",
    "sqlx::migrate::resolve_blocking",
    "sqlx::migrate::resolve_blocking_with_config",
];

/// sqlx types that carry SQL text. `SqlStr` has no public tuple constructor,
/// so disallowing the type also closes `Transaction::begin(.., Some(text))`
/// and a hand-built `Migration`.
const TYPES: &[&str] = &[
    "sqlx::AssertSqlSafe",
    "sqlx::SqlStr",
    "sqlx::QueryBuilder",
    "sqlx::RawSql",
    "sqlx::migrate::Migration",
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

/// The gate's lint names and the groups that contain them; the first three
/// are the lints themselves.
const GATE_LINTS: &[&str] = &[
    concat!("disallowed", "_method"),
    concat!("disallowed", "_type"),
    concat!("disallowed", "_macro"),
    concat!("clippy", "::all"),
    concat!("clippy", "::default"),
    concat!("clippy", "::style"),
    // Silencing an unfulfilled probe would hide a gap in the gate.
    concat!("unfulfilled", "_lint_expectations"),
];

/// The only suppressions of the gate, each pinned to its whole item, by the
/// file under `src` that holds it. Whitespace is ignored.
fn exceptions() -> Vec<(&'static str, String)> {
    // `sqlx::migrate!` names `sqlx::SqlStr` in its expansion, so each bundled
    // migrator carries the one type exception.
    let migrator = |name: &str, directory: &str| {
        format!(
            concat!(
                "#[expect(clippy::disallowed",
                "_types, reason = \"sqlx::migrate! embeds each migration as a sqlx::SqlStr\")]",
                "pub static {name}: Migrator = sqlx::migrate!(\"{directory}\");",
            ),
            name = name,
            directory = directory,
        )
    };
    vec![
        (
            "statements.rs",
            concat!(
                "#[expect(clippy::disallowed",
                "_methods, reason = \"the one place SQL text reaches sqlx\")]",
                "pub(crate) fn query<'q>(self) -> Query<'q, Postgres, PgArguments> {",
                "    sqlx::query(self.sql())",
                "}",
            )
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

fn without_whitespace(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
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

/// The paths a `clippy.toml` list names.
fn listed(config: &str, key: &str) -> BTreeSet<String> {
    let start = config
        .find(&format!("\n{key} = ["))
        .unwrap_or_else(|| panic!("clippy.toml has no {key} list"));
    let list = &config[start..];
    let list = &list[..list.find("\n]").expect("the list is closed")];
    list.split("path = \"")
        .skip(1)
        .map(|rest| rest[..rest.find('"').expect("a quoted path")].to_owned())
        .collect()
}

/// The contents of each `allow(...)` or `expect(...)` lint list in `code`,
/// skipping `.expect(` and `::expect(` calls.
fn lint_lists(code: &str) -> Vec<&str> {
    let mut lists = Vec::new();
    for opener in [concat!("allow", "("), concat!("expect", "(")] {
        for (start, _) in code.match_indices(opener) {
            let before = code[..start].chars().next_back();
            if before.is_some_and(|c| c == '.' || c == ':' || c == '_' || c.is_alphanumeric()) {
                continue;
            }
            let open = start + opener.len();
            let mut depth = 1;
            let end = code[open..]
                .char_indices()
                .find_map(|(offset, c)| {
                    match c {
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        _ => {}
                    }
                    (depth == 0).then_some(open + offset)
                })
                .unwrap_or(code.len());
            lists.push(&code[open..end]);
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
        let mut code = without_whitespace(&fs::read_to_string(&path).expect("source is readable"));
        if file == PROBES {
            // A probe expects the lint to fire; it never allows it.
            for lint in &GATE_LINTS[..3] {
                code = code.replace(
                    &format!("{}(clippy::{lint}", concat!("#[ex", "pect")),
                    "#[probe(",
                );
            }
        }
        for (_, pinned) in exceptions.iter().filter(|(owner, _)| *owner == file) {
            let pinned = without_whitespace(pinned);
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
        for lint in GATE_LINTS {
            if code.contains(lint) {
                problems.push(format!("{file}: names {lint} outside a pinned exception"));
            }
        }
        for list in lint_lists(&code) {
            if list.contains("warnings") || list.contains('$') {
                problems.push(format!("{file}: suppresses `{list}`"));
            }
        }
        // Code from outside `src` would escape this scan.
        for include in [concat!("#[pa", "th"), concat!("include", "!(")] {
            if code.contains(include) {
                problems.push(format!("{file}: uses {include}"));
            }
        }
    }
    // The probes are test code: catalog_guard compiles only under test.
    let root = without_whitespace(&fs::read_to_string(source.join("lib.rs")).expect("lib.rs"));
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
    // Split so this fixture is not itself a lint list.
    let code = without_whitespace(concat!(
        "#[al",
        "low(dead_code, warnings)] fn a() { x.expect(\"warnings\"); }",
        "#[cfg_attr(test, ex",
        "pect($lint))] fn b() {}",
        "Option::expect(value, \"$1\");",
    ));
    assert_eq!(lint_lists(&code), ["dead_code,warnings", "$lint"]);
}
