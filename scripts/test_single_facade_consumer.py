"""Failure controls for the single-dependency facade consumer check."""
import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import check_single_facade_consumer as runner
from check_single_facade_consumer import (CONSUMER_SOURCE, HARNESS_SOURCE, IDENTITIES, MARKERS,
                                          consumer_manifest, executable, harness_manifest,
                                          require_markers, validate_single_dependency,
                                          workspace_names)
from runledger_source import copy_source, validate_consumer

NAMES = {'batter', 'batter-core', 'batter-runledger', 'batter-runlimit', 'runledger-core',
         'runledger-postgres', 'runledger-runtime', 'runledger-test-support', 'runlimit-core',
         'runlimit-http', 'runlimit-postgres'}


def package(name, dependencies):
    return dict(packages=[dict(name=name, dependencies=[dict(name=dependency,
                                                             uses_default_features=False)
                                                        for dependency in dependencies])])


class ManifestTests(unittest.TestCase):
    def test_declared_manifests_name_one_workspace_package_and_no_patch(self):
        source = Path('/copied').resolve()
        for text, name in ((consumer_manifest(source), 'single-facade-consumer'),
                           (harness_manifest(source), 'single-facade-harness')):
            with self.subTest(package=name):
                self.assertNotIn('[patch', text)
                self.assertIn('default-features = false', text)
                # Exactly one workspace path dependency, the facade.
                paths = [line for line in text.splitlines() if 'path = ' in line
                         and not line.startswith('path = ')]
                self.assertEqual(len(paths), 1, paths)
                self.assertTrue(paths[0].startswith('batter = {'), paths[0])

    def test_a_native_dependency_or_patch_cannot_pass(self):
        good = package('single-facade-consumer', ['batter', 'tokio', 'sqlx', 'serde_json'])
        registry = validate_single_dependency('[dependencies]\n', good, NAMES)
        self.assertEqual(registry, ['serde_json', 'sqlx', 'tokio'])
        for extra in ('runledger-runtime', 'runledger-core', 'runlimit-postgres',
                      'batter-runledger', 'runledger-test-support'):
            bad = copy.deepcopy(good)
            bad['packages'][0]['dependencies'].append(dict(name=extra,
                                                          uses_default_features=False))
            with self.subTest(extra=extra), self.assertRaisesRegex(ValueError, 'only batter'):
                validate_single_dependency('[dependencies]\n', bad, NAMES)
        with self.assertRaisesRegex(ValueError, 'no \\[patch\\] section'):
            validate_single_dependency('[patch."https://example.invalid"]\n', good, NAMES)

    def test_default_facade_features_and_one_root_package_are_required(self):
        defaulted = package('single-facade-consumer', ['tokio'])
        defaulted['packages'][0]['dependencies'].append(dict(name='batter'))
        with self.assertRaisesRegex(ValueError, 'default features'):
            validate_single_dependency('[dependencies]\n', defaulted, NAMES)
        empty = dict(packages=[])
        with self.assertRaisesRegex(ValueError, 'exactly one package'):
            validate_single_dependency('[dependencies]\n', empty, NAMES)
        duplicated = package('single-facade-consumer', ['batter'])
        duplicated['packages'].append(duplicated['packages'][0])
        with self.assertRaisesRegex(ValueError, 'exactly one package'):
            validate_single_dependency('[dependencies]\n', duplicated, NAMES)
        # The harness is checked under its own package name.
        harness = package('single-facade-harness', ['batter', 'tokio'])
        validate_single_dependency('[dependencies]\n', harness, NAMES,
                                   package='single-facade-harness')
        with self.assertRaisesRegex(ValueError, 'exactly one package'):
            validate_single_dependency('[dependencies]\n', harness, NAMES)

    def test_workspace_names_come_from_members_only(self):
        metadata = dict(workspace_members=['a 1', 'b 1'],
                        packages=[dict(name='batter', id='a 1'), dict(name='sqlx', id='c 1'),
                                  dict(name='runlimit-core', id='b 1')])
        self.assertEqual(workspace_names(metadata), {'batter', 'runlimit-core'})


