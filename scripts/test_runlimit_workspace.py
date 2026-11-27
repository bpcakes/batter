"""Negative controls for native source ownership and preserved import artifacts."""
import copy
import hashlib
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest

from check_runlimit_workspace import (
    EXPECTED_PUBLISH,
    EXPECTED_VERSION,
    NATIVE,
    UNEXECUTED_RECORDS,
    validate_assets,
    validate_graph,
    validate_provenance,
    validate_published,
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
            validate_assets(root, required={})
            migration = native / assets[-1]
            migration.write_text('altered published SQL')
            with self.assertRaisesRegex(ValueError, 'changed or missing'):
                validate_assets(root, required={})
            migration.unlink()
            with self.assertRaisesRegex(ValueError, 'changed or missing'):
                validate_assets(root, required={})

    def test_provenance_records_are_inventoried_regular_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            native = root / 'runlimit'
            record = native / 'provenance' / 'runlimit-postgres' / 'superseded.sql'
            record.parent.mkdir(parents=True)
            record.write_text('SELECT 1')
            for name in ('LICENSE-MIT', 'LICENSE-APACHE'):
                (native / name).write_text(name)
            (native / 'provenance' / 'README.md').write_text('records')
            (native / 'provenance' / '.DS_Store').write_bytes(b'finder metadata')
            names = ['LICENSE-MIT', 'LICENSE-APACHE', 'provenance/runlimit-postgres/superseded.sql']
            expected = {name: hashlib.sha256((native / name).read_bytes()).hexdigest()
                        for name in names}
            (native / 'upstream-assets.json').write_text(json.dumps(expected))
            validate_assets(root, required={})
            self.assertTrue(eligible(record, root))

            stray = record.with_name('stray.sql')
            stray.write_text('SELECT 2')
            with self.assertRaisesRegex(ValueError, 'uninventoried file: provenance/'):
                validate_assets(root, required={})
            stray.unlink()

            live = native / 'runlimit-postgres' / 'src' / 'superseded.sql'
            live.parent.mkdir(parents=True)
            live.write_bytes(record.read_bytes())
            record.unlink()
            record.symlink_to(live)
            with self.assertRaisesRegex(ValueError, 'provenance holds a symlink'):
                validate_assets(root, required={})
            record.unlink()
            record.write_bytes(live.read_bytes())
            (native / 'provenance' / 'outward').symlink_to(live.parent)
            with self.assertRaisesRegex(ValueError, 'provenance holds a symlink'):
                validate_assets(root, required={})
            (native / 'provenance' / 'outward').unlink()
            validate_assets(root, required={})

            moved = root / 'elsewhere'
            (native / 'provenance').rename(moved)
            (native / 'provenance').symlink_to(moved)
            with self.assertRaisesRegex(ValueError, 'provenance is a symlink'):
                validate_assets(root, required={})

    def test_superseded_records_cannot_move_back_with_their_inventory(self):
        digest, record = next(iter(UNEXECUTED_RECORDS.items()))
        base = {'LICENSE-MIT': 'a', 'LICENSE-APACHE': 'b'}
        others = {name: value for value, name in UNEXECUTED_RECORDS.items() if value != digest}
        archived = {**base, **{'provenance/runlimit-postgres/' + n: v for n, v in others.items()}}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            native = root / 'runlimit'
            for layout, message in (
                    ({**archived, 'runlimit-postgres/src/' + record: digest}, 'only under'),
                    ({**archived, 'runlimit-postgres/src/' + record: digest,
                      'provenance/runlimit-postgres/' + record: digest}, 'only under'),
                    (archived, 'only under')):
                with self.subTest(layout=sorted(layout)), self.assertRaisesRegex(ValueError, message):
                    validate_provenance(root, layout, UNEXECUTED_RECORDS)
            validate_provenance(root, {**archived, 'provenance/runlimit-postgres/' + record: digest},
                                UNEXECUTED_RECORDS)
            self.assertFalse(native.exists())

    def test_a_package_archive_cannot_carry_a_record(self):
        record = b'SELECT 1'
        expected = {'LICENSE-MIT': 'a', 'LICENSE-APACHE': 'b',
                    'runlimit-postgres/migrations/initial.sql': hashlib.sha256(b'SELECT 0').hexdigest(),
                    'provenance/runlimit-postgres/superseded.sql': hashlib.sha256(record).hexdigest()}
        with tempfile.TemporaryDirectory() as directory:
            def archive(name, members):
                path = Path(directory) / name
                with tarfile.open(path, 'w:gz') as bundle:
                    for member, data in members:
                        info = tarfile.TarInfo(member)
                        info.size = len(data)
                        bundle.addfile(info, io.BytesIO(data))
                return path
            clean = archive('clean.crate', [('runlimit-postgres-0.4.0/migrations/initial.sql',
                                             b'SELECT 0')])
            validate_published([clean], expected)
            for member in ('runlimit-postgres-0.4.0/src/superseded.sql',
                           'runlimit-postgres-0.4.0/archived query.txt',
                           'runlimit-postgres-0.4.0/README.md'):
                with self.subTest(member=member), self.assertRaisesRegex(
                        ValueError, 'would publish provenance record provenance/'):
                    validate_published([clean, archive('copy.crate', [(member, record)])],
                                       expected)

            sources = [('runlimit-postgres-0.4.0/src/lib.rs',
                        b'const SQL: &str = include_str!("live.sql");'),
                       ('runlimit-postgres-0.4.0/src/live.sql', b'SELECT 2'),
                       ('runlimit-postgres-0.4.0/migrations/initial.sql', b'SELECT 0'),
                       ('runlimit-postgres-0.4.0/tests/live.rs',
                        b'const M: &str = include_str!("../migrations/./initial.sql");')]
            validate_published([archive('includes.crate', sources)], expected)
            for test in (b'include_str!("../../provenance/runlimit-postgres/superseded.sql")',
                         b'include_bytes![r#"../src/missing.sql"#]',
                         b'include_str! { "/etc/hosts" }'):
                with self.subTest(test=test), self.assertRaisesRegex(ValueError, 'not in the archive'):
                    validate_published([archive('include.crate', sources + [
                        ('runlimit-postgres-0.4.0/tests/archived.rs', b'const S: &str = ' + test + b';')])],
                        expected)
