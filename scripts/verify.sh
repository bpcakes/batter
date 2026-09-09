#!/usr/bin/env bash
# --bootstrap intentionally formats the initial authored snapshot and resolves
# dependencies. Later runs require and preserve the resulting Cargo.lock.
set -euo pipefail
cd "$(dirname "$0")/.."

case "${1:-}" in
  ""|--bootstrap) ;;
  *) printf 'Usage: %s [--bootstrap]\n' "$0" >&2; exit 2 ;;
esac
for tool in cargo rustc rustfmt python3; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    printf 'Required tool missing: %s. No Rust checks were run.\n' "$tool" >&2
    exit 127
  fi
done
python3 -c 'import sys; sys.exit("Python 3.9 or newer is required; no Rust checks were run." if sys.version_info < (3, 9) else 0)'
rustc --version --verbose
cargo --version
if [[ "${1:-}" == --bootstrap ]]; then
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
cargo fmt --all -- --check
cargo check -p batter --lib --no-default-features --locked
printf 'Workspace tests require loopback TCP sockets and Unix subprocess permissions.\n'
cargo test -p batter --no-default-features --lib --tests --locked
cargo test --workspace --all-features --all-targets --locked
cargo test --workspace --all-features --doc --locked
cargo clippy --workspace --all-features --all-targets --locked -- -D warnings
RUSTDOCFLAGS="${RUSTDOCFLAGS:-} -D warnings" cargo doc --workspace --all-features --no-deps --locked
printf 'Rust verification, including live loopback readiness tests, completed successfully. HTTP process smokes and live PostgreSQL checks are separate.\n'