class ExecutionTests(unittest.TestCase):
    def test_every_temporary_build_overrides_hostile_sqlx_environment(self):
        # Execute the actual launch commands with a tiny Cargo stand-in. Unlike
        # command-list inspection this proves the child receives the override.
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            cargo = directory / 'cargo.py'
            cargo.write_text('import json, os, sys\n'
                             'assert os.environ["SQLX_OFFLINE"] == "true"\n'
                             'assert os.environ["DATABASE_URL"] == "postgres://unavailable"\n'
                             'if "build" in sys.argv:\n'
                             ' print(json.dumps(dict(reason="compiler-artifact",\n'
                             '       target=dict(name="consumer"), executable="/built/consumer")))\n')

            def execute(command, cwd, **_kwargs):
                return subprocess.run(command, cwd=cwd, check=True, capture_output=True,
                                      text=True).stdout

            for offline in ('false', None):
                with self.subTest(offline=offline), mock.patch.dict(os.environ), \
                        mock.patch.object(runner, 'run', side_effect=execute):
                    os.environ['DATABASE_URL'] = 'postgres://unavailable'
                    if offline is None:
                        os.environ.pop('SQLX_OFFLINE', None)
                    else:
                        os.environ['SQLX_OFFLINE'] = offline
                    command = [sys.executable, str(cargo)]
                    self.assertEqual(runner.build(command, directory, directory, 'consumer'),
                                     '/built/consumer')
                    runner.check_completion(command, directory, directory)
                    runner.check_completion(command, directory, directory, 'single-facade-harness')
                    runner.run_harness(command, directory, directory, '/built/consumer')

    def test_each_required_identity_must_be_single_local_and_present(self):
        source, consumer = Path('/copied').resolve(), Path('/consumer').resolve()
        packages = [dict(name=name, source=None, manifest_path=str(source / name / 'Cargo.toml'))
                    for name in IDENTITIES]
        good = dict(packages=packages)

        def validate(metadata):
            validate_consumer(metadata, source, consumer, set(), required_identities=IDENTITIES)

        validate(good)
        for index, name in enumerate(IDENTITIES):
            for mode in ('missing', 'duplicate', 'remote', 'sibling'):
                bad = copy.deepcopy(good)
                if mode == 'missing':
                    del bad['packages'][index]
                elif mode == 'duplicate':
                    bad['packages'].append(bad['packages'][index])
                elif mode == 'remote':
                    bad['packages'][index].update(source='git+fixture', version='1')
                else:
                    bad['packages'][index]['manifest_path'] = '/sibling/Cargo.toml'
                with self.subTest(name=name, mode=mode), self.assertRaises(ValueError):
                    validate(bad)

    def test_every_marker_must_appear_exactly_once(self):
        require_markers("\n".join(MARKERS))
        for dropped in MARKERS:
            partial = "\n".join(marker for marker in MARKERS if marker != dropped)
            with self.subTest(dropped=dropped), self.assertRaisesRegex(RuntimeError, 'exactly once'):
                require_markers(partial)
            repeated = "\n".join((*MARKERS, dropped))
            with self.subTest(repeated=dropped), self.assertRaisesRegex(RuntimeError, 'exactly once'):
                require_markers(repeated)
        for output in ('', 'Finished dev profile', ' '.join(MARKERS)):
            with self.subTest(output=output), self.assertRaises(RuntimeError):
                require_markers(output)

    def test_only_the_named_binary_artifact_is_selected(self):
        lines = ['not json',
                 '{"reason":"compiler-artifact","target":{"name":"other"},"executable":"/other"}',
                 '{"reason":"compiler-artifact","target":{"name":"consumer"},"executable":null}',
                 '{"reason":"build-finished","success":true}',
                 '{"reason":"compiler-artifact","target":{"name":"consumer"},'
                 '"executable":"/built/consumer"}']
        self.assertEqual(executable("\n".join(lines), 'consumer'), '/built/consumer')
        with self.assertRaisesRegex(RuntimeError, 'did not report an executable'):
            executable("\n".join(lines[:4]), 'consumer')


