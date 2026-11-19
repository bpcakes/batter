#!/usr/bin/env python3
"""Refresh native SQLx caches using an already migrated PostgreSQL 18 database."""
import argparse
from contextlib import contextmanager
import ipaddress
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
from urllib.parse import parse_qsl, quote, unquote, urlsplit

from runledger_source import copy_source, run

ROOT = Path(__file__).resolve().parent.parent
MIGRATION_PACKAGES = ("runledger-postgres", "runledger-test-support")
CACHE_PACKAGES = ("runledger-postgres", "runledger-runtime")


def migrations_current(output):
    saw_migration = False
    for line in output.splitlines():
        if "different checksum" in line or re.match(r"^(applied|local) migration (had|has) checksum\s", line):
            return False
        if re.match(r"^\d+/", line):
            saw_migration = True
            if not re.match(r"^\d+/installed(?:\s|$)", line):
                return False
    return saw_migration


def sync_files(source, destination, suffix):
    files = list(source.glob("*" + suffix))
    if not files:
        raise ValueError("refusing to synchronize empty generated assets")
    destination.mkdir(parents=True, exist_ok=True)
    names = {path.name for path in files}
    for path in files:
        shutil.copy2(path, destination / path.name)
    for path in destination.glob("*" + suffix):
        if path.name not in names:
            path.unlink()


# Query parameters both psql and SQLx accept, with the psql variable for each.
CARRIED_PARAMETERS = {"sslmode": "PGSSLMODE", "application_name": "PGAPPNAME", "options": "PGOPTIONS"}
# The preparation build checks runledger-postgres and runledger-runtime, whose
# SQLx dependency enables no TLS backend, so the query macros cannot connect
# over TLS. Only modes that do not require it are accepted.
UNVERIFIED_SSL_MODES = ("disable", "allow", "prefer")
# Every variable SQLx 0.9.1 reads, plus libpq's documented connection
# variables: the names the Cargo probe checks the query macros' compiler for.
# The process environment itself is refused on any PG* name.
POSTGRES_VARIABLES = (
    "PGHOST", "PGHOSTADDR", "PGPORT", "PGDATABASE", "PGUSER", "PGPASSWORD", "PGPASSFILE",
    "PGREQUIREAUTH", "PGCHANNELBINDING", "PGSERVICE", "PGSERVICEFILE", "PGOPTIONS", "PGAPPNAME",
    "PGSSLMODE", "PGREQUIRESSL", "PGSSLNEGOTIATION", "PGSSLCOMPRESSION", "PGSSLCERT", "PGSSLKEY",
    "PGSSLCERTMODE", "PGSSLROOTCERT", "PGSSLCRL", "PGSSLCRLDIR", "PGSSLSNI", "PGREQUIREPEER",
    "PGSSLMINPROTOCOLVERSION", "PGSSLMAXPROTOCOLVERSION", "PGGSSENCMODE", "PGKRBSRVNAME",
    "PGGSSLIB", "PGGSSDELEGATION", "PGCONNECT_TIMEOUT", "PGCLIENTENCODING",
    "PGTARGETSESSIONATTRS", "PGLOADBALANCEHOSTS", "PGMINPROTOCOLVERSION", "PGMAXPROTOCOLVERSION",
    "PGDATESTYLE", "PGTZ", "PGGEQO", "PGSYSCONFDIR", "PGLOCALEDIR",
)


def refuse_ambient_postgres_environment(environment):
    """Refuse any PG* variable: psql and SQLx both fill unset settings from
    them, so the refresh takes every connection setting from DATABASE_URL
    rather than carry or translate inherited ones. Names only, never values."""
    names = sorted(key for key in environment if key.startswith("PG"))
    if names:
        raise ValueError("unset " + ", ".join(names) + ": the refresh takes every connection "
                         "setting from DATABASE_URL")


def is_ip_literal(host):
    """An IPv4 or IPv6 address without a zone index."""
    if "%" in host:
        return False
    try:
        ipaddress.ip_address(host)
    except ValueError:
        return False
    return True


