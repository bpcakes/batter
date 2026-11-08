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

use inventory::{executable_roots, members, production_sources, rust_sources, workspace_root};

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
        ("use std::env;\nfn f() { let _ = env::var(\"X\"); }", 1),
        (
            "use std::env::{self as e};\nfn f() { let _ = e::var(\"X\"); }",
            1,
        ),
        (
            "use ::std::env as process;\nfn f() { let _ = process::vars(); }",
            1,
        ),
        ("use std::env::*;\nfn f() { let _ = var(\"X\"); }", 1),
        ("use std::*;\nfn f() { let _ = env::var(\"X\"); }", 1),
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
        "/// std::env::var is read by the root, never here.\nfn f() {}",
        "// use std::env;\nfn f() {}",
        "/* std::env::var(\"X\") */ fn f() {}",
        "fn f() -> &'static str { \"std::env::var\" }",
        "fn f() -> &'static str { r#\"use std::env;\"# }",
        "fn f<'a>(x: &'a str) -> char { let _ = x; 'e' }",
        "fn f() { let _ = my_env::var(\"X\"); let _ = environment::var(); }",
        "use crate::settings::source;",
    ] {
        assert_eq!(paths::reads(allowed), 0, "{allowed}");
    }
}

/// Every read or mutation in the scanned production sources below `root`, by
/// workspace-relative path, with how many lines read.
fn scan(root: &Path) -> BTreeMap<String, usize> {
    let roots = executable_roots(root);
    let mut found = BTreeMap::new();
    for member in members(root) {
        if !member.starts_with("crates/") {
            continue;
        }
        let source = root.join(&member).join("src");
        if !source.is_dir() {
            continue;
        }
        let mut sources = Vec::new();
        rust_sources(&source, &mut sources);
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
    }
    found
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
        let scratch = Self(root);
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
    // Importing the module is one reach, and a second path is another.
    scratch.write(
        "crates/flags-core/src/reads.rs",
        "use std::env;\npub fn planted() -> bool {\n    env::var(\"FLAG_X\").is_ok() || std::env::var_os(\"Y\").is_some()\n}\n",
    );
    assert_eq!(
        scan(&scratch.0),
        BTreeMap::from([("crates/flags-core/src/reads.rs".to_owned(), 2)])
    );
}
