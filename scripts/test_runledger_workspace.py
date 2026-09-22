"""Negative controls for the shared-workspace contract, independent of Git state."""
import copy
from pathlib import Path
import tempfile
import unittest

from check_runledger_workspace import (
    EXPECTED_PUBLISH,
    EXPECTED_VERSIONS,
    NATIVE,
    validate_assets,
    validate_graph,
)
from package import eligible


class GraphTests(unittest.TestCase):
    def setUp(self):
        self.root = Path('/workspace').resolve()
        names = ["runledger-" + suffix for suffix in NATIVE] + ["batter-core", "batter-sqlx", "batter"]
        packages = []
        nodes = []
        for name in names:
            directory = "runledger" if name.startswith("runledger-") else "crates"
            packages.append(dict(name=name, id=name, source=None,
                                 publish=EXPECTED_PUBLISH.copy(),
                                 version=EXPECTED_VERSIONS.get(name, "0.0.1"),
                                 manifest_path=str(self.root / directory / name / "Cargo.toml")))
            nodes.append(dict(id=name, deps=[]))
        self.metadata = dict(packages=packages, workspace_members=names,
                             resolve=dict(nodes=nodes))
        self.edge("runledger-postgres", "batter-sqlx")
        self.edge("batter-sqlx", "batter-core")

    def edge(self, source, target):
        node = next(n for n in self.metadata["resolve"]["nodes"] if n["id"] == source)
        node["deps"].append(dict(pkg=target, dep_kinds=[dict(kind=None)]))

    def test_local_graph_passes_without_a_git_repository(self):
        validate_graph(self.metadata, self.root)

    def test_remote_or_sibling_foundation_is_rejected(self):
        for field, value in [("source", "git+https://example.invalid/foundation"),
                             ("manifest_path", "/sibling/batter-sqlx/Cargo.toml")]:
            with self.subTest(field=field):
                metadata = copy.deepcopy(self.metadata)
                next(p for p in metadata["packages"] if p["name"] == "batter-sqlx")[field] = value
                with self.assertRaisesRegex(ValueError, "resolve from this workspace"):
                    validate_graph(metadata, self.root)

    def test_duplicate_foundation_is_rejected(self):
        self.metadata["packages"].append(copy.deepcopy(self.metadata["packages"][-2]))
        with self.assertRaisesRegex(ValueError, "exactly one package"):
            validate_graph(self.metadata, self.root)

    def test_transitive_facade_dependency_is_rejected(self):
        self.edge("runledger-runtime", "runledger-postgres")
        self.edge("batter-sqlx", "batter")
        with self.assertRaisesRegex(ValueError, "must not depend on the Batter facade"):
            validate_graph(self.metadata, self.root)

    def test_reverse_foundation_dependency_is_rejected(self):
        self.edge("batter-core", "runledger-core")
        with self.assertRaisesRegex(ValueError, "foundation must not depend"):
            validate_graph(self.metadata, self.root)

    def test_wrong_publication_policy_is_rejected(self):
        self.metadata["packages"][0]["publish"] = None
        with self.assertRaisesRegex(ValueError, "crates.io publication"):
            validate_graph(self.metadata, self.root)

    def test_missing_workspace_membership_is_rejected(self):
        self.metadata["workspace_members"].remove("runledger-core")
        with self.assertRaisesRegex(ValueError, "workspace member"):
            validate_graph(self.metadata, self.root)


class AssetTests(unittest.TestCase):
    def test_migration_and_cache_drift_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for suffix, folders in [('.sql', ['migrations', 'runledger-postgres/migrations',
                                             'runledger-test-support/migrations']),
                                    ('.json', ['.sqlx', 'runledger-postgres/.sqlx',
                                              'runledger-runtime/.sqlx'])]:
                for folder in folders:
                    directory = root / 'runledger' / folder
                    directory.mkdir(parents=True, exist_ok=True)
                    (directory / ('fixture' + suffix)).write_text('original')
            validate_assets(root)
            for path in ['runledger-postgres/migrations/fixture.sql',
                         'runledger-runtime/.sqlx/fixture.json']:
                target = root / 'runledger' / path
                target.write_text('changed')
                with self.assertRaisesRegex(ValueError, 'differs|differ'):
                    validate_assets(root)
                target.write_text('original')


class ArchiveAssetTests(unittest.TestCase):
    def test_native_compile_time_sql_and_migrations_are_archived(self):
        root = Path(__file__).resolve().parent.parent
        native = root / "runledger"
        required = [native / "runledger-postgres/src/jobs/queue/claim_ids.sql"]
        for directory in [native / "migrations", native / "runledger-postgres/migrations",
                          native / "runledger-test-support/migrations"]:
            migrations = list(directory.glob("*.sql"))
            self.assertTrue(migrations, directory)
            required.extend(migrations)
        for path in required:
            with self.subTest(path=path):
                self.assertTrue(eligible(path, root), "archive must retain compiled SQL and migrations")
