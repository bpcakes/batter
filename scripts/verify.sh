#!/usr/bin/env bash
# --bootstrap intentionally formats the initial authored snapshot and resolves
# dependencies. Later runs require and preserve the resulting Cargo.lock.
set -euo pipefail
cd "$(dirname "$0")/.."

bootstrap=false
plan_id=
while [[ $# -gt 0 ]]; do
  case "$1" in
    --bootstrap) bootstrap=true; shift ;;
    --plan-id)
      if [[ $# -lt 2 || -z "$2" || "$2" == --* ]]; then
        printf '%s\n' '--plan-id requires a plan ID' >&2
        exit 2
      fi
      plan_id=$2
      shift 2
      ;;
    *) printf 'Usage: %s [--bootstrap] [--plan-id ID]\n' "$0" >&2; exit 2 ;;
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
if [[ -n "$plan_id" ]]; then
  exec scripts/jig work check --plan-id "$plan_id"
fi
exec scripts/jig check --profile verify --comparison-base origin/master
