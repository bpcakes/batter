#!/usr/bin/env python3
"""Enforce local native Runlimit identities, ownership and immutable import assets."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import re
import tarfile
import tempfile

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
    '740a2ec53f208cddce451f69508e4c9264b14b74523b5fc11cf522bf7adbf243': 'attempt_cleanup.sql',
}
# runlimit-postgres's SQL gate lints and the Clippy groups that contain them
# (`clippy-driver -Whelp`); see runlimit-postgres/src/catalog_guard/gate.rs.
SQL_GATE_LINTS = frozenset({'all', 'default', 'style', 'disallowed_methods',
                            'disallowed_types', 'disallowed_macros'})
ENABLED_LEVELS = frozenset({'warn', 'deny', 'forbid'})
# Clippy still applies these renamed spellings to the gate lints.
RENAMED_LINTS = {'disallowed_method': 'disallowed_methods', 'disallowed_type': 'disallowed_types'}
# The probe query, inline and in probe.sql, whose offline metadata the macro
# probes expand from.
PROBE_QUERY = 'SELECT 1 AS one'
# A `[lints.rust]` or `[lints.clippy]` header, and one entry inside it.
LINT_TABLE = re.compile(r'^\[lints(?:\.(rust|clippy))?\]\s*(?:#.*)?$')
LINT_ENTRY = re.compile(r'^([A-Za-z0-9_-]+)\s*=\s*(?:"([a-z]+)"|\{(.*)\})\s*(?:#.*)?$')
# One field of an inline lint table: a quoted level, an integer priority, or
# a one-line array of quoted check-cfg strings.
INLINE_FIELD = re.compile(r'\s*(level|priority|check-cfg)\s*=\s*("[a-z]+"|-?\d+|'
                          r'\[\s*(?:(?:"[^"\\]*"|\'[^\']*\')\s*,?\s*)*\])\s*(?:,|$)')
# The query macros Clippy reports outside any lint scope; see
# runlimit-postgres/src/catalog_guard/gate/macro_probes.rs.
MACRO_PROBES = 'src/catalog_guard/gate/macro_probes.rs'


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


def toml_lines(text):
    """The lines of a TOML document: TOML ends a line only at LF or CRLF, so
    U+2028 and the other separators `str.splitlines` honours stay in it."""
    return [line[:-1] if line.endswith('\r') else line for line in text.split('\n')]


def first_key(dotted):
    """The first component of a dotted TOML key, unquoted, or None for an escaped one.

    Only that component matters: an escape in a later one, as in
    `[target."cfg(target_os = \\"linux\\")".dependencies]`, leaves it readable.
    """
    match = re.match(r'\s*(?:"((?:[^"\\]|\\.)*)"|\'([^\']*)\'|([A-Za-z0-9_-]+))', dotted)
    if not match:
        return None if '\\' in dotted.split('.', 1)[0] else ''
    if match.group(1) is not None and '\\' in match.group(1):
        return None
    return next(part for part in match.groups() if part is not None)


def inline_level(fields):
    """The level of an inline lint table's fields, or None unless every field is readable."""
    found, position = {}, 0
    while position < len(fields):
        match = INLINE_FIELD.match(fields, position)
        if not match or match.group(1) in found or match.end() == position:
            return None
        found[match.group(1)] = match.group(2)
        position = match.end()
    level = found.get('level', '')
    return level.strip('"') if level.startswith('"') else None


def listed_paths(config, key):
    """The paths of one clippy.toml list's active `{ path = ..., reason = ... },` lines."""
    lines = iter(toml_lines(config))
    if f'{key} = [' not in lines:
        raise ValueError(f'clippy.toml has no {key} list')
    paths = []
    for line in (line.strip() for line in lines):
        if line == ']':
            return paths
        if not line or line.startswith('#'):
            continue
        match = re.fullmatch(r'\{ path = "([^"]+)", reason = "[^"]*" \},', line)
        if not match:
            raise ValueError(f'clippy.toml {key}: unreadable line {line}')
        paths.append(match.group(1))
    raise ValueError(f'clippy.toml {key} is not closed')


# The SQL gate's lints, which the probes trigger.
GATE_CODES = frozenset({'clippy::disallowed_macros', 'clippy::disallowed_methods',
                        'clippy::disallowed_types'})


def macro_reports(messages):
    """Read Clippy's JSON `messages` into the `(file, macro)` pairs reported as
    disallowed macros and the errors that are not the gate's own diagnostics."""
    reports, unexpected = set(), []
    for line in messages.splitlines():
        try:
            message = json.loads(line)
        except ValueError:
            continue
        diagnostic = message.get('message') or {}
        if message.get('reason') != 'compiler-message':
            continue
        code = (diagnostic.get('code') or {}).get('code')
        text = diagnostic.get('message', '')
        name = re.search(r'disallowed macro `([^`]+)`', text)
        if code == 'clippy::disallowed_macros' and name:
            reports.update((span['file_name'], name.group(1))
                           for span in diagnostic.get('spans', []) if span.get('is_primary'))
        elif (diagnostic.get('level') == 'error' and code not in GATE_CODES
              and not text.startswith('aborting due to')):
            unexpected.append(text)
    return reports, unexpected


