#!/usr/bin/env python3
"""Replay normalized pdfTeX file traces against candidate manifest shardings."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import sys
from collections import defaultdict
from pathlib import Path
from typing import Any


def canonical_json(value: Any) -> bytes:
    return (json.dumps(value, ensure_ascii=False, separators=(",", ":")) + "\n").encode()


def shard_index(key: str, bits: int) -> int:
    digest = hashlib.sha256(key.encode()).digest()
    return int.from_bytes(digest[:2], "big") >> (16 - bits) if bits else 0


def gzip_size(value: bytes) -> int:
    # mtime=0 is the portable equivalent of the publisher measurement's
    # `gzip -n -c`; gzip output length is independent of the OS header byte.
    return len(gzip.compress(value, compresslevel=6, mtime=0))


def load_traces(
    directory: Path, files: dict[str, Any]
) -> tuple[list[tuple[Path, set[str]]], set[str], set[str]]:
    by_path: dict[str, list[str]] = defaultdict(list)
    for key, entry in files.items():
        by_path[entry["virtualPath"]].append(key)

    traces = sorted(directory.rglob("files.txt"))
    trace_keys: list[tuple[Path, set[str]]] = []
    all_keys: set[str] = set()
    unmatched: set[str] = set()
    for trace in traces:
        keys: set[str] = set()
        for line in trace.read_text(encoding="utf-8").splitlines():
            virtual_path = line.strip()
            if not virtual_path:
                continue
            candidates = by_path.get(virtual_path)
            if not candidates:
                unmatched.add(virtual_path)
                continue
            basename = virtual_path.rsplit("/", 1)[-1]
            basename_keys = [
                key for key in candidates if key.split(":", 1)[1] == basename
            ]
            # The recorder reports the resolved physical path, not the lookup
            # spelling. Model publisher lookup precedence by preferring the
            # basename alias, then the shortest deterministic alias.
            keys.add(min(basename_keys or candidates, key=lambda key: (len(key), key)))
        trace_keys.append((trace, keys))
        all_keys.update(keys)
    return trace_keys, all_keys, unmatched


def shard_file(key: str, entry: dict[str, Any], files: dict[str, Any]) -> dict[str, Any]:
    result = {
        "virtualPath": entry["virtualPath"],
        "object": entry["object"],
        "sha256": entry["sha256"],
        "bytes": entry["bytes"],
    }
    dependencies = []
    for dependency_key in entry.get("dependencies", []):
        target = files[dependency_key]
        dependencies.append(
            {
                "key": dependency_key,
                "virtualPath": target["virtualPath"],
                "object": target["object"],
                "sha256": target["sha256"],
                "bytes": target["bytes"],
            }
        )
    if dependencies:
        result["dependencies"] = dependencies
    return result


def measure(
    manifest: dict[str, Any], trace_keys: list[tuple[Path, set[str]]], bits: int
) -> dict[str, int | str]:
    files = manifest["files"]
    count = 1 << bits
    partitions: list[dict[str, Any]] = [dict() for _ in range(count)]
    for key in sorted(files):
        partitions[shard_index(key, bits)][key] = shard_file(key, files[key], files)

    paper_shards = [
        {shard_index(key, bits) for key in keys} for _, keys in trace_keys
    ]
    touched = set().union(*paper_shards)
    digests: list[str] = []
    shard_gzip_sizes: dict[int, int] = {}
    for index, partition in enumerate(partitions):
        encoded = canonical_json(
            {
                "schema": 1,
                "distribution": manifest["distribution"],
                "index": index,
                "files": partition,
            }
        )
        digests.append(hashlib.sha256(encoded).hexdigest())
        if index in touched:
            shard_gzip_sizes[index] = gzip_size(encoded)

    root: dict[str, Any] = {
        "schema": 2,
        "distribution": manifest["distribution"],
        "objectsBaseUrl": manifest["objectsBaseUrl"],
        "shardBits": bits,
        "shardCount": count,
        "shards": digests,
    }
    if manifest.get("formats"):
        root["formats"] = manifest["formats"]
    root_bytes = canonical_json(root)
    root_gzip_bytes = gzip_size(root_bytes)
    union_shard_gzip_bytes = sum(shard_gzip_sizes.values())
    independent_shard_requests = sum(len(shards) for shards in paper_shards)
    independent_shard_gzip_bytes = sum(
        sum(shard_gzip_sizes[index] for index in shards) for shards in paper_shards
    )
    return {
        "shard_bits": bits,
        "shards": count,
        "root_sha256": hashlib.sha256(root_bytes).hexdigest(),
        "union_cold_requests": 1 + len(touched),
        "union_cold_shard_requests": len(touched),
        "union_cold_gzip_bytes": root_gzip_bytes + union_shard_gzip_bytes,
        "independent_cold_requests": len(trace_keys) + independent_shard_requests,
        "independent_cold_shard_requests": independent_shard_requests,
        "independent_cold_gzip_bytes": len(trace_keys) * root_gzip_bytes
        + independent_shard_gzip_bytes,
        "root_gzip_bytes": root_gzip_bytes,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path, help="schema-1 monolithic manifest.json")
    parser.add_argument("traces", type=Path, help="profile results containing files.txt traces")
    parser.add_argument("--shard-bits", default="6,8,10", help="comma-separated candidates")
    parser.add_argument("--output", type=Path, help="write report instead of stdout")
    args = parser.parse_args()

    manifest_bytes = args.manifest.read_bytes()
    manifest = json.loads(manifest_bytes)
    if manifest.get("schema") != 1:
        parser.error("manifest must use monolithic schema 1")
    bits_values = [int(value) for value in args.shard_bits.split(",")]
    if not bits_values or any(bits < 0 or bits > 16 for bits in bits_values):
        parser.error("shard bits must be between 0 and 16")

    trace_keys, keys, unmatched = load_traces(args.traces, manifest["files"])
    if not trace_keys:
        parser.error(f"no files.txt traces below {args.traces}")
    rows = [measure(manifest, trace_keys, bits) for bits in bits_values]
    lines = [
        f"# manifest_sha256\t{hashlib.sha256(manifest_bytes).hexdigest()}",
        f"# trace_files\t{len(trace_keys)}",
        f"# matched_lookup_keys\t{len(keys)}",
        f"# unmatched_virtual_paths\t{len(unmatched)}",
        "# lookup_projection\tresolved paths use basename-precedence aliases",
        "shard_bits\tshards\troot_sha256\tunion_cold_requests\t"
        "union_cold_shard_requests\tunion_cold_gzip_bytes\t"
        "independent_cold_requests\tindependent_cold_shard_requests\t"
        "independent_cold_gzip_bytes\troot_gzip_bytes",
    ]
    for row in rows:
        columns = (
            "shard_bits",
            "shards",
            "root_sha256",
            "union_cold_requests",
            "union_cold_shard_requests",
            "union_cold_gzip_bytes",
            "independent_cold_requests",
            "independent_cold_shard_requests",
            "independent_cold_gzip_bytes",
            "root_gzip_bytes",
        )
        lines.append("\t".join(str(row[column]) for column in columns))
    output = "\n".join(lines) + "\n"
    if args.output:
        args.output.write_text(output, encoding="utf-8")
    else:
        sys.stdout.write(output)
    for virtual_path in sorted(unmatched):
        print(f"unmatched trace input: {virtual_path}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
