#!/usr/bin/env python3
"""Refresh native SQLx caches using an already migrated PostgreSQL 18 database."""
import argparse
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

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


def refresh(root):
    root = Path(root).resolve()
    database = os.environ.get("DATABASE_URL")
    if not database:
        raise ValueError("DATABASE_URL must identify an already migrated PostgreSQL 18 database")
    cli_version = run(["cargo", "sqlx", "--version"], root).split()
    if not cli_version or cli_version[-1] != "0.9.0":
        raise ValueError("SQLx CLI 0.9.0 is required to match the workspace dependency")
    # psql expands URI connection strings through --dbname, not PGDATABASE.
    # Do not expose its credential-bearing arguments or connection diagnostics.
    try:
        result = subprocess.run(
            ["psql", "--dbname", database, "-X", "-qAt", "-v", "ON_ERROR_STOP=1", "-c",
             "SELECT current_setting('server_version_num')"],
            env={**os.environ, "PGCONNECT_TIMEOUT": "10"},
            cwd=root, capture_output=True, text=True, timeout=20)
    except (OSError, subprocess.TimeoutExpired):
        raise RuntimeError("PostgreSQL version query could not complete; check psql and DATABASE_URL") from None
    if result.returncode:
        raise RuntimeError("PostgreSQL version query failed; check DATABASE_URL and connectivity")
    version = result.stdout.strip()
    if not re.fullmatch(r"18\d{4}", version):
        raise ValueError("SQLx metadata refresh requires PostgreSQL 18")
    print("SQLx refresh server_version_num=" + version)
    info = run(["env", "NO_COLOR=1", "CARGO_TERM_COLOR=never", "cargo", "sqlx", "migrate", "info",
                "--no-dotenv", "--source", "runledger/migrations"], root)
    if not migrations_current(info):
        raise ValueError("canonical Runledger migrations must be installed without checksum drift")
    target = Path(os.environ.get("CARGO_TARGET_DIR", root / "target")).resolve() / "runledger-prepare"
    with tempfile.TemporaryDirectory(prefix="batter-native-prepare-") as directory:
        candidate = copy_source(root, Path(directory) / "source")
        native = candidate / "runledger"
        for package in MIGRATION_PACKAGES:
            sync_files(native / "migrations", native / package / "migrations", ".sql")
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
