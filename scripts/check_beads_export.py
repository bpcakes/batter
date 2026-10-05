#!/usr/bin/env python3
"""Reject a Beads export whose identifiers collide.

Each checkout and linked worktree keeps its own gitignored tracker database, and
that database assigns comment IDs locally. Git merges two branches' exports line
by line without a conflict, yet one comment ID on two rows makes every
`br sync --import-only` fail semantic verification, even into a fresh database.
Duplicate issue rows break imports in the same way. This check needs only the
committed export, so it runs before any tracker command.
"""

import argparse
from collections import defaultdict
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_EXPORT = ROOT / ".beads" / "issues.jsonl"
REMEDY = (
    "Comment IDs are local to each tracker database and are not cited by number. "
    "Renumber the later duplicates above the export's highest comment ID, then "
    "confirm that `br sync --import-only` succeeds in a fresh copy of `.beads/`."
)


def identifier_problems(lines):
    """Return one message per malformed row or colliding issue/comment ID."""
    problems = []
    issue_lines = defaultdict(list)
    comment_owners = defaultdict(list)
    for number, line in enumerate(lines, start=1):
        if not line.strip():
            continue
        try:
            row = json.loads(line)
        except json.JSONDecodeError as error:
            problems.append(f"line {number}: invalid JSON ({error.msg})")
            continue
        issue = row.get("id") if isinstance(row, dict) else None
        if not isinstance(issue, str) or not issue:
            problems.append(f"line {number}: row has no issue ID")
            continue
        issue_lines[issue].append(number)
        comments = row.get("comments") or []
        if not isinstance(comments, list):
            problems.append(f"line {number}: {issue} has a non-list comments field")
            continue
        for comment in comments:
            comment_id = comment.get("id") if isinstance(comment, dict) else None
            if isinstance(comment_id, bool) or not isinstance(comment_id, int):
                problems.append(f"line {number}: {issue} has a comment without an integer ID")
                continue
            comment_owners[comment_id].append(issue)
    for issue, numbers in sorted(issue_lines.items()):
        if len(numbers) > 1:
            problems.append(f"issue {issue} appears on lines {', '.join(map(str, numbers))}")
    for comment_id, owners in sorted(comment_owners.items()):
        if len(owners) > 1:
            problems.append(f"comment ID {comment_id} is used by {', '.join(owners)}")
    return problems


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("export", nargs="?", type=Path, default=DEFAULT_EXPORT,
                        help="Beads JSONL export (default: .beads/issues.jsonl)")
    export = parser.parse_args(argv).export
    try:
        lines = export.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        print(f"Beads export {export}: cannot read ({error.strerror})", file=sys.stderr)
        return 1
    problems = identifier_problems(lines)
    if problems:
        print(f"Beads export {export}: {len(problems)} identifier problem(s)", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        print(REMEDY, file=sys.stderr)
        return 1
    print(f"Beads export identifiers are unique ({sum(1 for line in lines if line.strip())} issues)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
