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
    package_archives,
    validate_provenance,
    validate_published,
    validate_sql_gate,
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

            for stray in (record.with_name('stray.sql'), record.with_name('old.psql')):
                stray.write_text('SELECT 2')
                with self.subTest(stray=stray.name), self.assertRaisesRegex(
                        ValueError, 'uninventoried file: provenance/'):
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
            for name in ('provenance/../history/' + record, 'provenance/./' + record,
                         'provenance', 'runlimit-postgres/provenance/' + record):
                with self.subTest(name=name), self.assertRaisesRegex(ValueError, 'only under'):
                    validate_provenance(root, {**archived, name: digest}, UNEXECUTED_RECORDS)
            self.assertFalse(native.exists())

    def test_a_package_archive_cannot_carry_a_record(self):
        record = b'SELECT 1 FROM superseded'
        records = {'provenance/runlimit-postgres/superseded.sql': record}
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
                                             b'SELECT 0'),
                                            ('runlimit-postgres-0.4.0/src/partial.sql',
                                             record[:-1])])
            validate_published([clean], records)
            for member, data in (('runlimit-postgres-0.4.0/src/superseded.sql', record),
                                 ('runlimit-postgres-0.4.0/archived query.txt', record),
                                 ('runlimit-postgres-0.4.0/README.md', record),
                                 ('runlimit-postgres-0.4.0/src/prefixed.sql',
                                  b'-- kept for reference\n' + record + b'\n-- end\n')):
                with self.subTest(member=member), self.assertRaisesRegex(
                        ValueError, 'would publish provenance record provenance/'):
                    validate_published([clean, archive('copy.crate', [(member, data)])],
                                       records)

    def test_package_verification_refuses_a_directory_inside_the_checkout(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for inside in (root, root / 'target' / 'package-check'):
                with self.subTest(inside=inside), self.assertRaisesRegex(ValueError, 'inside the checkout'):
                    package_archives(root, inside)


class SqlGateTests(unittest.TestCase):
    MANIFEST = (b'[package]\nname = "runlimit-postgres"\n\n[lints.rust]\nunsafe_code = "forbid"\n\n'
                b'[lints.clippy]\nall = { level = "warn", priority = -1 }\n'
                b'pedantic = { level = "warn", priority = -1 }\n')
    SHARED = b'cognitive-complexity-threshold = 20\n'
    LOCAL = SHARED + b'disallowed-methods = [{ path = "sqlx::query" }]\n'

    def tree(self, manifest=MANIFEST, local=LOCAL):
        temporary = tempfile.TemporaryDirectory(prefix='runlimit-sql-gate-')
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        package = root / 'runlimit' / 'runlimit-postgres'
        package.mkdir(parents=True)
        (package / 'Cargo.toml').write_bytes(manifest)
        (package / 'clippy.toml').write_bytes(local)
        (root / 'runlimit' / 'clippy.toml').write_bytes(self.SHARED)
        return root

    def test_the_checkout_and_a_whole_fixture_pass(self):
        validate_sql_gate(Path(__file__).resolve().parent.parent)
        validate_sql_gate(self.tree())

    def test_lowering_a_gate_lint_or_its_group_fails(self):
        for setting in (b'style = "allow"', b'default = { level = "allow" }',
                        b'disallowed_methods = "allow"', b'disallowed-methods = "allow"',
                        b'disallowed_types = { priority = 1, level = "allow" }',
                        b'disallowed-macros = { level = "allow", priority = 1 }'):
            with self.subTest(setting=setting), self.assertRaisesRegex(ValueError, 'lowers'):
                validate_sql_gate(self.tree(self.MANIFEST + setting + b'\n'))
        lowered_all = self.MANIFEST.replace(b'all = { level = "warn", priority = -1 }',
                                            b'all = { level = "allow", priority = -1 }')
        with self.assertRaisesRegex(ValueError, 'clippy::all'):
            validate_sql_gate(self.tree(lowered_all))

    def test_lint_levels_set_in_another_form_fail_closed(self):
        for manifest in (self.MANIFEST + b'\n[lints.clippy.style]\nlevel = "allow"\n',
                         self.MANIFEST + b"\n['lints'.clippy.disallowed_methods]\nlevel = \"allow\"\n",
                         self.MANIFEST + b'\n[ "lints" . clippy . style ]\nlevel = "allow"\n',
                         self.MANIFEST + b'\n["\\u006cints".clippy.style]\nlevel = "allow"\n',
                         b"'lints'.clippy.style = \"allow\"\n" + self.MANIFEST,
                         b'lints.clippy.style = "allow"\n' + self.MANIFEST,
                         self.MANIFEST + b'"style" = "allow"\n',
                         self.MANIFEST + b'style = { level = "allow", note = 1 }\n'):
            with self.subTest(manifest=manifest), self.assertRaises(ValueError):
                validate_sql_gate(self.tree(manifest))

    def test_allowing_warnings_or_inheriting_workspace_levels_fails(self):
        for lint in (b'warnings', b'unfulfilled_lint_expectations', b'unfulfilled-lint-expectations'):
            allowed = self.MANIFEST.replace(b'unsafe_code = "forbid"', lint + b' = "allow"')
            with self.subTest(lint=lint), self.assertRaisesRegex(ValueError, 'lowers'):
                validate_sql_gate(self.tree(allowed))
        inherited = (b'[package]\nname = "runlimit-postgres"\n\n[lints]\nworkspace = true\n')
        with self.assertRaisesRegex(ValueError, 'inherits'):
            validate_sql_gate(self.tree(inherited))

    def test_dropping_a_shared_clippy_setting_fails(self):
        with self.assertRaisesRegex(ValueError, 'cognitive-complexity-threshold'):
            validate_sql_gate(self.tree(local=b'disallowed-methods = []\n'))
