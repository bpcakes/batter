"""Check source archives and active documentation links."""
import hashlib
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[1]


class ArchiveTests(unittest.TestCase):
    def test_static_package_checks_active_docs_but_not_agent_history(self):
        with tempfile.TemporaryDirectory(prefix="batter-package-check-") as directory:
            root = Path(directory)
            for relative in [".agent/plans/old.md", ".agent/reviews/old.md"]:
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("[historical evidence](/tmp/removed-evidence.json)\n")

            command = [sys.executable, str(ROOT / "scripts/check_package.py"), "--root", str(root)]
            result = subprocess.run(command, capture_output=True, text=True, timeout=30)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

            (root / "README.md").write_text("[missing](missing.md)\n")
            result = subprocess.run(command, capture_output=True, text=True, timeout=30)
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("README.md: missing link target missing.md", result.stdout)

    def test_archive_keeps_jig_sources_and_excludes_local_artifacts(self):
        with tempfile.TemporaryDirectory(prefix="batter-package-test-") as directory:
            repo = Path(directory) / "repo"
            (repo / "scripts").mkdir(parents=True)
            shutil.copy2(ROOT / "scripts/package.py", repo / "scripts/package.py")
            included = ["Cargo.toml", "Cargo.lock", "scripts/jig", "scripts/install-jig.sh", ".jig.toml",
                        ".agent/jig-contract.json", ".jig/file-budget.toml",
                        "crates/batter/Cargo.toml", "crates/batter/src/lib.rs",
                        "crates/batter/examples/worker.rs",
                        "crates/batter-core/Cargo.toml", "crates/batter-core/src/lib.rs",
                        "crates/batter-core/tests/lifecycle.rs",
                        "test-support/process/watchdog.rs",
                        "crates/batter-axum/Cargo.toml", "crates/batter-axum/src/lib.rs",
                        "crates/batter/examples/http_service.rs",
                        "crates/batter-test-support/Cargo.toml", "crates/batter-test-support/src/lib.rs",
                        "examples/postgres-lifecycle/Cargo.toml", "examples/postgres-lifecycle/src/main.rs",
                        "runlimit/LICENSE-MIT", "runlimit/LICENSE-APACHE",
                        "runlimit/runlimit-postgres/attempts-migrations/initial.sql",
                        "runlimit/runlimit-postgres/gcra-migrations/initial.sql",
                        "runlimit/runlimit-postgres/src/attempt_cleanup.sql",
                        "runledger/llms.txt", "runledger/docs/evidence/change.patch",
                        "runledger/runledger-postgres/Cargo.toml",
                        "runledger/runledger-postgres/src/jobs/queue/claim_ids.sql",
                        "runledger/migrations/202603280001_runledger_baseline.up.sql",
                        "runledger/runledger-postgres/migrations/202603280001_runledger_baseline.up.sql",
                        "runledger/runledger-test-support/migrations/202603280001_runledger_baseline.up.sql",
                        "runledger/.sqlx/query-example.json",
                        "runledger/runledger-postgres/.sqlx/query-example.json",
                        "runledger/runledger-runtime/.sqlx/query-example.json"]
            excluded = [".agent/state/receipts.jsonl", ".agent/plans/old.md", ".agent/.cache/runtime.json", ".agent/.cache/adopt/backup.md",
                        ".agent/runtime/session.json", ".agent/tmp/note.md",
                        ".agent/state/adopt-last.json", ".agent/plans/example.md.lock",
                        ".env", "target/cache.rs", "validation/local/check.json",
                        "crates/batter/target/cache.rs", "examples/postgres-lifecycle/target/cache.rs",
                        "examples/postgres-lifecycle/.env", "runledger/runledger-postgres/.env",
                        "runledger/runledger-postgres/target/cache.rs"]
            for name in included + excluded:
                path = repo / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("fixture sentinel\n")
            output = Path(directory) / "source.zip"
            subprocess.run([sys.executable, str(repo / "scripts/package.py"), "--output", str(output)],
                           check=True, capture_output=True, text=True, timeout=30)
            self.assertFalse((repo / "SHA256SUMS").exists())
            checksum = output.with_suffix(output.suffix + ".sha256")
            self.assertEqual(
                checksum.read_text(),
                f"{hashlib.sha256(output.read_bytes()).hexdigest()}  {output.name}\n",
            )
            with zipfile.ZipFile(output) as archive:
                names = set(archive.namelist())
                inventory = archive.read("batter/SHA256SUMS").decode()
                inventory_entries = {
                    name: digest
                    for line in inventory.splitlines()
                    for digest, name in [line.split("  ", 1)]
                }
                source_members = {
                    name.removeprefix("batter/")
                    for name in names
                    if name != "batter/SHA256SUMS"
                }
                self.assertEqual(set(inventory_entries), source_members)
                for name, expected_digest in inventory_entries.items():
                    self.assertEqual(
                        expected_digest,
                        hashlib.sha256(archive.read("batter/" + name)).hexdigest(),
                    )
                for name in included:
                    self.assertIn("batter/" + name, names)
                for name in excluded:
                    self.assertNotIn("batter/" + name, names)
                self.assertTrue(archive.getinfo("batter/scripts/jig").external_attr >> 16 & 0o111)


if __name__ == "__main__":
    unittest.main()
