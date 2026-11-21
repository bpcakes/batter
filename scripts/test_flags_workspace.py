"""Negative controls for the flags packages' identities, dependency direction and roots."""
import copy
from pathlib import Path
import subprocess
import sys
import unittest

from check_flags_workspace import (
    ADAPTER_DEV,
    ADAPTER_NORMAL,
    COMMAND,
    LIBRARIES,
    PACKAGES,
    validate_core_bare,
    validate_core_manifest,
    validate_graph,
    validate_identities,
    validate_manifests,
    validate_roots,
)

OTHERS = ['batter', 'batter-core', 'batter-sqlx', 'batter-axum', 'batter-runledger',
          'batter-test-support', 'runledger-core', 'tokio', 'bridge']


class Fixture(unittest.TestCase):
    """A workspace whose flags graph is exactly the shipped one."""

    def setUp(self):
        self.root = Path('/workspace').resolve()
        names = list(PACKAGES) + OTHERS
        self.metadata = dict(
            packages=[dict(name=n, id=n, source=None, version='0.0.1', rust_version='1.100.0',
                           license='MIT', publish=[] if n == COMMAND else ['crates-io'],
                           manifest_path=str(self.root / 'crates' / n / 'Cargo.toml'))
                      for n in names],
            workspace_members=names,
            resolve=dict(nodes=[dict(id=n, deps=[]) for n in names]))
        self.edge('batter-flags-postgres', 'batter-flags-core')
        self.edge('batter-flags-cli', 'batter-flags-core')
        self.edge('batter-flags-cli', 'batter-flags-postgres')
        self.edge('batter', 'batter-flags')
        self.package('batter')['dependencies'] = [
            dict(name='batter-flags', kind=None, optional=True)]
        for target in ADAPTER_NORMAL:
            self.edge('batter-flags', target)
        for target in ADAPTER_DEV:
            self.edge('batter-flags', target, kind='dev')

    def edge(self, source, target, kind=None):
        node = next(n for n in self.metadata['resolve']['nodes'] if n['id'] == source)
        node['deps'].append(dict(pkg=target, dep_kinds=[dict(kind=kind)]))

    def package(self, name):
        return next(p for p in self.metadata['packages'] if p['name'] == name)


class IdentityTests(Fixture):
    def test_the_shipped_identities_pass(self):
        validate_identities(self.metadata, self.root)

    def test_rule_1_each_identity_mutation_fails(self):
        baseline = copy.deepcopy(self.metadata)
        mutations = {
            'duplicate': lambda p: self.metadata['packages'].append(copy.deepcopy(p)),
            'source': lambda p: p.update(source='git+https://example.invalid/flags'),
            'path': lambda p: p.update(manifest_path='/sibling/Cargo.toml'),
            'member': lambda p: self.metadata['workspace_members'].remove(p['id']),
            'version': lambda p: p.update(version='0.1.0'),
            'rust_version': lambda p: p.update(rust_version='1.94'),
            'license': lambda p: p.update(license='Apache-2.0'),
            'publish': lambda p: p.update(publish=None),
        }
        for mutation, apply in mutations.items():
            self.metadata = copy.deepcopy(baseline)
            apply(self.package('batter-flags-core'))
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                validate_identities(self.metadata, self.root)

    def test_rule_1_a_published_command_fails(self):
        self.package(COMMAND)['publish'] = ['crates-io']
        with self.assertRaisesRegex(ValueError, 'publish'):
            validate_identities(self.metadata, self.root)


class GraphTests(Fixture):
    def test_the_shipped_graph_passes(self):
        validate_graph(self.metadata)

    def test_rule_2_a_transitive_batter_edge_of_the_core_fails(self):
        self.edge('batter-flags-core', 'bridge', kind='dev')
        self.edge('bridge', 'batter-core')
        with self.assertRaisesRegex(ValueError, 'batter-flags-core must depend on no Batter'):
            validate_graph(self.metadata)

    def test_rule_4_a_foundation_that_reaches_a_flags_package_fails(self):
        self.edge('batter-axum', 'bridge')
        self.edge('bridge', 'batter-flags-core')
        with self.assertRaisesRegex(ValueError, 'batter-axum must not depend on a flags package'):
            validate_graph(self.metadata)

    def test_rule_5_an_edge_from_another_package_fails(self):
        self.edge('batter-sqlx', 'batter-flags-core', kind='dev')
        with self.assertRaisesRegex(ValueError, 'batter-sqlx may not depend on batter-flags-core'):
            validate_graph(self.metadata)

    def test_rule_5_a_mandatory_or_dev_facade_edge_fails(self):
        for change in (dict(optional=False), dict(kind='dev')):
            self.setUp()
            self.package('batter')['dependencies'][0].update(change)
            with self.subTest(change=change), self.assertRaisesRegex(ValueError, 'optional normal edge'):
                validate_graph(self.metadata)

    def test_rule_6_the_adapter_reaching_the_facade_fails(self):
        self.edge('batter-flags', 'bridge')
        self.edge('bridge', 'batter')
        with self.assertRaisesRegex(ValueError, 'never depend on the facade'):
            validate_graph(self.metadata)

    def test_rule_6_a_dev_edge_made_normal_fails(self):
        node = next(n for n in self.metadata['resolve']['nodes'] if n['id'] == 'batter-flags')
        for dep in node['deps']:
            if dep['pkg'] == 'batter-runledger':
                dep['dep_kinds'] = [dict(kind=None)]
        with self.assertRaisesRegex(ValueError, 'unexpected normal Batter edges'):
            validate_graph(self.metadata)


