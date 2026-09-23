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
