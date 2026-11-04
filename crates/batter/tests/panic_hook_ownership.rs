//! Process panic-hook ownership, checked in source because the hook is
//! process-global: no Batter library installs, takes or replaces it, and every
//! checked-in executable root that consumes Batter installs the redacting hook
//! as its first statement, except three named tooling roots.
//!
//! The library scan covers every production code path of every workspace
//! library, including paths no test exercises, but not dependencies; the
//! `panic_hook` target in `batter-core` checks the exercised entrypoints at run
//! time. A library file is exempt only when it is a binary root that the
//! inventory below classifies, or when a module under `#[cfg(test)]` loads it
//! and no production module does, as `modules` derives from module declarations
//! rather than file names or directories. Every checked-in executable root is
//! classified, meaning each workspace member's binaries, examples and
//! benchmarks and the consumer sources that scripts build outside the
//! workspace: a root that consumes Batter installs the hook, three named
//! tooling roots keep the default hook, and native Runledger, Runlimit and
//! Runflag roots, which name no Batter crate, keep their own panic policy. Programs
//! that scripts generate as compile and identity probes are not checked-in
//! roots and are not classified.

#[path = "panic_hook_ownership/modules.rs"]
mod modules;

use std::{
    fs,
    path::{Path, PathBuf},
};

/// The `std::panic` functions that install, take or replace the process hook.
const HOOK_FUNCTIONS: [&str; 3] = ["set_hook", "take_hook", "update_hook"];
const INSTALL_FIRST: &str = "std::panic::set_hook(batter::redacting_panic_hook());";

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Workspace members as listed in the root manifest's `members` array.
fn members(root: &Path) -> Vec<String> {
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
fn library_sources(root: &Path) -> Vec<PathBuf> {
    members(root)
        .into_iter()
        .filter(|member| !member.starts_with("examples/"))
        .map(|member| root.join(member).join("src"))
        .filter(|source| source.join("lib.rs").is_file())
        .collect()
}

/// Every Rust source below `directory`, with its contents.
fn rust_sources(directory: &Path, found: &mut Vec<(PathBuf, String)>) {
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
fn production_sources(
    sources: Vec<(PathBuf, String)>,
    roots: &[PathBuf],
) -> Vec<(PathBuf, String)> {
    let test_only = modules::test_only(&sources);
    sources
        .into_iter()
        .filter(|(path, _)| !test_only.contains(path) && !roots.contains(path))
        .collect()
}

#[test]
fn only_binary_roots_and_cfg_test_modules_are_exempt() {
    let file = |path: &str, source: &str| (PathBuf::from(path), source.to_owned());
    let sources = vec![
        file("/src/lib.rs", "#[cfg(test)]\nmod tests;\nmod helpers;\n"),
        file("/src/tests.rs", ""),
        file("/src/helpers.rs", "mod tests;\n"),
        file("/src/helpers/tests.rs", ""),
        file("/src/main.rs", ""),
        file("/src/bin/tool.rs", ""),
        file("/src/bin/server/main.rs", ""),
        file("/src/bin/server/support.rs", ""),
        file("/src/bin/unrecognized.rs", ""),
        file("/src/x/main.rs", ""),
        file("/src/x/bin/y.rs", ""),
    ];
    // The roots the inventory classified: it recognized no `main` in
    // `/src/bin/unrecognized.rs`, so that file is not one.
    let roots = [
        "/src/main.rs",
        "/src/bin/tool.rs",
        "/src/bin/server/main.rs",
    ]
    .map(PathBuf::from);
    let production: Vec<_> = production_sources(sources, &roots)
        .into_iter()
        .map(|(path, _)| path)
        .collect();
    // A binary's helper module and an unclassified file under `src/bin/` are
    // neither roots nor test-only, so they are scanned.
    let expected = [
        "/src/lib.rs",
        "/src/helpers.rs",
        "/src/helpers/tests.rs",
        "/src/bin/server/support.rs",
        "/src/bin/unrecognized.rs",
        "/src/x/main.rs",
        "/src/x/bin/y.rs",
    ];
    assert_eq!(production, expected.map(PathBuf::from));
}

fn is_identifier(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Whether `line` names a hook function as a whole identifier.
fn names_hook_function(line: &str) -> bool {
    HOOK_FUNCTIONS.iter().any(|function| {
        line.match_indices(function).any(|(start, _)| {
            let bytes = line.as_bytes();
            let end = start + function.len();
            (start == 0 || !is_identifier(bytes[start - 1]))
                && (end == bytes.len() || !is_identifier(bytes[end]))
        })
    })
}

/// A line that names a hook function is allowed only as a `//` comment,
/// including rustdoc. Code, attributes and strings that name one are not.
fn violates(line: &str) -> bool {
    names_hook_function(line) && !line.trim_start().starts_with("//")
}

#[test]
fn the_line_rule_flags_code_and_keeps_comments() {
    for code in [
        "    std::panic::set_hook(hook);",
        "use std::panic::{self, take_hook as previous};",
        "    let _ = panic::update_hook(|previous, info| previous(info));",
        "    run(); // and set_hook",
    ] {
        assert!(violates(code), "{code}");
    }
    for allowed in [
        "/// std::panic::set_hook(batter::redacting_panic_hook());",
        "    // A later take_hook replaces it.",
        "//! update_hook",
        "    let set_hooks = reset_hook_count();",
    ] {
        assert!(!violates(allowed), "{allowed}");
    }
}

#[test]
fn no_batter_library_installs_takes_or_replaces_the_panic_hook() {
    let root = workspace_root();
    let sources = library_sources(&root);
    for package in [
        "crates/batter",
        "crates/batter-core",
        "crates/batter-axum",
        "runledger/runledger-runtime",
        "runlimit/runlimit-core",
    ] {
        assert!(
            sources.contains(&root.join(package).join("src")),
            "{package} is not scanned"
        );
    }
    let roots = executable_roots(&root);
    let mut violations = Vec::new();
    let mut comment_mentions = 0;
    let mut exempt_installers = 0;
    for source in &sources {
        let mut files = Vec::new();
        rust_sources(source, &mut files);
        assert!(!files.is_empty(), "{} has no sources", source.display());
        let production = production_sources(files.clone(), &roots);
        for (file, text) in &files {
            let exempt = !production.iter().any(|(path, _)| path == file);
            for (index, line) in text.lines().enumerate() {
                if exempt {
                    exempt_installers += usize::from(violates(line));
                } else if violates(line) {
                    violations.push(format!("{}:{}", file.display(), index + 1));
                } else if names_hook_function(line) {
                    comment_mentions += 1;
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "library code names a panic-hook installer; keep test hooks in modules \
         declared under `#[cfg(test)]`: {violations:?}"
    );
    // The redacting hook's own rustdoc names `set_hook`, so the comment
    // allowance is exercised rather than vacuous.
    assert!(comment_mentions > 0);
    // A `#[cfg(test)]` module in a native library installs a hook, so the
    // exemption is exercised on real declarations rather than vacuous. If that
    // test moves, the self-tests still cover the exemption rule.
    assert!(exempt_installers > 0);
}

/// Tooling that keeps Rust's default hook, so that its failures print the
/// diagnostic payloads they exist to show. None is an application root.
const TOOLING_ROOTS: [&str; 3] = [
    // The live-suite preflight that `scripts/reference_live.py` runs first.
    "examples/reference-service/examples/reference_preflight.rs",
    // The provisioning harness that runs the single-facade consumer.
    "consumers/single_facade_harness.rs",
    // The HTTP adapter's dispatch benchmark.
    "crates/batter-axum/benches/declared_dispatch.rs",
];

/// Source directories built outside the workspace by consumer scripts.
const EXTERNAL_SOURCES: [&str; 2] = ["consumers", "runlimit/smoke"];

/// Native packages stay independent of the facade; a root there that names no
/// Batter crate keeps its own panic policy.
const NATIVE_PREFIXES: [&str; 3] = ["runledger/", "runlimit/", "runflag/"];

/// The native operator root's own payload-free hook, installed first. The
/// root names no Batter crate, so it is classified native, and this keeps its
/// panic policy checked in the same place as the consuming roots'.
const NATIVE_OPERATOR_ROOT: &str = "runflag/runflag-cli/src/main.rs";
const NATIVE_OPERATOR_INSTALL: &str = "std::panic::set_hook(Box::new(payload_free_hook));";

/// The two accepted spellings, through the facade or the core crate.
const INSTALL_STATEMENTS: [&str; 2] = [
    INSTALL_FIRST,
    "std::panic::set_hook(batter_core::redacting_panic_hook());",
];

fn is_main_signature(line: &str) -> bool {
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
fn first_statement_of_main(source: &str) -> Result<&str, &'static str> {
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

fn has_main(source: &str) -> bool {
    first_statement_of_main(source) != Err("no top-level main")
}

#[test]
fn the_first_statement_rule_reads_main_bodies() {
    let decorated = "#[tokio::main(flavor = \"current_thread\")]\nasync fn main() -> ExitCode {\n    \
                     // A comment first.\n\n    std::panic::set_hook(batter::redacting_panic_hook());\n}\n";
    assert_eq!(first_statement_of_main(decorated), Ok(INSTALL_FIRST));
    let wrapped = "pub async fn main(\n) -> Result<\n    (),\n    Error,\n> {\n    \
                   std::panic::set_hook(batter::redacting_panic_hook());\n}\n";
    assert_eq!(first_statement_of_main(wrapped), Ok(INSTALL_FIRST));
    // Configuration loaded before the hook is installed is rejected.
    let late = "fn main() {\n    let settings = load();\n    \
                std::panic::set_hook(batter::redacting_panic_hook());\n}\n";
    assert_eq!(first_statement_of_main(late), Ok("let settings = load();"));
    assert_eq!(
        first_statement_of_main("fn main() {\n}\n"),
        Err("main is empty")
    );
    assert!(!has_main("fn helper() {}\n    fn main() {}\n"));
}

/// Rust sources directly in `directory`, plus each subdirectory's `main.rs`.
fn target_sources(directory: &Path) -> Vec<PathBuf> {
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
fn declared_target_paths(package: &Path) -> Vec<PathBuf> {
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
fn executable_roots(root: &Path) -> Vec<PathBuf> {
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

/// A native root lives in a native package and names no Batter crate.
fn is_native(root: &Path, path: &Path) -> bool {
    let relative = path.strip_prefix(root).unwrap().to_string_lossy();
    NATIVE_PREFIXES
        .iter()
        .any(|prefix| relative.starts_with(prefix))
        && !fs::read_to_string(path).unwrap().contains("batter")
}

/// Every executable root is classified: a root that consumes Batter installs
/// the hook first, a named tooling root keeps the default hook, and a native
/// root keeps its package's own policy. This checks this repository's roots,
/// not that an application installs the hook: no local check can see another
/// binary's `main`.
#[test]
fn every_batter_consuming_root_installs_the_redacting_hook_first() {
    let root = workspace_root();
    let roots = executable_roots(&root);
    for tooling in TOOLING_ROOTS {
        assert!(
            roots.contains(&root.join(tooling)),
            "{tooling} is no longer a root"
        );
    }
    let (native, consuming): (Vec<_>, Vec<_>) =
        roots.iter().partition(|path| is_native(&root, path));
    for native_root in [
        "runledger/runledger-tui/src/main.rs",
        "runledger/runledger-runtime/examples/producer_worker/worker.rs",
        "runlimit/smoke/native_consumer.rs",
        NATIVE_OPERATOR_ROOT,
    ] {
        assert!(
            native.contains(&&root.join(native_root)),
            "{native_root} is not a native root"
        );
    }
    let checked: Vec<_> = consuming
        .into_iter()
        .filter(|path| {
            !TOOLING_ROOTS
                .iter()
                .any(|tooling| **path == root.join(tooling))
        })
        .collect();
    for required in [
        "crates/batter/examples/http_service.rs",
        "crates/batter/examples/finite_command.rs",
        "examples/reference-service/src/main.rs",
        "examples/reference-service/examples/retire_startup_controls.rs",
        "examples/reference-service/tests/fixtures/signal_witness_fixture.rs",
        "examples/postgres-lifecycle/src/main.rs",
        "consumers/single_facade_consumer.rs",
        "runlimit/smoke/facade_consumer.rs",
    ] {
        assert!(
            checked.contains(&&root.join(required)),
            "{required} is not checked"
        );
    }
    // The exact inventory: adding or removing a root updates these counts.
    assert_eq!(checked.len(), 16, "Batter-consuming roots: {checked:#?}");
    assert_eq!(native.len(), 13, "native roots: {native:#?}");
    let operator = fs::read_to_string(root.join(NATIVE_OPERATOR_ROOT)).unwrap();
    assert_eq!(
        first_statement_of_main(&operator),
        Ok(NATIVE_OPERATOR_INSTALL),
        "{NATIVE_OPERATOR_ROOT}: first statement of main"
    );
    for path in checked {
        let source = fs::read_to_string(path).unwrap();
        let first = first_statement_of_main(&source);
        assert!(
            first.is_ok_and(|statement| INSTALL_STATEMENTS.contains(&statement)),
            "{}: first statement of main is {first:?}",
            path.display()
        );
    }
}
