#!/usr/bin/env python3
"""Project a full TeX82 TRIP trace onto its stable command profile."""

import json
import sys
from pathlib import Path


def fail(message: str) -> None:
    raise SystemExit(f"project-tex82-trip-command: {message}")


def main() -> None:
    if len(sys.argv) != 3:
        fail("usage: project-tex82-trip-command.py INPUT OUTPUT")
    source, destination = map(Path, sys.argv[1:])
    lines = source.read_text(encoding="utf-8").splitlines()
    if not lines:
        fail(f"empty input: {source}")

    projected = [json.loads(lines[0])]
    for line in lines[1:]:
        record = json.loads(line)
        semantic = record.get("semantic", {})
        data = semantic.get("data", {})
        if (
            semantic.get("event") == "input"
            and data.get("transition") == "stop"
            and data.get("reason") == "source"
            and data.get("name") == "read_stream"
        ):
            # TeX82 §§360/483 use name=m+1 even when a closed \read stream
            # obtains the line from the terminal. The bounded TRIP profile
            # predates that logical-name distinction and contracts all such
            # one-line returns to the physical terminal source.
            data["name"] = "terminal"
        projected.append(record)

    destination.write_text(
        "".join(json.dumps(record, separators=(",", ":")) + "\n" for record in projected),
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
