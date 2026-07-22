#!/usr/bin/env python3
"""Run an Umber command in a bounded, fully reaped process group."""

import argparse
import ctypes
import functools
import os
import signal
import subprocess
import sys
import time


def ps_process_rows():
    result = subprocess.run(
        ["ps", "-axo", "pid=,pgid=,rss=,command="], capture_output=True, check=True, text=True
    )
    for line in result.stdout.splitlines():
        parts = line.strip().split(None, 3)
        if len(parts) >= 3:
            yield int(parts[0]), int(parts[1]), int(parts[2]), line.strip()


@functools.cache
def darwin_libproc():
    libproc = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
    libproc.proc_listpgrppids.argtypes = [ctypes.c_int, ctypes.c_void_p, ctypes.c_int]
    libproc.proc_listpgrppids.restype = ctypes.c_int
    libproc.proc_pidinfo.argtypes = [
        ctypes.c_int,
        ctypes.c_int,
        ctypes.c_uint64,
        ctypes.c_void_p,
        ctypes.c_int,
    ]
    libproc.proc_pidinfo.restype = ctypes.c_int
    libproc.proc_name.argtypes = [ctypes.c_int, ctypes.c_void_p, ctypes.c_uint32]
    libproc.proc_name.restype = ctypes.c_int
    return libproc


def darwin_group_rows(pgid):
    libproc = darwin_libproc()

    buffer_size = libproc.proc_listpgrppids(pgid, None, 0)
    if buffer_size < 0:
        raise OSError(ctypes.get_errno(), "proc_listpgrppids sizing failed")
    if buffer_size == 0:
        return []

    pid_capacity = max(1, buffer_size // ctypes.sizeof(ctypes.c_int))
    pids = (ctypes.c_int * pid_capacity)()
    pid_count = libproc.proc_listpgrppids(pgid, pids, ctypes.sizeof(pids))
    if pid_count < 0:
        raise OSError(ctypes.get_errno(), "proc_listpgrppids failed")

    # PROC_PIDTASKINFO begins with virtual size and resident size, both in bytes.
    task_info = (ctypes.c_uint64 * 12)()
    name = ctypes.create_string_buffer(1024)
    rows = []
    for pid in pids[:pid_count]:
        info_size = libproc.proc_pidinfo(pid, 4, 0, task_info, ctypes.sizeof(task_info))
        if info_size < 2 * ctypes.sizeof(ctypes.c_uint64):
            continue
        rss_kib = (task_info[1] + 1023) // 1024
        name.value = b""
        name_size = libproc.proc_name(pid, name, ctypes.sizeof(name))
        command = name.value.decode(errors="replace") if name_size > 0 else "?"
        rows.append((pid, pgid, rss_kib, f"{pid} {pgid} {rss_kib} {command}"))
    return rows


def linux_group_rows(pgid):
    page_size = os.sysconf("SC_PAGE_SIZE")
    rows = []
    with os.scandir("/proc") as entries:
        for entry in entries:
            if not entry.name.isdigit():
                continue
            pid = int(entry.name)
            try:
                with open(f"/proc/{pid}/stat", encoding="utf-8") as stat_file:
                    stat = stat_file.read()
                fields = stat[stat.rfind(")") + 2 :].split()
                if int(fields[2]) != pgid:
                    continue
                with open(f"/proc/{pid}/statm", encoding="utf-8") as statm_file:
                    resident_pages = int(statm_file.read().split()[1])
                rss_kib = (resident_pages * page_size + 1023) // 1024
                with open(f"/proc/{pid}/cmdline", "rb") as command_file:
                    command = command_file.read().replace(b"\0", b" ").strip()
            except (FileNotFoundError, PermissionError, ProcessLookupError, ValueError, IndexError):
                continue
            command_text = command.decode(errors="replace") or "?"
            rows.append((pid, pgid, rss_kib, f"{pid} {pgid} {rss_kib} {command_text}"))
    return rows


def group_rows(pgid):
    if sys.platform == "darwin":
        return darwin_group_rows(pgid)
    if sys.platform.startswith("linux"):
        return linux_group_rows(pgid)
    return [row for row in ps_process_rows() if row[1] == pgid]


def signal_group(pgid, sig):
    try:
        os.killpg(pgid, sig)
    except ProcessLookupError:
        pass
    except PermissionError:
        # Sandboxed macOS can reject killpg after the session leader exits even
        # though its former descendants remain signalable individually.
        for pid, _, _, _ in group_rows(pgid):
            try:
                os.kill(pid, sig)
            except ProcessLookupError:
                pass


def parse_args():
    parser = argparse.ArgumentParser()
    parser.add_argument("--timeout-seconds", type=float, required=True)
    parser.add_argument("--max-rss-mib", type=int, required=True)
    parser.add_argument("--term-grace-seconds", type=float, default=5)
    parser.add_argument("--progress-file")
    parser.add_argument("--progress-timeout-seconds", type=float)
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
    if args.progress_timeout_seconds is not None:
        if args.progress_file is None:
            parser.error("--progress-timeout-seconds requires --progress-file")
        if not 0 < args.progress_timeout_seconds <= args.timeout_seconds:
            parser.error("progress timeout must be in (0, timeout] seconds")
    elif args.progress_file is not None:
        parser.error("--progress-file requires --progress-timeout-seconds")
    return args


def main():
    args = parse_args()
    child = subprocess.Popen(args.command, start_new_session=True)
    pgid = child.pid
    deadline = time.monotonic() + args.timeout_seconds
    rss_limit_kib = args.max_rss_mib * 1024
    reason = None
    progress_signature = None
    last_progress = time.monotonic()
    while child.poll() is None:
        rss_kib = sum(row[2] for row in group_rows(pgid))
        if rss_kib > rss_limit_kib:
            reason = f"aggregate RSS {rss_kib} KiB exceeded {rss_limit_kib} KiB"
            break
        if time.monotonic() >= deadline:
            reason = f"timeout exceeded {args.timeout_seconds:g} seconds"
            break
        if args.progress_file is not None:
            try:
                stat = os.stat(args.progress_file)
                signature = (stat.st_size, stat.st_mtime_ns)
            except FileNotFoundError:
                signature = None
            if signature != progress_signature:
                progress_signature = signature
                last_progress = time.monotonic()
            elif time.monotonic() - last_progress >= args.progress_timeout_seconds:
                reason = (
                    "no progress-file change for "
                    f"{args.progress_timeout_seconds:g} seconds"
                )
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
