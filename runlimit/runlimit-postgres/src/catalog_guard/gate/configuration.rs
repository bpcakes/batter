//! The configurations the crate compiles, which must be the ones its checks
//! see: the statement guard reads the test build's statements, and the SQL
//! entry-point gate is a Clippy lint, so it holds only for code a Clippy run
//! compiles. Statements compile the same in every build, the crate compiles
//! conditionally only on what the Clippy runs set both ways, and the
//! statement generator admits only literal text.

use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use super::{is_identifier, manifest_directory, normalize, rust_files};

/// The test-only inventory helpers the shared statement generator defines,
/// each by its item head and how often it appears. Any other conditional
/// compilation in statements.rs, or any at all in a family's file, could
/// give the production build other SQL than the tests check.
const TEST_HELPERS: [(&str, usize); 5] = [
    ("#[cfg(test)] pub(crate) const ALL: &[Self]", 1),
    (
        "#[cfg(test)] pub(crate) const fn name(self) -> &'static str",
        1,
    ),
    (
        "#[cfg(test)] pub(crate) const fn file(self) -> Option<&'static str>",
        2,
    ),
    ("#[cfg(test)] fn name(self) -> String", 1),
    (
        "#[cfg(test)] pub(crate) fn all() -> Vec<(String, Statement)>",
        1,
    ),
];

/// Whether normalized `code` compiles anything conditionally.
fn conditional(code: &str) -> bool {
    configuration_words(code).next().is_some()
}

/// Each identifier in normalized `code` that begins with `cfg`: `cfg` and
/// `cfg_attr`, and any other spelling that selects code by configuration,
/// such as `cfg_select!`, which the configuration checks reject outright.
fn configuration_words(code: &str) -> impl Iterator<Item = (usize, &str)> {
    code.match_indices("cfg").filter_map(|(at, _)| {
        if code[..at].chars().next_back().is_some_and(is_identifier) {
            return None;
        }
        let end = code[at..]
            .find(|character: char| !is_identifier(character))
            .map_or(code.len(), |length| at + length);
        Some((at, &code[at..end]))
    })
}

// The statement guard reads the test build's statements, so every build
// must compile the same ones: a family's file holds no conditional
// compilation, inside a macro or not; statements.rs holds only the pinned
// test helpers; and lib.rs declares the module unconditionally.
#[test]
fn every_build_compiles_the_same_statements() {
    let source = manifest_directory().join("src");
    let mut problems = Vec::new();
    for (file, path) in rust_files(&source) {
        let mut code = normalize(&fs::read_to_string(&path).expect("source is readable"));
        if file == "statements.rs" {
            for (head, count) in TEST_HELPERS {
                let head = normalize(head);
                if code.matches(&head).count() != count {
                    problems.push(format!(
                        "statements.rs: `{head}` is not there {count} times"
                    ));
                }
                code = code.replace(&head, "");
            }
        } else if !file.starts_with("statements/") {
            continue;
        }
        if conditional(&code) {
            problems.push(format!("{file}: compiles statements conditionally"));
        }
    }
    let root = normalize(&fs::read_to_string(source.join("lib.rs")).expect("lib.rs"));
    let declaration = root
        .find("mod statements;")
        .expect("lib.rs declares statements");
    let item_start = root[..declaration].rfind([';', '}']).map_or(0, |at| at + 1);
    if conditional(&root[item_start..declaration]) {
        problems.push("lib.rs: declares statements conditionally".to_owned());
    }
    assert!(problems.is_empty(), "{problems:#?}");
    for family in [
        "#[cfg(test)]statements!{A=\"x\";}",
        "macro_rules!pick{()=>{#[cfg(not(test))]statements!{A=\"y\";}};}pick!();",
        "#[cfg_attr(test,doc=\"x\")]mod inner{}",
        "const B:bool=cfg!(test);",
        "cfg_select!{test=>{statements!{A=\"x\";}}_=>{statements!{A=\"y\";}}}",
    ] {
        assert!(conditional(&normalize(family)), "{family}");
    }
}

/// The one item the macro probes' flag may compile.
const PROBE_MODULE: &str = "#[cfg(runlimit_macro_probes)] mod macro_probes;";

