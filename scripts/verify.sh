#!/usr/bin/env bash
# --bootstrap intentionally formats the initial authored snapshot and resolves
# dependencies. Later runs require and preserve the resulting Cargo.lock.
set -euo pipefail
cd "$(dirname "$0")/.."

bootstrap=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --bootstrap) bootstrap=true; shift ;;
    *) printf 'Usage: %s [--bootstrap]\n' "$0" >&2; exit 2 ;;
  esac
done
for tool in cargo rustc rustfmt python3; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    printf 'Required tool missing: %s. No Rust checks were run.\n' "$tool" >&2
    exit 127
  fi
done
python3 -c 'import sys; sys.exit("Python 3.9 or newer is required; no Rust checks were run." if sys.version_info < (3, 9) else 0)'
rustc --version --verbose
cargo --version
if [[ "$bootstrap" == true ]]; then
  printf 'Bootstrap: formatting sources and generating a lockfile if absent.\n'
  cargo fmt --all
  if [[ ! -f Cargo.lock ]]; then
    cargo generate-lockfile
  fi
fi
if [[ ! -f Cargo.lock ]]; then
  printf 'Cargo.lock is absent. Run this script with --bootstrap once; review and commit the result.\n' >&2
  exit 2
fi
printf 'Workspace tests require loopback TCP sockets and Unix subprocess permissions.\n'
cargo fmt --all -- --check
bash scripts/check_file_budget.sh
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings -D clippy::mod_module_files
cargo clippy -p runlimit-core -p runlimit-memory -p runlimit-postgres -p runlimit-http -p runlimit-axum --all-targets --locked -- -D warnings
# The reference package's default build excludes its opt-in metrics exporter.
cargo clippy -p batter-example-reference-service --all-targets --locked -- -D warnings -D clippy::mod_module_files
for part in workspace no-default-features doctests consumers runlimit scripts; do
  python3 scripts/test_matrix.py "$part"
done
RUSTDOCFLAGS="${RUSTDOCFLAGS:-} -D warnings" cargo doc --workspace --all-features --no-deps --locked
python3 scripts/check_http_smokes.py
