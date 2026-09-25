#!/usr/bin/env python3
"""Set an address-space ceiling in the child, then exec the recorded command."""

from __future__ import annotations

import os
import resource
import sys


def main() -> None:
    mib = int(sys.argv[1])
    command = sys.argv[2:]
    if mib < 1 or not command:
        raise SystemExit("usage: arxiv_resource_limit.py MIB COMMAND [ARG ...]")
    ceiling = mib * 1024 * 1024
    resource.setrlimit(resource.RLIMIT_AS, (ceiling, ceiling))
    os.execvpe(command[0], command, os.environ)


if __name__ == "__main__":
    main()
