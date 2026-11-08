//! The inventory both source scans read: the workspace members, their
//! library sources minus executable roots and test-only files, and every
//! checked-in executable root.
#![allow(dead_code, reason = "shared by two scans; each uses part of it")]

use std::{
    fs,
    path::{Path, PathBuf},
};

use super::modules;

/// Source directories built outside the workspace by consumer scripts.
pub(crate) const EXTERNAL_SOURCES: [&str; 2] = ["consumers", "runlimit/smoke"];

pub(crate) fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Workspace members as listed in the root manifest's `members` array.
pub(crate) fn members(root: &Path) -> Vec<String> {
    let manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let start = manifest.find("members = [").expect("workspace members") + "members = [".len();
    let end = start
        + manifest[start..]
            .find(']')
            .expect("end of workspace members");
    manifest[start..end]
        .split(',')
        .map(|member| member.trim().trim_matches('"').to_owned())
        .filter(|member| !member.is_empty())
        .collect()
}

/// Library packages: members with `src/lib.rs`, except the example packages,
/// which are application roots that may own a process hook.
pub(crate) fn library_sources(root: &Path) -> Vec<PathBuf> {
    members(root)
        .into_iter()
        .filter(|member| !member.starts_with("examples/"))
        .map(|member| root.join(member).join("src"))
        .filter(|source| source.join("lib.rs").is_file())
        .collect()
}

/// Every Rust source below `directory`, with its contents.
pub(crate) fn rust_sources(directory: &Path, found: &mut Vec<(PathBuf, String)>) {
    let mut entries: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            rust_sources(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let source = fs::read_to_string(&path).unwrap();
            found.push((path, source));
        }
    }
}

/// The production sources of one library `src` tree: everything except the
/// files that a `#[cfg(test)]` module loads and no production module does, and
/// exactly the `roots` that the executable-root inventory classifies, such as
/// `src/main.rs` and `src/bin/*.rs`. A binary's helper module, and a file under
/// `src/bin/` whose `main` the inventory does not recognize, are scanned.
pub(crate) fn production_sources(
    sources: Vec<(PathBuf, String)>,
    roots: &[PathBuf],
) -> Vec<(PathBuf, String)> {
    let test_only = modules::test_only(&sources);
    sources
        .into_iter()
        .filter(|(path, _)| !test_only.contains(path) && !roots.contains(path))
        .collect()
}

pub(crate) fn is_main_signature(line: &str) -> bool {
    [
        "fn main(",
        "async fn main(",
        "pub fn main(",
        "pub async fn main(",
    ]
    .iter()
    .any(|signature| line.starts_with(signature))
}

/// The first statement of the top-level `fn main` in `source`.
pub(crate) fn first_statement_of_main(source: &str) -> Result<&str, &'static str> {
    let lines: Vec<&str> = source.lines().collect();
    let signature = lines
        .iter()
        .position(|line| is_main_signature(line))
        .ok_or("no top-level main")?;
    let body = signature
        + lines[signature..]
            .iter()
            .position(|line| line.ends_with('{'))
            .ok_or("main has no body")?;
    lines[body + 1..]
        .iter()
        .map(|line| line.trim())
        .find(|line| !line.is_empty() && !line.starts_with("//"))
        .filter(|line| *line != "}")
        .ok_or("main is empty")
}

pub(crate) fn has_main(source: &str) -> bool {
    first_statement_of_main(source) != Err("no top-level main")
}

/// Rust sources directly in `directory`, plus each subdirectory's `main.rs`.
pub(crate) fn target_sources(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut sources = Vec::new();
    for path in entries.map(|entry| entry.unwrap().path()) {
        if path.is_dir() {
            sources.push(path.join("main.rs"));
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
    sources
}

/// Paths that a manifest's `[[bin]]`, `[[example]]` and `[[bench]]` tables
/// declare explicitly.
pub(crate) fn declared_target_paths(package: &Path) -> Vec<PathBuf> {
    let manifest = fs::read_to_string(package.join("Cargo.toml")).unwrap();
    manifest
        .split("\n[[")
        .skip(1)
        .filter(|table| {
            ["bin]]", "example]]", "bench]]"]
                .iter()
                .any(|kind| table.starts_with(kind))
        })
        .filter_map(|table| {
            table
                .lines()
                .find_map(|line| line.trim().strip_prefix("path = "))
                .map(|path| package.join(path.trim_matches('"')))
        })
        .collect()
}

/// Every checked-in executable root: each workspace member's `src/main.rs`,
/// `src/bin/`, `examples/`, `benches/` and declared target paths, and the
/// consumer sources that scripts build outside the workspace.
pub(crate) fn executable_roots(root: &Path) -> Vec<PathBuf> {
    let mut declared = Vec::new();
    let mut discovered = Vec::new();
    for member in members(root) {
        let package = root.join(member);
        declared.extend(declared_target_paths(&package));
        discovered.push(package.join("src/main.rs"));
        for directory in ["src/bin", "examples", "benches"] {
            discovered.extend(target_sources(&package.join(directory)));
        }
    }
    for external in EXTERNAL_SOURCES {
        discovered.extend(target_sources(&root.join(external)));
    }
    for path in &declared {
        let source =
            fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert!(has_main(&source), "{} declares no main", path.display());
    }
    let mut roots = declared;
    roots.extend(
        discovered
            .into_iter()
            .filter(|path| fs::read_to_string(path).is_ok_and(|source| has_main(&source))),
    );
    roots.sort();
    roots.dedup();
    roots
}