def parse_database_url(database):
    """Parse DATABASE_URL once; psql and SQLx then receive only these parts.

    psql (libpq), SQLx CLI and the query macros read some URL text differently,
    and libpq and SQLx both read ambient PG* settings, which `refresh` refuses.
    Rather than check that every parser agrees, this script parses the URL once,
    gives psql the parts as its own PG* variables, and gives SQLx a URL rebuilt
    from the same parts. Only what both can receive alike is accepted: one host
    or socket directory, an optional port, one database, optional credentials,
    and the sslmode (disable, allow or prefer), application_name and options
    parameters, each once.
    """
    try:
        parts = urlsplit(database)
        port = parts.port
        pairs = parse_qsl(parts.query, keep_blank_values=True, strict_parsing=bool(parts.query),
                          errors="strict")
    except (ValueError, UnicodeDecodeError):
        # The parser's message can quote the URL authority, credentials included.
        raise ValueError("DATABASE_URL is not a valid URL") from None
    if parts.scheme not in ("postgres", "postgresql") or not database.startswith(parts.scheme + "://"):
        raise ValueError("DATABASE_URL must be a postgres:// URI")
    if "#" in database:
        raise ValueError("DATABASE_URL must not contain '#'; encode it as %23")
    host = parts.netloc.rpartition("@")[2]
    # Check the whole authority before slicing an IPv6 literal out of it: the
    # slice would drop a second host, and urlsplit takes its port.
    if "," in host:
        raise ValueError("DATABASE_URL must name exactly one host and one database")
    if host.startswith("["):
        # Only a bracketed literal followed by an optional port: slicing at the
        # first "]" would otherwise drop the rest of a malformed authority.
        close = host.find("]")
        if close == -1 or not re.fullmatch(r"(:[0-9]*)?", host[close + 1:]):
            raise ValueError("DATABASE_URL must name exactly one host and one database")
        host = host[1:close]
        # Brackets hold only an IPv6 literal: an IPvFuture such as [v1.db]
        # would otherwise reach the resolver as a DNS name.
        if ":" not in host or not is_ip_literal(host):
            raise ValueError("DATABASE_URL's bracketed host must be an IPv6 literal without a zone")
    elif port is not None or host.endswith(":"):
        # An empty port (`host:`) means the default; urlsplit reports None for
        # it, so the delimiter must still come off, or a socket directory
        # such as %2Ftmp: would name /tmp: instead of /tmp.
        host = host.rpartition(":")[0]
    try:
        # Strict decoding: a replacement character would name another object.
        host = unquote(host, errors="strict")
        database_name = unquote(parts.path[1:], errors="strict")
        user = unquote(parts.username, errors="strict") if parts.username is not None else None
        password = unquote(parts.password, errors="strict") if parts.password is not None else None
    except UnicodeDecodeError:
        raise ValueError("DATABASE_URL must percent-encode UTF-8 text") from None
    keys = [key for key, _ in pairs]
    # SQLx's URL parser drops "." and ".." path segments, so those names would
    # select no database there while psql checks one.
    if (not host or "," in host or not database_name or "/" in database_name
            or database_name in (".", "..")):
        raise ValueError("DATABASE_URL must name exactly one host and one database")
    if port == 0:
        raise ValueError("DATABASE_URL must not name port 0")
    # SQLx resolves a TCP authority host as written, without percent-decoding,
    # so only names that need no encoding reach both clients identically. An
    # IPv6 literal must be one SQLx parses: a zone index (%) is refused.
    if host.startswith("/"):
        pass
    elif ":" in host:
        if "%" in host or not is_ip_literal(host):
            raise ValueError("DATABASE_URL's IPv6 host must be a literal address without a zone")
    elif not re.fullmatch(r"[A-Za-z0-9._~-]+", host):
        raise ValueError("DATABASE_URL's TCP host must use only letters, digits, '.', '_', '~' and '-'")
    if len(set(keys)) != len(keys):
        raise ValueError("DATABASE_URL may declare each parameter once")
    # A hostaddr equal to an IP literal host names the address the host already
    # does (the Runledger-owned container's pin), so it is redundant for both
    # clients and dropped; any other hostaddr is refused.
    if "hostaddr" in keys:
        if dict(pairs)["hostaddr"] != host or not is_ip_literal(host):
            raise ValueError("DATABASE_URL may declare hostaddr only as its own IP literal host")
        pairs = [(key, value) for key, value in pairs if key != "hostaddr"]
        keys = [key for key, _ in pairs]
    if any(key not in CARRIED_PARAMETERS for key in keys):
        raise ValueError("DATABASE_URL may declare only sslmode, application_name and options, each once")
    if dict(pairs).get("sslmode", "prefer") not in UNVERIFIED_SSL_MODES:
        raise ValueError("DATABASE_URL's sslmode must be disable, allow or prefer: the query macros "
                         "the refresh compiles have no TLS backend, so a TLS-required refresh is "
                         "unsupported")
    return {
        "host": host,
        "port": port or 5432,
        "database": database_name,
        "user": user,
        "password": password,
        "parameters": dict(pairs),
    }


