//! PostgreSQL URLs reach SQLx only through an endpoint boundary that refuses
//! an ambient or declared `hostaddr`.
//!
//! SQLx 0.9.1 connects to an ambient `PGHOSTADDR` instead of a URL's host,
//! checks a Unix socket before a declared `hostaddr`, and offers no way to read
//! or clear the address. Every checked-in Rust source in the workspace members
//! and the standalone consumers therefore turns a URL into native options only
//! through `batter_sqlx::connect_options` (also `batter::sqlx::connect_options`
//! and `runledger_postgres::connect_options`), native Runledger's
//! `runledger_test_support::connect_options` or Runlimit's private live-test
//! helper.
//!
//! Clippy's `disallowed-methods`, configured in the root, `runledger/` and
//! `runlimit/` `clippy.toml` files, is the primary check: it resolves aliases
//! and inferred types for the URL-taking `Pool`/`PoolOptions` `connect` and
//! `connect_lazy`, `Connection::connect`, `ConnectOptions::from_url` and
//! `PgListener::connect` in every workspace crate. This text scan adds what
//! Clippy cannot express or does not lint: `PgConnectOptions::from_str`, typed
//! `parse` and inferred-type `parse` on a line naming a URL, for the workspace
//! and the standalone consumers, plus the call forms for those consumers.
//! Options built with `new_without_pgpass` and setters are not scanned; their
//! connecting call sites call `batter_sqlx::refuse_ambient_host_address` first,
//! and inert placeholders use an unreachable port. Named exemptions below are
//! the boundaries themselves and files whose `connect` calls are TLS or TCP,
//! not SQLx. Comments are not scanned. This is a source check, not a proof that
//! no dependency or generated program parses a URL.
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
        "crates/batter-sqlx/src/endpoint.rs",
        "the Batter boundary itself",
    ),
    (
        "runledger/runledger-test-support/src/endpoint.rs",
        "the native Runledger fixture boundary itself",
    ),
    (
        "runlimit/runlimit-postgres/tests/support/endpoint.rs",
        "the native Runlimit live-test boundary itself",
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

/// An inferred-type `.parse()` whose receiver names a URL: the target type is
/// invisible to a text scan, so every such parse is treated as SQLx's unless
/// the line names another target type.
fn untyped_url_parse(code: &str) -> bool {
    const OTHER_TARGETS: [&str; 3] = [": tokio_postgres::Config", ": url::Url", ": Url "];
    code.contains(".parse()")
        && code.to_ascii_lowercase().contains("url")
        && !OTHER_TARGETS.iter().any(|target| code.contains(target))
}

/// Whether `needle` occurs where its first character starts an identifier or
/// path segment, so `PgProfiledPool::connect(` does not match `Pool::connect(`.
fn contains_identifier(code: &str, needle: &str) -> bool {
    code.match_indices(needle).any(|(index, _)| {
        needle.starts_with('>')
            || !code[..index]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
            || code[..index].ends_with("Pg")
                && !code[..index - 2]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}

/// `parse::<…PgConnectOptions>`, whatever path prefix names the type.
fn turbofish_parse(code: &str) -> bool {
    code.match_indices("parse::<").any(|(index, needle)| {
        let target = &code[index + needle.len()..];
        target
            .split('>')
            .next()
            .is_some_and(|path| path.trim().ends_with("PgConnectOptions"))
    })
}

/// A `PgConnectOptions` declaration whose `.parse()` follows on a later line
/// of the same statement, which the line rules cannot see.
fn multiline_typed_parse(lines: &[&str], index: usize) -> bool {
    let first = lines[index].trim_start();
    if first.starts_with("//") || !first.contains("PgConnectOptions =") || first.contains(';') {
        return false;
    }
    lines[index + 1..]
        .iter()
        .map(|line| line.trim_start())
        .filter(|line| !line.starts_with("//"))
        .take_while(|line| !line.is_empty())
        .scan(false, |ended, line| {
            if *ended {
                return None;
            }
            *ended = line.contains(';');
            Some(line)
        })
        .any(|line| line.contains(".parse()"))
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
    if turbofish_parse(code) || (code.contains("PgConnectOptions =") && code.contains(".parse()")) {
        found.push("typed PgConnectOptions parse");
    } else if untyped_url_parse(code) {
        found.push("untyped URL parse");
    }
    for (constructor, name) in [
        ("Pool::connect(", "Pool::connect(url)"),
        ("Pool::connect_lazy(", "Pool::connect_lazy(url)"),
        ("DbPool::connect(", "Pool::connect(url)"),
        ("DbPool::connect_lazy(", "Pool::connect_lazy(url)"),
        ("PgListener::connect(", "PgListener::connect(url)"),
        (">::connect(", "Pool::connect(url)"),
        (">::connect_lazy(", "Pool::connect_lazy(url)"),
    ] {
        if contains_identifier(code, constructor) {
            found.push(name);
        }
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
            if !empty {
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
        "    let options = std::env::var(\"BATTER_SQLX_ADMIN_URL\")?.parse()?;",
        "    url.parse()",
        "    let pool = PgPool::connect(&url).await?;",
        "    let pool = sqlx::PgPool::connect_lazy(&url)?;",
        "    let pool = Pool::<Postgres>::connect(&url).await?;",
        "    let pool = DbPool::connect(&url).await?;",
        "    let listener = PgListener::connect(&url).await?;",
        "    let options = dsn.parse::<postgres::PgConnectOptions>()?;",
        "    let options = dsn.parse::< sqlx::postgres::PgConnectOptions >()?;",
        "        .connect_lazy(\"postgres://db.example/jobs\")",
        "        .connect_lazy(UNUSED_LAZY_POOL_URL)",
    ] {
        assert!(!violations(line).is_empty(), "{line}");
    }
    for line in [
        "        .connect_with(batter::sqlx::connect_options(&url)?)",
        "        .connect_lazy_with(PgConnectOptions::new_without_pgpass().host(\"127.0.0.1\").port(1))",
        "    let mut probe = options.connect().await?;",
        "    let listener = PgListener::connect_with(&pool).await?;",
        "    // PgConnection::connect(&url) in a comment",
        "    /// let o: PgConnectOptions = url.parse()?;",
        "let options = batter_sqlx::connect_options(url)?;",
        "    PgProfiledPool::connect(options, profile, max_connections).await",
        "    let database = PgProfiledPool::connect_lazy(connect_options, profile, pool_options)?;",
        "        let admin: tokio_postgres::Config = normalized.as_str().parse().unwrap();",
        "    let port: u16 = value.parse()?;",
    ] {
        assert!(violations(line).is_empty(), "{line}");
    }
}

#[test]
fn multiline_typed_parses_are_recognized_within_one_statement() {
    let flagged = ["    let opts: PgConnectOptions =", "        dsn.parse()?;"];
    assert!(multiline_typed_parse(&flagged, 0));
    let chained = [
        "    let opts: sqlx::postgres::PgConnectOptions =",
        "        dsn",
        "            .parse()",
        "            .unwrap();",
    ];
    assert!(multiline_typed_parse(&chained, 0));
    let built = [
        "    let opts: PgConnectOptions =",
        "        batter_sqlx::connect_options(&dsn)?;",
        "    let port: u16 = value.parse()?;",
    ];
    assert!(
        !multiline_typed_parse(&built, 0),
        "a later statement's parse is not attributed"
    );
    let single = [
        "    let opts: PgConnectOptions = connect_options(&dsn)?;",
        "    let n: u16 = v.parse()?;",
    ];
    assert!(
        !multiline_typed_parse(&single, 0),
        "one-line declarations are the line rule's"
    );
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
        let lines: Vec<&str> = text.lines().collect();
        for (number, line) in lines.iter().enumerate() {
            let mut found = violations(line);
            if multiline_typed_parse(&lines, number) {
                found.push("typed PgConnectOptions parse");
            }
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
