#!/usr/bin/env python3
"""Generate the pinned TeX82 module inventory."""

from __future__ import annotations

import hashlib
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parents[1]
SOURCE = ROOT / "third_party/texlive-source/src/texk/web2c/tex.web"
OUT = ROOT / "tests/tex82-properties"
EXPECTED_SHA256 = "c62ab513ef167e93f71a23bd34f311e243210afd7c7a0f9b779614b71e398324"
MODULE = re.compile(r"^@[ *]")
PART = re.compile(r"^@\* \\\[([0-9]+)\] (.*)$")


def fail(message: str) -> None:
    raise SystemExit(f"generate-tex82-property-inventory: {message}")


def write_json(path: pathlib.Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")


def main() -> None:
    source = SOURCE.read_bytes()
    digest = hashlib.sha256(source).hexdigest()
    if digest != EXPECTED_SHA256:
        fail(f"tex.web SHA-256 mismatch: expected {EXPECTED_SHA256}, got {digest}")

    lines = source.decode("utf-8").splitlines()
    starts = [index for index, line in enumerate(lines) if MODULE.match(line)]
    if len(starts) != 1380:
        fail(f"expected 1380 modules, found {len(starts)}")

    inventory = []
    part = 0
    for offset, start in enumerate(starts):
        number = offset + 1
        end = starts[offset + 1] if offset + 1 < len(starts) else len(lines)
        heading = lines[start][2:].strip()
        match = PART.match(lines[start])
        if match:
            part = int(match.group(1))
            heading = match.group(2).strip()
        module_bytes = ("\n".join(lines[start:end]) + "\n").encode()
        inventory.append(
            {
                "module": number,
                "part": part,
                "heading": heading,
                "start_line": start + 1,
                "end_line": end,
                "sha256": hashlib.sha256(module_bytes).hexdigest(),
            }
        )

    OUT.mkdir(parents=True, exist_ok=True)
    write_json(
        OUT / "modules.json",
        {
            "schema": 1,
            "source": "tex.web",
            "source_sha256": digest,
            "module_count": len(inventory),
            "generation_rule": "Each source line beginning @* or @ followed by space starts one WEB module.",
            "modules": inventory,
        },
    )


if __name__ == "__main__":
    main()