def psql_environment(parts):
    """The environment psql connects with: no inherited PG* setting, the parsed parts."""
    environment = {key: value for key, value in os.environ.items() if not key.startswith("PG")}
    environment.update(PGHOST=parts["host"], PGPORT=str(parts["port"]),
                       PGDATABASE=parts["database"], PGCONNECT_TIMEOUT="10")
    if parts["user"] is not None:
        environment["PGUSER"] = parts["user"]
    if parts["password"] is not None:
        environment["PGPASSWORD"] = parts["password"]
    for key, value in parts["parameters"].items():
        environment[CARRIED_PARAMETERS[key]] = value
    return environment


def sqlx_database_url(parts):
    """The URL SQLx receives, rebuilt from the parsed parts and fully encoded."""
    def encode(text):
        return quote(text, safe="")
    host = parts["host"]
    # A socket directory may contain ':'; only a non-path host is an IPv6 literal.
    host = encode(host) if host.startswith("/") else f"[{host}]" if ":" in host else host
    userinfo = ""
    password = parts["password"]
    if parts["user"] is not None or password:
        userinfo = encode(parts["user"] or "")
        if password:
            userinfo += ":" + encode(password)
        userinfo += "@"
    url = f"postgres://{userinfo}{host}:{parts['port']}/{encode(parts['database'])}"
    pairs = list(parts["parameters"].items())
    if password == "":
        # SQLx reads an empty userinfo password as none and may then read a
        # passfile; an explicit empty password travels as a parameter.
        pairs.append(("password", ""))
    query = "&".join(f"{key}={encode(value)}" for key, value in pairs)
    return url + ("?" + query if query else "")


@contextmanager
def sqlx_environment(database_url):
    """Run SQLx CLI and Cargo children with the rebuilt DATABASE_URL, restoring
    this process's environment afterwards. `refresh` already refused every
    PG* variable, so the children inherit none."""
    previous = dict(os.environ)
    try:
        os.environ["DATABASE_URL"] = database_url
        yield
    finally:
        os.environ.clear()
        os.environ.update(previous)


