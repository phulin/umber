#!/usr/bin/env python3
"""Check consumed format resources against a SHA-verified construction closure."""

import argparse
from pathlib import Path
import re
import sys


AHASH64 = re.compile(r"[0-9a-f]{16}\Z")
KEY = re.compile(r"[a-z][a-z0-9-]*:[^\t\r\n]+\Z")


def fail(message: str) -> None:
    raise ValueError(message)


def identity(raw_bytes: str, raw_ahash64: str) -> tuple[int, str]:
    if (
        not raw_bytes.isascii()
        or not raw_bytes.isdecimal()
        or (raw_bytes.startswith("0") and raw_bytes != "0")
    ):
        fail(f"invalid byte length: {raw_bytes!r}")
    if not AHASH64.fullmatch(raw_ahash64):
        fail(f"invalid aHash64: {raw_ahash64!r}")
    return int(raw_bytes), raw_ahash64


def read_authorized(path: Path) -> dict[str, tuple[int, str]]:
    authorized: dict[str, tuple[int, str]] = {}
    for line_number, line in enumerate(path.read_text().splitlines(), 1):
        fields = line.split("\t")
        if len(fields) != 3:
            fail(f"{path}:{line_number}: expected key, aHash64, bytes")
        key, ahash64, raw_bytes = fields
        if not KEY.fullmatch(key) or key in authorized:
            fail(f"{path}:{line_number}: invalid or duplicate authorized key {key!r}")
        authorized[key] = identity(raw_bytes, ahash64)
    return authorized


def verify(receipt: Path, authorized: dict[str, tuple[int, str]], main_key: str) -> None:
    lines = receipt.read_text().splitlines()
    if len(lines) < 2 or lines[0] != "umber-input-admissions-v1":
        fail(f"{receipt}: missing input-admissions v1 header")
    main = lines[1].split("\t")
    if len(main) != 3 or main[0] != "main":
        fail(f"{receipt}: missing main input identity")
    if main_key not in authorized or identity(main[1], main[2]) != authorized[main_key]:
        fail(f"{receipt}: main input differs from locked {main_key}")

    seen: set[str] = set()
    for line_number, line in enumerate(lines[2:], 3):
        fields = line.split("\t")
        if len(fields) != 5 or fields[0] != "file" or fields[1] not in ("used", "admitted"):
            fail(f"{receipt}:{line_number}: malformed file admission")
        _, status, key, raw_bytes, ahash64 = fields
        if not KEY.fullmatch(key) or key in seen:
            fail(f"{receipt}:{line_number}: invalid or duplicate file key {key!r}")
        seen.add(key)
        observed = identity(raw_bytes, ahash64)
        if status == "used" and authorized.get(key) != observed:
            fail(f"{receipt}:{line_number}: consumed {key} is outside the locked source closure")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--authorized", type=Path, required=True)
    parser.add_argument("--main-key", required=True)
    args = parser.parse_args()
    try:
        verify(args.receipt, read_authorized(args.authorized), args.main_key)
    except (OSError, UnicodeError, ValueError) as error:
        print(f"verify-latex-format-inputs.py: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
