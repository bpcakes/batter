#!/usr/bin/env python3
"""Enforce local native Runlimit identities, ownership and immutable import assets."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess

from package import EXCLUDED_PARTS, source_files

ROOT = Path(__file__).resolve().parent.parent
NATIVE = ('core', 'memory', 'postgres', 'http', 'axum')
EXPECTED_PUBLISH = ['crates-io']
EXPECTED_VERSION = '0.4.0'
# Imported statements that no code path executes. They stay byte-identical
# outside every package, so they are neither published nor compiled.
PROVENANCE = 'provenance'
# A literal include in any delimiter, as a normal or raw string.
INCLUDE = re.compile(r'include_(?:str|bytes)!\s*[(\[{]\s*'
                     r'(?:r(?P<hashes>#*)"(?P<raw>.*?)"(?P=hashes)|"(?P<plain>[^"\\]*)")')


def validate_graph(metadata, root):
    root = Path(root).resolve()
    packages = metadata['packages']
    selected = {}
    for suffix in NATIVE:
        name = 'runlimit-' + suffix
        matches = [p for p in packages if p['name'] == name]
        if len(matches) != 1:
            raise ValueError('expected exactly one native identity: ' + name)
        package = matches[0]
        if (package.get('source') is not None or Path(package['manifest_path']).resolve()
                != root / 'runlimit' / name / 'Cargo.toml'):
            raise ValueError('native package must resolve locally: ' + name)
        if package['id'] not in metadata['workspace_members']:
            raise ValueError('native package must be a workspace member: ' + name)
        if package.get('publish') != EXPECTED_PUBLISH:
            raise ValueError('native package must target crates.io publication: ' + name)
        if package['version'] != EXPECTED_VERSION:
            raise ValueError('native package has an unexpected release version: ' + name)
        selected[name] = package['id']
    nodes = {n['id']: n for n in metadata['resolve']['nodes']}
    names = {p['id']: p['name'] for p in packages}

    def closure(start):
        seen, pending = set(), [start]
        while pending:
            current = pending.pop()
            if current in seen:
                continue
            seen.add(current)
            pending.extend(d['pkg'] for d in nodes[current]['deps']
                           if any(k['kind'] is None for k in d['dep_kinds']))
        return {names[p] for p in seen}

    for name, ident in selected.items():
        if any(n == 'batter' or n.startswith('batter-') for n in closure(ident)):
            raise ValueError('native Runlimit cannot depend on Batter: ' + name)
    core = closure(selected['runlimit-core'])
    if core & {'tokio', 'axum', 'tower', 'http', 'sqlx'}:
        raise ValueError('native core must be runtime, transport and database independent')
    for package in packages:
        if package['name'] == 'batter-core' and any(n.startswith('runlimit-') for n in closure(package['id'])):
            raise ValueError('foundation cannot depend on native Runlimit')


def validate_assets(root):
    native = Path(root) / 'runlimit'
    expected = json.loads((native / 'upstream-assets.json').read_text())
    if not expected or not {'LICENSE-MIT', 'LICENSE-APACHE'} <= set(expected):
        raise ValueError('import inventory must retain both licenses and SQL assets')
    for name, digest in expected.items():
        path = native / name
        if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != digest:
            raise ValueError('immutable imported asset changed or missing: ' + name)
    validate_provenance(Path(root), expected)


def provenance_records(root, expected):
    """Return `{resolved path: inventory name}` for the unexecuted import records."""
    native = Path(root) / 'runlimit'
    provenance = native / PROVENANCE
    records = {}
    for path in sorted(provenance.rglob('*')) if provenance.is_dir() else ():
        name = path.relative_to(native).as_posix()
        if path.is_symlink() or not path.is_file() or path == provenance / 'README.md':
            continue
        if name not in expected:
            raise ValueError('provenance holds an uninventoried file: ' + name)
        records[path.resolve()] = name
    return records


def validate_provenance(root, expected):
    """Reject accidental reuse of an unexecuted import record.

    This is a tripwire for copying and embedding, not a Rust parser. Outside
    `provenance/` it rejects a `.sql` file with a record's bytes, a symlink
    that resolves to a record, and a Rust source that contains a record's path
    or whose literal `include_str!`/`include_bytes!` path resolves to one.
    `validate_packaged` checks the published file lists themselves.
    """
    root = Path(root)
    records = provenance_records(root, expected)
    provenance = (root / 'runlimit' / PROVENANCE).resolve()
    by_digest = {expected[name]: name for name in records.values()}
    for directory, names, files in os.walk(root):
        names[:] = [n for n in names if n not in EXCLUDED_PARTS]
        for entry in names + files:
            path = Path(directory) / entry
            relative = path.relative_to(root).as_posix()
            if path.is_symlink():
                target = path.resolve()
                if target == provenance or provenance in target.parents:
                    raise ValueError(f'{relative} links into provenance/')
    for path in source_files(root):
        if provenance in path.resolve().parents:
            continue
        relative = path.relative_to(root).as_posix()
        if path.suffix == '.sql':
            record = by_digest.get(hashlib.sha256(path.read_bytes()).hexdigest())
            if record is not None:
                raise ValueError(f'provenance record {record} is copied to {relative}')
        elif path.suffix == '.rs':
            text = path.read_text()
            for name in records.values():
                if name in text:
                    raise ValueError(f'{relative} names provenance record {name}; refer '
                                     'to a record by file name, never by path')
            for match in INCLUDE.finditer(text):
                target = (path.parent / (match.group('raw') or match.group('plain') or '')).resolve()
                if target in records:
                    raise ValueError(f'{relative} includes provenance record {records[target]}')


def validate_packaged(root, expected, packages):
    """Reject a published native package whose file list holds a record."""
    by_digest = {expected[name]: name
                 for name in provenance_records(root, expected).values()}
    for package, directory, listed in packages:
        for name in listed:
            path = Path(directory) / name
            if not path.is_file():
                continue
            record = by_digest.get(hashlib.sha256(path.read_bytes()).hexdigest())
            if record is not None:
                raise ValueError(f'{package} would publish provenance record {record} as {name}')


def package_listings(root):
    """Yield `(package, directory, files)` from Cargo's own package file lists."""
    for suffix in NATIVE:
        name = 'runlimit-' + suffix
        listed = subprocess.check_output(
            ['cargo', 'package', '-p', name, '--list', '--allow-dirty', '--locked'],
            cwd=root, text=True).split()
        yield name, Path(root) / 'runlimit' / name, listed


if __name__ == '__main__':
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--format-version', '1', '--all-features', '--locked'], cwd=ROOT))
    validate_graph(metadata, ROOT)
    validate_assets(ROOT)
    validate_packaged(ROOT, json.loads((ROOT / 'runlimit' / 'upstream-assets.json').read_text()),
                      package_listings(ROOT))
    print('Runlimit: five publishable local identities, native ownership and imported SQL/licenses verified')
