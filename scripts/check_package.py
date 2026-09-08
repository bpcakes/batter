#!/usr/bin/env python3
"""Static package checks. This is NOT a Rust parser, compiler, or test runner."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import tomllib

EXCLUDED = {"target", ".git", "__pycache__"}


def files(root: Path, suffix: str) -> list[Path]:
    return sorted(p for p in root.rglob(f"*{suffix}") if not EXCLUDED.intersection(p.relative_to(root).parts))


def rust_delimiters(text: str) -> str | None:
    """Balance punctuation while skipping comments/literals, not Rust grammar."""
    stack: list[tuple[str, int]] = []
    pairs = {")": "(", "]": "[", "}": "{"}
    i = 0
    while i < len(text):
        if text.startswith("//", i):
            end = text.find("\n", i)
            i = len(text) if end < 0 else end + 1
            continue
        if text.startswith("/*", i):
            depth = 1
            i += 2
            while i < len(text) and depth:
                if text.startswith("/*", i):
                    depth += 1
                    i += 2
                elif text.startswith("*/", i):
                    depth -= 1
                    i += 2
                else:
                    i += 1
            if depth:
                return "unterminated block comment"
            continue
        raw = re.match(r'(?:br|r)(#*)"', text[i:])
        if raw:
            end_marker = '"' + raw.group(1)
            end = text.find(end_marker, i + raw.end())
            if end < 0:
                return "unterminated raw string"
            i = end + len(end_marker)
            continue
        if text[i] == '"':
            i += 1
            closed = False
            while i < len(text):
                if text[i] == "\\":
                    i += 2
                elif text[i] == '"':
                    i += 1
                    closed = True
                    break
                else:
                    i += 1
            if not closed:
                return "unterminated string"
            continue
        # A lifetime such as 'static is NOT a character literal.
        character = re.match(r"'(?:\\(?:u\{[0-9A-Fa-f_]+\}|x[0-9A-Fa-f]{2}|.)|[^'\\\n])'", text[i:])
        if character:
            i += character.end()
            continue
        value = text[i]
        if value in "([{":
            stack.append((value, text.count("\n", 0, i) + 1))
        elif value in pairs:
            if not stack or stack[-1][0] != pairs[value]:
                return f"unmatched {value!r} at line {text.count(chr(10), 0, i) + 1}"
            stack.pop()
        i += 1
    return None if not stack else f"unclosed delimiter {stack[-1]}"


def check(root: Path) -> dict[str, object]:
    failures: list[str] = []
    manifests = files(root, ".toml")
    for path in manifests:
        try:
            tomllib.loads(path.read_text())
        except (tomllib.TOMLDecodeError, UnicodeError) as error:
            failures.append(f"{path.relative_to(root)}: {error}")
    rust = files(root, ".rs")
    for path in rust:
        problem = rust_delimiters(path.read_text())
        if problem:
            failures.append(f"{path.relative_to(root)}: {problem}")
    link_count = 0
    markdown = files(root, ".md")
    for path in markdown:
        for match in re.finditer(r"(?<!!)\[[^\]]+\]\(([^)]+)\)", path.read_text()):
            target = match.group(1).split("#", 1)[0]
            if not target or re.match(r"[a-zA-Z][a-zA-Z0-9+.-]*:", target):
                continue
            link_count += 1
            if not (path.parent / target).exists():
                failures.append(f"{path.relative_to(root)}: missing link target {target}")
    count = sum(len(re.findall(r"#\[(?:tokio::)?test(?:\([^]]*\))?\]", path.read_text())) for path in rust)
    return {
        "kind": "static-package-inspection-not-rust-validation",
        "snapshot_date": "2026-09-07",
        "rust_files": len(rust),
        "rust_lines": sum(len(path.read_text().splitlines()) for path in rust),
        "authored_test_cases": count,
        "parsed_toml_files": len(manifests),
        "markdown_files": len(markdown),
        "checked_internal_file_links": link_count,
        "rust_delimiter_check": "lexical-only-not-grammar-or-types",
        "cargo_or_rustc_executed": False,
        "rust_tests_executed": False,
        "failures": failures,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    result = check(args.root.resolve())
    text = json.dumps(result, indent=2) + "\n"
    print(text, end="")
    if args.output:
        args.output.write_text(text)
    return 1 if result["failures"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
