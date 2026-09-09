#!/usr/bin/env bash
# External fixture only. Ordinary workspace tests leave these cases ignored.
set -euo pipefail
cd "$(dirname "$0")/.."
exec python3 scripts/sqlx_live.py
