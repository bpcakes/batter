"""Copy explicit workspace patches into independent consumer roots.

Cargo does not inherit dependency-root patches. Path values must remain tied to
the original root, while git revisions keep their exact declared identity.
"""
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parent.parent

# These repository-controlled manifests use one-line inline tables with double-
# quoted keys/strings. This is deliberately a fail-closed reader for that schema,
# not a TOML parser. Keep the verification entrypoint's Python 3.9 minimum.
STRING = r'"(?:[^"\\]|\\.)*"'


def table_body(text, header):
    match = re.search(r"(?m)^" + re.escape(header) + r"\s*$", text)
    if not match:
        raise RuntimeError("missing consumer dependency table " + header)
    rest = text[match.end():]
    next_table = re.search(r"(?m)^\[", rest)
    return rest[:next_table.start()] if next_table else rest


def string_field(body, name):
    match = re.search(r"\b" + re.escape(name) + r"\s*=\s*(" + STRING + r")", body)
    if not match:
        raise RuntimeError("expected double-quoted dependency field " + name)
    return json.loads(match.group(1))


def consumer_patches(needed=None):
    text = (ROOT / "Cargo.toml").read_text()
    lines = []
    headers = re.findall(r"(?m)^\[patch\.(" + STRING + r")\]\s*$", text)
    if len(headers) != len(re.findall(r"(?m)^\[patch\.", text)):
        raise RuntimeError("consumer patches require double-quoted source tables")
    for quoted_source in headers:
        source = json.loads(quoted_source)
        entries = []
        body = table_body(text, "[patch." + quoted_source + "]")
        for line in body.splitlines():
            if not line.strip() or line.lstrip().startswith("#"):
                continue
            entry = re.fullmatch(r"\s*([\w-]+)\s*=\s*\{(.*)\}\s*", line)
            if not entry:
                raise RuntimeError("consumer patches require one-line inline tables")
            name, fields_text = entry.groups()
            if needed is not None and name not in needed:
                continue
            fields = re.findall(r"([\w-]+)\s*=\s*(" + STRING + r")", fields_text)
            remainder = re.sub(r"[\w-]+\s*=\s*" + STRING, "", fields_text)
            if remainder.replace(",", "").strip() or not fields:
                raise RuntimeError("consumer patch fields must be double-quoted strings")
            settings = {key: json.loads(value) for key, value in fields}
            if "path" in settings:
                settings["path"] = str((ROOT / settings["path"]).resolve())
            fields = ", ".join(key + " = " + json.dumps(value)
                               for key, value in settings.items())
            entries.append(name + " = { " + fields + " }")
        if entries:
            lines.append("\n[patch." + json.dumps(source) + "]")
            lines.extend(entries)
    return "\n".join(lines) + "\n"


def git_dependency(name, manifest_path, section):
    body = table_body((ROOT / manifest_path).read_text(), "[" + ".".join(section) + "]")
    entry = re.search(r"(?m)^" + re.escape(name) + r"\s*=\s*\{([^\n]*)\}\s*$", body)
    if not entry:
        raise RuntimeError("expected one-line pinned git dependency " + name)
    return name + " = { git = " + json.dumps(string_field(entry.group(1), "git")) + ", rev = " + json.dumps(string_field(entry.group(1), "rev")) + " }"
