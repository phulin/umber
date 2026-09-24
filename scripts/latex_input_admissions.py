"""Validated semantic resource-admission receipts from the Umber CLI."""

from pathlib import Path
import re


AHASH64 = re.compile(r"[0-9a-f]{16}\Z")
KEY = re.compile(r"[a-z][a-z0-9-]*:[^\t\r\n]+\Z")
Identity = tuple[int, str]
Admission = tuple[str, str, Identity]


def identity(raw_bytes: str, raw_ahash64: str) -> Identity:
    if (
        not raw_bytes.isascii()
        or not raw_bytes.isdecimal()
        or (raw_bytes.startswith("0") and raw_bytes != "0")
    ):
        raise ValueError(f"invalid byte length: {raw_bytes!r}")
    if not AHASH64.fullmatch(raw_ahash64):
        raise ValueError(f"invalid aHash64: {raw_ahash64!r}")
    return int(raw_bytes), raw_ahash64


def read_authorized(path: Path) -> dict[str, Identity]:
    authorized: dict[str, Identity] = {}
    for line_number, line in enumerate(path.read_text().splitlines(), 1):
        fields = line.split("\t")
        if len(fields) != 3:
            raise ValueError(f"{path}:{line_number}: expected key, aHash64, bytes")
        key, ahash64, raw_bytes = fields
        if not KEY.fullmatch(key) or key in authorized:
            raise ValueError(f"{path}:{line_number}: invalid or duplicate authorized key {key!r}")
        authorized[key] = identity(raw_bytes, ahash64)
    return authorized


def read_receipt(path: Path) -> tuple[Identity, list[Admission]]:
    lines = path.read_text().splitlines()
    if len(lines) < 2 or lines[0] != "umber-input-admissions-v1":
        raise ValueError(f"{path}: missing input-admissions v1 header")
    main = lines[1].split("\t")
    if len(main) != 3 or main[0] != "main":
        raise ValueError(f"{path}: missing main input identity")
    main_identity = identity(main[1], main[2])
    seen: set[str] = set()
    admissions: list[Admission] = []
    for line_number, line in enumerate(lines[2:], 3):
        fields = line.split("\t")
        if len(fields) != 5 or fields[0] != "file" or fields[1] not in ("used", "admitted"):
            raise ValueError(f"{path}:{line_number}: malformed file admission")
        _, status, key, raw_bytes, ahash64 = fields
        if not KEY.fullmatch(key) or key in seen:
            raise ValueError(f"{path}:{line_number}: invalid or duplicate file key {key!r}")
        seen.add(key)
        admissions.append((status, key, identity(raw_bytes, ahash64)))
    return main_identity, admissions