class CompilerGateTests(unittest.TestCase):
    @staticmethod
    def execute(command, cwd, **_kwargs):
        return subprocess.run(command, cwd=cwd, check=True, capture_output=True,
                              text=True).stdout

    def fixture(self, parent, manifest, source_name):
        source = parent / 'source'
        (source / 'consumers').mkdir(parents=True)
        root = parent / 'package'
        root.mkdir()
        text = manifest(source)
        # Keep the actual generated package, lint and binary tables; substitute
        # one std-only dependency so this compiler control needs no native build.
        start, end = text.index('[dependencies]'), text.index('[lints.rust]')
        text = text[:start] + '[dependencies]\nwarned = { path = "../warned" }\n' + text[end:]
        (root / 'Cargo.toml').write_text(text)
        dependency = parent / 'warned'
        (dependency / 'src').mkdir(parents=True)
        (dependency / 'Cargo.toml').write_text(
            '[package]\nname = "warned"\nversion = "0.0.0"\nedition = "2024"\n[workspace]\n')
        (dependency / 'src/lib.rs').write_text(
            'pub fn dependency_warning() {\n    let unused = 1;\n}\n')
        entry = source / source_name
        entry.write_text('fn main() {}\n')
        cargo = runner.selected_cargo(runner.ROOT)
        self.execute([*cargo, 'generate-lockfile', '--offline'], root)
        return cargo, root, entry

    def test_package_warnings_fail_without_promoting_dependency_warnings(self):
        for manifest, source, name in (
                (consumer_manifest, CONSUMER_SOURCE, 'single-facade-consumer'),
                (harness_manifest, HARNESS_SOURCE, 'single-facade-harness')):
            with self.subTest(package=name), tempfile.TemporaryDirectory() as directory, \
                    mock.patch.object(runner, 'run', side_effect=self.execute):
                cargo, root, entry = self.fixture(Path(directory), manifest, source)
                # A dependency warning remains a warning, and explicit discard
                # remains permitted; this gate guards accidental unused results.
                entry.write_text('fn result() -> Result<(), ()> { Ok(()) }\n'
                                 'fn main() { let _ = result(); }\n')
                runner.build(cargo, root, root / 'target', name)
                entry.write_text('fn result() -> Result<(), ()> { Ok(()) }\n'
                                 'fn main() { result(); }\n')
                with self.assertRaises(subprocess.CalledProcessError) as failed:
                    runner.build(cargo, root, root / 'target', name)
                diagnostics = [json.loads(line) for line in failed.exception.stdout.splitlines()]
                self.assertTrue(any(
                    item.get('reason') == 'compiler-message'
                    and item['message']['level'] == 'error'
                    and item['message'].get('code', {}).get('code') == 'unused_must_use'
                    for item in diagnostics))

    def test_formatting_drift_fails_for_both_standalone_packages(self):
        for manifest, source in ((consumer_manifest, CONSUMER_SOURCE),
                                 (harness_manifest, HARNESS_SOURCE)):
            with self.subTest(source=source), tempfile.TemporaryDirectory() as directory, \
                    mock.patch.object(runner, 'run', side_effect=self.execute):
                cargo, root, entry = self.fixture(Path(directory), manifest, source)
                runner.check_format(cargo, root)
                entry.write_text('fn main( ) { }\n')
                with self.assertRaises(subprocess.CalledProcessError) as failed:
                    runner.check_format(cargo, root)
                self.assertIn('Diff in', failed.exception.stdout)


class SourceCopyTests(unittest.TestCase):
    def test_source_copy_retains_both_consumer_sources_without_git(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            root = parent / 'root'
            keep = [CONSUMER_SOURCE, HARNESS_SOURCE, 'consumers/README.md',
                    'consumers/single_facade_job_tests.rs',
                    'consumers/single_facade_completion.rs',
                    'consumers/single_facade_completion_tests.rs',
                    'consumers/single_facade_quota.rs',
                    'consumers/single_facade_quota_tests.rs',
                    'consumers/single_facade_harness_completion.rs',
                    'consumers/single_facade_harness_tests.rs',
                    'crates/batter/Cargo.toml',
                    'runledger/runledger-test-support/Cargo.toml']
            for name in keep + ['.git/config', 'consumers/.env', 'target/consumer']:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text('fixture')
            result = copy_source(root, parent / 'copy')
            for name in keep:
                self.assertEqual((result / name).read_text(), 'fixture')
            for name in ('.git', 'consumers/.env', 'target/consumer'):
                self.assertFalse((result / name).exists())


if __name__ == '__main__':
    unittest.main()
