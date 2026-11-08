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
//! tooling roots keep the default hook, and native Runledger and Runlimit
//! roots, which name no Batter crate, keep their own panic policy. Programs
//! that scripts generate as compile and identity probes are not checked-in
//! roots and are not classified.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "panic_hook_ownership/inventory.rs"]
mod inventory;
#[path = "panic_hook_ownership/modules.rs"]
mod modules;

use std::{
    fs,
    path::{Path, PathBuf},
};

use inventory::{
    executable_roots, first_statement_of_main, has_main, library_sources, production_sources,
    rust_sources, workspace_root,
};

/// The `std::panic` functions that install, take or replace the process hook.
const HOOK_FUNCTIONS: [&str; 3] = ["set_hook", "take_hook", "update_hook"];
const INSTALL_FIRST: &str = "std::panic::set_hook(batter::redacting_panic_hook());";

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

/// Native packages stay independent of the facade; a root there that names no
/// Batter crate keeps its own panic policy.
const NATIVE_PREFIXES: [&str; 2] = ["runledger/", "runlimit/"];

/// The two accepted spellings, through the facade or the core crate.
const INSTALL_STATEMENTS: [&str; 2] = [
    INSTALL_FIRST,
    "std::panic::set_hook(batter_core::redacting_panic_hook());",
];

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
        "crates/batter-flags-cli/src/main.rs",
        "crates/batter/examples/rollout.rs",
    ] {
        assert!(
            checked.contains(&&root.join(required)),
            "{required} is not checked"
        );
    }
    // The exact inventory: adding or removing a root updates these counts.
    assert_eq!(checked.len(), 18, "Batter-consuming roots: {checked:#?}");
    assert_eq!(native.len(), 12, "native roots: {native:#?}");
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
