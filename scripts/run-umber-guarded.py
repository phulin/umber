#!/usr/bin/env python3
"""Run an Umber command in a bounded, fully reaped process group."""

import argparse
import os
import signal
import subprocess
import sys
import time


def process_rows():
    result = subprocess.run(
        ["ps", "-axo", "pid=,pgid=,rss=,command="], capture_output=True, check=True, text=True
    )
    for line in result.stdout.splitlines():
        parts = line.strip().split(None, 3)
        if len(parts) >= 3:
            yield int(parts[0]), int(parts[1]), int(parts[2]), line.strip()


def group_rows(pgid):
    return [row for row in process_rows() if row[1] == pgid]


def signal_group(pgid, sig):
    try:
        os.killpg(pgid, sig)
    except ProcessLookupError:
        pass


def parse_args():
    parser = argparse.ArgumentParser()
    parser.add_argument("--timeout-seconds", type=float, required=True)
    parser.add_argument("--max-rss-mib", type=int, required=True)
    parser.add_argument("--term-grace-seconds", type=float, default=5)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.command[:1] == ["--"]:
        args.command = args.command[1:]
    if not args.command:
        parser.error("a command is required after --")
    if not 0 < args.timeout_seconds <= 600:
        parser.error("timeout must be in (0, 600] seconds")
    if not 0 < args.max_rss_mib <= 6144:
        parser.error("aggregate RSS limit must be in (0, 6144] MiB")
    if not 0 <= args.term_grace_seconds <= 5:
        parser.error("TERM grace must be in [0, 5] seconds")
    return args


def main():
    args = parse_args()
    child = subprocess.Popen(args.command, start_new_session=True)
    pgid = child.pid
    deadline = time.monotonic() + args.timeout_seconds
    rss_limit_kib = args.max_rss_mib * 1024
    reason = None
    while child.poll() is None:
        rss_kib = sum(row[2] for row in group_rows(pgid))
        if rss_kib > rss_limit_kib:
            reason = f"aggregate RSS {rss_kib} KiB exceeded {rss_limit_kib} KiB"
            break
        if time.monotonic() >= deadline:
            reason = f"timeout exceeded {args.timeout_seconds:g} seconds"
            break
        time.sleep(0.1)

    if reason is not None:
        print(f"guard: {reason}; sending TERM to process group {pgid}", file=sys.stderr)
        signal_group(pgid, signal.SIGTERM)
        grace_deadline = time.monotonic() + args.term_grace_seconds
        while child.poll() is None and time.monotonic() < grace_deadline:
            time.sleep(0.05)
        signal_group(pgid, signal.SIGKILL)

    status = child.wait()
    survivors = group_rows(pgid)
    if survivors:
        signal_group(pgid, signal.SIGKILL)
        print("guard: surviving process-group members after reap:", file=sys.stderr)
        for row in survivors:
            print(row[3], file=sys.stderr)
        return 125
    return 124 if reason is not None else status


if __name__ == "__main__":
    sys.exit(main())
