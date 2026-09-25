"""Derived Kpathsea filename database over an authenticated release runtime."""

from __future__ import annotations

import hashlib
import json
import os
import tempfile
from collections import defaultdict
from pathlib import Path

from arxiv_corpus import sha256_file


class ReferenceRuntimeError(ValueError):
    pass


def index_bytes(snapshot: Path) -> bytes:
    directories: dict[str, set[str]] = defaultdict(set)
    directories["."]
    for line in (snapshot / "runtime.files").read_text(encoding="utf-8").splitlines():
        fields = line.split("\t")
        if len(fields) != 3:
            raise ReferenceRuntimeError("invalid selected runtime inventory")
        name = fields[0]
        if (not name.startswith("texmf-dist/")
                or any(part in ("", ".", "..") for part in name.split("/"))
                or any(ord(character) < 32 for character in name)):
            raise ReferenceRuntimeError(f"unsafe runtime inventory path: {name}")
        directory, _, filename = name.rpartition("/")
        if directory not in directories:
            parent = directory
            while parent:
                ancestor, _, child = parent.rpartition("/")
                directories[ancestor or "."].add(child)
                parent = ancestor
        directories[directory].add(filename)
    lines = ["% ls-R derived from authenticated TeX Live runtime.files", ""]
    for directory, names in sorted(directories.items()):
        lines.append(".:" if directory == "." else f"./{directory}:")
        lines.extend(sorted(names))
        lines.append("")
    return ("\n".join(lines) + "\n").encode()


def source_identity(snapshot: Path) -> dict[str, str | int]:
    return {"schema": 1, "runtime_root": str((snapshot / "texmf-dist").resolve()),
            "acquisition_sha256": sha256_file(snapshot / "acquisition.json"),
            "inventory_sha256": sha256_file(snapshot / "runtime.files")}


def verify_reference_runtime(snapshot: Path, record: dict) -> Path:
    root = Path(record["root"])
    if root.is_symlink():
        raise ReferenceRuntimeError("reference runtime root is a symlink")
    expected = source_identity(snapshot)
    if any(record.get(key) != value for key, value in expected.items()):
        raise ReferenceRuntimeError("reference filename database names different source inputs")
    link = root / "texmf-dist"
    if not link.is_symlink() or link.resolve() != (snapshot / "texmf-dist").resolve():
        raise ReferenceRuntimeError("reference runtime link changed")
    index = root / "ls-R"
    if (index.is_symlink() or not index.is_file()
            or index.read_bytes() != index_bytes(snapshot)
            or sha256_file(index) != record.get("index_sha256")):
        raise ReferenceRuntimeError("reference filename database changed")
    if set(path.name for path in root.iterdir()) != {"texmf-dist", "ls-R"}:
        raise ReferenceRuntimeError("unexpected reference runtime files")
    return root


def prepare_reference_runtime(snapshot: Path, output: Path) -> dict:
    """Publish an immutable index without adding files to the source snapshot."""
    snapshot = snapshot.resolve()
    identity = source_identity(snapshot)
    digest = hashlib.sha256(json.dumps(identity, sort_keys=True).encode()).hexdigest()[:20]
    root = output.resolve() / f"reference-runtime-{digest}"
    data = index_bytes(snapshot)
    record = {**identity, "root": str(root), "index_sha256": hashlib.sha256(data).hexdigest()}
    if not root.exists():
        output.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="reference-index.", dir=output) as temporary:
            staged = Path(temporary) / "runtime"
            staged.mkdir()
            (staged / "texmf-dist").symlink_to(snapshot / "texmf-dist", target_is_directory=True)
            (staged / "ls-R").write_bytes(data)
            os.rename(staged, root)
    verify_reference_runtime(snapshot, record)
    return record
