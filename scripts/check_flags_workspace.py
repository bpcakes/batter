#!/usr/bin/env python3
"""Enforce the flags packages' identities, dependency direction and crate roots.

ADR-016 places four packages under `crates/`: three libraries on Batter's
release line and the operator command, which stays unpublished until its
release-train record. The rules, each with a negative control in
`test_flags_workspace.py`:

1. Each package resolves locally as one workspace member at version 0.0.1,
   `rust-version` 1.98.1 and license MIT; the libraries target crates.io and
   the command is `publish = false`.
2. `batter-flags-core` depends on no Batter, Runledger or Runlimit package, for
   any dependency kind.
3. `batter-flags-core`'s normal closure without default features holds no async
   runtime, transport, database, subscriber, argument parser or serialization
   library, and its manifest declares `serde` and `serde_json` optional.
4. No closure of `batter-core`, `batter-axum` or `batter-runledger` holds a
   flags package.
5. Only the flags packages themselves and the facade's optional `batter-flags`
   edge depend on a flags package.
6. `batter-flags`' closure never holds the facade; its Batter edges are exactly
   the normal ones below and the two dev-only test packages.
7. Every flags package inherits the workspace lints, and the command's own
   `sqlx` entry selects a rustls TLS feature, because feature unification
   elsewhere would hide its absence.
8. Every flags crate root forbids unsafe code.
"""
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parent.parent
LIBRARIES = ('batter-flags-core', 'batter-flags-postgres', 'batter-flags')
COMMAND = 'batter-flags-cli'
PACKAGES = LIBRARIES + (COMMAND,)
VERSION = '0.0.1'
RUST_VERSION = '1.98.1'
LICENSE = 'MIT'
CORE_EXCLUDED = {'tokio', 'tokio-util', 'sqlx', 'axum', 'tower', 'http', 'hyper', 'reqwest',
                 'tracing-subscriber', 'clap', 'serde', 'serde_json'}
CORE_OPTIONAL = ('serde', 'serde_json')
FOUNDATIONS = ('batter-core', 'batter-axum', 'batter-runledger')
# Each flags package may depend on these flags packages, and nothing else may.
FLAGS_EDGES = {
    'batter-flags-core': set(),
    'batter-flags-postgres': {'batter-flags-core'},
    'batter-flags': {'batter-flags-core', 'batter-flags-postgres'},
    'batter-flags-cli': {'batter-flags-core', 'batter-flags-postgres'},
    'batter': {'batter-flags'},
}
ADAPTER_NORMAL = {'batter-core', 'batter-flags-core', 'batter-flags-postgres', 'batter-sqlx',
                  'batter-axum', 'runledger-core'}
ADAPTER_DEV = {'batter-runledger', 'batter-test-support'}
ROOTS = {'batter-flags-core': 'src/lib.rs', 'batter-flags-postgres': 'src/lib.rs',
         'batter-flags': 'src/lib.rs', 'batter-flags-cli': 'src/main.rs'}


def is_batter_family(name):
    """Whether a package is Batter's, or a native Runledger or Runlimit one."""
    return (name == 'batter' or name.startswith(('batter-', 'runledger-', 'runlimit-'))
            or name == 'runledger')


def validate_identities(metadata, root):
    """Rule 1."""
    root = Path(root).resolve()
    for name in PACKAGES:
        matches = [p for p in metadata['packages'] if p['name'] == name]
        if len(matches) != 1:
            raise ValueError('expected exactly one flags identity: ' + name)
        package = matches[0]
        if (package.get('source') is not None or Path(package['manifest_path']).resolve()
                != root / 'crates' / name / 'Cargo.toml'):
            raise ValueError('flags package must resolve locally: ' + name)
        if package['id'] not in metadata['workspace_members']:
            raise ValueError('flags package must be a workspace member: ' + name)
        if package['version'] != VERSION:
            raise ValueError('flags package is off the Batter release line: ' + name)
        if package.get('rust_version') != RUST_VERSION:
            raise ValueError('flags package must declare rust-version ' + RUST_VERSION + ': ' + name)
        if package.get('license') != LICENSE:
            raise ValueError('flags package must declare license MIT: ' + name)
        expected = [] if name == COMMAND else ['crates-io']
        if package.get('publish') != expected:
            raise ValueError('flags package has the wrong publish setting: ' + name)


def _graph(metadata):
    nodes = {n['id']: n for n in metadata['resolve']['nodes']}
    names = {p['id']: p['name'] for p in metadata['packages']}
    ids = {p['name']: p['id'] for p in metadata['packages']
           if p['id'] in metadata['workspace_members']}
    return nodes, names, ids


def _closure(nodes, names, start):
    """Every package one build of `start` can hold: its direct edges of any
    kind, then their normal and build edges."""
    seen, pending = set(), [d['pkg'] for d in nodes[start]['deps']]
    while pending:
        current = pending.pop()
        if current in seen:
            continue
        seen.add(current)
        pending.extend(d['pkg'] for d in nodes[current]['deps']
                       if any(k['kind'] in (None, 'build') for k in d['dep_kinds']))
    return {names[p] for p in seen}


