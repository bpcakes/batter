#!/usr/bin/env bash
# Check local changes or the exact GitHub event comparison without receipts.
set -euo pipefail
cd "$(dirname "$0")/.."

case "${JIG_EVENT_NAME:-local}" in
  pull_request)
    scripts/jig file-budget check \
      --exact-tree "${JIG_PULL_REQUEST_BASE:?pull request base is required}" \
      --provenance explicit
    ;;
  push)
    scripts/jig file-budget check \
      --exact-tree "${JIG_PUSH_BEFORE:?push before is required}" \
      --provenance push_before
    ;;
  merge_group)
    scripts/jig file-budget check \
      --exact-tree "${JIG_MERGE_GROUP_BASE:?merge group base is required}" \
      --provenance explicit
    ;;
  local|workflow_dispatch)
    scripts/jig file-budget check --base origin/master
    ;;
  *)
    printf 'Unsupported CI event: %s\n' "$JIG_EVENT_NAME" >&2
    exit 2
    ;;
esac
