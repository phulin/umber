"""Bounded scheduling and host capacity for independent arXiv corpus jobs."""

from __future__ import annotations

import math
import os
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from typing import Callable, TypeVar

T = TypeVar("T")
U = TypeVar("U")


def _cgroup_value(name: str) -> str | None:
    path = Path("/sys/fs/cgroup") / name
    try:
        return path.read_text().strip()
    except OSError:
        return None


def available_cpus() -> int:
    cpus = len(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else (os.cpu_count() or 1)
    quota = _cgroup_value("cpu.max")
    if quota:
        amount, period = quota.split()[:2]
        if amount != "max" and int(period) > 0:
            cpus = min(cpus, max(1, math.ceil(int(amount) / int(period))))
    return max(1, cpus)


def linux_available_memory_bytes(path: Path = Path("/proc/meminfo")) -> int | None:
    try:
        for line in path.read_text().splitlines():
            if line.startswith("MemAvailable:"):
                return int(line.split()[1]) * 1024
    except (OSError, ValueError, IndexError):
        pass
    return None


def available_memory_bytes() -> int:
    try:
        available = os.sysconf("SC_AVPHYS_PAGES") * os.sysconf("SC_PAGE_SIZE")
    except (OSError, ValueError):
        available = 2 * 1024**3
    linux_available = linux_available_memory_bytes()
    if linux_available is not None:
        available = linux_available
    limit, current = _cgroup_value("memory.max"), _cgroup_value("memory.current")
    if limit and limit != "max" and current:
        available = min(available, max(0, int(limit) - int(current)))
    return max(0, available)


def automatic_jobs(max_rss_mib: int) -> int:
    cpus = available_cpus()
    memory = available_memory_bytes()
    reserve = max(2 * 1024**3, memory // 5)
    memory_jobs = max(1, (memory - reserve) // (max_rss_mib * 1024**2))
    return max(1, min(max(1, cpus - 1), memory_jobs))


def run_jobs(items: list[T], jobs: int, worker: Callable[[T], U],
             on_complete: Callable[[U], None] | None = None) -> list[U]:
    """Bound execution while returning results in source order.

    Each worker owns its paper's directory. A completed receipt is written by
    that worker, so interruption can leave harmless gaps in lock order.
    """
    if jobs == 1:
        results = []
        for item in items:
            result = worker(item)
            if on_complete:
                on_complete(result)
            results.append(result)
        return results
    pool = ThreadPoolExecutor(max_workers=jobs)
    futures = {pool.submit(worker, item): index for index, item in enumerate(items)}
    results: dict[int, U] = {}
    try:
        for future in as_completed(futures):
            result = future.result()
            results[futures[future]] = result
            if on_complete:
                on_complete(result)
    except BaseException:
        pool.shutdown(wait=True, cancel_futures=True)
        raise
    pool.shutdown(wait=True)
    return [results[index] for index in range(len(items))]
