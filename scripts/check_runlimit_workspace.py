#!/usr/bin/env python3
"""Enforce local native Runlimit identities, ownership and immutable import assets."""
import hashlib
import json
from pathlib import Path
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parent.parent
NATIVE = ('core', 'memory', 'postgres', 'http', 'axum')
EXPECTED_PUBLISH = ['crates-io']
EXPECTED_VERSION = '0.4.0'
# Imported statements that no code path executes. They stay byte-identical
# outside every package; `package_archives` and `validate_published` check the
# archives Cargo builds and verifies.
PROVENANCE = 'provenance'
# Operating-system metadata that may appear in provenance/ without a digest.
PROVENANCE_METADATA = frozenset({'.DS_Store'})
# Digests of the superseded imported statements, which must stay inventoried
# under provenance/, whatever path the inventory gives them.
UNEXECUTED_RECORDS = {
    '29c13de727bea2b67dec3a6cdaf7bfb317cd13f849ee4396fec293b55597d667': 'attempt_complete.sql',
    'e9b9bc4cacae015fd06f14d8e6eeb6b0e10e77f17344d52598a5828598a4a49c': 'cleanup_expired.sql',
}


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


def validate_assets(root, required=None):
    native = Path(root) / 'runlimit'
    expected = json.loads((native / 'upstream-assets.json').read_text())
    if not expected or not {'LICENSE-MIT', 'LICENSE-APACHE'} <= set(expected):
        raise ValueError('import inventory must retain both licenses and SQL assets')
    for name, digest in expected.items():
        path = native / name
        if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != digest:
            raise ValueError('immutable imported asset changed or missing: ' + name)
    validate_provenance(Path(root), expected, required)


def validate_provenance(root, expected, required=None):
    """Keep every unexecuted import record an inventoried file under provenance/.

    Each required record, found by digest, must be inventoried only under a
    canonical `provenance/` path that resolves inside the directory. Every
    file there except the README and known operating-system metadata must be
    inventoried, whatever its extension, and no symlink may appear in it or
    replace it.
    """
    native = Path(root) / 'runlimit'
    provenance = native / PROVENANCE
    for digest, record in (UNEXECUTED_RECORDS if required is None else required).items():
        names = [name for name, value in expected.items() if value == digest]
        if not names or not all(is_provenance_path(native, name) for name in names):
            raise ValueError(f'superseded record {record} must be inventoried only under '
                             f'{PROVENANCE}/, not {names}')
    if provenance.is_symlink():
        raise ValueError('provenance is a symlink: ' + PROVENANCE)
    if not provenance.is_dir():
        return
    for path in sorted(provenance.rglob('*')):
        name = path.relative_to(native).as_posix()
        if path.is_symlink():
            raise ValueError('provenance holds a symlink: ' + name)
        if path.is_dir() or path == provenance / 'README.md' or path.name in PROVENANCE_METADATA:
            continue
        if name not in expected:
            raise ValueError('provenance holds an uninventoried file: ' + name)


def is_provenance_path(native, name):
    """Whether inventory `name` is a canonical path that resolves in provenance/."""
    parts = name.split('/')
    if len(parts) < 2 or parts[0] != PROVENANCE or any(part in ('', '.', '..') for part in parts):
        return False
    provenance = (native / PROVENANCE).resolve()
    return provenance in (native / name).resolve().parents


def record_contents(root, expected):
    """Return `{inventory name: bytes}` for every record under provenance/."""
    native = Path(root) / 'runlimit'
    prefix = PROVENANCE + '/'
    return {name: (native / name).read_bytes() for name in sorted(expected)
            if name.startswith(prefix)}


def validate_published(archives, records):
    """Reject a native package archive that would publish a record's bytes.

    The archive is what crates.io receives, so any member, under any name and
    however Cargo sourced it, that contains a record's complete bytes, alone
    or inside a larger file, is rejected.
    """
    for archive in archives:
        with tarfile.open(archive, 'r:gz') as bundle:
            for member in bundle.getmembers():
                if not member.isfile():
                    continue
                data = bundle.extractfile(member).read()
                for name, content in records.items():
                    if content in data:
                        raise ValueError(f'{Path(archive).name} would publish provenance '
                                         f'record {name} in {member.name}')


def package_archives(root, target_directory):
    """Package and verify every native crate, returning the archives Cargo built.

    Cargo's verification builds each library from its extracted archive,
    where `provenance/` and every other path outside the crate are absent, so
    a library that embeds a record fails here. It runs once with default
    features and once with all features, so code compiled under either set
    is built. Cargo does not build a package's test, example or bench targets
    from the archive; that pre-existing gap is `batter-88bd` and is not
    specific to records.
    """
    packages = [argument for suffix in NATIVE for argument in ('-p', 'runlimit-' + suffix)]
    for features in ([], ['--all-features']):
        command = ['cargo', 'package', '--allow-dirty', '--locked', *features, *packages]
        result = subprocess.run(command, cwd=root, capture_output=True, text=True)
        if result.returncode != 0:
            raise ValueError(f'native packaging or verification failed ({" ".join(features) or "default features"}):\n'
                             + result.stderr[-4000:])
    return [Path(target_directory) / 'package' / f'runlimit-{suffix}-{EXPECTED_VERSION}.crate'
            for suffix in NATIVE]

if __name__ == '__main__':
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--format-version', '1', '--all-features', '--locked'], cwd=ROOT))
    validate_graph(metadata, ROOT)
    validate_assets(ROOT)
    expected = json.loads((ROOT / 'runlimit' / 'upstream-assets.json').read_text())
    validate_published(package_archives(ROOT, metadata['target_directory']),
                       record_contents(ROOT, expected))
    print('Runlimit: five publishable local identities, native ownership and imported SQL/licenses verified')
