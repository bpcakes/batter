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
from urllib.parse import quote, unquote, urlsplit

from runledger_compiler_endpoint import (cargo_build_config, cargo_compiler_endpoint, guard_preparation,
                                        is_client_variable, refresh_build_root)
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
# over TLS. Only modes that do not require it are accepted, and both clients
# then connect without TLS, so psql's checks use the macros' transport (and
# the same pg_hba.conf rule) rather than negotiating TLS the macros cannot.
UNVERIFIED_SSL_MODES = ("disable", "allow", "prefer")


def refuse_ambient_postgres_environment(environment):
    """Refuse any client PG* variable: psql and SQLx both fill unset settings
    from them, so the refresh takes every connection setting from DATABASE_URL
    rather than carry or translate inherited ones. Names only, never values."""
    names = sorted(key for key in environment if is_client_variable(key))
    if names:
        raise ValueError("unset " + ", ".join(names) + ": the refresh takes every connection "
                         "setting from DATABASE_URL")


def is_ip_literal(host):
    """An IPv4 or IPv6 address without a zone index or an ambiguous octet.

    Python 3.9.0-3.9.4 read a leading-zero IPv4 octet as decimal, while the C
    resolver can read it as octal (127.000.000.010 is 127.0.0.8), so such a
    spelling is never treated as an address.
    """
    if "%" in host:
        return False
    if ":" not in host and any(len(octet) > 1 and octet.startswith("0") for octet in host.split(".")):
        return False
    try:
        ipaddress.ip_address(host)
    except ValueError:
        return False
    return True


