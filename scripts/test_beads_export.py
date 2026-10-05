"""Negative controls for colliding identifiers in the Beads export."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from check_beads_export import identifier_problems

SCRIPT = Path(__file__).resolve().parent / "check_beads_export.py"


def row(issue, *comment_ids):
    comments = [dict(id=cid, issue_id=issue, author="agent", text=f"note {cid}",
                     created_at="2026-10-05T09:00:00Z") for cid in comment_ids]
    return json.dumps(dict(id=issue, title=issue, status="open", comments=comments or None))


class IdentifierTests(unittest.TestCase):
    def test_unique_issue_and_comment_ids_pass(self):
        self.assertEqual(identifier_problems([row("demo-a", 1, 2), "", row("demo-b", 3), row("demo-c")]), [])

    def test_comment_ids_from_two_tracker_databases_are_rejected(self):
        # Two worktree databases each numbered new comments from the same base;
        # Git merged both rows cleanly, but br cannot import the result.
        problems = identifier_problems([row("demo-worktree", 404, 405, 406),
                                        row("demo-checkout", 404, 405, 406, 407)])
        self.assertEqual(problems, [f"comment ID {cid} is used by demo-worktree, demo-checkout"
                                    for cid in (404, 405, 406)])

    def test_comment_id_repeated_within_one_issue_is_rejected(self):
        self.assertEqual(identifier_problems([row("demo-a", 7, 7)]),
                         ["comment ID 7 is used by demo-a, demo-a"])

    def test_duplicate_issue_rows_are_rejected(self):
        self.assertEqual(identifier_problems([row("demo-a"), row("demo-b"), row("demo-a")]),
                         ["issue demo-a appears on lines 1, 3"])

    def test_malformed_rows_are_reported_by_line(self):
        problems = identifier_problems([
            "{not json",
            json.dumps(dict(title="missing id")),
            json.dumps(dict(id="demo-a", comments=[dict(id="7")])),
            json.dumps(dict(id="demo-b", comments=[dict(id=True)])),
            json.dumps(dict(id="demo-c", comments={"id": 1})),
        ])
        self.assertEqual(len(problems), 5)
        self.assertTrue(problems[0].startswith("line 1: invalid JSON"))
        self.assertEqual(problems[1:], [
            "line 2: row has no issue ID",
            "line 3: demo-a has a comment without an integer ID",
            "line 4: demo-b has a comment without an integer ID",
            "line 5: demo-c has a non-list comments field",
        ])


class CommandTests(unittest.TestCase):
    def run_check(self, *lines):
        with tempfile.TemporaryDirectory(prefix="beads-export-") as directory:
            export = Path(directory) / "issues.jsonl"
            export.write_text("".join(line + "\n" for line in lines))
            return subprocess.run([sys.executable, str(SCRIPT), str(export)],
                                  capture_output=True, text=True, timeout=30)

    def test_clean_export_exits_zero(self):
        result = self.run_check(row("demo-a", 1), row("demo-b", 2))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("unique (2 issues)", result.stdout)

    def test_collision_exits_nonzero_with_owners_and_remedy(self):
        result = self.run_check(row("demo-a", 1), row("demo-b", 1))
        self.assertEqual(result.returncode, 1)
        self.assertIn("comment ID 1 is used by demo-a, demo-b", result.stderr)
        self.assertIn("br sync --import-only", result.stderr)

    def test_missing_export_fails(self):
        result = subprocess.run([sys.executable, str(SCRIPT), "/nonexistent/issues.jsonl"],
                                capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 1)
        self.assertIn("cannot read", result.stderr)


if __name__ == "__main__":
    unittest.main()
