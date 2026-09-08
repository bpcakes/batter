#!/usr/bin/env bash
# Run from the checkout root, using GitHub event metadata supplied by CI.
set -euo pipefail

case "${JIG_EVENT_NAME:?JIG_EVENT_NAME is required}" in
  pull_request)
    scripts/jig check repo:file-budget \
      --comparison-exact-tree "${JIG_PULL_REQUEST_BASE:?pull request base is required}" \
      --comparison-provenance explicit
    ;;
  push)
    scripts/jig check repo:file-budget \
      --comparison-exact-tree "${JIG_PUSH_BEFORE:?push before is required}" \
      --comparison-provenance push_before
    ;;
  merge_group)
    scripts/jig check repo:file-budget \
      --comparison-exact-tree "${JIG_MERGE_GROUP_BASE:?merge group base is required}" \
      --comparison-provenance explicit
    ;;
  workflow_dispatch)
    scripts/jig check repo:file-budget --comparison-base origin/master
    ;;
  *)
    printf 'Unsupported CI event: %s\n' "$JIG_EVENT_NAME" >&2
    exit 2
    ;;
esac
