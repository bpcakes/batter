"""Independent failure controls for exported consumers and native SQLx refresh."""
from contextlib import contextmanager, nullcontext
import copy
import os
from pathlib import Path
import subprocess
import sys
import shutil
import tempfile
import time
import unittest
from unittest.mock import patch

from check_runledger_workspace import validate_readme
from check_runledger_consumer import ROUND_TRIP, require_round_trip, selected_cargo
from refresh_runledger_sqlx import (environment_bytes, refuse_sqlx_config, cargo_compiler_endpoint, migrations_current, parse_database_url,
                                     psql_environment, query, refresh, sqlx_database_url)
from runledger_compiler_endpoint import guard_preparation, is_client_variable, refresh_build_root
from runledger_source import copy_source, run, validate_consumer


class SourceTests(unittest.TestCase):
    def test_standalone_toolchain_comes_from_source_or_explicit_override(self):
        with patch.dict(os.environ, {}, clear=True), \
             patch('check_runledger_consumer.run', return_value='1.100.0-host (overridden by source)') as command:
            self.assertEqual(selected_cargo(Path('/source')), ['cargo', '+1.100.0-host'])
            command.assert_called_once_with(['rustup', 'show', 'active-toolchain'], Path('/source'))
        with patch.dict(os.environ, RUSTUP_TOOLCHAIN='1.94.0'), \
             patch('check_runledger_consumer.run') as command:
            self.assertEqual(selected_cargo(Path('/source')), ['cargo', '+1.94.0'])
            command.assert_not_called()

    def test_missing_ignored_failed_or_different_worker_test_cannot_pass(self):
        good = f"test {ROUND_TRIP} ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored"
        require_round_trip(good)
        for output in ('test result: ok. 0 passed; 0 failed; 0 ignored',
                       good.replace('1 passed; 0 failed; 0 ignored', '0 passed; 0 failed; 1 ignored'),
                       good.replace(ROUND_TRIP, 'tests::unrelated'),
                       good.replace('... ok', '... FAILED')):
            with self.subTest(output=output), self.assertRaises(RuntimeError):
                require_round_trip(output)

    def test_quick_start_must_match_compiled_sources(self):
        root = Path(__file__).resolve().parent.parent
        validate_readme(root)
        with tempfile.TemporaryDirectory() as directory:
            copy_root = Path(directory)
            native = copy_root / "runledger"
            native.mkdir()
            text = (root / "runledger/README.md").read_text()
            paths = ["producer_worker/" + name + ".rs" for name in ("shared", "producer", "worker")]
            paths.append("support/database.rs")
            for name in paths:
                relative = Path('runledger-runtime/examples') / name
                path = native / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes((root / 'runledger' / relative).read_bytes())
            (native / 'README.md').write_text(text.replace('struct PrintGreeting;', 'struct UnsupportedHandler;'))
            with self.assertRaisesRegex(ValueError, 'differs'):
                validate_readme(copy_root)
            (native / 'README.md').write_text('No compiled snippets')
            with self.assertRaisesRegex(ValueError, 'all four'):
                validate_readme(copy_root)

    def test_copy_retains_build_assets_without_git_secrets_or_outputs(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            root = parent / "original"
            keep = ["Cargo.toml", "Cargo.lock", ".cargo/config.toml", ".agent/jig-contract.json",
                    "docs/.beads/retained.md",
                    "runledger/runledger-postgres/src/claim.sql",
                    "runledger/.sqlx/query-fixture.json", "runledger/migrations/001.sql"]
            omit = [".git", ".env", "runledger/.env", "target/generated.rs", ".agent/.cache/data.json",
                    "crates/nested/target/generated.rs", ".beads/issues.jsonl",
                    ".claude/settings.local.json"]
            for name in keep + omit:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("fixture")
            (root / "leaked.rs").symlink_to(root / ".env")
            result = copy_source(root, parent / "copy")
            for name in keep:
                self.assertEqual((result / name).read_text(), "fixture")
            for name in omit + ["leaked.rs"]:
                self.assertFalse((result / name).exists(), name)
            with self.assertRaisesRegex(ValueError, "outside"):
                copy_source(root, root / "recursive")

    def test_consumer_rejects_sibling_sources_duplicates_and_dependency_drift(self):
        source, consumer = Path('/copied').resolve(), Path('/consumer').resolve()
        names = ["batter", "batter-core", "batter-sqlx", "batter-runledger",
                 "runledger-core", "runledger-postgres", "runledger-runtime"]
        packages = [dict(name=n, source=None, manifest_path=str(source / n / "Cargo.toml")) for n in names]
        good = dict(packages=packages)
        validate_consumer(good, source, consumer, set())
        sibling = copy.deepcopy(good)
        sibling['packages'][0]['manifest_path'] = str(Path('/sibling/Cargo.toml').resolve())
        with self.assertRaisesRegex(ValueError, "outside"):
            validate_consumer(sibling, source, consumer, set())
        duplicate = copy.deepcopy(good)
        duplicate['packages'].append(duplicate['packages'][0])
        with self.assertRaisesRegex(ValueError, "one copied"):
            validate_consumer(duplicate, source, consumer, set())
        drift = copy.deepcopy(good)
        drift['packages'].append(dict(name='dependency', version='2', source='registry+fixture'))
        with self.assertRaisesRegex(ValueError, "locked external"):
            validate_consumer(drift, source, consumer, {('dependency', '1', 'registry+fixture')})


def cargo_workspace_parent(candidates):
    # Cargo searches every physical ancestor of its cwd, even with a private
    # CARGO_HOME. This applies to system temp directories as well as TMPDIR.
    for candidate in candidates:
        parent = Path(candidate).resolve()
        if parent.is_dir() and os.access(parent, os.W_OK | os.X_OK) and not any(
            (ancestor / '.cargo' / name).exists()
            for ancestor in (parent, *parent.parents) for name in ('config', 'config.toml')
        ):
            return parent
    raise RuntimeError('Cargo probe tests need a writable temp directory outside Cargo configuration')


@contextmanager
def cargo_probe_workspace(database_url):
    parent = cargo_workspace_parent((tempfile.gettempdir(), '/tmp', '/var/tmp'))
    # The manifest need only be readable. Wrappers and Cargo output need an
    # executable filesystem, including when the caller puts TMPDIR on noexec.
    build_volume = refresh_build_root(Path(__file__).resolve().parents[1])
    with tempfile.TemporaryDirectory(dir=parent) as directory, tempfile.TemporaryDirectory(dir=build_volume) as build:
        root = Path(directory)
        (root / '.cargo').mkdir()
        shutil.copy(Path(__file__).resolve().parents[1] / 'rust-toolchain.toml', root)
        home = Path(build) / 'cargo-home'
        home.mkdir()
        compiler_settings = {'RUSTC', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER', 'RUSTFLAGS'}
        environment = {key: value for key, value in os.environ.items()
                       if not key.startswith('CARGO_') and key not in compiler_settings}
        target = Path(build) / 'target'
        target.mkdir()
        environment.update(CARGO_HOME=str(home), CARGO_TARGET_DIR=str(target),
                           DATABASE_URL=database_url)
        with patch.dict(os.environ, environment, clear=True):
            yield root, home, target


class RefreshTests(unittest.TestCase):
    def setUp(self):
        environment = patch.dict(os.environ)
        environment.start()
        self.addCleanup(environment.stop)
        # The refresh refuses any PG* variable; tests start from a shell
        # without them, inside the patch so the caller's environment returns.
        for key in [key for key in os.environ if key.startswith('PG')]:
            del os.environ[key]
        self.build = Path('/unused/build')
        layout = patch('refresh_runledger_sqlx.refresh_build_root', return_value=self.build)
        layout.start()
        self.addCleanup(layout.stop)
        # The real Cargo probe compiles a crate; its own test exercises it.
        probe = patch('refresh_runledger_sqlx.cargo_compiler_endpoint', return_value=([], True, "/test/rustc"))
        probe.start()
        self.addCleanup(probe.stop)

    def test_connection_failure_does_not_expose_url_or_prepare(self):
        secret = 'postgresql://user:private-marker@localhost/database'
        with patch.dict(os.environ, DATABASE_URL=secret), \
             patch('refresh_runledger_sqlx.run', return_value='sqlx-cli 0.9.1') as command, \
             patch('refresh_runledger_sqlx.subprocess.run',
                   return_value=subprocess.CompletedProcess([], 2, '', secret)) as psql:
            with self.assertRaisesRegex(RuntimeError, 'version query failed') as error:
                refresh(Path.cwd())
            self.assertNotIn('private-marker', str(error.exception))
            self.assertEqual(command.call_count, 1)
            # psql gets the parsed parts as its own variables, never the URL text,
            # and no inherited PG* setting.
            argv, environment = psql.call_args.args[0], psql.call_args.kwargs['env']
            self.assertFalse(any('private-marker' in item for item in argv))
            self.assertEqual((environment[b'PGHOST'], environment[b'PGPORT'], environment[b'PGDATABASE'],
                              environment[b'PGUSER'], environment[b'PGPASSWORD']),
                             (b'localhost', b'5432', b'database', b'user', b'private-marker'))
            # The caller's environment is restored after the SQLx children ran.
            self.assertEqual(os.environ['DATABASE_URL'], secret)

    def test_utf8_database_url_survives_a_non_utf8_locale(self):
        # A child under LC_ALL=C sees a raw UTF-8 URL as surrogates in
        # os.environ; the refresh must still read it as UTF-8.
        script = (
            'import os, sys\n'
            'sys.path.insert(0, sys.argv[1])\n'
            'from unittest.mock import patch\n'
            'import refresh_runledger_sqlx as r\n'
            'if sys.platform.startswith("linux"):\n'
            '    assert sys.getfilesystemencoding() == "ascii", sys.getfilesystemencoding()\n'
            'seen = {}\n'
            'def stop(root, parts):\n'
            '    seen["url"] = os.environ["DATABASE_URL"]\n'
            '    seen["psql"] = r.environment_bytes(r.psql_environment(parts))[b"PGDATABASE"]\n'
            'with patch.object(r, "refresh_with", stop):\n'
            '    r.refresh(".")\n'
            'print(seen["url"], seen["psql"])\n')
        environment = {key: value for key, value in os.environ.items() if not key.startswith('PG')}
        environment.update(LC_ALL='C')
        environment = {os.fsencode(k): os.fsencode(v) for k, v in environment.items()}
        environment[b'DATABASE_URL'] = 'postgres://u@db.invalid/café'.encode('utf-8')
        child = [sys.executable, '-I', '-X', 'utf8=0', '-c', script, str(Path(__file__).resolve().parent)]
        result = subprocess.run(child, env=environment, capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(),
                         "postgres://u@db.invalid:5432/caf%C3%A9?sslmode=disable b'caf\\xc3\\xa9'")

        environment[b'DATABASE_URL'] = b'postgres://user:private-marker@db.invalid/invalid\xff'
        invalid = subprocess.run(child, env=environment, capture_output=True, timeout=60)
        self.assertNotEqual(invalid.returncode, 0)
        self.assertIn(b'DATABASE_URL must be UTF-8', invalid.stderr)
        self.assertNotIn(b'private-marker', invalid.stderr)
        self.assertNotIn(b'UnicodeDecodeError', invalid.stderr)

    def test_client_variable_rule_matches_the_shared_vector(self):
        # batter-sqlx and Runledger's test support implement the same rule.
        root = Path(__file__).resolve().parent.parent
        sqlx, runledger = (
            (root / path).read_text(encoding="utf-8") for path in (
                "crates/batter-sqlx/src/test_support/endpoint/client_variables.txt",
                "runledger/runledger-test-support/src/client_variables.txt"))
        # Each package keeps its own copy so its tests build from the crate.
        self.assertEqual(sqlx, runledger)
        cases = [line.split(" ", 1) for line in sqlx.splitlines() if not line.startswith("#")]
        self.assertTrue(cases)
        for verdict, name in cases:
            self.assertEqual(is_client_variable(name), verdict == "refuse", name)

    def test_ambient_postgres_variables_are_refused_before_any_command(self):
        secret = 'postgresql://user:private-marker@localhost/database'
        # PGDATA and PG_COLOR are read by no client and pass through.
        with patch.dict(os.environ, DATABASE_URL=secret, PGSSLMODE='verify-full',
                        PGHOSTADDR='192.0.2.1', PGDATA='/unused', PG_COLOR='never'), \
             patch('refresh_runledger_sqlx.run') as command, \
             patch('refresh_runledger_sqlx.subprocess.run') as psql:
            with self.assertRaisesRegex(ValueError, 'unset PGHOSTADDR, PGSSLMODE: ') as error:
                refresh(Path.cwd())
            self.assertNotIn('192.0.2.1', str(error.exception))
            self.assertNotIn('private-marker', str(error.exception))
            command.assert_not_called()
            psql.assert_not_called()

    def test_database_url_is_parsed_once_for_both_clients(self):
        parts = parse_database_url(
            'postgresql://us%40er:p%40ss+w%20rd@[::1]:6543/my%20db?sslmode=prefer&application_name=a+b')
        self.assertEqual(parts, {'host': '::1', 'port': 6543, 'database': 'my db', 'user': 'us@er',
                                 'password': 'p@ss+w rd',
                                 'parameters': {'application_name': 'a+b', 'sslmode': 'disable'}})
        rebuilt = sqlx_database_url(parts)
        self.assertEqual(rebuilt, 'postgres://us%40er:p%40ss%2Bw%20rd@[::1]:6543/my%20db'
                                  '?application_name=a%2Bb&sslmode=disable')
        self.assertEqual(parse_database_url(rebuilt), parts)
        colon = parse_database_url('postgres://u@%2Ftmp%2Fpg%3A18/db')
        self.assertEqual(sqlx_database_url(colon), 'postgres://u@%2Ftmp%2Fpg%3A18:5432/db?sslmode=disable&password=')
        # Empty userinfo and an omitted password let both clients look up the
        # same passfile entry for a DNS or IPv4 host.
        empty = parse_database_url('postgres://u:@db.invalid:5432/db')
        self.assertEqual(empty, parse_database_url('postgres://u@db.invalid:5432/db'))
        for passfile in (empty, parse_database_url('postgres://u@192.0.2.1/db')):
            self.assertNotIn('PGPASSWORD', psql_environment(passfile))
            self.assertNotIn('PGPASSFILE', psql_environment(passfile))
            self.assertNotIn('password=', sqlx_database_url(passfile))
        self.assertEqual(sqlx_database_url(empty), 'postgres://u@db.invalid:5432/db?sslmode=disable')
        # For a socket or IPv6 host they would key the entry differently, so
        # neither reads a passfile: psql gets an empty one, SQLx an empty password.
        socket = parse_database_url('postgres://u@%2Fvar%2Frun%2FPostgres/db')
        self.assertEqual((socket['host'], socket['port']), ('/var/run/Postgres', 5432))
        self.assertEqual(sqlx_database_url(socket),
                         'postgres://u@%2Fvar%2Frun%2FPostgres:5432/db?sslmode=disable&password=')
        for no_passfile in (socket, parse_database_url('postgres://u:@[::1]/db'),
                            parse_database_url('postgres://%2Ftmp/db')):
            self.assertNotIn('PGPASSWORD', psql_environment(no_passfile))
            self.assertEqual(psql_environment(no_passfile)['PGPASSFILE'], os.devnull)
            self.assertTrue(sqlx_database_url(no_passfile).endswith('&password='))
        # psql receives URL fields as the UTF-8 bytes the URL encodes,
        # independent of the locale's filesystem encoding.
        cafe = parse_database_url('postgres://u@db.invalid/caf%C3%A9')
        self.assertEqual(environment_bytes(psql_environment(cafe))[b'PGDATABASE'], b'caf\xc3\xa9')
        # A raw "+" in a query value is literal, as in libpq, and reaches SQLx
        # encoded; %20 is the space.
        plus = parse_database_url('postgres://u@db.invalid/db?options=-c%20search_path%3Dtenant+blue')
        self.assertEqual(plus['parameters']['options'], '-c search_path=tenant+blue')
        self.assertEqual(psql_environment(plus)['PGOPTIONS'], '-c search_path=tenant+blue')
        self.assertTrue(sqlx_database_url(plus).endswith('?options=-c%20search_path%3Dtenant%2Bblue&sslmode=disable'))
        # One encoded segment may name a database containing "/" or "%".
        slashed = parse_database_url('postgres://u@db.invalid/sales%2Farchive')
        self.assertEqual(slashed['database'], 'sales/archive')
        self.assertEqual(psql_environment(slashed)['PGDATABASE'], 'sales/archive')
        self.assertTrue(sqlx_database_url(slashed).endswith('/sales%2Farchive?sslmode=disable'))
        self.assertEqual(parse_database_url('postgres://u@db.invalid/tenant%252')['database'], 'tenant%2')
        # An empty port means the default, for a TCP host and a socket directory.
        self.assertEqual(parse_database_url('postgres://u@localhost:/db')['host'], 'localhost')
        empty_socket_port = parse_database_url('postgres://u@%2Ftmp:/db')
        self.assertEqual((empty_socket_port['host'], empty_socket_port['port']), ('/tmp', 5432))
        # A redundant loopback pin repeating the host is dropped.
        owned = parse_database_url('postgres://runledger:runledger@127.0.0.1:5432/db?hostaddr=127.0.0.1')
        self.assertEqual(sqlx_database_url(owned), 'postgres://runledger:runledger@127.0.0.1:5432/db?sslmode=disable')
        # The same address in another spelling is the same pin.
        expanded = parse_database_url('postgres://u@[0:0:0:0:0:0:0:1]/db?hostaddr=::1')
        self.assertEqual(expanded['host'], '0:0:0:0:0:0:0:1')
        self.assertNotIn('hostaddr', expanded['parameters'])
        self.assertEqual(parse_database_url('postgres://u@db~blue.example/db')['host'], 'db~blue.example')
        with patch.dict(os.environ, PGHOSTADDR='192.0.2.1', PGSSLMODE='disable', HOME='/home/x'):
            environment = psql_environment(parts)
        self.assertNotIn('PGHOSTADDR', environment)
        # Both clients use the macros' only transport, whatever the URL accepted.
        self.assertEqual(environment['PGSSLMODE'], 'disable')
        self.assertEqual(environment['PGGSSENCMODE'], 'disable')
        self.assertEqual(environment['HOME'], '/home/x')

    def test_urls_that_cannot_reach_both_clients_alike_never_run_a_command(self):
        for url, message in (
                ('host=db-a hostaddr=192.0.2.1 password=private-marker', 'postgres:// URI'),
                ('postgresql://u:private-marker@db-a/postgres?hostaddr=192.0.2.1', 'hostaddr only as its own'),
                ('postgresql://u:private-marker@db-a/postgres?host=db-b', 'may declare only'),
                ('postgresql://u:private-marker@db-a/postgres?service=other', 'may declare only'),
                ('postgresql://u:private-marker@db-a/postgres?sslmode=disable&sslmode=prefer', 'parameter once'),
                ('postgresql://u:private-marker@db-a,db-b/postgres', 'exactly one host'),
                # Python 3.9's urlsplit accepts this; newer versions reject it first.
                ('postgresql://u:private-marker@[::1],db2:6543/postgres', 'exactly one host|not a valid URL'),
                ('postgresql://u:private-marker@[::1]other:6543/postgres', 'exactly one host|not a valid URL'),
                ('postgresql://u:private-marker@[v1.db]:6543/postgres', 'IPv6 literal|not a valid URL'),
                ('postgresql://u:private-marker@db-a/tenant\nblue', 'control characters'),
                ('postgresql://u:private-marker@db-a/tenant%2', 'malformed percent escape'),
                ('postgresql://u:private-marker@db-a/tenant%00blue', 'NUL'),
                ('postgresql://u:pass%00word@db-a/postgres', 'NUL'),
                ('postgresql://u:private-marker@db-a/postgres?application_name=a%00b', 'NUL'),
                ('postgresql://u:private-marker@db-a/postgres?sslmode', 'not a valid URL'),
                ('postgresql://u:p@db-a,db-b@db-c/jobs', '%40'),
                ('postgresql://u:private-marker@db-a/sales ', '%20'),
                ('postgresql://u:private-marker@db-a/postgres?options=-c%20search_path=private', 'not a valid URL'),
                ('postgresql://u:private-marker@db-a/tenant%zz', 'malformed percent escape'),
                ('postgresql://u:private-marker@db-a/tenant\tblue', 'control characters'),
                ('postgresql://u:private-marker@d\rb-a/postgres', 'control characters'),
                # The preparation build has no SQLx TLS backend.
                ('postgresql://u:private-marker@db-a/postgres?sslmode=verify-full', 'TLS-required refresh'),
                ('postgresql://u:private-marker@db-a/postgres?sslmode=require', 'TLS-required refresh'),
                ('postgresql://u:private-marker@db-a/postgres?sslrootcert=system', 'may declare only'),
                ('postgresql://u:private-marker@db-a/postgres?sslmode=verify-full&sslrootcert=certs/root.pem',
                 'may declare only'),
                ('postgresql://u:private-marker@db-a:5432', 'exactly one host'),
                ('postgresql://u:private-marker@db-a/one/two', 'exactly one host'),
                ('postgresql://u:private-marker@db-a/postgres?application_name=a#b', "must not contain '#'"),
                ('postgresql://user:private-marker@ho\uff0fst/database', 'not a valid URL'),
                ('postgresql://u:private-marker@db-a/%2E', 'exactly one host'),
                ('postgresql://u:private-marker@db-a/..', 'exactly one host'),
                ('postgresql://u:private-marker@db-a:0/postgres', 'port 0'),
                ('postgresql://u:private-marker@db-a/%FF', 'UTF-8'),
                ('postgresql://u:private-marker@db+blue.example/postgres', 'TCP host'),
                ('postgresql://u:private-marker@db%2Bblue.example/postgres', 'TCP host'),
                ('postgresql://u:private-marker@[::1%251]/postgres', 'zone'),
                ('postgresql://u:private-marker@127.0.0.1/postgres?hostaddr=192.0.2.1', 'hostaddr'),
                ('postgresql://u:private-marker@[::1]/postgres?hostaddr=::2', 'hostaddr'),
                ('postgresql://u:private-marker@127.000.000.010/postgres?hostaddr=127.0.0.10', 'hostaddr'),
                ('postgresql://u:private-marker@[::1]/postgres?hostaddr=::1%25lo0', 'hostaddr'),
                ('postgresql://u:private-marker@db.example/postgres?hostaddr=db.example', 'hostaddr'),
                ('postgresql://u:private-marker@127.0.0.1/postgres?hostaddr=127.0.0.1&hostaddr=127.0.0.1', 'once')):
            with self.subTest(url=url), \
                 patch.dict(os.environ, DATABASE_URL=url), \
                 patch('refresh_runledger_sqlx.run') as command, \
                 patch('refresh_runledger_sqlx.subprocess.run') as psql:
                with self.assertRaisesRegex(ValueError, message) as error:
                    refresh(Path.cwd())
                self.assertNotIn('private-marker', str(error.exception))
                self.assertIsNone(error.exception.__cause__)
                command.assert_not_called()
                psql.assert_not_called()

    def test_sqlx_config_is_refused_before_any_command(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'runledger/runledger-postgres').mkdir(parents=True)
            (root / 'target/x').mkdir(parents=True)
            (root / 'target/x/sqlx.toml').write_text('ignored build output')
            # Long s (U+017F) folds to "s" on case-insensitive volumes only.
            (root / 'runledger/runledger-postgres/sqlx.toml').write_text(
                '[common]\ndatabase-url-var = "OTHER_DATABASE_URL"\n')
            with patch.dict(os.environ, DATABASE_URL='postgresql://u@db.invalid/database'), \
                 patch('refresh_runledger_sqlx.run') as command, \
                 patch('refresh_runledger_sqlx.subprocess.run') as psql:
                with self.assertRaisesRegex(ValueError, 'remove sqlx.toml .runledger/runledger-postgres/sqlx.toml'):
                    refresh(root)
                command.assert_not_called()
                psql.assert_not_called()

    def test_sqlx_config_of_any_file_type_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            os.mkfifo(root / 'sqlx.toml')
            # Refused from the directory listing alone; the FIFO is never opened.
            with self.assertRaisesRegex(ValueError, 'remove sqlx.toml'):
                refuse_sqlx_config(root)

    def test_sqlx_config_lookup_follows_the_filesystem(self):
        # A long-s (U+017F) spelling is the file SQLx opens exactly when the
        # volume folds it to sqlx.toml (macOS by default), and never on Linux.
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / '\u017fqlx.toml').write_text('[common]\ndatabase-url-var = "OTHER"\n')
            if (root / 'sqlx.toml').is_file():
                with self.assertRaisesRegex(ValueError, 'remove sqlx.toml'):
                    refuse_sqlx_config(root)
            else:
                refuse_sqlx_config(root)

    def test_cargo_env_endpoint_is_refused_before_any_command(self):
        url = 'postgresql://u@db.invalid/database'
        for answer, message in (((['PGHOSTADDR'], True), 'remove PGHOSTADDR'),
                                ((['PGOPTIONS', 'PGSSLMODE'], True), 'remove PGOPTIONS, PGSSLMODE'),
                                (([], False), 'remove DATABASE_URL')):
            with self.subTest(answer=answer), \
                 patch.dict(os.environ, DATABASE_URL=url), \
                 patch('refresh_runledger_sqlx.cargo_compiler_endpoint', return_value=(*answer, "/test/rustc")) as probe, \
                 patch('refresh_runledger_sqlx.run') as command, \
                 patch('refresh_runledger_sqlx.subprocess.run') as psql:
                with self.assertRaisesRegex(ValueError, message):
                    refresh(Path.cwd())
                probe.assert_called_once_with(Path.cwd().resolve(), 'postgres://u@db.invalid:5432/database?sslmode=disable', self.build)
                command.assert_not_called()
                psql.assert_not_called()

    @unittest.skipUnless(shutil.which('cargo'), 'requires cargo')
    def test_refresh_resolves_relative_and_split_cargo_build_volumes(self):
        url = 'postgres://u@db.invalid/database'
        with cargo_probe_workspace(url) as (root, _home, target):
            (root / 'Cargo.toml').write_text('[workspace]\nmembers = []\nresolver = "2"\n')
            self.assertEqual(refresh_build_root(root), target / 'runledger-prepare/target')
            del os.environ['CARGO_TARGET_DIR']
            config = root / '.cargo/config.toml'
            config.write_text('[build]\ntarget-dir = "local target"\n')
            self.assertEqual(refresh_build_root(root), root.resolve() / 'local target/runledger-prepare/target')
            config.write_text('[build]\ntarget-dir = "final"\nbuild-dir = "intermediate"\n')
            self.assertEqual(refresh_build_root(root), root.resolve() / 'intermediate/runledger-prepare/target')

    @unittest.skipUnless(shutil.which('cargo'), 'requires cargo')
    def test_probe_selects_a_workspace_outside_configured_temp_parents(self):
        url = 'postgres://u@db.invalid/database'
        with cargo_probe_workspace(url) as (root, _home, target):
            candidates = []
            for name in ('config', 'config.toml'):
                ancestor = root / name
                candidate = ancestor / 'nested' / 'tmp'
                candidate.mkdir(parents=True)
                (ancestor / '.cargo').mkdir()
                (ancestor / '.cargo' / name).write_text('[env]\nPGHOSTADDR = "192.0.2.1"\n')
                candidates.append(candidate)
            clean = root / 'clean'
            clean.mkdir()
            parent = cargo_workspace_parent([*candidates, clean])
            self.assertEqual(parent, clean.resolve())
            self.assertEqual(cargo_compiler_endpoint(parent, url, target)[:2], ([], True))

    @unittest.skipUnless(shutil.which('cargo'), 'requires cargo')
    def test_cargo_probe_sees_included_home_and_forced_env_settings(self):
        # Cargo, not this script, resolves config discovery, includes and force.
        url = 'postgres://u:private-marker@db.invalid:5432/database'
        with patch.dict(os.environ, RUSTC_WRAPPER='/usr/bin/false'), cargo_probe_workspace(url) as (root, home, target):
            self.assertEqual(cargo_compiler_endpoint(root, url, target)[:2], ([], True))
            (root / '.cargo/extra.toml').write_text(
                '[env]\nPGHOSTADDR = "192.0.2.1"\nPGOPTIONS = "-c search_path=shadow"\nPGDATA = "/x"\n')
            (root / '.cargo/config.toml').write_text('include = ["extra.toml"]\n')
            self.assertEqual(cargo_compiler_endpoint(root, url, target)[:2], (['PGHOSTADDR', 'PGOPTIONS'], True))
            (root / '.cargo/config.toml').write_text('[env]\nSQLX_OFFLINE = "true"\n')
            (home / 'config.toml').write_text(
                '[env]\nDATABASE_URL = { value = "postgres://other.invalid:5432/db", force = true }\n')
            self.assertEqual(cargo_compiler_endpoint(root, url, target)[:2], ([], False))
            # Forcing a would-be expected-value variable too changes nothing:
            # the comparison is bound to the URL in the parent process.
            (home / 'config.toml').write_text(
                '[env]\nDATABASE_URL = { value = "postgres://other.invalid:5432/db", force = true }\n'
                'BATTER_PROBE_EXPECTED_DATABASE_URL = { value = "postgres://other.invalid:5432/db", force = true }\n')
            self.assertEqual(cargo_compiler_endpoint(root, url, target)[:2], ([], False))

    @unittest.skipUnless(shutil.which('cargo'), 'requires cargo')
    def test_probe_observes_rustc_even_with_an_unusable_target_runner(self):
        url = 'postgres://u:compiler-private-marker@db.invalid/database'
        with cargo_probe_workspace(url) as (root, home, target):
            # A target program cannot run, but a target compilation can.
            (root / '.cargo/config.toml').write_text(
                "[target.'cfg(unix)']\nrunner = 'false'\n")
            scratch = target / 'probe'
            scratch.mkdir()
            with patch('runledger_compiler_endpoint.tempfile.TemporaryDirectory', return_value=nullcontext(scratch)) as temp:
                self.assertEqual(cargo_compiler_endpoint(root, url, target)[:2], ([], True))
                self.assertEqual(temp.call_args.kwargs['dir'], target)
            for artifact in (scratch / 'target').rglob('*'):
                if artifact.is_file():
                    self.assertFalse(b'compiler-private-marker' in artifact.read_bytes(), artifact.name)

    @unittest.skipUnless(shutil.which('cargo'), 'requires cargo')
    def test_prepare_checks_the_actual_query_crate_compiler_environment(self):
        packages = ('runledger-postgres', 'runledger-runtime')
        url = 'postgres://u:compiler-private-marker@db.invalid/database'
        with cargo_probe_workspace(url) as (root, home, target):
            (root / 'Cargo.toml').write_text(
                '[workspace]\nmembers = ["runledger-postgres", "runledger-runtime"]\nresolver = "2"\n')
            for package in packages:
                crate = root / package
                (crate / 'src').mkdir(parents=True)
                (crate / 'Cargo.toml').write_text(
                    f'[package]\nname = "{package}"\nversion = "0.0.0"\nedition = "2021"\n')
                (crate / 'src/lib.rs').touch()
            # A wrapper may alter only the real query package or return an
            # artifact without invoking rustc. Either Cargo wrapper is opaque.
            wrapper = home / 'package-wrapper'
            wrapper.write_text('#!/bin/sh\nif [ "$CARGO_PKG_NAME" = runledger-postgres ]; then\n'
                               'export PGHOSTADDR=192.0.2.1\nfi\nexec "$@"\n')
            wrapper.chmod(0o700)
            config = root / '.cargo/config.toml'
            for key in ('rustc-wrapper', 'rustc-workspace-wrapper'):
                config.write_text(f'[build]\n{key} = "{wrapper}"\n')
                with self.assertRaisesRegex(ValueError, 'wrapper-free'):
                    cargo_compiler_endpoint(root, url, target)
            config.unlink()
            names, matches, compiler = cargo_compiler_endpoint(root, url, target)
            self.assertEqual((names, matches), ([], True))
            def check():
                with guard_preparation(compiler, url, packages, target) as (launcher, config):
                    self.assertEqual(launcher.parent.parent, target)
                    return subprocess.run(['cargo', 'check', '--config', str(config),
                                           '--workspace', '--offline'], cwd=root,
                                          env=dict(os.environ, RUSTC=str(launcher)), capture_output=True,
                                          check=True, timeout=60)

            check()
            with self.assertRaisesRegex(ValueError, 'every query package'):
                with guard_preparation(compiler, url, packages, target):
                    pass
            # Cargo injects a package build script's rustc-env *after* applying
            # its own configuration. Only the real compilation sees the setting.
            for assignment in ('PGHOSTADDR=192.0.2.1', 'DATABASE_URL=another-private-marker'):
                (root / packages[0] / 'build.rs').write_text(
                    f'fn main() {{ println!("cargo:rustc-env={assignment}"); }}\n')
                with self.assertRaises(subprocess.CalledProcessError) as failure:
                    check()
                self.assertIn(b'refusing a compiler endpoint', failure.exception.stderr)
                self.assertNotIn(b'compiler-private-marker', failure.exception.stderr)

    def test_migration_states_reject_incomplete_or_different_schema(self):
        self.assertTrue(migrations_current('001/installed initial\n002/installed next'))
        for output in ('', 'noise', '001/pending initial', '001/unknown initial',
                       '001/installed (different checksum) initial',
                       '001/installed initial\n002/pending next',
                       '001/installed (different checksum) initial\napplied migration had checksum abc\nlocal migration has checksum def'):
            with self.subTest(output=output):
                self.assertFalse(migrations_current(output))

    def fixture(self, root):
        for name in ('migrations/001_new.up.sql', 'migrations/001_new.down.sql', '.sqlx/query-old.json',
                     'runledger-postgres/migrations/old.sql', 'runledger-test-support/migrations/old.sql',
                     'runledger-postgres/.sqlx/query-old.json', 'runledger-runtime/.sqlx/query-old.json'):
            path = root / "runledger" / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(name)

    def test_preparation_and_offline_verification_failures_preserve_original_assets(self):
        for fail_at in ('prepare', 'check'):
            with self.subTest(fail_at=fail_at), tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / 'original'
                self.fixture(root)
                before = {p.relative_to(root): p.read_bytes() for p in root.rglob('*') if p.is_file()}

                def command(args, cwd, **_kwargs):
                    if '--version' in args:
                        return 'sqlx-cli-sqlx 0.9.1'
                    if 'info' in args:
                        return '001/installed initial'
                    if fail_at in args:
                        raise RuntimeError('deliberate command failure')
                    if 'prepare' in args:
                        cache = Path(cwd) / '.sqlx'
                        cache.mkdir()
                        (cache / 'query-new.json').write_text('{}')
                    return ''

                with patch.dict(os.environ, DATABASE_URL='postgresql://fixture/fixture'), \
                     patch('refresh_runledger_sqlx.guard_preparation', return_value=nullcontext(('rustc', 'config'))), \
                     patch('refresh_runledger_sqlx.run', side_effect=command), \
                     patch('refresh_runledger_sqlx.query', side_effect=['180006', '1:true']):
                    with self.assertRaisesRegex(RuntimeError, 'deliberate'):
                        refresh(root)
                after = {p.relative_to(root): p.read_bytes() for p in root.rglob('*') if p.is_file()}
                self.assertEqual(before, after)

    def test_unsupported_server_and_pending_migrations_never_prepare(self):
        for version, info in [('170006', '001/installed initial'), ('180006', '001/pending initial')]:
            calls = []

            def command(args, _cwd):
                calls.append(args)
                return 'sqlx-cli 0.9.1' if '--version' in args else info

            with patch.dict(os.environ, DATABASE_URL='postgresql://fixture/fixture'), \
                 patch('refresh_runledger_sqlx.run', side_effect=command), \
                 patch('refresh_runledger_sqlx.subprocess.run', return_value=subprocess.CompletedProcess([], 0, version)):
                with self.assertRaises(ValueError):
                    refresh(Path.cwd())
            self.assertFalse(any('prepare' in args for args in calls))

    def test_extra_missing_or_failed_migration_inventory_never_prepares(self):
        for inventory in ('1:true\n2:true', '', '1:false'):
            with self.subTest(inventory=inventory), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self.fixture(root)
                with patch.dict(os.environ, DATABASE_URL='postgresql://fixture/fixture'), \
                     patch('refresh_runledger_sqlx.run', side_effect=['sqlx-cli 0.9.1', '1/installed initial']) as command, \
                     patch('refresh_runledger_sqlx.query', side_effect=['180006', inventory]):
                    with self.assertRaisesRegex(ValueError, 'inventory must exactly match'):
                        refresh(root)
                    self.assertEqual(command.call_count, 2)

    @unittest.skipUnless(os.environ.get('RUNLEDGER_REFRESH_LIVE') == '1',
                         'set RUNLEDGER_REFRESH_LIVE=1 with Docker, psql and SQLx CLI 0.9.1')
    def test_database_ahead_of_checkout_is_rejected_before_preparation(self):
        root = Path(__file__).resolve().parent.parent
        container = run(['docker', 'run', '-d', '--rm', '-e', 'POSTGRES_PASSWORD=fixture',
                         '-p', '127.0.0.1::5432', 'postgres:18'], root).strip()
        try:
            deadline = time.monotonic() + 60
            while subprocess.run(['docker', 'exec', container, 'pg_isready', '-h', '127.0.0.1', '-U', 'postgres'],
                                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode:
                if time.monotonic() >= deadline:
                    self.fail('owned PostgreSQL fixture did not become ready')
                time.sleep(0.25)
            port = run(['docker', 'port', container, '5432/tcp'], root).strip().rsplit(':', 1)[1]
            with tempfile.TemporaryDirectory() as directory, \
                 patch.dict(os.environ, DATABASE_URL=f'postgresql://postgres:fixture@127.0.0.1:{port}/postgres'):
                # The setup commands below are SQLx CLI too; no PG* setting may
                # send them elsewhere than the owned container.
                for key in [key for key in os.environ if key.startswith('PG')]:
                    del os.environ[key]
                source = Path(directory)
                migrations = source / 'runledger/migrations'
                shutil.copytree(root / 'runledger/migrations', migrations)
                extra = migrations / '999999999999_review_fixture.up.sql'
                extra.write_text('SELECT 1;')
                run(['cargo', 'sqlx', 'migrate', 'run', '--no-dotenv', '--source', str(migrations)], source)
                extra.unlink()
                info = run(['env', 'NO_COLOR=1', 'CARGO_TERM_COLOR=never', 'cargo', 'sqlx',
                            'migrate', 'info', '--no-dotenv', '--source', str(migrations)], source)
                self.assertTrue(migrations_current(info))  # SQLx omits the extra applied version.
                self.assertIn('999999999999', query(source, parse_database_url(os.environ['DATABASE_URL']),
                                                   'SELECT version FROM _sqlx_migrations', 'inventory'))
                # Exercise both native clients' default passfile lookup for empty
                # userinfo. Keep Cargo's installed tools available in this private HOME.
                passfile = source / '.pgpass'
                passfile.touch(mode=0o600)
                passfile.write_text(f'127.0.0.1:{port}:postgres:postgres:fixture\n')
                with patch.dict(os.environ, HOME=str(source),
                                CARGO_HOME=os.environ.get('CARGO_HOME', str(Path.home() / '.cargo')),
                                RUSTUP_HOME=os.environ.get('RUSTUP_HOME', str(Path.home() / '.rustup')),
                                DATABASE_URL=f'postgresql://postgres:@127.0.0.1:{port}/postgres'), \
                     patch('refresh_runledger_sqlx.copy_source') as prepare_copy:
                    with self.assertRaisesRegex(ValueError, 'inventory must exactly match'):
                        refresh(source)
                    prepare_copy.assert_not_called()
        finally:
            run(['docker', 'rm', '-f', '-v', container], root)
