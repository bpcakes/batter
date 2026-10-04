"""Exercise the pinned release cache and failed downloads without source builds."""
from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
VERSION = (ROOT / ".jig/runtime-version").read_text().strip()


class JigRuntimeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.env = {k: v for k, v in os.environ.items() if not k.startswith(("GIT_", "JIG_"))}
        cls.env.update(GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_SYSTEM=os.devnull,
                       GIT_TERMINAL_PROMPT="0")
        result = subprocess.run(
            ["scripts/install-jig.sh", "--repository-scope", "--profile", "runtime", "--resolve-only"],
            cwd=ROOT, env=cls.env, text=True, capture_output=True, timeout=30,
        )
        if result.returncode:
            raise RuntimeError("Run scripts/jig file-budget validate before these tests.")
        cls.runtime = Path(result.stdout.strip())

    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="batter-jig-runtime-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.repo = self.root / "repo"
        for name in [".jig.toml", ".jig/runtime-version", ".agent/jig-contract.json",
                     "scripts/jig", "scripts/install-jig.sh"]:
            path = self.repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, path)
        self.shims = self.root / "shims"
        self.shims.mkdir()
        self.calls = self.root / "calls"
        self.test_env = dict(self.env, PATH=str(self.shims) + os.pathsep + self.env["PATH"],
                             JIG_TEST_CALLS=str(self.calls))
        for tool in ["cargo", "rustc", "curl"]:
            self.shim(tool, 'printf "%s\\n" "forbidden ' + tool + '" >> "$JIG_TEST_CALLS"\nexit 99\n')

    def shim(self, name, body):
        path = self.shims / name
        path.write_text("#!/bin/sh\n" + body)
        path.chmod(0o755)

    def launch(self, repo=None):
        return subprocess.run(["scripts/jig", "--version"], cwd=repo or self.repo,
                              env=self.test_env, text=True, capture_output=True, timeout=30)

    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.repo, env=self.env, check=True,
                              text=True, capture_output=True, timeout=30)

    def test_executable_only_restore_in_checkout_and_linked_worktree(self):
        self.git("init", "--template=", "-b", "master")
        self.git("add", ".")
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                 "commit", "-m", "baseline")
        linked = self.root / "linked"
        self.git("worktree", "add", "--detach", str(linked))
        for repo, base in [(self.repo, ".git/jig-tools"), (linked, ".agent/.cache/jig")]:
            with self.subTest(layout=base):
                binary = repo / base / f"release-{VERSION}-contract-8-runtime/bin/jig"
                binary.parent.mkdir(parents=True)
                shutil.copy2(self.runtime, binary)
                before = binary.stat()
                result = self.launch(repo)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(result.stdout.strip(), f"jig {VERSION}")
                self.assertEqual(binary.stat().st_ino, before.st_ino)
                self.assertEqual(binary.stat().st_mtime_ns, before.st_mtime_ns)
                self.assertFalse(self.calls.exists(), "Cache reuse invoked a download or compiler")
                self.assertFalse((repo / ".agent/state").exists())

    def test_checksum_mismatch_does_not_build_or_publish(self):
        # Real HTTPS URLs reach this transport fixture; its archive has a wrong checksum.
        self.shim("curl", '''output=""
for arg in "$@"; do
  if [ "$previous" = --output ]; then output="$arg"; fi
  previous="$arg"
done
printf '%s\\n' "$arg" >> "$JIG_TEST_CALLS"
case "$output" in
  *.sha256) archive="${output%.sha256}"; archive="${archive##*/}"
    printf '%064d  %s\\n' 0 "$archive" > "$output" ;;
  *) printf 'corrupt archive' > "$output" ;;
esac
printf '200'
''')
        result = self.launch()
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("SHA-256 mismatch", result.stderr)
        calls = self.calls.read_text()
        self.assertIn(f"/releases/download/v{VERSION}/jig-{VERSION}-", calls)
        self.assertIn(".tar.gz.sha256", calls)
        self.assertNotIn("forbidden", calls)
        self.assertEqual(list(self.repo.glob(".agent/.cache/jig/**/bin/jig")), [])

    def test_download_error_does_not_build(self):
        result = self.launch()
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("Jig release download failed", result.stderr)
        self.assertEqual(self.calls.read_text().strip(), "forbidden curl")

    def test_malformed_pin_fails_before_download_or_build(self):
        (self.repo / ".jig/runtime-version").write_text("latest\n")
        result = self.launch()
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("Invalid .jig/runtime-version", result.stderr)
        self.assertFalse(self.calls.exists())


if __name__ == "__main__":
    unittest.main()
