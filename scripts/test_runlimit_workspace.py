"""Negative controls for native source ownership and preserved import artifacts."""
import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from check_runlimit_workspace import (
    EXPECTED_PUBLISH,
    EXPECTED_VERSION,
    NATIVE,
    validate_assets,
    validate_graph,
)
from package import eligible


class GraphTests(unittest.TestCase):
    def setUp(self):
        self.root = Path('/workspace').resolve()
        names = ['runlimit-' + s for s in NATIVE] + ['batter', 'batter-core', 'tokio', 'bridge']
        self.metadata = dict(packages=[dict(name=n, id=n, source=None,
                                           publish=EXPECTED_PUBLISH.copy(),
                                           version=EXPECTED_VERSION,
                                           manifest_path=str(self.root / 'runlimit' / n / 'Cargo.toml'))
                                       for n in names], workspace_members=names,
                             resolve=dict(nodes=[dict(id=n, deps=[]) for n in names]))

    def edge(self, source, target):
        node = next(n for n in self.metadata['resolve']['nodes'] if n['id'] == source)
        node['deps'].append(dict(pkg=target, dep_kinds=[dict(kind=None)]))

    def test_local_graph_passes_without_git(self):
        validate_graph(self.metadata, self.root)

    def test_duplicate_remote_external_published_and_nonmember_packages_fail(self):
        baseline = copy.deepcopy(self.metadata)
        for mutation in ('duplicate', 'source', 'path', 'publish', 'member'):
            self.metadata = copy.deepcopy(baseline)
            p = self.metadata['packages'][0]
            if mutation == 'duplicate': self.metadata['packages'].append(copy.deepcopy(p))
            if mutation == 'source': p['source'] = 'git+https://example.invalid/native'
            if mutation == 'path': p['manifest_path'] = '/sibling/Cargo.toml'
            if mutation == 'publish': p['publish'] = None
            if mutation == 'member': self.metadata['workspace_members'].remove(p['id'])
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                validate_graph(self.metadata, self.root)

    def test_transitive_batter_dependency_fails(self):
        self.edge('runlimit-postgres', 'bridge')
        self.edge('bridge', 'batter-core')
        with self.assertRaisesRegex(ValueError, 'cannot depend on Batter'):
            validate_graph(self.metadata, self.root)

    def test_runtime_in_native_core_fails(self):
        self.edge('runlimit-core', 'tokio')
        with self.assertRaisesRegex(ValueError, 'independent'):
            validate_graph(self.metadata, self.root)

    def test_native_dependency_in_foundation_fails(self):
        self.edge('batter-core', 'runlimit-core')
        with self.assertRaisesRegex(ValueError, 'foundation cannot'):
            validate_graph(self.metadata, self.root)


class AssetTests(unittest.TestCase):
    def test_changed_or_missing_import_asset_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            native = root / 'runlimit'
            native.mkdir()
            assets = ['LICENSE-MIT', 'LICENSE-APACHE', 'runlimit-postgres/migrations/initial.sql']
            expected = {}
            for name in assets:
                path = native / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(name)
                expected[name] = hashlib.sha256(path.read_bytes()).hexdigest()
                self.assertTrue(eligible(path, root))
            (native / 'upstream-assets.json').write_text(json.dumps(expected))
            validate_assets(root)
            migration = native / assets[-1]
            migration.write_text('altered published SQL')
            with self.assertRaisesRegex(ValueError, 'changed or missing'):
                validate_assets(root)
            migration.unlink()
            with self.assertRaisesRegex(ValueError, 'changed or missing'):
                validate_assets(root)

    def test_provenance_records_stay_inventoried_unpackaged_and_unembedded(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            native = root / 'runlimit'
            record = native / 'provenance' / 'runlimit-postgres' / 'superseded.sql'
            record.parent.mkdir(parents=True)
            record.write_text('SELECT 1')
            for name in ('LICENSE-MIT', 'LICENSE-APACHE'):
                (native / name).write_text(name)
            (native / 'provenance' / 'README.md').write_text('records')
            names = ['LICENSE-MIT', 'LICENSE-APACHE', 'provenance/runlimit-postgres/superseded.sql']
            expected = {name: hashlib.sha256((native / name).read_bytes()).hexdigest()
                        for name in names}
            (native / 'upstream-assets.json').write_text(json.dumps(expected))
            validate_assets(root)
            self.assertTrue(eligible(record, root))

            stray = record.with_name('stray.sql')
            stray.write_text('SELECT 2')
            with self.assertRaisesRegex(ValueError, 'uninventoried file: provenance/'):
                validate_assets(root)
            stray.unlink()

            for restored in (native / 'runlimit-postgres' / 'src' / 'superseded.sql',
                             root / 'crates' / 'adapter' / 'sql' / 'renamed.sql'):
                restored.parent.mkdir(parents=True, exist_ok=True)
                restored.write_bytes(record.read_bytes())
                with self.subTest(restored=restored), self.assertRaisesRegex(ValueError, 'copied to'):
                    validate_assets(root)
                restored.unlink()
            (native / 'runlimit-postgres' / 'src' / 'live.sql').write_text('SELECT 3')
            validate_assets(root)

            native_source = native / 'runlimit-postgres' / 'src' / 'lib.rs'
            adapter_source = root / 'crates' / 'adapter' / 'src' / 'lib.rs'
            for source in (native_source, adapter_source):
                source.parent.mkdir(parents=True, exist_ok=True)
                source.write_text('// The imported superseded.sql is never executed.\n')
            validate_assets(root)
            for source, text in (
                    (native_source, 'const SQL: &str = include_str!('
                                    '"../../provenance/runlimit-postgres/superseded.sql");'),
                    (native_source, 'const SQL: &str = include_str!('
                                    'r"../../provenance/runlimit-postgres/superseded.sql");'),
                    (native_source, 'const SQL: &[u8] = include_bytes!(r#"../../provenance/'
                                    'runlimit-postgres/superseded.sql"#);'),
                    (adapter_source, 'const SQL: &str = include_str!("../../../runlimit/'
                                     'provenance/runlimit-postgres/superseded.sql");'),
                    (native_source, '// see provenance/runlimit-postgres/superseded.sql')):
                original = source.read_text()
                source.write_text(text + '\n')
                with self.subTest(text=text), self.assertRaisesRegex(ValueError, 'names provenance'):
                    validate_assets(root)
                source.write_text(original)