def validate_macro_probes(root):
    """Require Clippy to report every disallowed query macro at its probe.

    `disallowed_macros` is reported outside any local lint scope, so an
    attribute cannot expect it; the probes compile only under
    `--cfg runlimit_macro_probes`, which this check passes to Clippy. They
    expand from the committed `.sqlx` metadata, so SQLx runs offline whatever
    the caller's environment says. Every lint is capped at warn, a forbidden
    one included, which no later `-W` could lower, and the gate lints and
    `warnings` are raised to warn, so the expected reports never fail the
    build: the run must both succeed and report each macro at its probe.
    """
    package = (Path(root) / 'runlimit' / 'runlimit-postgres').resolve()
    expected = set(listed_paths((package / 'clippy.toml').read_text(), 'disallowed-macros'))
    cache = package / '.sqlx'
    probe_file = (package / MACRO_PROBES).with_name('probe.sql').read_text()
    for query in (PROBE_QUERY, probe_file):
        entry = cache / f'query-{hashlib.sha256(query.encode()).hexdigest()}.json'
        if not entry.is_file() or json.loads(entry.read_text()).get('query') != query:
            raise ValueError(f'runlimit-postgres/.sqlx lacks the probe metadata for {query!r}')
    # The package's own cache only: SQLx would read SQLX_OFFLINE_DIR first.
    environment = {**os.environ, 'SQLX_OFFLINE': 'true', 'SQLX_OFFLINE_DIR': str(cache)}
    result = subprocess.run(
        ['cargo', 'clippy', '-p', 'runlimit-postgres', '--lib', '--tests', '--locked',
         '--message-format=json', '--', '--cfg', 'runlimit_macro_probes',
         '--cap-lints', 'warn', '-W', 'warnings',
         *(flag for code in sorted(GATE_CODES) for flag in ('-W', code))],
        cwd=root, env=environment, capture_output=True, text=True)
    reports, errors = macro_reports(result.stdout)
    probes = f'runlimit/runlimit-postgres/{MACRO_PROBES}'
    reported = {name for file, name in reports if file == probes}
    if result.returncode != 0 or errors:
        raise ValueError('the macro probes failed to build:\n'
                         + '\n'.join(errors or [result.stderr[-4000:]]))
    if reported != expected:
        raise ValueError(f'Clippy must report each disallowed macro at its probe; '
                         f'missing {sorted(expected - reported)}')


def top_level_lines(text):
    """Each line of a TOML document that starts outside a value, with its number.

    A line inside a multi-line string or a multi-line array continues the
    value before it, so a `[` there opens no table. A single-line string left
    open at the end of its line is no TOML this reader trusts, so it raises.
    """
    quote, depth = None, 0
    for number, raw in enumerate(toml_lines(text), 1):
        if quote is None and depth == 0:
            yield number, raw
            if raw.strip().startswith('['):
                continue
        index = 0
        while index < len(raw):
            if quote in ('"""', "'''"):
                if raw.startswith(quote, index):
                    # Up to two quotes may end the content before the
                    # closing three, so the whole run closes the string.
                    run = len(raw) - index - len(raw[index:].lstrip(quote[0]))
                    if run > 5:
                        raise ValueError(f'Cargo.toml:{number}: unreadable string delimiter')
                    quote, index = None, index + run
                else:
                    index += 2 if quote == '"""' and raw[index] == '\\' else 1
                continue
            if quote is not None:
                if quote == '"' and raw[index] == '\\':
                    index += 2
                    continue
                if raw[index] == quote:
                    quote = None
                index += 1
                continue
            if raw.startswith('"""', index) or raw.startswith("'''", index):
                quote, index = raw[index:index + 3], index + 3
                continue
            character = raw[index]
            if character in '"\'':
                quote = character
            elif character == '#':
                break
            elif character in '[{':
                depth += 1
            elif character in ']}':
                depth = max(depth - 1, 0)
            index += 1
        if quote in ('"', "'"):
            raise ValueError(f'Cargo.toml:{number}: unterminated string')