/// Where each attribute's contents start and end in normalized `code`.
fn attribute_spans(code: &str) -> Vec<(usize, usize)> {
    code.match_indices('#')
        .filter_map(|(at, _)| {
            let bang = usize::from(code[at + 1..].starts_with('!'));
            let start = at + 1 + bang + 1;
            code[at + 1 + bang..].starts_with('[').then_some(())?;
            let mut depth = 1_usize;
            let end = code[start..].char_indices().find_map(|(offset, c)| {
                match c {
                    '[' => depth += 1,
                    ']' => depth -= 1,
                    _ => {}
                }
                (depth == 0).then_some(start + offset)
            })?;
            Some((start, end))
        })
        .collect()
}

/// Whether the word at `at` sits in a `use` item, which could import a
/// configuration macro under another name. A use tree holds no `;`, so the
/// item runs from its `use` keyword to the next one, groups such as
/// `use core::{cfg_select as pick, fmt};` included; `use<..>` is precise
/// capturing in a signature, not an import.
fn in_use_item(code: &str, at: usize) -> bool {
    code.match_indices("use").any(|(start, _)| {
        let after = &code[start + "use".len()..];
        let keyword = !code[..start].chars().next_back().is_some_and(is_identifier)
            && !after.chars().next().is_some_and(is_identifier)
            && !after.starts_with('<');
        keyword && start < at && code[start..].find(';').is_some_and(|end| at < start + end)
    })
}

/// The configuration predicates in normalized `code`: each of a `cfg` or
/// `cfg_attr` attribute and a `cfg!` call. Any other configuration syntax (a
/// `cfg`-prefixed macro such as `cfg_select!`, a `cfg` word elsewhere in an
/// attribute, or one a `use` item imports) yields its word, which no check
/// accepts. An ordinary name such as `cfg_for_pool(...)` is none.
fn configuration_predicates(code: &str) -> Vec<String> {
    let attributes = attribute_spans(code);
    let mut found = Vec::new();
    for (at, word) in configuration_words(code) {
        let rest = &code[at + word.len()..];
        let in_attribute = attributes
            .iter()
            .any(|&(start, end)| (start..end).contains(&at));
        let macro_call = rest.starts_with('!');
        if !in_attribute && !macro_call && !in_use_item(code, at) {
            continue;
        }
        let arguments = match (word, in_attribute, macro_call) {
            ("cfg_attr", true, false) => rest.strip_prefix('(').map(|arguments| (true, arguments)),
            ("cfg", true, false) => rest.strip_prefix('(').map(|arguments| (false, arguments)),
            ("cfg", _, true) => rest.strip_prefix("!(").map(|arguments| (false, arguments)),
            _ => None,
        };
        let Some((attribute, arguments)) = arguments else {
            found.push(word.to_owned());
            continue;
        };
        let mut depth = 1_usize;
        let end = arguments
            .char_indices()
            .find(|&(_, character)| match character {
                '(' => {
                    depth += 1;
                    false
                }
                ')' => {
                    depth -= 1;
                    depth == 0
                }
                ',' => attribute && depth == 1,
                _ => false,
            })
            .map_or(arguments.len(), |(index, _)| index);
        found.push(arguments[..end].to_owned());
    }
    found
}

/// The top-level comma-separated parts of a normalized predicate list.
fn predicate_parts(list: &str) -> Vec<&str> {
    let mut depth = 0_usize;
    let mut start = 0;
    let mut parts = Vec::new();
    for (index, character) in list.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&list[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&list[start..]);
    parts
}

/// Whether the Clippy runs set normalized `predicate` both ways: `test`
/// through `--all-targets` and the crate's one optional feature through the
/// default and all-features runs, joined by `not`, `all` and `any`. A
/// release-only, platform-only or other branch would compile code no run
/// lints. The macro probes' flag is no such predicate: its one declaration
/// is pinned by [`PROBE_MODULE`].
fn linted(predicate: &str) -> bool {
    if predicate == "test" || predicate == normalize("feature = \"serde\"") {
        return true;
    }
    ["not(", "all(", "any("].iter().any(|combinator| {
        predicate
            .strip_prefix(combinator)
            .and_then(|inner| inner.strip_suffix(')'))
            .is_some_and(|inner| predicate_parts(inner).into_iter().all(linted))
    })
}

