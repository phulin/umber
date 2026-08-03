#!/usr/bin/env python3
"""Hermetic regression for the bounded TeX82 TRIP command projection."""

import json
import subprocess
import tempfile
from pathlib import Path


root = Path(__file__).resolve().parent.parent
header = {"schema": 1, "manifest": "0" * 64}
events = [
    {"sequence": 0, "semantic": {"event": "input", "data": {"transition": "stop", "reason": "source", "name": "read_stream"}}},
    {"sequence": 1, "semantic": {"event": "input", "data": {"transition": "retire", "reason": "source", "name": "read_stream"}}},
    {"sequence": 2, "semantic": {"event": "input", "data": {"transition": "stop", "reason": "source", "name": "terminal"}}},
]

with tempfile.TemporaryDirectory() as directory:
    source = Path(directory) / "raw.jsonl"
    output = Path(directory) / "projected.jsonl"
    source.write_text("".join(json.dumps(row, separators=(",", ":")) + "\n" for row in [header, *events]))
    subprocess.run([root / "scripts/project-tex82-trip-command.py", source, output], check=True)
    rows = [json.loads(line) for line in output.read_text().splitlines()]

assert rows[1]["semantic"]["data"]["name"] == "terminal"
assert rows[2]["semantic"]["data"]["name"] == "read_stream"
assert rows[3]["semantic"]["data"]["name"] == "terminal"
assert [row["sequence"] for row in rows[1:]] == [0, 1, 2]
print("TeX82 TRIP command projection tests passed")
