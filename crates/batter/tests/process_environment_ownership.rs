//! Process-environment ownership, checked in source: no Batter library
//! production source reads or mutates the process environment, or discovers a
//! directory through it, outside the one declared seam.
//!
//! The scan covers the `src/` tree of every workspace member under `crates/`,
//! the flags family included, minus the executable roots the shared inventory
//! classifies (the operator command's `src/main.rs` captures its connection
//! settings there) and the files only a `#[cfg(test)]` module declaration
//! loads. Native Runledger and Runlimit read the environment by design and are
//! out of scope, as are dependencies: the reads that remain there by design
//! are listed in ADR-016. Settings themselves are read through
//! `batter_core::settings`, from a source the application root supplies, so
//! naming a `VarError` type is not a read.
//!
//! A read is any path that reaches `std::env` in code, comments and literals
//! removed: a `use` of the module, a glob, an alias or a group however it is
//! wrapped, and any call or macro fragment below it, except the items that
//! read nothing of the environment, such as `VarError` and `args`
//! (`paths::ALLOWED`). `paths` says what the reader cannot see.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "panic_hook_ownership/inventory.rs"]
mod inventory;
#[path = "panic_hook_ownership/modules.rs"]
mod modules;
#[path = "process_environment_ownership/paths.rs"]
mod paths;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use inventory::{executable_roots, members, production_sources, workspace_root};

/// The one seam, and how many reads it holds: the OpenTelemetry adapter
/// refuses ambient `OTEL_*` settings, which its upstream builders would
/// otherwise read, by checking that none is set. It never reads a value.
const DECLARED_SEAMS: [(&str, usize); 1] = [("crates/batter-otlp/src/lib.rs", 1)];

#[test]
fn every_spelling_of_an_environment_read_is_counted_and_types_and_comments_are_not() {
    for (source, count) in [
        ("fn f() { let _ = std::env::var(\"FLAG_X\"); }", 1),
        ("fn f() { let _ = ::std::env::var_os(\"X\"); }", 1),
        ("fn f() { for _ in std :: env :: vars_os() {} }", 1),
        ("fn f() { unsafe { std::env::set_var(\"A\", \"b\") } }", 1),
        (
            "fn f() { let _ = std::env::current_dir(); let _ = std::env::home_dir(); }",
            2,
        ),
        ("fn f() { let _ = std::env::temp_dir(); }", 1),
        ("use std::env;\nfn f() { let _ = env::var(\"X\"); }", 2),
        (
            "use std::env::{self as e};\nfn f() { let _ = e::var(\"X\"); }",
            1,
        ),
        (
            "use ::std::env as process;\nfn f() { let _ = process::vars(); }",
            1,
        ),
        ("use std::env::*;\nfn f() { let _ = var(\"X\"); }", 1),
        ("use std::*;\nfn f() { let _ = env::var(\"X\"); }", 2),
        (
            "use std as platform;\nfn f() { let _ = platform::env::var(\"X\"); }",
            2,
        ),
        ("extern crate std as platform;", 1),
        (
            "use{std::{env as e}};\nfn f() { let _ = e::var(\"X\"); }",
            1,
        ),
        ("fn f() { let _ = std::r#env::var(\"X\"); }", 1),
        (
            "fn f() { let r#use = std::env::var(\"X\"); let _ = r#use; }",
            1,
        ),
        (
            "macro_rules! read { ($root:ident) => { $root::env::var(\"X\") }; }",
            1,
        ),
        (
            "use std::{\n    env as e,\n    fs,\n};\nfn f() { let _ = e::var(\"X\"); }",
            1,
        ),
        ("use std::env::{var_os, VarError};", 1),
        (
            "macro_rules! read { ($f:ident) => { std::env::$f(\"X\") }; }",
            1,
        ),
    ] {
        assert_eq!(paths::reads(source), count, "{source}");
    }
    for allowed in [
        "fn f(e: std::env::VarError) {}",
        "use std::{env::VarError, ffi::OsStr};",
        "use std::env::VarError;",
        "fn f() { let _ = std::env::args(); let _ = std::env::current_exe(); }",
        "fn f(p: &str) { let _ = std::env::split_paths(p); let _ = std::env::join_paths([p]); }",
        "fn f(_: std::env::SplitPaths<'_>, _: std::env::Vars, _: std::env::VarsOs) {}",
        "/// std::env::var is read by the root, never here.\nfn f() {}",
        "// use std::env;\nfn f() {}",
        "/* std::env::var(\"X\") */ fn f() {}",
        "fn f() -> &'static str { \"std::env::var\" }",
        "fn f() -> &'static str { r#\"use std::env;\"# }",
        "fn f<'a>(x: &'a str) -> char { let _ = x; 'e' }",
        "fn f() { let _ = my_env::var(\"X\"); let _ = environment::var(); }",
        "use crate::settings::source;",
        "fn iter() -> impl Iterator<Item = u8> + use<> { [1].into_iter() }",
    ] {
        assert_eq!(paths::reads(allowed), 0, "{allowed}");
    }
}