// The SQL entry-point gate is a lint, so it holds only for code a Clippy run
// compiles: the crate compiles conditionally only on what those runs set.
#[test]
fn clippy_lints_every_configuration_the_crate_compiles() {
    let source = manifest_directory().join("src");
    let mut problems = Vec::new();
    for (file, path) in rust_files(&source) {
        let mut code = normalize(&fs::read_to_string(&path).expect("source is readable"));
        // The probe run caps every lint and reads only the probes' reports,
        // so its flag compiles that one module and nothing else.
        if file == "catalog_guard/gate.rs" {
            let probe_module = normalize(PROBE_MODULE);
            if code.matches(&probe_module).count() != 1 {
                problems.push(format!("{file}: `{PROBE_MODULE}` is not declared once"));
            }
            code = code.replace(&probe_module, "");
        }
        for predicate in configuration_predicates(&code) {
            if !linted(&predicate) {
                problems.push(format!(
                    "{file}: compiles on `{predicate}`, which no Clippy run sets"
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
    let code = normalize(
        "#[cfg(not(debug_assertions))] fn a() {} #[cfg_attr(test, expect(x))] fn b() {}
         const C: bool = cfg!(all(test, feature = \"serde\")); fn my_cfg() {}",
    );
    let predicates = configuration_predicates(&code);
    assert_eq!(
        predicates
            .iter()
            .map(|predicate| linted(predicate))
            .collect::<Vec<_>>(),
        [false, true, true],
        "{predicates:?}"
    );
    for unlinted in [
        "unix",
        "target_os = \"macos\"",
        "any(test, panic = \"abort\")",
        "runlimit_macro_probes",
        "all(runlimit_macro_probes, feature = \"serde\")",
    ] {
        assert!(!linted(&normalize(unlinted)), "{unlinted}");
    }
    let selected = normalize("cfg_select! { test => { const A: u8 = 1; } _ => {} }");
    assert_eq!(configuration_predicates(&selected), ["cfg_select"]);
    assert!(conditional(&selected));
    for imported in [
        "use core::cfg_select as pick; pick! { test => {} _ => {} }",
        "use core::{cfg_select as pick, fmt}; pick! { test => {} _ => {} }",
        "use core::{fmt, macros::{cfg_select}};",
    ] {
        assert_eq!(
            configuration_predicates(&normalize(imported)),
            ["cfg_select"],
            "{imported}"
        );
    }
    let ordinary = normalize(
        "fn cfg_for_pool() {} fn b() { let cfg = 1; cfg_for_pool(); }
         fn c() -> impl Sized + use<> { let cfg_x = 1; cfg_x }",
    );
    assert_eq!(configuration_predicates(&ordinary), Vec::<String>::new());
}

// A family can reach the generator's internal arms, so the emitting arm
// itself admits only a literal or a literal include path. rustc compiles the
// generator against a stub: a declared literal builds, and a constant handed
// straight to the emitting arm does not.
#[test]
fn the_statement_generator_admits_only_literal_text() {
    let source = fs::read_to_string(manifest_directory().join("src/statements.rs"))
        .expect("statements.rs is readable");
    let start = source
        .find("macro_rules! statements {")
        .expect("statements.rs defines the generator");
    let end = start + source[start..].find("\n}\n").expect("the generator ends") + 3;
    let generator = &source[start..end];
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is after the epoch")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "runlimit-generator-{}-{started}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).expect("create the probe directory");
    let build = |family: &str| {
        let file = directory.join("probe.rs");
        fs::write(
            &file,
            format!(
                "pub mod statements {{ #[derive(Clone, Copy)] pub struct Statement; }}\n\
                 {generator}\n\
                 pub mod family {{\n\
                 impl Id {{ pub(crate) const fn statement(self) -> crate::statements::Statement {{\n\
                 crate::statements::Statement }} }}\n\
                 {family}\n}}\n"
            ),
        )
        .expect("write the probe");
        let output = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .current_dir(manifest_directory())
            .args([
                "--edition",
                "2024",
                "--crate-type",
                "lib",
                "--emit",
                "metadata",
                "-o",
            ])
            .arg(directory.join("probe.rmeta"))
            .arg(&file)
            .output()
            .expect("run rustc");
        (
            output.status.success(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };
    let declared = build("statements! { SAMPLE = \"SELECT 1\"; }");
    let constant =
        build("const TEXT: &str = \"SELECT 1\"; statements!(@emit { [] SAMPLE literal TEXT });");
    fs::remove_dir_all(&directory).expect("remove the probe directory");
    assert!(declared.0, "a declared literal builds:\n{}", declared.1);
    assert!(!constant.0, "a constant must not build");
    assert!(
        constant
            .1
            .contains("a statement's text is a string literal"),
        "{}",
        constant.1
    );
}