def connection_query_pairs(query):
    """Decode a connection URI query as PostgreSQL does: percent escapes only.

    parse_qsl applies HTML form decoding, turning a raw "+" into a space, but
    libpq keeps it literal, so ?options=-c%20search_path%3Dtenant+blue names
    tenant+blue. Every field must be key=value with a non-empty key.
    """
    pairs = []
    for field in query.split("&") if query else ():
        # libpq refuses a second raw "=" in a field; %3D stays valid in a value.
        key, separator, value = field.partition("=")
        if not separator or not key or "=" in value:
            raise ValueError("malformed query field")
        pairs.append((unquote(key, errors="strict"), unquote(value, errors="strict")))
    return pairs


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
    # urlsplit silently deletes raw tabs and line breaks anywhere in the URL,
    # so /tenant<newline>blue would name tenantblue; refuse every raw control
    # character instead. Percent-encoded ones are kept as supplied.
    if any(ord(character) < 0x20 or ord(character) == 0x7F for character in database):
        raise ValueError("DATABASE_URL must percent-encode control characters such as tabs and line breaks")
    # unquote leaves a malformed escape such as %2 in place, which the rebuild
    # would then encode as %252; psql rejects the original. %25 stays valid.
    if re.search(r"%(?![0-9A-Fa-f]{2})", database):
        raise ValueError("DATABASE_URL contains a malformed percent escape")
    try:
        parts = urlsplit(database)
        port = parts.port
        pairs = connection_query_pairs(parts.query)
    except (ValueError, UnicodeDecodeError):
        # The parser's message can quote the URL authority, credentials included.
        raise ValueError("DATABASE_URL is not a valid URL") from None
    if parts.scheme not in ("postgres", "postgresql") or not database.startswith(parts.scheme + "://"):
        raise ValueError("DATABASE_URL must be a postgres:// URI")
    if "#" in database:
        raise ValueError("DATABASE_URL must not contain '#'; encode it as %23")
    # libpq trims raw edge spaces (/sales<space> selects sales) where this
    # parser would keep them; %20 stays valid for an intentional space.
    if " " in database:
        raise ValueError("DATABASE_URL must percent-encode spaces as %20")
    # libpq ends the userinfo at the first raw "@", rpartition at the last, so
    # a second one would split the authority differently; %40 is a literal @.
    if parts.netloc.count("@") > 1:
        raise ValueError("DATABASE_URL must percent-encode @ in credentials as %40")
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
        # Empty userinfo and omitted passwords both let libpq consult its
        # passfile; see passfile_lookup for when both clients may.
        password = unquote(parts.password, errors="strict") if parts.password else None
    except UnicodeDecodeError:
        raise ValueError("DATABASE_URL must percent-encode UTF-8 text") from None
    # A decoded NUL (%00) cannot travel in a Unix environment variable or a C
    # string, so psql could never receive it; refuse before any command runs.
    decoded = [host, database_name, user or "", password or ""] + [text for pair in pairs for text in pair]
    if any("\x00" in text for text in decoded):
        raise ValueError("DATABASE_URL must not encode a NUL character (%00)")
    keys = [key for key, _ in pairs]
    # SQLx's URL parser drops "." and ".." path segments, so those names would
    # select no database there while psql checks one.
    # A raw "/" is a second path segment; an encoded %2F is part of the one
    # database name, which both clients decode (SQLx percent-decodes the path).
    if (not host or "," in host or not database_name or "/" in parts.path[1:]
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
        # Compare addresses, not spellings: [0:0:0:0:0:0:0:1] and ::1 are one
        # address, as Runledger's external fixture also reads them.
        pin = dict(pairs)["hostaddr"]
        if (not is_ip_literal(host) or not is_ip_literal(pin)
                or ipaddress.ip_address(pin) != ipaddress.ip_address(host)):
            raise ValueError("DATABASE_URL may declare hostaddr only as its own IP literal host")
        pairs = [(key, value) for key, value in pairs if key != "hostaddr"]
        keys = [key for key, _ in pairs]
    if any(key not in CARRIED_PARAMETERS for key in keys):
        raise ValueError("DATABASE_URL may declare only sslmode, application_name and options, each once")
    if dict(pairs).get("sslmode", "prefer") not in UNVERIFIED_SSL_MODES:
        raise ValueError("DATABASE_URL's sslmode must be disable, allow or prefer: the query macros "
                         "the refresh compiles have no TLS backend, so a TLS-required refresh is "
                         "unsupported")
    pairs = [(key, value) for key, value in pairs if key != "sslmode"] + [("sslmode", "disable")]
    return {
        "host": host,
        "port": port or 5432,
        "database": database_name,
        "user": user,
        "password": password,
        "parameters": dict(pairs),
    }


def passfile_lookup(parts):
    """Whether both clients may look up a passwordless login in ~/.pgpass.

    They choose the entry by host, port, database and user, but key the host
    differently for a Unix socket (libpq searches localhost for its default
    socket directory, SQLx a directory it probes for itself) and an IPv6
    literal (SQLx keeps the URL brackets). Only for a DNS or IPv4 host do both
    look up the same entry; otherwise neither consults a passfile. A
    passfile the clients still read differently (SQLx checks its permissions
    only on Linux, and libpq ends a password at an unescaped ':') makes one
    client fail authentication, which stops the refresh before it writes.
    """
    host = parts["host"]
    return parts["password"] is None and not host.startswith("/") and ":" not in host


def environment_bytes(environment):
    """The child environment as bytes: PG* values (the parsed URL fields) as
    UTF-8, as the URL meant them, and inherited values round-tripped through
    the filesystem encoding they were read with. Passing str would encode the
    URL fields with the locale's encoding, which can fail or change them."""
    return {os.fsencode(key): value.encode("utf-8") if key.startswith("PG") else os.fsencode(value)
            for key, value in environment.items()}


def psql_environment(parts):
    """The environment psql connects with: no inherited PG* setting, the parsed parts."""
    environment = {key: value for key, value in os.environ.items() if not key.startswith("PG")}
    # libpq defaults gssencmode to prefer, which sslmode=disable does not
    # cover; the query macros never use GSS encryption, so psql must not either.
    environment["PGGSSENCMODE"] = "disable"
    environment.update(PGHOST=parts["host"], PGPORT=str(parts["port"]),
                       PGDATABASE=parts["database"], PGCONNECT_TIMEOUT="10")
    if parts["user"] is not None:
        environment["PGUSER"] = parts["user"]
    if parts["password"] is not None:
        environment["PGPASSWORD"] = parts["password"]
    elif not passfile_lookup(parts):
        environment["PGPASSFILE"] = os.devnull
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
    if parts["user"] is not None or password is not None:
        userinfo = encode(parts["user"] or "")
        if password is not None:
            userinfo += ":" + encode(password)
        userinfo += "@"
    url = f"postgres://{userinfo}{host}:{parts['port']}/{encode(parts['database'])}"
    pairs = list(parts["parameters"].items())
    if password is None and not passfile_lookup(parts):
        # An explicit empty password keeps SQLx from reading a passfile, as
        # PGPASSFILE does for psql; a server that asks for one refuses both.
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
        # SQLx CLI tests exists() and then reads the path, whatever its type:
        # a FIFO or a directory counts too.
        if candidate.exists():
            relative = candidate.relative_to(directory)
            raise ValueError("remove sqlx.toml (" + relative.as_posix()
                             + "): SQLx CLI could read another database URL variable than psql checked")


def refuse_cargo_endpoint(directory, database, build):
    names, same_url, compiler = cargo_compiler_endpoint(directory, database, build)
    if names:
        raise ValueError("remove " + ", ".join(names) + " from Cargo [env]: the query macros would read "
                         "settings psql did not check")
    if not same_url:
        raise ValueError("remove DATABASE_URL from Cargo [env]: the query macros would use another database than psql checked")
    return compiler


def query(root, parts, sql, purpose):
    # The connection comes only from the parsed parts, never from an argument:
    # credentials stay out of the process list and connection diagnostics.
    try:
        result = subprocess.run(
            # -w: fail rather than prompt when the server asks for a password.
            ["psql", "-X", "-w", "-qAt", "-v", "ON_ERROR_STOP=1", "-c", sql],
            env=environment_bytes(psql_environment(parts)),
            # Both queries return ASCII identifiers. Discard libpq diagnostics,
            # which may contain endpoint text in the client locale.
            cwd=root, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            encoding="ascii", errors="replace", timeout=20)
    except (OSError, subprocess.TimeoutExpired):
        raise RuntimeError(f"PostgreSQL {purpose} query could not complete; check psql and DATABASE_URL") from None
    if result.returncode:
        raise RuntimeError(f"PostgreSQL {purpose} query failed; check DATABASE_URL and connectivity")
    return result.stdout.strip()


def refresh(root):
    root = Path(root).resolve()
    refuse_ambient_postgres_environment(os.environ)
    raw_url = os.environb.get(b"DATABASE_URL")
    if not raw_url:
        raise ValueError("DATABASE_URL must identify an already migrated PostgreSQL 18 database")
    try:
        database = raw_url.decode("utf-8")
    except UnicodeDecodeError:
        raise ValueError("DATABASE_URL must be UTF-8") from None
    parts = parse_database_url(database)
    with sqlx_environment(sqlx_database_url(parts)):
        refresh_with(root, parts)


def refresh_with(root, parts):
    rebuilt = os.environ["DATABASE_URL"]
    refuse_sqlx_config(root)
    target = refresh_build_root(root)
    # Fail before any database command. Cargo applies [env] to builds, not to
    # external subcommands such as `cargo sqlx migrate info`, so the binding
    # check is the source copy's probe below, under the configuration
    # discovery preparation really uses; this one only refuses early.
    refuse_cargo_endpoint(root, rebuilt, target)
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
    with tempfile.TemporaryDirectory(prefix="batter-native-prepare-") as directory:
        candidate = copy_source(root, Path(directory) / "source")
        native = candidate / "runledger"
        for package in MIGRATION_PACKAGES:
            sync_files(native / "migrations", native / package / "migrations", ".sql")
        refuse_sqlx_config(candidate)
        compiler = refuse_cargo_endpoint(candidate, rebuilt, target)
        with guard_preparation(compiler, rebuilt, CACHE_PACKAGES, target) as (launcher, config):
            run(["env", "CARGO_TARGET_DIR=" + str(target), "CARGO_BUILD_BUILD_DIR=" + str(target),
                 "RUSTC=" + str(launcher),
                 "cargo", "--config", str(config), "sqlx", "prepare", "--workspace", "--no-dotenv",
                 "--", "--config", str(config), "-p", "runledger-postgres", "-p", "runledger-runtime",
                 "--all-targets", "--all-features", "--locked"], candidate)
        generated = candidate / ".sqlx"
        for destination in (native / ".sqlx", *(native / p / ".sqlx" for p in CACHE_PACKAGES)):
            sync_files(generated, destination, ".json")
        run(["env", "SQLX_OFFLINE=true", "cargo", "check", "-p", "runledger-postgres",
             "-p", "runledger-runtime", "--all-targets", "--all-features", "--locked", "--offline",
             "--config", cargo_build_config(target), "--target-dir", str(target)], candidate)
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
