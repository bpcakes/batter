"""Failure controls for exported Runlimit graph and executable completion."""
import copy
from pathlib import Path
import tempfile
import unittest

from check_runlimit_consumer import EXECUTIONS, IDENTITIES, require_execution
from runledger_source import copy_source, validate_consumer


class ConsumerTests(unittest.TestCase):
    def test_each_required_identity_must_be_single_local_and_present(self):
        source, consumer = Path('/copied').resolve(), Path('/consumer').resolve()
        packages = [dict(name=n, source=None, manifest_path=str(source / n / 'Cargo.toml'))
                    for n in IDENTITIES]
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
        drift = copy.deepcopy(good)
        drift['packages'].append(dict(name='external', version='2', source='registry+fixture'))
        with self.assertRaisesRegex(ValueError, 'locked external'):
            validate_consumer(drift, source, consumer, {('external', '1', 'registry+fixture')},
                              required_identities=IDENTITIES)

    def test_missing_duplicate_or_different_completion_cannot_pass(self):
        for _, marker in EXECUTIONS:
            require_execution('build output\n' + marker + '\n', marker)
            for output in ('', 'Finished dev profile', marker + '\n' + marker,
                           'prefix ' + marker, marker + ' FAILED'):
                with self.subTest(output=output), self.assertRaises(RuntimeError):
                    require_execution(output, marker)

    def test_source_copy_retains_all_native_smokes_sql_and_license_assets(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            root = parent / 'root'
            keep = ['runlimit/smoke/native_consumer.rs', 'runlimit/smoke/facade_consumer.rs',
                    'runlimit/LICENSE-MIT', 'runlimit/LICENSE-APACHE',
                    'runlimit/runlimit-postgres/attempts-migrations/001.sql',
                    'runlimit/runlimit-postgres/gcra-migrations/001.sql',
                    'runlimit/runlimit-postgres/migrations/001.sql',
                    'runlimit/runlimit-postgres/src/attempt_complete.sql']
            for name in keep + ['.git', 'runlimit/.env', 'target/output.rs']:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text('fixture')
            result = copy_source(root, parent / 'copy')
            for name in keep:
                self.assertEqual((result / name).read_text(), 'fixture')
            for name in ('.git', 'runlimit/.env', 'target/output.rs'):
                self.assertFalse((result / name).exists())


if __name__ == '__main__':
    unittest.main()
