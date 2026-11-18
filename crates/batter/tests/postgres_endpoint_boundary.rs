//! PostgreSQL URLs reach SQLx only through an endpoint boundary that refuses
//! an ambient `PGHOSTADDR`.
//!
//! SQLx 0.9.1, like libpq, connects to an ambient `PGHOSTADDR` instead of a
//! URL's host whenever the URL has no `hostaddr` parameter, and its options
//! offer no way to read or clear that address. Every checked-in Rust source in
//! the workspace members and the standalone consumers therefore turns a URL
//! into native options only through `batter_sqlx::connect_options` (also
//! `batter::sqlx::connect_options`), native Runledger's
//! `runledger_test_support::connect_options` or Runlimit's private live-test
//! helper. The scan rejects the URL-taking SQLx forms everywhere else:
//! `PgConnection::connect`, pool `connect`/`connect_lazy` with a non-literal
//! URL, `PgConnectOptions::from_str`/`from_url` and typed `parse`.
//!
//! Literal placeholder URLs passed to `connect_lazy` never connect and are
//! allowed. Options built with `new_without_pgpass` and setters are not
//! scanned; their connecting call sites call
//! `batter_sqlx::refuse_ambient_host_address` first. The boundaries parse with
//! an untyped `parse` the scan does not match. Named exemptions below are
//! native operator roots that keep libpq semantics and files whose `connect`
//! calls are TLS or TCP, not SQLx. Comments are not
//! scanned. This is a source check, not a proof that no dependency or
//! generated program parses a URL.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

use std::{
    fs,
    path::{Path, PathBuf},
};

/// Files allowed to contain URL-taking SQLx forms, with the reason.
const EXEMPT: [(&str, &str); 7] = [
    (
        "crates/batter/tests/postgres_endpoint_boundary.rs",
        "this scan's own pattern and fixture strings",
    ),
    (
        "runledger/runledger-tui/src/database.rs",
        "native operator root; honours PGHOSTADDR like psql",
    ),
    (
        "runledger/runledger-postgres/examples/support/database.rs",
        "native example root; keeps libpq semantics",
    ),
    (
        "runledger/runledger-runtime/examples/support/database.rs",
        "native example root; keeps libpq semantics",
    ),
    (
        "crates/batter-axum/tests/tls_serving/support.rs",
        "TLS connector, not SQLx",
    ),
    (
        "crates/batter-outbound-http/src/transport.rs",
        "TLS connector, not SQLx",
    ),
    (
        "crates/batter/examples/outbound_exchange.rs",
        "TCP socket, not SQLx",
    ),
];

/// Files whose only `connect` calls are TCP sockets rather than SQLx pools.
const TCP_ONLY: [&str; 1] = ["crates/batter-outbound-http/tests/support/peer.rs"];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

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

fn rust_files(directory: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.file_name().is_some_and(|name| name == "target") {
            continue;
        }
        if path.is_dir() {
            rust_files(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
}

fn sources(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for member in members(root) {
        rust_files(&root.join(member), &mut found);
    }
    rust_files(&root.join("consumers"), &mut found);
    found.sort();
    found
}

/// The argument text after `open`, up to the end of the line.
fn argument_after(line: &str, open: usize) -> &str {
    line[open..].trim_start()
}

fn allowed_lazy_argument(argument: &str) -> bool {
    argument.starts_with('"') || argument.starts_with("UNUSED_LAZY_POOL_URL")
}

/// URL-taking SQLx forms on one source line; comments are skipped.
fn violations(line: &str) -> Vec<&'static str> {
    let code = line.trim_start();
    if code.starts_with("//") {
        return Vec::new();
    }
    let mut found = Vec::new();
    if code.contains("PgConnection::connect(") {
        found.push("PgConnection::connect");
    }
    if code.contains("PgConnectOptions::from_str(")
        || code.contains("PgConnectOptions::from_url(")
        || code.contains("ConnectOptions::from_url(")
    {
        found.push("PgConnectOptions::from_str/from_url");
    }
    if code.contains("parse::<PgConnectOptions>")
        || code.contains("parse::<sqlx::postgres::PgConnectOptions>")
        || (code.contains("PgConnectOptions =") && code.contains(".parse()"))
    {
        found.push("typed PgConnectOptions parse");
    }
    for (method, name) in [
        (".connect(", "pool connect(url)"),
        (".connect_lazy(", "connect_lazy(url)"),
    ] {
        let mut from = 0;
        while let Some(offset) = code[from..].find(method) {
            let open = from + offset + method.len();
            let argument = argument_after(code, open);
            let empty = argument.starts_with(')');
            let placeholder = method == ".connect_lazy(" && allowed_lazy_argument(argument);
            if !empty && !placeholder {
                found.push(name);
            }
            from = open;
        }
    }
    found
}

#[test]
fn detector_recognizes_every_url_taking_form_and_its_allowances() {
    for line in [
        "let mut c = PgConnection::connect(&url).await?;",
        "let c = sqlx::PgConnection::connect(harness.database.url()).await?;",
        "let o = PgConnectOptions::from_str(&url)?;",
        "let o: PgConnectOptions = url.parse()?;",
        "let o: sqlx::postgres::PgConnectOptions = url.parse()?;",
        "let o = url.parse::<PgConnectOptions>()?;",
        "        .connect(&database_url)",
        "        .connect(harness.admin_database_url())",
        "        .connect_lazy(&url)?;",
    ] {
        assert!(!violations(line).is_empty(), "{line}");
    }
    for line in [
        "        .connect_with(batter::sqlx::connect_options(&url)?)",
        "        .connect_lazy(\"postgres://unused@127.0.0.1:1/unused\")",
        "        .connect_lazy(UNUSED_LAZY_POOL_URL)",
        "    let mut probe = options.connect().await?;",
        "    // PgConnection::connect(&url) in a comment",
        "    /// let o: PgConnectOptions = url.parse()?;",
        "let options = batter_sqlx::connect_options(url)?;",
    ] {
        assert!(violations(line).is_empty(), "{line}");
    }
}

#[test]
fn postgres_urls_reach_sqlx_only_through_an_endpoint_boundary() {
    let root = workspace_root();
    let exempt: Vec<&str> = EXEMPT.iter().map(|(path, _)| *path).collect();
    let mut failures = Vec::new();
    let mut used = vec![false; EXEMPT.len()];
    for path in sources(&root) {
        let relative = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let text = fs::read_to_string(&path).unwrap();
        let tcp_only = TCP_ONLY.contains(&relative.as_str());
        for (number, line) in text.lines().enumerate() {
            let found = violations(line);
            if found.is_empty() {
                continue;
            }
            if let Some(index) = exempt.iter().position(|item| *item == relative) {
                used[index] = true;
                continue;
            }
            if tcp_only && found.iter().all(|name| *name == "pool connect(url)") {
                continue;
            }
            failures.push(format!("{relative}:{}: {}", number + 1, found.join(", ")));
        }
    }
    assert!(
        failures.is_empty(),
        "route these through batter_sqlx::connect_options (or the native Runledger/Runlimit \
         boundary):\n{}",
        failures.join("\n")
    );
    let unused: Vec<_> = EXEMPT
        .iter()
        .zip(used)
        .filter(|(_, used)| !used)
        .map(|((path, reason), _)| format!("{path} ({reason})"))
        .collect();
    assert!(unused.is_empty(), "stale exemptions: {unused:?}");
}
