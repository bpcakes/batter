"""Independent failure controls for exported consumers and native SQLx refresh."""
import copy
import os
from pathlib import Path
import subprocess
import shutil
import tempfile
import time
import unittest
from unittest.mock import patch

from check_runledger_workspace import validate_readme
from check_runledger_consumer import ROUND_TRIP, require_round_trip, selected_cargo
from refresh_runledger_sqlx import migrations_current, query, refresh
from runledger_source import copy_source, run, validate_consumer


class SourceTests(unittest.TestCase):
    def test_standalone_toolchain_comes_from_source_or_explicit_override(self):
        with patch.dict(os.environ, {}, clear=True), \
             patch('check_runledger_consumer.run', return_value='1.98.1-host (overridden by source)') as command:
            self.assertEqual(selected_cargo(Path('/source')), ['cargo', '+1.98.1-host'])
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
            keep = ["Cargo.toml", "Cargo.lock", ".cargo/config.toml",
                    "runledger/runledger-postgres/src/claim.sql",
                    "runledger/.sqlx/query-fixture.json", "runledger/migrations/001.sql"]
            omit = [".git", ".env", "runledger/.env", "target/generated.rs", ".agent/.cache/data.json"]
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


class RefreshTests(unittest.TestCase):
    def test_connection_failure_does_not_expose_url_or_prepare(self):
        secret = 'postgresql://user:private-marker@localhost/database'
        with patch.dict(os.environ, DATABASE_URL=secret), \
             patch('refresh_runledger_sqlx.run', return_value='sqlx-cli 0.9.0') as command, \
             patch('refresh_runledger_sqlx.subprocess.run',
                   return_value=subprocess.CompletedProcess([], 2, '', secret)) as psql:
            with self.assertRaisesRegex(RuntimeError, 'version query failed') as error:
                refresh(Path.cwd())
            self.assertNotIn('private-marker', str(error.exception))
            self.assertEqual(command.call_count, 1)
            self.assertEqual(psql.call_args.args[0][1:3], ['--dbname', secret])

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
                        return 'sqlx-cli-sqlx 0.9.0'
                    if 'info' in args:
                        return '001/installed initial'
                    if fail_at in args:
                        raise RuntimeError('deliberate command failure')
                    if 'prepare' in args:
                        cache = Path(cwd) / '.sqlx'
                        cache.mkdir()
                        (cache / 'query-new.json').write_text('{}')
                    return ''

                with patch.dict(os.environ, DATABASE_URL='postgresql://fixture'), \
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
                return 'sqlx-cli 0.9.0' if '--version' in args else info

            with patch.dict(os.environ, DATABASE_URL='postgresql://fixture'), \
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
                with patch.dict(os.environ, DATABASE_URL='postgresql://fixture'), \
                     patch('refresh_runledger_sqlx.run', side_effect=['sqlx-cli 0.9.0', '1/installed initial']) as command, \
                     patch('refresh_runledger_sqlx.query', side_effect=['180006', inventory]):
                    with self.assertRaisesRegex(ValueError, 'inventory must exactly match'):
                        refresh(root)
                    self.assertEqual(command.call_count, 2)

    @unittest.skipUnless(os.environ.get('RUNLEDGER_REFRESH_LIVE') == '1',
                         'set RUNLEDGER_REFRESH_LIVE=1 with Docker, psql and SQLx CLI 0.9.0')
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
                self.assertIn('999999999999', query(source, os.environ['DATABASE_URL'],
                                                   'SELECT version FROM _sqlx_migrations', 'inventory'))
                with patch('refresh_runledger_sqlx.copy_source') as prepare_copy:
                    with self.assertRaisesRegex(ValueError, 'inventory must exactly match'):
                        refresh(source)
                    prepare_copy.assert_not_called()
        finally:
            run(['docker', 'rm', '-f', '-v', container], root)
