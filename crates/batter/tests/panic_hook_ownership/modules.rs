//! Which library source files are test-only, derived from module declarations
//! rather than file names: a file is test-only when it belongs to a module
//! declared under `#[cfg(test)]`, directly or through that module's own
//! declarations. A name such as `tests.rs` carries no exemption.
//!
//! The reader expects rustfmt layout: one declaration per line, its attributes
//! on the lines directly above it. A declaration it cannot resolve, such as a
//! `#[path]` inside an inline module or a submodule of a file loaded through
//! `#[path]` without its own `#[path]`, leaves its files scanned as production
//! code, which can only make the scan stricter.

use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};

/// One `mod` declaration, with the inline modules that enclose it.
struct Declaration {
    name: String,
    gated: bool,
    path: Option<String>,
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
    // Inline modules still open, as (indent, name, gated).
    let mut open: Vec<(usize, String, bool)> = Vec::new();
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
            let path = attributes.iter().find_map(|attribute| {
                let path = attribute.strip_prefix("#[path = \"")?.strip_suffix("\"]")?;
                Some(path.to_owned())
            });
            found.push(Declaration {
                name: name.to_owned(),
                gated,
                path,
                inline,
                enclosing: open
                    .iter()
                    .map(|(_, name, gated)| (name.clone(), *gated))
                    .collect(),
            });
            if inline {
                open.push((indent, name.to_owned(), gated));
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
    let path = declaration.path.as_ref()?;
    (!declaration.inline && declaration.enclosing.is_empty())
        .then(|| normalize(&file.parent().unwrap().join(path)))
}

/// The test-only files of one crate's sources.
pub(crate) struct TestOnly {
    files: BTreeSet<PathBuf>,
    directories: BTreeSet<PathBuf>,
}

impl TestOnly {
    pub(crate) fn contains(&self, path: &Path) -> bool {
        self.files.contains(path)
            || self
                .directories
                .iter()
                .any(|directory| path.starts_with(directory))
    }

    fn add(&mut self, file: &Path, declaration: &Declaration, path_loaded: &BTreeSet<PathBuf>) {
        if declaration.path.is_some() {
            // Only a top-level `#[path]` declaration resolves to a known file.
            self.files.extend(path_target(file, declaration));
            return;
        }
        if path_loaded.contains(file) {
            return;
        }
        let mut base = module_directory(file);
        base.extend(declaration.enclosing.iter().map(|(name, _)| name));
        let directory = base.join(&declaration.name);
        if !declaration.inline {
            self.files
                .insert(base.join(format!("{}.rs", declaration.name)));
            self.files.insert(directory.join("mod.rs"));
        }
        self.directories.insert(directory);
    }
}

/// Derive which of `sources` are test-only, following `#[cfg(test)]`
/// declarations and then every declaration inside a test-only file.
pub(crate) fn test_only(sources: &[(PathBuf, String)]) -> TestOnly {
    let parsed: Vec<_> = sources
        .iter()
        .map(|(file, source)| (file, declarations(source)))
        .collect();
    let path_loaded: BTreeSet<PathBuf> = parsed
        .iter()
        .flat_map(|(file, declarations)| {
            declarations
                .iter()
                .filter_map(|declaration| path_target(file, declaration))
        })
        .collect();
    let mut test_only = TestOnly {
        files: BTreeSet::new(),
        directories: BTreeSet::new(),
    };
    loop {
        let size = (test_only.files.len(), test_only.directories.len());
        for (file, declarations) in &parsed {
            let file_is_test_only = test_only.contains(file);
            for declaration in declarations {
                if file_is_test_only
                    || declaration.gated
                    || declaration.enclosing.iter().any(|(_, gated)| *gated)
                {
                    test_only.add(file, declaration, &path_loaded);
                }
            }
        }
        if size == (test_only.files.len(), test_only.directories.len()) {
            return test_only;
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
             mod checks;\n#[path = \"pipeline/guard.rs\"]\nmod guard;\nmod startup;\n",
        ),
        file("/src/helpers.rs", "pub(super) mod tests;\n"),
        file("/src/helpers/tests.rs", "fn untested() {}\n"),
        file("/src/tests.rs", "mod nested;\n"),
        file("/src/tests/nested.rs", ""),
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
    // Negative controls: names carry no exemption. A `tests` module without
    // `#[cfg(test)]`, a production `#[path]` module beside a test one, a
    // module declared after an inline test module closes, and a nested
    // `main.rs` remain production code.
    for path in [
        "/src/lib.rs",
        "/src/helpers.rs",
        "/src/helpers/tests.rs",
        "/src/pipeline/guard.rs",
        "/src/startup.rs",
        "/src/startup/after.rs",
        "/src/x/main.rs",
    ] {
        assert!(!test_only.contains(Path::new(path)), "{path} is test-only");
    }
}
