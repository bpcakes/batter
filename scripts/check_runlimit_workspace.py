#!/usr/bin/env python3
"""Enforce local native Runlimit identities, ownership and immutable import assets."""
import hashlib
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parent.parent
NATIVE = ('core', 'memory', 'postgres', 'http', 'axum')
EXPECTED_PUBLISH = ['crates-io']
EXPECTED_VERSION = '0.4.0'
# Imported statements that no code path executes. They stay byte-identical
# outside every package, so they are neither published nor compiled.
PROVENANCE = 'provenance'
EMBEDDED_PROVENANCE = re.compile(r'include_(?:str|bytes)!\s*\(\s*"[^"]*\b' + PROVENANCE + '/')


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
    provenance = native / PROVENANCE
    for path in sorted(provenance.rglob('*')) if provenance.is_dir() else ():
        name = path.relative_to(native).as_posix()
        if path.is_file() and path.name != 'README.md' and name not in expected:
            raise ValueError('provenance holds an uninventoried file: ' + name)
    for path in sorted(native.rglob('*.rs')):
        if provenance in path.parents or 'target' in path.relative_to(native).parts:
            continue
        if EMBEDDED_PROVENANCE.search(path.read_text()):
            raise ValueError('source embeds an unexecuted provenance record: '
                             + path.relative_to(native).as_posix())


if __name__ == '__main__':
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--format-version', '1', '--all-features', '--locked'], cwd=ROOT))
    validate_graph(metadata, ROOT)
    validate_assets(ROOT)
    print('Runlimit: five publishable local identities, native ownership and imported SQL/licenses verified')