/// Every read or mutation in the scanned production sources below `root`, by
/// workspace-relative path, with how many lines read.
fn scan(root: &Path) -> BTreeMap<String, usize> {
    // Canonical, as the inventory's own root is, so a module a `#[path]`
    // declaration loads compares equal to the file found below it.
    let root = &root.canonicalize().unwrap();
    let roots = executable_roots(root);
    // Every scoped crate's files in one set, so a file one crate loads only
    // for tests and another loads for production is production.
    let mut sources = Vec::new();
    for member in members(root) {
        let source = root.join(&member).join("src");
        if member.starts_with("crates/") && source.is_dir() {
            every_file(&source, &mut sources);
        }
    }
    let mut found = BTreeMap::new();
    for (path, text) in production_sources(sources, &roots) {
        let count = paths::reads(&text);
        if count > 0 {
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            found.insert(relative, count);
        }
    }
    found
}

/// Every UTF-8 file below `directory`, whatever its extension, so a module a
/// `#[path]` declaration loads from a file not named `.rs` is read too.
fn every_file(directory: &Path, found: &mut Vec<(PathBuf, String)>) {
    let mut entries: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            every_file(&path, found);
        } else if let Ok(text) = fs::read_to_string(&path) {
            found.push((path, text));
        }
    }
}

#[test]
fn no_batter_library_reads_the_process_environment_outside_its_seam() {
    let root = workspace_root();
    let declared: BTreeMap<String, usize> = DECLARED_SEAMS
        .iter()
        .map(|(path, count)| ((*path).to_owned(), *count))
        .collect();
    assert_eq!(
        scan(&root),
        declared,
        "environment reads outside the declared seam, or a seam whose count changed"
    );
    // The scope reaches the flags family, the core and the operator
    // command's modules, whose root alone is exempt.
    let members = members(&root);
    for package in [
        "crates/batter-core",
        "crates/batter-flags-core",
        "crates/batter-flags",
        "crates/batter-flags-postgres",
        "crates/batter-flags-cli",
        "crates/batter-otlp",
    ] {
        assert!(
            members.iter().any(|member| member == package),
            "{package} is not a member"
        );
    }
    let roots = executable_roots(&root);
    assert!(roots.contains(&root.join("crates/batter-flags-cli/src/main.rs")));
}

