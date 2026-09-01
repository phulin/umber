#!/usr/bin/env python3
"""Verify, rank, and exact-binary symbolize copy-attribution probe output."""

from __future__ import annotations

import argparse
import hashlib
import pathlib
import re
import subprocess
import sys
from dataclasses import dataclass


FIELD = re.compile(r"([a-z_]+)=([^ ]+)")
APPLICATION_CLASSES = {"application_direct", "application_ancestor"}


@dataclass(frozen=True)
class Bin:
    api: str
    caller_class: str
    address: int
    calls: int
    bytes: int
    module: str | None
    module_offset: int | None


def fields(line: str) -> dict[str, str]:
    return dict(FIELD.findall(line))


def parse_report(path: pathlib.Path) -> tuple[list[Bin], dict[str, tuple[int, int]], list[str]]:
    bins: list[Bin] = []
    totals: dict[str, tuple[int, int]] = {}
    tables: list[str] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        values = fields(line)
        if line.startswith("COPY_CALLER "):
            bins.append(
                Bin(
                    values["api"],
                    values["class"],
                    int(values["address"], 16),
                    int(values["calls"]),
                    int(values["bytes"]),
                    values.get("module"),
                    int(values["module_offset"], 16)
                    if "module_offset" in values
                    else None,
                )
            )
        elif line.startswith("COPY_TOTAL "):
            calls = int(values["calls"])
            byte_count = int(values["bytes"])
            caller_calls = int(values["caller_calls"])
            caller_bytes = int(values["caller_bytes"])
            if (calls, byte_count) != (caller_calls, caller_bytes):
                raise ValueError(
                    f"{values['api']} probe reconciliation failed: "
                    f"total={(calls, byte_count)} callers={(caller_calls, caller_bytes)}"
                )
            totals[values["api"]] = (calls, byte_count)
        elif line.startswith("COPY_TABLE "):
            tables.append(line)
    if set(totals) != {"memcpy", "memmove"}:
        raise ValueError("report must contain reconciled memcpy and memmove totals")
    for api, total in totals.items():
        caller_sum = (
            sum(item.calls for item in bins if item.api == api),
            sum(item.bytes for item in bins if item.api == api),
        )
        if caller_sum != total:
            raise ValueError(f"{api} parsed bins {caller_sum} do not sum to {total}")
    return bins, totals, tables


def symbolize(binary: pathlib.Path, addresses: set[int]) -> dict[int, list[tuple[str, str]]]:
    if not addresses:
        return {}
    requested = {max(0, address - 1): address for address in addresses}
    request = "".join(f"0x{address:x}\n" for address in sorted(requested))
    result = subprocess.run(
        ["addr2line", "-a", "-f", "-C", "-i", "-e", str(binary)],
        input=request,
        text=True,
        capture_output=True,
        check=True,
    )
    resolved: dict[int, list[tuple[str, str]]] = {}
    current: int | None = None
    lines = result.stdout.splitlines()
    index = 0
    while index < len(lines):
        line = lines[index]
        if line.startswith("0x"):
            queried = int(line, 16)
            if queried not in requested:
                raise ValueError(f"unexpected addr2line address {line}")
            current = requested[queried]
            resolved[current] = []
            index += 1
            continue
        if current is None or index + 1 >= len(lines):
            raise ValueError("malformed addr2line output")
        resolved[current].append((line, lines[index + 1]))
        index += 2
    return resolved


def binary_sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=pathlib.Path)
    parser.add_argument("--report", required=True, type=pathlib.Path)
    parser.add_argument("--limit", type=int, default=40)
    arguments = parser.parse_args()

    bins, totals, tables = parse_report(arguments.report)
    ranked = {
        api: sorted(
            (item for item in bins if item.api == api),
            key=lambda item: (-item.bytes, -item.calls, item.caller_class, item.address),
        )[: arguments.limit]
        for api in ("memcpy", "memmove")
    }
    addresses = {
        item.address
        for api_bins in ranked.values()
        for item in api_bins
        if item.caller_class in APPLICATION_CLASSES
    }
    symbols = symbolize(arguments.binary, addresses)
    for address, frames in symbols.items():
        if not frames or any(
            function == "??" or location.startswith("??:")
            for function, location in frames
        ):
            raise ValueError(f"application address 0x{address:x} is not fully symbolized")

    print("COPY_ATTRIBUTION_REPORT schema=1")
    print(f"BINARY path={arguments.binary.resolve()} sha256={binary_sha256(arguments.binary)}")
    for api in ("memcpy", "memmove"):
        calls, byte_count = totals[api]
        print(f"TOTAL api={api} calls={calls} bytes={byte_count}")
        for rank, item in enumerate(ranked[api], start=1):
            print(
                f"RANK api={api} rank={rank} class={item.caller_class} "
                f"address=0x{item.address:x} calls={item.calls} bytes={item.bytes}"
            )
            if item.caller_class in APPLICATION_CLASSES:
                for frame, (function, location) in enumerate(symbols[item.address]):
                    print(f"  FRAME index={frame} function={function} source={location}")
            else:
                print(
                    f"  EXTERNAL module={item.module or 'unknown'} "
                    f"module_offset=0x{item.module_offset or 0:x}"
                )
    for table in tables:
        print(table)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, subprocess.CalledProcessError, ValueError) as error:
        print(f"copy attribution: {error}", file=sys.stderr)
        raise SystemExit(1) from error
