//! Which library source files are test-only, derived from module declarations
//! rather than file names or directories. A file is test-only when a
//! `#[cfg(test)]` declaration, or any declaration inside a test-only module,
//! resolves to it and no production declaration does: a file that a production
//! `mod` or `#[path]` declaration loads is scanned even when a test module loads
//! it too. Neither a name such as `tests.rs` nor a place inside a test module's
//! directory carries an exemption.
//!
//! The reader expects rustfmt layout: one declaration per line, its attributes
//! on the lines directly above it. It places no file for a declaration whose
//! location it does not follow: a `#[path]` inside an inline module, any
//! declaration inside an inline module that carries a path attribute, a path
//! set by another attribute such as `cfg_attr`, and a submodule of a file
//! loaded through `#[path]` without its own `#[path]`. A file that no
//! declaration resolves to, such as a crate root or a file loaded through one
//! of those forms, counts as production, and so do its ungated submodules.
//! Because such a declaration in production code may load a file that a test
//! module also loads, it leaves no file of its library test-only. A form the
//! reader does not follow can therefore only make the scan stricter. The
//! reader does not see modules that a macro declares or Rust code that
//! `include!` inserts.

use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};

/// Where a declaration's file is, as far as its attributes tell.
enum Location {
    /// The default file for its name.
    Default,
    /// A plain `#[path = "..."]`.
    Path(String),
    /// Unknown: another attribute names a path, as `cfg_attr` can, or an
    /// enclosing inline module carries one.
    Unknown,
}

/// One `mod` declaration, with the inline modules that enclose it.
struct Declaration {
    name: String,
    gated: bool,
    location: Location,
    inline: bool,
    /// Enclosing inline modules, outermost first, with their own gating.
    enclosing: Vec<(String, bool)>,
}

/// `mod NAME;` or `mod NAME {`, after an optional visibility, as `(NAME, inline)`.
fn module_item(line: &str) -> Option<(&str, bool)> {
    let mut rest = line.trim_start();
    if let Some(after) = rest.strip_prefix("pub") {
        rest = match after.strip_prefix('(') {
            Some(group) => &group[group.find(')')? + 1..],
            None if after.starts_with(char::is_whitespace) => after,
            None => return None,
        }
        .trim_start();
    }
    let rest = rest.strip_prefix("mod ")?.trim_start();
    let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))?;
    let (name, tail) = rest.split_at(end);
    match tail.trim_start().chars().next() {
        Some(';') if !name.is_empty() => Some((name, false)),
        Some('{') if !name.is_empty() => Some((name, true)),
        _ => None,
    }
}

/// The `mod` declarations of one rustfmt-formatted source file.
fn declarations(source: &str) -> Vec<Declaration> {
    let mut found = Vec::new();
    let mut attributes: Vec<&str> = Vec::new();
    // Inline modules still open, as (indent, name, gated, relocated).
    let mut open: Vec<(usize, String, bool, bool)> = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("#[") {
            attributes.push(trimmed);
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if trimmed.starts_with('}') && open.last().is_some_and(|(depth, ..)| *depth == indent) {
            open.pop();
        } else if let Some((name, inline)) = module_item(line) {
            open.retain(|(depth, ..)| *depth < indent);
            let gated = attributes.contains(&"#[cfg(test)]");
            let path = attributes
                .iter()
                .find_map(|attribute| attribute.strip_prefix("#[path = \"")?.strip_suffix("\"]"));
            let location = if open.iter().any(|(.., relocated)| *relocated) {
                Location::Unknown
            } else if let Some(path) = path {
                Location::Path(path.to_owned())
            } else if attributes
                .iter()
                .any(|attribute| attribute.contains("path"))
            {
                Location::Unknown
            } else {
                Location::Default
            };
            let relocated = !matches!(location, Location::Default);
            found.push(Declaration {
                name: name.to_owned(),
                gated,
                location,
                inline,
                enclosing: open
                    .iter()
                    .map(|(_, name, gated, _)| (name.clone(), *gated))
                    .collect(),
            });
            if inline {
                open.push((indent, name.to_owned(), gated, relocated));
            }
        }
        attributes.clear();
    }
    found
}

