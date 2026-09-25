"""Audit paper-run inputs against the locked source and selected TeX Live year."""

from __future__ import annotations

from functools import lru_cache
from pathlib import Path, PurePosixPath

from arxiv_corpus import archive_file_bytes, archive_members, sha256_file
from latex_input_admissions import read_receipt
from texlive import ahash64_bytes, ahash64_file


def fail(message: str) -> None:
    raise SystemExit(message)


@lru_cache(maxsize=4)
def runtime_names(runtime: Path) -> dict[str, tuple[Path, ...]]:
    """Index the already verified snapshot inventory for extra Umber reads."""
    names: dict[str, list[Path]] = {}
    inventory = runtime.parent / "runtime.files"
    root = runtime.parent
    for line in inventory.read_text(encoding="utf-8").splitlines():
        relative, _, _ = line.split("\t")
        if not relative.startswith("texmf-dist/") or "/tex/latex-dev/" in relative:
            continue
        names.setdefault(relative.rsplit("/", 1)[-1], []).append(root / relative)
    return {name: tuple(paths) for name, paths in names.items()}


def request_names(kind: str, name: str) -> tuple[str, ...]:
    """Include TeX's implicit .tex filename when a request omits that suffix."""
    if kind == "tex" and not name.endswith(".tex"):
        return (name, name + ".tex")
    return (name,)


def audit_umber_inputs(row: dict, row_dir: Path, proof: dict, admission: Path) -> dict:
    """Prove common read bytes match and extra reads come from selected inputs."""
    reference_run = row_dir / "reference"
    umber_run = row_dir / "umber"
    runtime = Path(proof["runtime_root"])
    config = Path(proof["generated_config"])
    fontmaps = Path(proof["generated_fontmaps"])
    source_members = {str(member["path"]): member for member in archive_members(row["archive"])}
    entry = source_members[row["entrypoint"]]
    main_path = umber_run / row["entrypoint"]
    if not main_path.is_file() or sha256_file(main_path) != entry["sha256"]:
        fail(f"Umber main input differs from locked source: {row['id']}")
    main, files = read_receipt(admission)
    if main != (entry["bytes"], ahash64_file(main_path)):
        fail(f"Umber main input admission differs from locked source: {row['id']}")
    common: dict[str, set[tuple[int, str]]] = {}
    recorder = reference_run / f"{row['jobname']}.fls"
    events = []
    for line in recorder.read_text(encoding="utf-8").splitlines():
        kind, _, name = line.partition(" ")
        if kind in ("INPUT", "OUTPUT"):
            source = Path(name)
            events.append((kind, (source if source.is_absolute() else reference_run / source).resolve()))
    outputs = {path for kind, path in events if kind == "OUTPUT"}
    written = set()
    checked = set()
    archive_bytes = None
    for kind, path in events:
        if kind == "OUTPUT":
            written.add(path)
            continue
        if path in written or path in checked:
            continue  # Generated reads are not external source admissions.
        checked.add(path)
        observed = None
        if path.is_relative_to(reference_run):
            relative = path.relative_to(reference_run).as_posix()
            member = source_members.get(relative)
            if member is None:
                continue
            if path in outputs:
                # The verified fresh view contained archive bytes at this first
                # read. The final on-disk file is a later generated revision.
                if archive_bytes is None:
                    archive_bytes = archive_file_bytes(row["archive"])
                data = archive_bytes[relative]
                observed = (len(data), ahash64_bytes(data))
            elif sha256_file(path) != member["sha256"]:
                fail(f"reference source input changed during run: {path}")
        elif path.is_relative_to(runtime):
            relative = path.relative_to(runtime).as_posix()
            # verify_snapshot has authenticated the selected runtime.
        elif path.is_relative_to(fontmaps):
            relative = path.relative_to(fontmaps).as_posix()
        elif path == config / "language.dat":
            relative = "language.dat"  # The format authority checked this input.
        else:
            continue  # The reference format itself is separately authenticated.
        if observed is None:
            observed = (path.stat().st_size, ahash64_file(path))
        parts = PurePosixPath(relative).parts
        for index in range(len(parts)):
            common.setdefault("/".join(parts[index:]), set()).add(observed)
    extras = []
    matched = 0
    ambiguous = 0
    for status, key, observed in files:
        if status != "used":
            continue
        kind, name = key.split(":", 1)
        request = PurePosixPath(name)
        if (request.is_absolute() or not name or ".." in request.parts
                or request.as_posix() != name):
            fail(f"Umber used unsafe request name {key}: {row['id']}")
        names = request_names(kind, name)
        basenames = tuple(PurePosixPath(candidate).name for candidate in names)
        expected = next((common[candidate] for candidate in
                         (names[0], basenames[0], *names[1:], *basenames[1:])
                         if candidate in common), None)
        if expected is not None:
            if observed not in expected:
                fail(f"Umber consumed {key} with bytes different from reference: {row['id']}")
            if len(expected) == 1:
                matched += 1
            else:
                ambiguous += 1
            continue
        candidates = [umber_run / relative for relative in source_members
                      if any(relative == candidate or relative.endswith("/" + candidate)
                             for candidate in names)]
        for basename in basenames:
            candidates.extend(runtime_names(runtime).get(basename, ()))
        candidates.extend(fontmaps.rglob(basenames[0]))
        if basenames[0] == "language.dat":
            candidates.append(config / "language.dat")
        selected = next((path for path in candidates
                         if path.is_file() and not path.is_symlink()
                         and path.stat().st_size == observed[0]
                         and ahash64_file(path) == observed[1]), None)
        if selected is None:
            fail(f"Umber consumed {key} outside selected source/runtime: {row['id']}")
        if selected.is_relative_to(umber_run):
            relative = selected.relative_to(umber_run).as_posix()
            if sha256_file(selected) != source_members[relative]["sha256"]:
                fail(f"Umber extra source input changed during run: {selected}")
        extras.append({"key": key, "path": str(selected), "bytes": observed[0],
                       "sha256": sha256_file(selected)})
    return {"common_reads": matched, "ambiguous_common_reads": ambiguous,
            "selected_extra_reads": extras}
