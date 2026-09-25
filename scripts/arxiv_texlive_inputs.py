"""Audit paper-run inputs against the locked source and selected TeX Live year."""

from __future__ import annotations

from functools import lru_cache
from pathlib import Path, PurePosixPath

from arxiv_corpus import archive_members, sha256_file
from latex_input_admissions import read_receipt
from texlive import ahash64_file


def fail(message: str) -> None:
    raise SystemExit(message)


@lru_cache(maxsize=4)
def runtime_names(runtime: Path) -> dict[str, tuple[Path, ...]]:
    """Index the already verified snapshot inventory for extra Umber reads."""
    names: dict[str, list[Path]] = {}
    inventory = runtime.parent / "runtime.files"
    for line in inventory.read_text(encoding="utf-8").splitlines():
        relative, _, _ = line.split("\t")
        if not relative.startswith("texmf-dist/") or "/tex/latex-dev/" in relative:
            continue
        path = runtime.parent / relative
        names.setdefault(path.name, []).append(path)
    return {name: tuple(paths) for name, paths in names.items()}


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
    for line in recorder.read_text(encoding="utf-8").splitlines():
        if not line.startswith("INPUT "):
            continue
        source = Path(line[6:])
        path = (source if source.is_absolute() else reference_run / source).resolve()
        if path.is_relative_to(reference_run):
            relative = path.relative_to(reference_run).as_posix()
            member = source_members.get(relative)
            if member is None:
                continue  # Generated auxiliary, excluded from external admissions.
            if sha256_file(path) != member["sha256"]:
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
        _, name = key.split(":", 1)
        request = PurePosixPath(name)
        if (request.is_absolute() or not name or ".." in request.parts
                or request.as_posix() != name):
            fail(f"Umber used unsafe request name {key}: {row['id']}")
        basename = request.name
        expected = common.get(name, common.get(basename))
        if expected is not None:
            if observed not in expected:
                fail(f"Umber consumed {key} with bytes different from reference: {row['id']}")
            if len(expected) == 1:
                matched += 1
            else:
                ambiguous += 1
            continue
        candidates = [umber_run / relative for relative in source_members
                      if relative == name or relative.endswith("/" + name)]
        candidates.extend(runtime_names(runtime).get(basename, ()))
        candidates.extend(fontmaps.rglob(basename))
        if basename == "language.dat":
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