def _direct(nodes, names, start):
    """Each direct dependency of `start` with the kinds it is declared as."""
    return {names[d['pkg']]: {k['kind'] for k in d['dep_kinds']} for d in nodes[start]['deps']}


def validate_graph(metadata):
    """Rules 2, 4, 5 and 6."""
    nodes, names, ids = _graph(metadata)
    core = _closure(nodes, names, ids['batter-flags-core'])
    family = sorted(n for n in core if is_batter_family(n))
    if family:
        raise ValueError('batter-flags-core must depend on no Batter package: ' + ', '.join(family))
    for name in FOUNDATIONS:
        held = sorted(_closure(nodes, names, ids[name]) & set(PACKAGES))
        if held:
            raise ValueError(name + ' must not depend on a flags package: ' + ', '.join(held))
    for name, ident in ids.items():
        edges = set(_direct(nodes, names, ident)) & set(PACKAGES)
        stray = sorted(edges - FLAGS_EDGES.get(name, set()))
        if stray:
            raise ValueError(name + ' may not depend on ' + ', '.join(stray))
    adapter = ids['batter-flags']
    if 'batter' in _closure(nodes, names, adapter):
        raise ValueError('batter-flags must never depend on the facade')
    direct = {n: kinds for n, kinds in _direct(nodes, names, adapter).items()
              if is_batter_family(n)}
    normal = {n for n, kinds in direct.items() if kinds & {None, 'build'}}
    dev = {n for n, kinds in direct.items() if kinds == {'dev'}}
    if normal != ADAPTER_NORMAL:
        raise ValueError('batter-flags has unexpected normal Batter edges: '
                         + ', '.join(sorted(normal ^ ADAPTER_NORMAL)))
    if dev != ADAPTER_DEV:
        raise ValueError('batter-flags has unexpected dev-only Batter edges: '
                         + ', '.join(sorted(dev ^ ADAPTER_DEV)))


def validate_core_bare(closure_names):
    """Rule 3, the closure: names from `cargo tree -p batter-flags-core
    --no-default-features -e normal`."""
    held = sorted(set(closure_names) & CORE_EXCLUDED)
    if held:
        raise ValueError('bare batter-flags-core holds ' + ', '.join(held))


def _dependency_line(manifest, name):
    """`name`'s entry in the manifest's `[dependencies]` table, if any."""
    table = re.search(r'^\[dependencies\]\s*\n(.*?)(?=^\[|\Z)', manifest, re.MULTILINE | re.DOTALL)
    if table is None:
        return None
    match = re.search(r'^' + re.escape(name) + r'\s*=\s*(.+)$', table.group(1), re.MULTILINE)
    return match.group(1) if match else None


def validate_core_manifest(manifest):
    """Rule 3, the manifest."""
    for name in CORE_OPTIONAL:
        line = _dependency_line(manifest, name)
        if line is None or not re.search(r'optional\s*=\s*true', line):
            raise ValueError('batter-flags-core must declare ' + name + ' optional')


def validate_manifests(manifests):
    """Rule 7, over each package's manifest text."""
    for name, manifest in manifests.items():
        if not re.search(r'^\[lints\]\s*\nworkspace\s*=\s*true', manifest, re.MULTILINE):
            raise ValueError(name + ' must inherit the workspace lints')
    sqlx = _dependency_line(manifests[COMMAND], 'sqlx')
    if sqlx is None or not re.search(r'"tls-rustls[a-z-]*"', sqlx):
        raise ValueError(COMMAND + "'s own sqlx entry must select a rustls TLS feature")


def validate_roots(sources):
    """Rule 8, over each package's crate root text."""
    for name, source in sources.items():
        if not re.search(r'^#!\[forbid\(unsafe_code\)\]', source, re.MULTILINE):
            raise ValueError(name + "'s crate root must forbid unsafe code")


def main():
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--format-version', '1', '--all-features', '--locked'], cwd=ROOT))
    validate_identities(metadata, ROOT)
    validate_graph(metadata)
    tree = subprocess.check_output(
        ['cargo', 'tree', '-p', 'batter-flags-core', '--no-default-features', '-e', 'normal',
         '--prefix', 'none', '--format', '{p}', '--locked'], cwd=ROOT, text=True)
    validate_core_bare(line.split()[0] for line in tree.splitlines() if line.strip())
    manifests = {name: (ROOT / 'crates' / name / 'Cargo.toml').read_text() for name in PACKAGES}
    validate_core_manifest(manifests['batter-flags-core'])
    validate_manifests(manifests)
    validate_roots({name: (ROOT / 'crates' / name / path).read_text()
                    for name, path in ROOTS.items()})
    print('Flags: four local identities, one-way dependency direction, a bare core and '
          'forbidden unsafe code verified')


if __name__ == '__main__':
    main()