def cargo_compiler_endpoint(directory, database):
    """What Cargo, run from `directory`, gives the query macros' compiler.

    Returns the POSTGRES_VARIABLES Cargo sets for the compiler and whether its
    `DATABASE_URL` equals `database`, read at run time in a process Cargo
    launches with the same `[env]` it gives rustc. `cargo sqlx prepare` runs `cargo check`,
    which applies `[env]` settings, including included, home and forced ones,
    even when the parent shell differs. Ask Cargo itself by compiling a
    throwaway probe from the same directory; it prints variable names and one
    boolean, never a value or either URL.
    """
    with tempfile.TemporaryDirectory(prefix="batter-cargo-env-probe-") as scratch:
        probe = Path(scratch)
        (probe / "src").mkdir()
        (probe / "Cargo.toml").write_text(
            '[package]\nname = "cargo-env-probe"\nversion = "0.0.0"\nedition = "2021"\n\n[workspace]\n')
        # Cargo gives a process it runs the same [env] as rustc, so the probe
        # reads the values at run time: nothing, credentials included, is
        # compiled into its artifacts. The expected URL arrives on stdin,
        # which Cargo [env] cannot replace.
        names = ", ".join(f'"{name}"' for name in POSTGRES_VARIABLES)
        (probe / "src/main.rs").write_text(
            'use std::io::Read;\n'
            'fn main() {\n'
            '    let mut expected = String::new();\n'
            '    std::io::stdin().read_to_string(&mut expected).expect("expected URL on stdin");\n'
            '    let set: Vec<&str> = [' + names + ']\n'
            '        .into_iter().filter(|name| std::env::var_os(name).is_some()).collect();\n'
            '    print!("{} {}", if set.is_empty() { "-".to_owned() } else { set.join(",") },\n'
            '           std::env::var("DATABASE_URL").ok().as_deref() == Some(expected.as_str()));\n'
            '}\n')
        try:
            result = subprocess.run(
                ["cargo", "run", "--quiet", "--offline", "--manifest-path", str(probe / "Cargo.toml"),
                 "--target-dir", str(probe / "target")],
                cwd=directory, capture_output=True, text=True, timeout=300, input=database)
        except (OSError, subprocess.TimeoutExpired):
            raise ValueError("could not ask Cargo which [env] settings apply") from None
    answer = result.stdout.split()
    names = [] if len(answer) != 2 or answer[0] == "-" else answer[0].split(",")
    if (result.returncode or len(answer) != 2 or answer[1] not in ("true", "false")
            or any(name not in POSTGRES_VARIABLES for name in names)):
        raise ValueError("could not ask Cargo which [env] settings apply")
    return names, answer[1] == "true"


def refuse_sqlx_config(directory):
    """Refuse any sqlx.toml: SQLx CLI 0.9.1 enables it by default, and its
    `[common] database-url-var` would make `migrate info` and `prepare` read
    another variable than the DATABASE_URL psql checked."""
    directory = Path(directory)
    for current, folders, files in os.walk(directory):
        # Build output and history are not source configuration.
        folders[:] = [name for name in folders if name not in ("target", ".git")]
        # Ask the filesystem for the exact path SQLx opens: case-insensitive
        # volumes (macOS by default) fold names beyond ASCII, so matching the
        # listing's spelling would miss a file SQLx finds.
        candidate = Path(current) / "sqlx.toml"
        if candidate.is_file():
            relative = candidate.relative_to(directory)
            raise ValueError("remove sqlx.toml (" + relative.as_posix()
                             + "): SQLx CLI could read another database URL variable than psql checked")


def refuse_cargo_endpoint(directory, database):
    names, same_url = cargo_compiler_endpoint(directory, database)
    if names:
        raise ValueError("remove " + ", ".join(names) + " from Cargo [env]: the query macros would read "
                         "settings psql did not check")
    if not same_url:
        raise ValueError("remove DATABASE_URL from Cargo [env]: the query macros would use another database than psql checked")


def query(root, parts, sql, purpose):
    # The connection comes only from the parsed parts, never from an argument:
    # credentials stay out of the process list and connection diagnostics.
    try:
        result = subprocess.run(
            ["psql", "-X", "-qAt", "-v", "ON_ERROR_STOP=1", "-c", sql],
            env=psql_environment(parts),
            cwd=root, capture_output=True, text=True, timeout=20)
    except (OSError, subprocess.TimeoutExpired):
        raise RuntimeError(f"PostgreSQL {purpose} query could not complete; check psql and DATABASE_URL") from None
    if result.returncode:
        raise RuntimeError(f"PostgreSQL {purpose} query failed; check DATABASE_URL and connectivity")
    return result.stdout.strip()