/// Where `file`'s submodules live: beside `lib.rs`, `main.rs` and `mod.rs`,
/// and otherwise in a directory named after the file's module.
fn module_directory(file: &Path) -> PathBuf {
    match file.file_name().and_then(|name| name.to_str()) {
        Some("lib.rs" | "main.rs" | "mod.rs") => file.parent().unwrap().to_path_buf(),
        _ => file.with_extension(""),
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            other => normalized.push(other),
        }
    }
    normalized
}

/// The target of a top-level `#[path]` declaration in `file`.
fn path_target(file: &Path, declaration: &Declaration) -> Option<PathBuf> {
    let Location::Path(path) = &declaration.location else {
        return None;
    };
    (!declaration.inline && declaration.enclosing.is_empty())
        .then(|| normalize(&file.parent().unwrap().join(path)))
}

/// The files that `declaration` in `file` loads: none for an inline module or
/// a location the reader does not follow, and for a default location both
/// candidates, since rustc rejects a module that has both (E0761).
fn targets(
    file: &Path,
    declaration: &Declaration,
    path_loaded: &BTreeSet<PathBuf>,
) -> Vec<PathBuf> {
    match &declaration.location {
        Location::Path(_) => path_target(file, declaration).into_iter().collect(),
        Location::Default if !declaration.inline && !path_loaded.contains(file) => {
            let mut base = module_directory(file);
            base.extend(declaration.enclosing.iter().map(|(name, _)| name));
            vec![
                base.join(format!("{}.rs", declaration.name)),
                base.join(&declaration.name).join("mod.rs"),
            ]
        }
        _ => Vec::new(),
    }
}

/// Derive which of `sources` are test-only: the files that a `#[cfg(test)]`
/// declaration or a declaration inside a test-only file resolves to, except
/// those that production code loads too. A production declaration whose file
/// the reader cannot place leaves nothing test-only.
pub(crate) fn test_only(sources: &[(PathBuf, String)]) -> BTreeSet<PathBuf> {
    let parsed: Vec<_> = sources
        .iter()
        .map(|(file, source)| (file.as_path(), declarations(source)))
        .collect();
    let path_loaded: BTreeSet<PathBuf> = parsed
        .iter()
        .flat_map(|(file, declarations)| {
            declarations
                .iter()
                .filter_map(|declaration| path_target(file, declaration))
        })
        .collect();
    // Each declaration as (declaring file, gated where it stands, files,
    // unplaced): an unplaced declaration loads a file the reader cannot name.
    let edges: Vec<_> = parsed
        .iter()
        .flat_map(|(file, declarations)| {
            declarations.iter().map(|declaration| {
                let gated =
                    declaration.gated || declaration.enclosing.iter().any(|(_, gated)| *gated);
                let files = targets(file, declaration, &path_loaded);
                let unplaced = !declaration.inline && files.is_empty();
                (*file, gated, files, unplaced)
            })
        })
        .collect();
    let resolved: BTreeSet<&PathBuf> = edges.iter().flat_map(|(_, _, files, _)| files).collect();
    // Production code starts at every file that no declaration resolves to.
    let mut production: BTreeSet<PathBuf> = sources
        .iter()
        .map(|(file, _)| file)
        .filter(|file| !resolved.contains(file))
        .cloned()
        .collect();
    let mut test = BTreeSet::new();
    loop {
        let size = (production.len(), test.len());
        for (file, gated, files, _) in &edges {
            if !*gated && production.contains(*file) {
                production.extend(files.iter().cloned());
            }
            if *gated || test.contains(*file) {
                test.extend(files.iter().cloned());
            }
        }
        if size == (production.len(), test.len()) {
            // Production code that loads a file the reader cannot name may
            // load any test file, so none can be exempt.
            let unplaced = edges.iter().any(|(file, gated, _, unplaced)| {
                *unplaced && !*gated && production.contains(*file)
            });
            return if unplaced {
                BTreeSet::new()
            } else {
                &test - &production
            };
        }
    }
}

