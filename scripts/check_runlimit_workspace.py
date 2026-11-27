#!/usr/bin/env python3
"""Enforce local native Runlimit identities, ownership and immutable import assets."""
import hashlib
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile

from package import eligible

ROOT = Path(__file__).resolve().parent.parent
NATIVE = ('core', 'memory', 'postgres', 'http', 'axum')
EXPECTED_PUBLISH = ['crates-io']
EXPECTED_VERSION = '0.4.0'
# Imported statements that no code path executes. They stay byte-identical
# outside every package; `validate_published` checks the archives Cargo builds.
PROVENANCE = 'provenance'


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


def validate_provenance(root, expected):
    """Require every unexecuted import record to be an inventoried regular file.

    Files that are not source material by `package.eligible`, such as Finder
    metadata, are ignored; a symlink anywhere in the directory is rejected so
    each record's bytes really live outside every package.
    """
    native = Path(root) / 'runlimit'
    provenance = native / PROVENANCE
    if not provenance.is_dir():
        return
    for path in sorted(provenance.rglob('*')):
        name = path.relative_to(native).as_posix()
        if path.is_symlink():
            raise ValueError('provenance holds a symlink: ' + name)
        if path == provenance / 'README.md' or not eligible(path, Path(root)):
            continue
        if name not in expected:
            raise ValueError('provenance holds an uninventoried file: ' + name)


def record_digests(expected):
    """Return `{digest: inventory name}` for the unexecuted import records."""
    prefix = PROVENANCE + '/'
    return {digest: name for name, digest in expected.items() if name.startswith(prefix)}


def validate_published(archives, expected):
    """Reject a native package archive that would publish a record's bytes.

    The archive is what crates.io receives, so this covers every way a file
    can reach it: a copy under any name, a symlink, or an entry Cargo takes
    from outside the crate directory.
    """
    records = record_digests(expected)
    for archive in archives:
        with tarfile.open(archive, 'r:gz') as bundle:
            for member in bundle.getmembers():
                if not member.isfile():
                    continue
                digest = hashlib.sha256(bundle.extractfile(member).read()).hexdigest()
                if digest in records:
                    raise ValueError(f'{Path(archive).name} would publish provenance record '
                                     f'{records[digest]} as {member.name}')


def package_archives(root, directory):
    """Package every native crate into `directory` without building it."""
    command = ['cargo', 'package', '--no-verify', '--allow-dirty', '--locked',
               '--target-dir', str(directory)]
    for suffix in NATIVE:
        command += ['-p', 'runlimit-' + suffix]
    subprocess.run(command, cwd=root, check=True, stdout=subprocess.DEVNULL,
                   stderr=subprocess.DEVNULL)
    return sorted((Path(directory) / 'package').glob('*.crate'))


if __name__ == '__main__':
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--format-version', '1', '--all-features', '--locked'], cwd=ROOT))
    validate_graph(metadata, ROOT)
    validate_assets(ROOT)
    with tempfile.TemporaryDirectory() as scratch:
        validate_published(package_archives(ROOT, scratch),
                           json.loads((ROOT / 'runlimit' / 'upstream-assets.json').read_text()))
    print('Runlimit: five publishable local identities, native ownership and imported SQL/licenses verified')
