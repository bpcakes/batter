#!/usr/bin/env python3
"""Enforce local native Runlimit identities, ownership and immutable import assets."""
import hashlib
import json
from pathlib import Path, PurePosixPath
import posixpath
import re
import subprocess
import tarfile

from package import eligible

ROOT = Path(__file__).resolve().parent.parent
NATIVE = ('core', 'memory', 'postgres', 'http', 'axum')
EXPECTED_PUBLISH = ['crates-io']
EXPECTED_VERSION = '0.4.0'
# Imported statements that no code path executes. They stay byte-identical
# outside every package; `package_archives` and `validate_published` check the
# archives Cargo builds and verifies.
PROVENANCE = 'provenance'
# Digests of the superseded imported statements, which must stay inventoried
# under provenance/, whatever path the inventory gives them.
UNEXECUTED_RECORDS = {
    '29c13de727bea2b67dec3a6cdaf7bfb317cd13f849ee4396fec293b55597d667': 'attempt_complete.sql',
    'e9b9bc4cacae015fd06f14d8e6eeb6b0e10e77f17344d52598a5828598a4a49c': 'cleanup_expired.sql',
}
# A literal include in any delimiter, as a plain string without escapes or a
# raw string.
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
    """Require every unexecuted import record to be an inventoried regular file.

    Files that are not source material by `package.eligible`, such as Finder
    metadata, are ignored; a symlink anywhere in the directory is rejected so
    each record's bytes really live outside every package.
    """
    prefix = PROVENANCE + '/'
    for digest, record in (UNEXECUTED_RECORDS if required is None else required).items():
        names = [name for name, value in expected.items() if value == digest]
        if not names or any(not name.startswith(prefix) for name in names):
            raise ValueError(f'superseded record {record} must be inventoried only under '
                             f'{prefix}, not {names}')
    native = Path(root) / 'runlimit'
    provenance = native / PROVENANCE
    if provenance.is_symlink():
        raise ValueError('provenance is a symlink: ' + PROVENANCE)
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
    """Return `{digest: name}` for every unexecuted import record."""
    prefix = PROVENANCE + '/'
    records = dict(UNEXECUTED_RECORDS)
    records.update({digest: name for name, digest in expected.items() if name.startswith(prefix)})
    return records


def validate_published(archives, expected):
    """Reject a native package archive that publishes or depends on a record.

    The archive is what crates.io receives. Any member, under any name and
    however Cargo sourced it, that carries a record's bytes is rejected. Every
    literal `include_str!`/`include_bytes!` in any of its Rust files, tests
    and examples included, must name a file inside the archive. Cargo's
    verification additionally builds each library from its archive; an
    include path assembled by another macro outside a library target is not
    checked.
    """
    records = record_digests(expected)
    for archive in archives:
        with tarfile.open(archive, 'r:gz') as bundle:
            members = {member.name: member for member in bundle.getmembers() if member.isfile()}
            for name, member in members.items():
                data = bundle.extractfile(member).read()
                digest = hashlib.sha256(data).hexdigest()
                if digest in records:
                    raise ValueError(f'{Path(archive).name} would publish provenance record '
                                     f'{records[digest]} as {name}')
                if not name.endswith('.rs'):
                    continue
                for match in INCLUDE.finditer(data.decode('utf-8', 'replace')):
                    literal = match.group('raw') if match.group('raw') is not None \
                        else match.group('plain')
                    target = posixpath.normpath(str(PurePosixPath(name).parent / literal))
                    if target not in members:
                        raise ValueError(f'{Path(archive).name}: {name} includes {literal}, '
                                         'which is not in the archive')


def package_archives(root, target_directory):
    """Package and verify every native crate, returning the archives Cargo built.

    Cargo's verification builds each crate, with all features, from its
    extracted archive, where `provenance/` and every other path outside the
    crate are absent, so a crate that embeds a record fails here.
    """
    command = ['cargo', 'package', '--allow-dirty', '--locked', '--all-features']
    for suffix in NATIVE:
        command += ['-p', 'runlimit-' + suffix]
    result = subprocess.run(command, cwd=root, capture_output=True, text=True)
    if result.returncode != 0:
        raise ValueError('native packaging or verification failed:\n' + result.stderr[-4000:])
    return [Path(target_directory) / 'package' / f'runlimit-{suffix}-{EXPECTED_VERSION}.crate'
            for suffix in NATIVE]


if __name__ == '__main__':
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--format-version', '1', '--all-features', '--locked'], cwd=ROOT))
    validate_graph(metadata, ROOT)
    validate_assets(ROOT)
    validate_published(package_archives(ROOT, metadata['target_directory']),
                       json.loads((ROOT / 'runlimit' / 'upstream-assets.json').read_text()))
    print('Runlimit: five publishable local identities, native ownership and imported SQL/licenses verified')
