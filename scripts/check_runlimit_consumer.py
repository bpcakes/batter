#!/usr/bin/env python3
"""Execute original native and facade quota consumers from a Git-free source copy."""
import argparse
import json
import os
from pathlib import Path
import tempfile

from check_runledger_consumer import selected_cargo
from check_runlimit_workspace import NATIVE, validate_assets
from runledger_source import copy_source, external_sources, run, validate_consumer

ROOT = Path(__file__).resolve().parent.parent
IDENTITIES = ('batter', 'batter-core', 'batter-sqlx', 'batter-axum', 'batter-runlimit',
              *('runlimit-' + suffix for suffix in NATIVE))
EXECUTIONS = (('native', 'request admitted'),
              ('facade', 'facade quota admission and denial passed'))


def require_execution(output, marker):
    if output.splitlines().count(marker) != 1:
        raise RuntimeError('consumer did not complete its expected assertions: ' + marker)


def manifest(source):
    dependencies = []
    for name in ('batter', 'batter-core', 'batter-runlimit',
                 *('runlimit-' + suffix for suffix in NATIVE)):
        group = 'runlimit' if name.startswith('runlimit-') else 'crates'
        extra = (', default-features = false, features = ["runlimit-memory", '
                 '"runlimit-postgres", "runlimit-axum"]') if name == 'batter' else ''
        dependencies.append(name + ' = { path = ' + json.dumps(str(source / group / name)) + extra + ' }')
    dependencies.append('tokio = { version = "1", features = ["macros", "rt"] }')
    binaries = ''.join('\n[[bin]]\nname = "runlimit-' + name + '-consumer"\npath = '
                       + json.dumps(str(source / 'runlimit/smoke' / (name + '_consumer.rs')))
                       + '\n' for name, _ in EXECUTIONS)
    return ('[package]\nname = "runlimit-source-consumer"\nversion = "0.0.0"\n'
            'edition = "2024"\nrust-version = "1.94"\npublish = false\n'
            '[workspace]\nresolver = "3"\n[dependencies]\n'
            + '\n'.join(dependencies) + '\n' + binaries)


def main():
    argparse.ArgumentParser(description=__doc__).parse_args()
    cargo = selected_cargo(ROOT)
    print('Runlimit standalone consumer toolchain=' + cargo[1], flush=True)
    locked = (ROOT / 'Cargo.lock').read_bytes()
    metadata = json.loads(run([*cargo, 'metadata', '--format-version', '1',
                               '--all-features', '--locked'], ROOT))
    target = Path(os.environ.get('CARGO_TARGET_DIR', ROOT / 'target')).resolve() / 'runlimit-consumer'
    with tempfile.TemporaryDirectory(prefix='batter-runlimit-consumer-') as directory:
        parent = Path(directory)
        source = copy_source(ROOT, parent / 'source')
        if (source / '.git').exists():
            raise RuntimeError('consumer source copy contains Git metadata')
        validate_assets(source)
        consumer = parent / 'consumer'
        consumer.mkdir()
        (consumer / 'Cargo.toml').write_text(manifest(source))
        (consumer / 'Cargo.lock').write_bytes(locked)
        resolved = json.loads(run([*cargo, 'metadata', '--format-version', '1', '--offline'], consumer))
        validate_consumer(resolved, source, consumer, external_sources(metadata),
                          required_identities=IDENTITIES)
        for name, marker in EXECUTIONS:
            output = run(['env', 'RUNLIMIT_KEY_SECRET=public-source-consumer-fixture-key',
                          'SQLX_OFFLINE=true', *cargo, 'run', '--bin',
                          'runlimit-' + name + '-consumer', '--locked', '--offline',
                          '--target-dir', str(target)], consumer, echo=True)
            require_execution(output, marker)
    if (ROOT / 'Cargo.lock').read_bytes() != locked:
        raise RuntimeError('root lock changed during standalone consumption')
    print('Git-free Runlimit source: native smoke, facade quota and all identities passed')


if __name__ == '__main__':
    main()