def manifest_lints(text):
    """The `[lints.rust]` and `[lints.clippy]` levels of a repository-controlled manifest.

    This is a fail-closed reader for that schema, not a TOML parser, so the
    verification entrypoint keeps its Python 3.9 minimum: a lint set in any
    other table, as a dotted key or in any other form raises. Names are
    normalized to underscores, as Cargo accepts either spelling.
    """
    levels = {'rust': {}, 'clippy': {}}
    table, root = None, True
    for number, raw in top_level_lines(text):
        line = raw.strip()
        if not line or line.startswith('#'):
            continue
        if line.startswith('['):
            match = LINT_TABLE.match(line)
            if match and match.group(1) is None:
                raise ValueError('runlimit-postgres sets [lints], which inherits or nests lint '
                                 'levels; its SQL gate sets [lints.rust] and [lints.clippy]')
            # Any other spelling of a lints table, quoted or escaped, fails closed.
            if not match and first_key(line.lstrip('[')) in ('lints', None):
                raise ValueError(f'Cargo.toml:{number}: unsupported lint table {line}')
            table, root = (match.group(1) if match else None), False
            continue
        if table is None:
            if root and first_key(line) in ('lints', None):
                raise ValueError(f'Cargo.toml:{number}: set lints in [lints.rust] or [lints.clippy]')
            continue
        entry = LINT_ENTRY.match(line)
        level = entry and (entry.group(2) or inline_level(entry.group(3) or ''))
        if not level:
            raise ValueError(f'Cargo.toml:{number}: unreadable lint entry {line}')
        name = entry.group(1).replace('-', '_')
        name = RENAMED_LINTS.get(name, name) if table == 'clippy' else name
        # Cargo applies every spelling of a lint, so one would hide another.
        if name in levels[table]:
            raise ValueError(f'Cargo.toml:{number}: lints.{table} sets {name} twice')
        levels[table][name] = level
    return levels


def validate_sql_gate(root):
    """Keep runlimit-postgres's SQL gate lints enabled and its Clippy settings whole.

    The crate's clippy.toml disallows every sqlx entry point that accepts SQL
    text, and its packaged unit tests pin that inventory. This checkout check
    covers what those tests cannot read reliably: the manifest must not lower
    a gate lint, a group containing one, rustc's `warnings` or the
    `unfulfilled_lint_expectations` that fails an unmet gate probe, nor inherit
    workspace levels, and the crate's clippy.toml, which Clippy reads instead
    of runlimit/clippy.toml, must repeat every shared setting.
    """
    package = Path(root) / 'runlimit' / 'runlimit-postgres'
    lints = manifest_lints((package / 'Cargo.toml').read_text())
    clippy = lints['clippy']
    if clippy.get('all') not in ENABLED_LEVELS:
        raise ValueError('runlimit-postgres must enable clippy::all, which holds its SQL gate')
    lowered = [f'clippy::{name}' for name, level in clippy.items()
               if name in SQL_GATE_LINTS and level not in ENABLED_LEVELS]
    lowered += [name for name, level in lints['rust'].items()
                if name in ('warnings', 'unfulfilled_lint_expectations')
                and level not in ENABLED_LEVELS]
    if lowered:
        raise ValueError(f'runlimit-postgres lowers its SQL gate lints: {sorted(lowered)}')
    # The shared file holds scalar `key = value` settings; each must appear as
    # the same top-level line in the crate's file.
    local = {line.strip() for line in toml_lines((package / 'clippy.toml').read_text())}
    dropped = sorted(line.strip() for line in
                     toml_lines((Path(root) / 'runlimit' / 'clippy.toml').read_text())
                     if line.strip() and not line.strip().startswith('#')
                     and line.strip() not in local)
    if dropped:
        raise ValueError(f'runlimit-postgres/clippy.toml drops shared settings: {dropped}')


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


def package_archives(root, directory):
    """Package and verify every native crate, returning the archives Cargo built.

    Packaging runs in the checkout, so the archives are exactly what crates.io
    would receive. `directory`, outside the checkout, is Cargo's target
    directory: verification extracts each archive there and builds its
    library, once with default features and once with all features, with
    `PWD` pointing there too. A library that embeds a record by a path
    relative to its crate, to the checkout's ancestors or to `PWD` therefore
    fails here. An absolute path into one machine's checkout is not detected.
    Cargo does not build a package's test, example or bench targets from the
    archive; that pre-existing gap is `batter-88bd` and is not specific to
    records.
    """
    root = Path(root).resolve()
    directory = Path(directory).resolve()
    if directory == root or root in directory.parents:
        raise ValueError(f'package verification directory {directory} is inside the checkout')
    packages = [argument for suffix in NATIVE for argument in ('-p', 'runlimit-' + suffix)]
    environment = {**os.environ, 'PWD': str(directory)}
    for features in ([], ['--all-features']):
        command = ['cargo', 'package', '--allow-dirty', '--locked', '--target-dir',
                   str(directory), *features, *packages]
        result = subprocess.run(command, cwd=root, env=environment, capture_output=True,
                                text=True)
        if result.returncode != 0:
            selection = ' '.join(features) or 'default features'
            raise ValueError(f'native packaging or verification failed ({selection}):\n'
                             + result.stderr[-4000:])
    return [directory / 'package' / f'runlimit-{suffix}-{EXPECTED_VERSION}.crate'
            for suffix in NATIVE]

if __name__ == '__main__':
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--format-version', '1', '--all-features', '--locked'], cwd=ROOT))
    validate_graph(metadata, ROOT)
    validate_assets(ROOT)
    validate_sql_gate(ROOT)
    validate_macro_probes(ROOT)
    expected = json.loads((ROOT / 'runlimit' / 'upstream-assets.json').read_text())
    with tempfile.TemporaryDirectory(prefix='runlimit-package-') as scratch:
        validate_published(package_archives(ROOT, scratch), record_contents(ROOT, expected))
    print('Runlimit: five publishable local identities, native ownership and imported SQL/licenses verified')
