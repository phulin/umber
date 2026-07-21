#!/usr/bin/env python3
"""Select a reproducibly random recent-arXiv candidate order from a snapshot."""

from __future__ import annotations

import argparse
import hashlib
import heapq
import json
import sys
import zipfile
from datetime import date, datetime, timezone
from email.utils import parsedate_to_datetime
from pathlib import Path
from typing import Any


def first_submission(record: dict[str, Any]) -> date:
    versions = record.get("versions")
    if not isinstance(versions, list) or not versions:
        raise ValueError("record has no versions")
    created = versions[0].get("created")
    if not isinstance(created, str):
        raise ValueError("first version has no creation date")
    timestamp = parsedate_to_datetime(created)
    if timestamp.tzinfo is None:
        timestamp = timestamp.replace(tzinfo=timezone.utc)
    return timestamp.astimezone(timezone.utc).date()


def shuffle_key(seed: str, identifier: str) -> bytes:
    return hashlib.sha256(f"{seed}\0{identifier}".encode()).digest()


def select(
    snapshot: Path, start: date, end: date, seed: str, limit: int
) -> tuple[list[tuple[bytes, str, str, date]], int, int]:
    selected: list[tuple[int, str, str, date, bytes]] = []
    rows = 0
    eligible = 0
    with zipfile.ZipFile(snapshot) as archive:
        members = [member for member in archive.infolist() if not member.is_dir()]
        if len(members) != 1:
            raise ValueError(f"expected one metadata member, found {len(members)}")
        with archive.open(members[0]) as source:
            for line_number, line in enumerate(source, 1):
                rows += 1
                try:
                    record = json.loads(line)
                    submitted = first_submission(record)
                    identifier = record["id"]
                    categories = record["categories"]
                except (KeyError, TypeError, ValueError, json.JSONDecodeError) as error:
                    raise ValueError(f"invalid metadata row {line_number}: {error}") from error
                if not start <= submitted <= end:
                    continue
                if not isinstance(identifier, str) or not isinstance(categories, str):
                    raise ValueError(f"invalid metadata row {line_number}: non-string fields")
                eligible += 1
                digest = shuffle_key(seed, identifier)
                inverse = -int.from_bytes(digest, "big")
                item = (inverse, identifier, categories, submitted, digest)
                if len(selected) < limit:
                    heapq.heappush(selected, item)
                elif inverse > selected[0][0]:
                    heapq.heapreplace(selected, item)
    ordered = sorted(
        (
            (digest, identifier, categories, submitted)
            for _, identifier, categories, submitted, digest in selected
        ),
        key=lambda item: (item[0], item[1]),
    )
    return ordered, rows, eligible


def parse_date(value: str) -> date:
    try:
        return datetime.strptime(value, "%Y-%m-%d").date()
    except ValueError as error:
        raise argparse.ArgumentTypeError(str(error)) from error


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("snapshot", type=Path, help="arxiv-metadata-oai-snapshot ZIP")
    parser.add_argument("--start", required=True, type=parse_date)
    parser.add_argument("--end", required=True, type=parse_date)
    parser.add_argument("--seed", default="umber-recent-arxiv-v1")
    parser.add_argument("--limit", type=int, default=500)
    parser.add_argument("--output", type=Path, help="write TSV instead of stdout")
    args = parser.parse_args()
    if args.start > args.end:
        parser.error("--start must not follow --end")
    if args.limit < 1:
        parser.error("--limit must be positive")
    if not args.snapshot.is_file():
        parser.error(f"snapshot not found: {args.snapshot}")

    try:
        selected, rows, eligible = select(
            args.snapshot, args.start, args.end, args.seed, args.limit
        )
    except (OSError, ValueError, zipfile.BadZipFile) as error:
        parser.error(str(error))
    lines = ["id\tcategories\tfirst_submitted\tshuffle_sha256"]
    for digest, identifier, categories, submitted in selected:
        lines.append(f"{identifier}\t{categories}\t{submitted}\t{digest.hex()}")
    output = "\n".join(lines) + "\n"
    if args.output:
        args.output.write_text(output, encoding="utf-8")
    else:
        sys.stdout.write(output)
    print(
        f"metadata_rows={rows} eligible={eligible} selected={len(selected)}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
