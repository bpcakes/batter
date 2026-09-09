"""Run the explicit reference probes with prerequisites and a wall-clock bound."""

import os
import re
import sys
from urllib.parse import parse_qs, urlsplit

from parallel_process import run_parallel

CASES = frozenset({
    "migrations_and_transactional_enqueue",
    "worker_startup_witness_and_shutdown",
    "lease_cleanup_defer_and_drop",
    "initialized_schema_upgrade",
})
COMMAND = ["cargo", "test", "-p", "batter-example-reference-service", "--test", "reference_live", "--locked"]


def local_endpoint(value):
    try:
        endpoint = urlsplit(value)
        query = parse_qs(endpoint.query)
        return (endpoint.scheme in ("postgres", "postgresql")
                and endpoint.hostname in ("localhost", "127.0.0.1", "::1")
                and bool(endpoint.path.strip("/"))
                and query.get("sslmode", ["disable"]) == ["disable"]
                and not any(key in query for key in ("host", "hostaddr", "service")))
    except ValueError:
        return False


def complete_inventory(output):
    return set(re.findall(r"^([a-z_]+): test$", output, re.MULTILINE)) == CASES


def complete_execution(output):
    executed = re.findall(r"^test ([a-z_]+) \.\.\. ok$", output, re.MULTILINE)
    return (len(executed) == len(CASES) and set(executed) == CASES
            and re.search(rf"^test result: ok\. {len(CASES)} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;", output, re.MULTILINE) is not None)


def run(command, timeout):
    # Reuse the repository's existing Unix process owner for timeout, capture,
    # signal escalation and group reaping. A watchdog exit is failed evidence.
    return run_parallel([command], timeout=timeout, output_limit=4 * 1024 * 1024,
                        retain_tail=True)[0]


def main():
    value = os.environ.get("POSTGRES_TEST_ADMIN_URL", "")
    if not local_endpoint(value):
        sys.exit("Set POSTGRES_TEST_ADMIN_URL to an explicitly selected local disposable PostgreSQL 18 server with sslmode=disable.")
    check = run(
        ["psql", "--dbname", value, "-XAt", "-v", "ON_ERROR_STOP=1", "-c",
         "SELECT current_setting('server_version_num')::int / 10000 = 18 "
         "AND (rolsuper OR rolcreatedb) FROM pg_roles WHERE rolname = current_user"],
        timeout=15,
    )
    if not check.ok or check.stdout.strip() != b"t":
        sys.exit("PostgreSQL preflight failed: requires major 18 and CREATE DATABASE authority.")
    inventory = run(COMMAND + ["--", "--ignored", "--list"], timeout=300)
    if not inventory.ok or not complete_inventory(inventory.stdout.decode("utf-8", errors="replace")):
        print(inventory.output, file=sys.stderr)
        sys.exit("Live probe inventory failed: required ignored cases must all compile and exist.")
    print(f"PostgreSQL 18 prerequisite passed; executing {len(CASES)} required live cases.", flush=True)
    result = run(COMMAND + ["--", "--ignored"], timeout=180)
    print(result.output, end="")
    if not result.ok or not complete_execution(result.stdout.decode("utf-8", errors="replace")):
        sys.exit("Live probes did not all execute successfully.")


if __name__ == "__main__":
    main()