#[test]
fn only_files_of_cfg_test_modules_are_test_only() {
    let file = |path: &str, source: &str| (PathBuf::from(path), source.to_owned());
    let sources = [
        file(
            "/src/lib.rs",
            "mod helpers;\n#[cfg(test)]\nmod tests;\n#[cfg(test)]\n#[path = \"pipeline/checks.rs\"]\n\
             mod checks;\n#[path = \"pipeline/guard.rs\"]\nmod guard;\nmod startup;\n\
             #[path = \"tests/helper.rs\"]\nmod helper;\n#[cfg(test)]\nmod fixtures;\n\
             #[path = \"fixtures.rs\"]\nmod shared;\n#[cfg(test)]\n#[path = \"moved\"]\n\
             mod relocated {\n    mod inner;\n}\n#[cfg(test)]\n\
             #[cfg_attr(test, path = \"elsewhere.rs\")]\nmod configured;\n",
        ),
        file("/src/helpers.rs", "pub(super) mod tests;\n"),
        file("/src/helpers/tests.rs", "fn untested() {}\n"),
        file("/src/tests.rs", "mod nested;\n"),
        file("/src/tests/nested.rs", ""),
        file("/src/tests/helper.rs", ""),
        file("/src/fixtures.rs", ""),
        file("/src/relocated/inner.rs", ""),
        file("/src/configured.rs", ""),
        file(
            "/src/pipeline/checks.rs",
            "#[path = \"checks/extra.rs\"]\nmod extra;\n",
        ),
        file("/src/pipeline/checks/extra.rs", ""),
        file("/src/pipeline/guard.rs", ""),
        file(
            "/src/startup.rs",
            "fn run() {}\n\n#[cfg(test)]\nmod tests {\n    mod supervisor;\n}\n\nmod after;\n",
        ),
        file("/src/startup/tests/supervisor.rs", ""),
        file("/src/startup/after.rs", ""),
        file("/src/x/main.rs", ""),
    ];
    let test_only = test_only(&sources);
    // A file or directory module, a `#[path]` module with its own `#[path]`
    // submodule, and an inline module with a file submodule.
    for path in [
        "/src/tests.rs",
        "/src/tests/nested.rs",
        "/src/pipeline/checks.rs",
        "/src/pipeline/checks/extra.rs",
        "/src/startup/tests/supervisor.rs",
    ] {
        assert!(
            test_only.contains(Path::new(path)),
            "{path} is not test-only"
        );
    }
    // Negative controls: names and places carry no exemption. A `tests`
    // module without `#[cfg(test)]`, a production `#[path]` module beside a
    // test one or inside a test module's directory, a test module's file that a
    // production `#[path]` module also loads, a module declared after an inline
    // test module closes, the default places of test modules whose path the
    // reader does not follow, and a nested `main.rs` remain production code.
    for path in [
        "/src/lib.rs",
        "/src/helpers.rs",
        "/src/helpers/tests.rs",
        "/src/pipeline/guard.rs",
        "/src/tests/helper.rs",
        "/src/fixtures.rs",
        "/src/startup.rs",
        "/src/startup/after.rs",
        "/src/relocated/inner.rs",
        "/src/configured.rs",
        "/src/x/main.rs",
    ] {
        assert!(!test_only.contains(Path::new(path)), "{path} is test-only");
    }
}

#[test]
fn production_modules_the_reader_cannot_place_exempt_nothing() {
    let file = |path: &str, source: &str| (PathBuf::from(path), source.to_owned());
    let library = |declarations: &str| {
        [
            file(
                "/src/lib.rs",
                &format!("#[cfg(test)]\nmod shared;\n#[cfg(test)]\nmod tests;\n{declarations}"),
            ),
            file("/src/shared.rs", ""),
            file("/src/loader.rs", "mod shared;\n"),
            file(
                "/src/tests.rs",
                "#[cfg_attr(unix, path = \"data.rs\")]\nmod data;\n",
            ),
        ]
    };
    // Control: when only test modules load it, the shared file is test-only,
    // even though a test module holds a declaration the reader cannot place.
    let control = test_only(&library(""));
    for path in ["/src/shared.rs", "/src/tests.rs"] {
        assert!(control.contains(Path::new(path)), "{path} is not test-only");
    }
    // A path set through `cfg_attr`, a `#[path]` inside an inline module and
    // a submodule of a file loaded through `#[path]`, each in production code
    // and each loading the shared file, leave nothing test-only.
    for production in [
        "#[cfg_attr(unix, path = \"shared.rs\")]\nmod platform;\n",
        "mod outer {\n    #[path = \"../shared.rs\"]\n    mod platform;\n}\n",
        "#[path = \"loader.rs\"]\nmod loader;\n",
    ] {
        let test_only = test_only(&library(production));
        assert!(
            test_only.is_empty(),
            "{production:?} left {test_only:?} test-only"
        );
    }
}
