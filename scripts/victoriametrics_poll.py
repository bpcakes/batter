#!/usr/bin/env python3
"""Private HTTP polling worker; the parent owns its total deadline and stop/reap."""

import sys
import urllib.error

from check_victoriametrics import get, verify


def main(mode, base):
    if mode not in ("health", "metrics"):
        raise ValueError("unknown polling mode")
    try:
        if mode == "health":
            get(base + "/health")
        else:
            verify(base)
    except (OSError, ValueError, AssertionError, urllib.error.URLError) as error:
        print(f"{type(error).__name__}: {error}", file=sys.stderr)
        # A completed availability/read-back check, not an interrupted process.
        return 3
    return 0


if __name__ == "__main__":
    raise SystemExit(main(*sys.argv[1:]))