def refresh(root):
    root = Path(root).resolve()
    refuse_ambient_postgres_environment(os.environ)
    database = os.environ.get("DATABASE_URL")
    if not database:
        raise ValueError("DATABASE_URL must identify an already migrated PostgreSQL 18 database")
    parts = parse_database_url(database)
    with sqlx_environment(sqlx_database_url(parts)):
        refresh_with(root, parts)


def refresh_with(root, parts):
    rebuilt = os.environ["DATABASE_URL"]
    refuse_sqlx_config(root)
    refuse_cargo_endpoint(root, rebuilt)
    cli_version = run(["cargo", "sqlx", "--version"], root).split()
    if not cli_version or cli_version[-1] != "0.9.1":
        raise ValueError("SQLx CLI 0.9.1 is required to match the workspace dependency")
    version = query(root, parts, "SELECT current_setting('server_version_num')", "version")
    if not re.fullmatch(r"18\d{4}", version):
        raise ValueError("SQLx metadata refresh requires PostgreSQL 18")
    print("SQLx refresh server_version_num=" + version)
    info = run(["env", "NO_COLOR=1", "CARGO_TERM_COLOR=never", "cargo", "sqlx", "migrate", "info",
                "--no-dotenv", "--source", "runledger/migrations"], root)
    if not migrations_current(info):
        raise ValueError("canonical Runledger migrations must be installed without checksum drift")
    # SQLx info iterates local migrations and omits extra database versions.
    expected = {str(int(path.name.split('_', 1)[0])) + ':true'
                for path in (root / 'runledger/migrations').glob('*.sql')
                if not path.name.endswith('.down.sql')}
    applied = query(root, parts,
                    "SELECT version::text || ':' || success::text FROM _sqlx_migrations ORDER BY version",
                    "migration inventory").splitlines()
    if not expected or set(applied) != expected:
        raise ValueError("database migration inventory must exactly match canonical Runledger migrations")
    target = Path(os.environ.get("CARGO_TARGET_DIR", root / "target")).resolve() / "runledger-prepare"
    with tempfile.TemporaryDirectory(prefix="batter-native-prepare-") as directory:
        candidate = copy_source(root, Path(directory) / "source")
        native = candidate / "runledger"
        for package in MIGRATION_PACKAGES:
            sync_files(native / "migrations", native / package / "migrations", ".sql")
        refuse_sqlx_config(candidate)
        refuse_cargo_endpoint(candidate, rebuilt)
        run(["env", "CARGO_TARGET_DIR=" + str(target), "cargo", "sqlx", "prepare",
             "--workspace", "--no-dotenv", "--", "-p", "runledger-postgres", "-p", "runledger-runtime",
             "--all-targets", "--all-features", "--locked"], candidate)
        generated = candidate / ".sqlx"
        for destination in (native / ".sqlx", *(native / p / ".sqlx" for p in CACHE_PACKAGES)):
            sync_files(generated, destination, ".json")
        run(["env", "SQLX_OFFLINE=true", "cargo", "check", "-p", "runledger-postgres",
             "-p", "runledger-runtime", "--all-targets", "--all-features", "--locked", "--offline",
             "--target-dir", str(target)], candidate)
        # Failed preparation/verification above cannot change the original checkout.
        for destination in (root / "runledger/.sqlx",
                            *(root / "runledger" / p / ".sqlx" for p in CACHE_PACKAGES)):
            sync_files(generated, destination, ".json")
        for package in MIGRATION_PACKAGES:
            sync_files(native / "migrations", root / "runledger" / package / "migrations", ".sql")
    print("Native SQLx caches and migration copies refreshed; review the working diff")


if __name__ == "__main__":
    argparse.ArgumentParser(description=__doc__).parse_args()
    refresh(ROOT)
