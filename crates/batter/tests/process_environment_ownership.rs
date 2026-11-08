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
//! A read or mutation is a whole-identifier `var`, `var_os`, `vars`,
//! `vars_os`, `set_var`, `remove_var`, `current_dir` or `home_dir` right after
//! a whole-identifier `env::`, or a `use std::` line that imports the `env`
//! module itself or one of those functions; importing a type such as
//! `env::VarError` is not one, and neither is a `//` comment, rustdoc
//! included.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "panic_hook_ownership/inventory.rs"]
mod inventory;
#[path = "panic_hook_ownership/modules.rs"]
mod modules;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use inventory::{executable_roots, members, production_sources, rust_sources, workspace_root};

/// The `std::env` functions that read or change the environment or a
/// directory found through it.
const READS: [&str; 8] = [
    "var",
    "var_os",
    "vars",
    "vars_os",
    "set_var",
    "remove_var",
    "current_dir",
    "home_dir",
];

/// The one seam, and how many reads it holds: the OpenTelemetry adapter
/// refuses ambient `OTEL_*` settings, which its upstream builders would
/// otherwise read, by checking that none is set. It never reads a value.
const DECLARED_SEAMS: [(&str, usize); 1] = [("crates/batter-otlp/src/lib.rs", 1)];

fn is_identifier(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Whether `line` names `word` as a whole identifier at byte `start`.
fn whole(line: &str, start: usize, word: &str) -> bool {
    let bytes = line.as_bytes();
    let end = start + word.len();
    line[start..].starts_with(word)
        && (start == 0 || !is_identifier(bytes[start - 1]))
        && (end == bytes.len() || !is_identifier(bytes[end]))
}

/// Whether `line` is a read or mutation under the rule above.
fn reads(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") {
        return false;
    }
    if trimmed.starts_with("use std::") && imports_env(trimmed) {
        return true;
    }
    line.match_indices("env::").any(|(start, matched)| {
        let after = start + matched.len();
        whole(line, start, "env") && READS.iter().any(|function| whole(line, after, function))
    })
}

/// Whether a `use std::` line imports the `env` module itself, or a reading
/// function through an `env::{..}` group; importing a type such as
/// `env::VarError` does not.
fn imports_env(line: &str) -> bool {
    line.match_indices("env").any(|(start, _)| {
        if !whole(line, start, "env") {
            return false;
        }
        let rest = &line[start + "env".len()..];
        match rest.strip_prefix("::{") {
            Some(group) => {
                let group = &group[..group.find('}').unwrap_or(group.len())];
                group
                    .split(',')
                    .map(|name| name.split_whitespace().next().unwrap_or(""))
                    .any(|name| READS.contains(&name))
            }
            None => !rest.starts_with("::"),
        }
    })
}

#[test]
fn the_line_rule_counts_reads_and_mutations_and_keeps_types_and_comments() {
    for read in [
        "    let value = std::env::var(\"FLAG_FAST_PATH\");",
        "    if env::var_os(CHILD).is_some() {",
        "    for (key, _) in std::env::vars_os() {",
        "    std::env::set_var(\"A\", \"b\");",
        "    unsafe { env::remove_var(\"A\") };",
        "    let here = std::env::current_dir()?;",
        "    let home = std::env::home_dir();",
        "use std::env;",
        "use std::{env, fs};",
        "use std::env as process_env;",
        "use std::env::{var_os, VarError};",
        "use std::env::var;",
        "    run(); let _ = std::env::vars(); // trailing comment",
    ] {
        assert!(reads(read), "{read}");
    }
    for allowed in [
        "        .with_cause(std::env::VarError::NotUnicode(value.to_owned()))",
        "/// std::env::var is read by the root, never here.",
        "    // env::var_os(CHILD) is the child's marker.",
        "//! use std::env;",
        "    let value = my_env::var(name);",
        "    let all = env::variables();",
        "    let args = std::env::args();",
        "use std::environment;",
        "use std::{env::VarError, ffi::OsStr};",
        "use std::env::VarError;",
        "    let text = \"environment\";",
    ] {
        assert!(!reads(allowed), "{allowed}");
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
            let count = text.lines().filter(|line| reads(line)).count();
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
    // A second read in the same file is counted too.
    scratch.write(
        "crates/flags-core/src/reads.rs",
        "use std::env;\npub fn planted() -> bool {\n    env::var(\"FLAG_X\").is_ok()\n}\n",
    );
    assert_eq!(
        scan(&scratch.0),
        BTreeMap::from([("crates/flags-core/src/reads.rs".to_owned(), 2)])
    );
}