class SourceTests(unittest.TestCase):
    MANIFEST = '[package]\nname = "x"\n\n[lints]\nworkspace = true\n\n[dependencies]\n'

    def test_rule_3_a_bare_core_holding_a_runtime_or_serialization_fails(self):
        validate_core_bare(['batter-flags-core', 'sha2', 'thiserror', 'tracing'])
        for held in ('tokio', 'serde_json'):
            with self.subTest(held=held), self.assertRaises(ValueError):
                validate_core_bare(['batter-flags-core', held])

    @staticmethod
    def dependency(name, **fields):
        return dict(dict(name=name, kind=None, target=None, optional=False, features=[]),
                    **fields)

    def test_rule_3_a_required_serialization_dependency_fails(self):
        optional = [self.dependency('serde', optional=True, features=['derive']),
                    self.dependency('serde_json', optional=True)]
        validate_core_manifest(dict(dependencies=optional))
        for replacement in (self.dependency('serde_json'),
                            self.dependency('serde_json', optional=True, kind='dev'),
                            self.dependency('serde_json', optional=True, target='cfg(unix)')):
            with self.subTest(entry=replacement), \
                    self.assertRaisesRegex(ValueError, 'serde_json optional'):
                validate_core_manifest(dict(dependencies=optional[:1] + [replacement]))

    def test_rule_7_lints_and_the_command_tls_feature_are_required(self):
        manifests = {name: self.MANIFEST for name in PACKAGES}
        tls = self.dependency('sqlx', features=['postgres', 'tls-rustls-ring'])
        packages = {COMMAND: dict(dependencies=[tls])}
        validate_manifests(manifests, packages)
        for without in (dict(tls, features=['postgres']), dict(tls, kind='dev'),
                        dict(tls, target='cfg(unix)')):
            with self.subTest(entry=without), \
                    self.assertRaisesRegex(ValueError, 'rustls TLS feature'):
                validate_manifests(manifests, {COMMAND: dict(dependencies=[without])})
        # A comment or extra space is the same declaration.
        spaced = self.MANIFEST.replace('workspace = true', 'workspace  =  true  # inherit')
        validate_manifests(dict(manifests, **{COMMAND: spaced}), packages)
        for broken in (self.MANIFEST.replace('workspace = true', 'unused = true'),
                       self.MANIFEST.replace('workspace = true', 'workspace = false'),
                       self.MANIFEST.replace('workspace = true\n', 'workspace = true\nrust.unsafe_code = "allow"\n'),
                       self.MANIFEST + '\n[lints.rust]\nunsafe_code = "allow"\n',
                       self.MANIFEST.replace('[lints]\nworkspace = true\n', ''),
                       self.MANIFEST + '\n[lints]\nworkspace = true\n',
                       'lints.workspace = true\n' + self.MANIFEST.replace('[lints]\nworkspace = true\n', '')):
            with self.subTest(manifest=broken), \
                    self.assertRaisesRegex(ValueError, 'inherit the workspace lints'):
                validate_manifests(dict(manifests, **{COMMAND: broken}), packages)

    def test_the_checker_runs_without_tomllib(self):
        """Python 3.9 and 3.10 have no tomllib; the checker must not need it."""
        probe = ('import sys; sys.modules["tomllib"] = None; sys.path.insert(0, sys.argv[1]); '
                 'import check_flags_workspace as c; '
                 'assert c.inherits_workspace_lints("[lints]\\nworkspace = true\\n")')
        subprocess.run([sys.executable, '-I', '-c', probe, str(Path(__file__).resolve().parent)],
                       check=True)

    def test_rule_8_a_root_without_forbidden_unsafe_code_fails(self):
        roots = {name: '//! Docs.\n#![forbid(unsafe_code)]\n' for name in PACKAGES}
        validate_roots(roots)
        with self.assertRaisesRegex(ValueError, "batter-flags's crate root"):
            validate_roots(dict(roots, **{'batter-flags': '//! Docs.\n'}))


if __name__ == '__main__':
    unittest.main()