/// A scratch workspace: one library in scope, one executable root, and one
/// native package out of scope.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("batter-env-scan-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let scratch = Self(root.canonicalize().unwrap());
        scratch.write(
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/flags-core\", \"crates/tool\", \"runledger/native\"]\n",
        );
        for package in ["crates/flags-core", "crates/tool", "runledger/native"] {
            scratch.write(
                &format!("{package}/Cargo.toml"),
                "[package]\nname = \"scratch\"\n",
            );
        }
        scratch
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_planted_read_is_counted_and_everything_exempt_is_not() {
    let scratch = Scratch::new();
    scratch.write(
        "crates/flags-core/src/lib.rs",
        "mod reads;\nmod types;\n#[cfg(test)]\nmod tests;\n",
    );
    // The planted read, in a production module of a library in scope.
    scratch.write(
        "crates/flags-core/src/reads.rs",
        "pub fn planted() -> bool {\n    std::env::var(\"FLAG_X\").is_ok()\n}\n",
    );
    // A lone `VarError` path names a type and reads nothing.
    scratch.write(
        "crates/flags-core/src/types.rs",
        "pub type Cause = std::env::VarError;\n",
    );
    // Only a `#[cfg(test)]` declaration loads this file.
    scratch.write(
        "crates/flags-core/src/tests.rs",
        "fn child() -> bool { std::env::var_os(\"CHILD\").is_some() }\n",
    );
    // A classified executable root captures its settings.
    scratch.write(
        "crates/tool/src/main.rs",
        "fn main() {\n    let _ = std::env::vars_os();\n}\n",
    );
    // A native package reads the environment by design.
    scratch.write(
        "runledger/native/src/lib.rs",
        "pub fn config() -> Option<String> { std::env::var(\"X\").ok() }\n",
    );
    let found = scan(&scratch.0);
    assert_eq!(
        found,
        BTreeMap::from([("crates/flags-core/src/reads.rs".to_owned(), 1)])
    );
    // The import, the `env::` call and the `std::env` path are three reaches.
    scratch.write(
        "crates/flags-core/src/reads.rs",
        "use std::env;\npub fn planted() -> bool {\n    env::var(\"FLAG_X\").is_ok() || std::env::var_os(\"Y\").is_some()\n}\n",
    );
    assert_eq!(
        scan(&scratch.0),
        BTreeMap::from([("crates/flags-core/src/reads.rs".to_owned(), 3)])
    );
}

#[test]
fn a_file_a_production_path_declaration_loads_is_scanned_from_any_root_spelling() {
    let scratch = Scratch::new();
    // A test module and a production `#[path]` module load the same file.
    scratch.write(
        "crates/flags-core/src/lib.rs",
        "#[cfg(test)]\nmod shared;\n#[path = \"shared.rs\"]\nmod production;\n",
    );
    scratch.write(
        "crates/flags-core/src/shared.rs",
        "pub fn planted() -> bool { std::env::var_os(\"X\").is_some() }\n",
    );
    let expected = BTreeMap::from([("crates/flags-core/src/shared.rs".to_owned(), 1)]);
    assert_eq!(scan(&scratch.0), expected);
    // A root spelled through `..` finds the same file.
    assert_eq!(scan(&scratch.0.join("crates").join("..")), expected);
}

#[test]
fn a_file_another_crate_loads_for_production_and_a_non_rust_module_are_scanned() {
    let scratch = Scratch::new();
    // `flags-core` loads `shared.rs` only for tests, and `ambient.inc` as a
    // production module whose file is not named `.rs`.
    scratch.write(
        "crates/flags-core/src/lib.rs",
        "#[cfg(test)]\nmod shared;\n#[path = \"ambient.inc\"]\nmod ambient;\n",
    );
    scratch.write(
        "crates/flags-core/src/shared.rs",
        "pub fn planted() -> bool { std::env::var_os(\"X\").is_some() }\n",
    );
    scratch.write(
        "crates/flags-core/src/ambient.inc",
        "pub fn ambient() -> bool { std::env::var(\"Y\").is_ok() }\n",
    );
    // The executable root of `tool` loads `shared.rs` as a production module.
    scratch.write(
        "crates/tool/src/main.rs",
        "#[path = \"../../flags-core/src/shared.rs\"]\nmod shared;\nfn main() {\n    let _ = shared::planted();\n}\n",
    );
    assert_eq!(
        scan(&scratch.0),
        BTreeMap::from([
            ("crates/flags-core/src/ambient.inc".to_owned(), 1),
            ("crates/flags-core/src/shared.rs".to_owned(), 1),
        ])
    );
}
