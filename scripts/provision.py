#!/usr/bin/env python3
"""Provision Umber's pinned TeX Live inputs, test assets, and publications."""

from __future__ import annotations

import argparse
import filecmp
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from pathlib import Path

import texlive
import texlive_release

NATIVE_ASSET_LOCK = Path("tests/native-test-assets.lock")
CONFORMANCE_TEXLIVE_LOCK = Path("tests/conformance-texlive.lock")
TRIP_MANIFEST = Path("tests/trip-manifest.txt")
SNAPSHOT_LOCK = Path("tests/texlive-snapshot.lock")
ORACLE_BUILDERS = {
    "trip": "build-trip-initex.sh",
    "tex82": "build-tex82-oracle.sh",
    "etex26": "build-etex26-oracle.sh",
    "pdftex14029": "build-pdftex14029-oracle.sh",
}


class ProvisionError(Exception):
    """Provisioning could not complete without violating a pinned identity."""


def _run(command: list[str], repo_root: Path, environment: dict[str, str] | None = None) -> None:
    try:
        subprocess.run(command, cwd=repo_root, env=environment, check=True)
    except (OSError, subprocess.CalledProcessError) as error:
        raise ProvisionError(f"command failed ({' '.join(command)}): {error}") from error


def _safe_relative(raw: str, label: str) -> Path:
    path = Path(raw)
    if path.is_absolute() or not path.parts or ".." in path.parts or "\\" in raw:
        raise ProvisionError(f"unsafe {label}: {raw}")
    return path


def read_native_asset_lock(repo_root: Path) -> dict[Path, str]:
    lock_path = repo_root / NATIVE_ASSET_LOCK
    assets: dict[Path, str] = {}
    for number, raw_line in enumerate(lock_path.read_text(encoding="utf-8").splitlines(), 1):
        fields = raw_line.split()
        if not fields or fields[0].startswith("#"):
            continue
        if fields[0] == "include-texlive" and len(fields) == 2:
            included = repo_root / _safe_relative(fields[1], "included TeX Live lock")
            for record in texlive.read_runtime_sources(
                included, require_destinations=True
            ):
                assert record.destination is not None
                if record.destination in assets:
                    raise ProvisionError(
                        f"{lock_path}:{number}: duplicate included native asset"
                    )
                assets[record.destination] = record.identity.digest
            continue
        if len(fields) != 2 or not texlive.valid_digest(fields[0], 64):
            raise ProvisionError(f"{lock_path}:{number}: expected SHA-256 and path")
        relative = _safe_relative(fields[1], "native asset path")
        if relative in assets:
            raise ProvisionError(f"{lock_path}:{number}: duplicate native asset")
        assets[relative] = fields[0]
    if not assets:
        raise ProvisionError(f"{lock_path}: asset allowlist is empty")
    return assets


def _verify_native(path: Path, expected: str, role: str) -> None:
    if not path.is_file() or path.is_symlink():
        raise ProvisionError(f"{role} is not a regular file: {path}")
    actual = texlive.hash_file(path)
    if actual != expected:
        raise ProvisionError(
            f"SHA-256 mismatch for {role} {path}: expected {expected}, got {actual}"
        )


def _copy_native(source: Path, destination: Path, expected: str) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{destination.name}.", dir=destination.parent
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as output, source.open("rb") as input_file:
            shutil.copyfileobj(input_file, output)
            output.flush()
            os.fsync(output.fileno())
        _verify_native(temporary, expected, "copied asset")
        os.chmod(temporary, 0o444)
        os.replace(temporary, destination)
    finally:
        temporary.unlink(missing_ok=True)


def _asset_destination(repo_root: Path, relative: Path, target_dir: Path | None) -> Path:
    if target_dir is not None and relative.parts[0] == "target":
        return target_dir.joinpath(*relative.parts[1:])
    return repo_root / relative


def _download_trip_inputs(repo_root: Path, offline: bool) -> None:
    destination_root = repo_root / "third_party/trip"
    destination_root.mkdir(parents=True, exist_ok=True)
    manifest = repo_root / TRIP_MANIFEST
    for number, raw_line in enumerate(manifest.read_text(encoding="utf-8").splitlines(), 1):
        fields = raw_line.split()
        if not fields or fields[0].startswith("#"):
            continue
        if len(fields) < 3 or not texlive.valid_digest(fields[1], 64):
            raise ProvisionError(f"{manifest}:{number}: malformed TRIP input record")
        name = _safe_relative(fields[0], "TRIP input name")
        if len(name.parts) != 1:
            raise ProvisionError(f"{manifest}:{number}: TRIP input must be a basename")
        expected, urls = fields[1], fields[2:]
        if len(urls) != len(set(urls)):
            raise ProvisionError(f"{manifest}:{number}: duplicate input locator")
        if any(not url.startswith(("https://", "http://127.0.0.1:", "http://localhost:")) for url in urls):
            raise ProvisionError(f"{manifest}:{number}: unsafe input locator")
        destination = destination_root / name
        if destination.exists():
            _verify_native(destination, expected, "cached TRIP input")
            continue
        if offline:
            raise ProvisionError(f"missing {destination} while running --offline")
        failures: list[str] = []
        for url in urls:
            temporary = destination.with_name(f".{destination.name}.download")
            try:
                request = urllib.request.Request(url, headers={"User-Agent": "umber-provision/1"})
                with urllib.request.urlopen(request, timeout=60) as response, temporary.open("wb") as output:
                    shutil.copyfileobj(response, output)
                actual = texlive.hash_file(temporary)
                if actual != expected:
                    failures.append(f"{url}: SHA-256 mismatch ({actual})")
                    continue
                os.replace(temporary, destination)
                break
            except (OSError, urllib.error.URLError) as error:
                failures.append(f"{url}: {error}")
            finally:
                temporary.unlink(missing_ok=True)
        else:
            raise ProvisionError(
                f"all locators failed for {name}: {'; '.join(failures)}"
            )


def _materialize_conformance(repo_root: Path, target_dir: Path, offline: bool) -> None:
    snapshot = target_dir / "texlive-snapshot"
    lock = repo_root / CONFORMANCE_TEXLIVE_LOCK
    texlive.materialize_snapshot(snapshot, lock_paths=(lock,), offline=offline)
    texlive.stage_runtime_sources(snapshot, lock, repo_root)


def _generate_primary_assets(repo_root: Path, target_dir: Path, offline: bool) -> None:
    _materialize_conformance(repo_root, target_dir, offline)
    _download_trip_inputs(repo_root, offline)
    environment = os.environ.copy()
    environment["CARGO_TARGET_DIR"] = str(target_dir)
    _run(
        [
            "cargo",
            "build",
            "--manifest-path",
            "tools/fixturegen/Cargo.toml",
            "--target-dir",
            str(target_dir),
        ],
        repo_root,
        environment,
    )
    _run([str(target_dir / "debug/fixturegen"), "--sync-corpus"], repo_root, environment)
    provision_oracles(repo_root, ("pdftex14029",), target_dir, offline)
    pdftex = target_dir / "pdftex14029-oracle/bin/umber-pdftex14029-oracle-instrumented"
    if not pdftex.is_file():
        raise ProvisionError(f"missing built pdfTeX 1.40.29 oracle: {pdftex}")
    environment["UMBER_REF_TEX"] = str(pdftex)
    environment["UMBER_REF_PDFTEX"] = str(pdftex)
    for case in ("story", "gentle", "trip", "etrip"):
        _run(
            [str(repo_root / "scripts/regen-fixtures.sh"), "--case", f"e2e/{case}"],
            repo_root,
            environment,
        )


def provision_worktree(
    path: Path, target_dir: Path | None = None, offline: bool = False
) -> int:
    repo_root = texlive.repository_root(path)
    if target_dir is not None:
        target_dir = target_dir.resolve()
        if not target_dir.is_relative_to(repo_root):
            raise ProvisionError(f"target directory is outside the worktree: {target_dir}")
    texlive.provision_source(repo_root, offline)
    assets = read_native_asset_lock(repo_root)
    missing: list[tuple[Path, str]] = []
    for relative, expected in assets.items():
        destination = _asset_destination(repo_root, relative, target_dir)
        if destination.exists() or destination.is_symlink():
            _verify_native(destination, expected, "existing asset")
        else:
            missing.append((relative, expected))
    if not missing:
        return 0
    primary = texlive.primary_checkout(repo_root)
    if primary == repo_root:
        _generate_primary_assets(repo_root, target_dir or repo_root / "target", offline)
        for relative, expected in missing:
            _verify_native(
                _asset_destination(repo_root, relative, target_dir),
                expected,
                "provisioned primary asset",
            )
        return len(missing)
    absent = [relative for relative, _ in missing if not (primary / relative).is_file()]
    if absent:
        rendered = "\n  ".join(str(path) for path in absent)
        raise ProvisionError(
            f"primary checkout {primary} lacks pinned assets:\n  {rendered}\n"
            f"Run python3 scripts/provision.py worktree {primary} first."
        )
    for relative, expected in missing:
        source = primary / relative
        _verify_native(source, expected, "primary asset")
        _copy_native(source, _asset_destination(repo_root, relative, target_dir), expected)
    return len(missing)


def _publisher_path(repo_root: Path, environment: dict[str, str]) -> Path:
    if "CARGO_TARGET_DIR" in environment:
        target = Path(environment["CARGO_TARGET_DIR"])
        if not target.is_absolute():
            target = repo_root / target
    else:
        target = repo_root / "tools/texlive-wasm-publish/target"
    return target / "release/texlive-wasm-publish"


def _trees_equal(left: Path, right: Path) -> bool:
    comparison = filecmp.dircmp(left, right)
    if comparison.left_only or comparison.right_only or comparison.diff_files or comparison.funny_files:
        return False
    return all(_trees_equal(left / name, right / name) for name in comparison.common_dirs)


def _stage_format_input_root(
    repo_root: Path, texmf_dist: Path, destination: Path
) -> int:
    """Stage the exact locked format closure as the publisher's first root."""
    lock = repo_root / "tests/latex-source.lock"
    staged: set[Path] = set()
    for number, raw_line in enumerate(lock.read_text(encoding="utf-8").splitlines(), 1):
        fields = raw_line.split()
        if not fields or fields[0].startswith("#"):
            continue
        kind = fields[0]
        if kind not in ("source", "local", "pdflatex-source", "pdflatex-local"):
            continue
        if len(fields) != 4:
            raise ProvisionError(f"{lock}:{number}: malformed format input record")
        relative = texlive._safe_relative(fields[1], label="format input path")
        identity = texlive.Identity(int(fields[2]), fields[3])
        if kind in ("source", "pdflatex-source"):
            source = texmf_dist / relative
            staged_relative = relative
        else:
            source = repo_root / relative
            staged_relative = Path("tex") / relative.name
        texlive.verify_file(source, identity, "sha256", "locked format input")
        if staged_relative in staged:
            raise ProvisionError(f"duplicate staged format input path: {staged_relative}")
        staged.add(staged_relative)
        output = destination / staged_relative
        output.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, output)
    if not staged:
        raise ProvisionError(f"{lock}: no format inputs to stage")
    return len(staged)


def provision_oracles(
    repo_root: Path,
    requested: tuple[str, ...],
    target_dir: Path,
    offline: bool,
) -> None:
    """Build pinned reference engines after provisioning their shared source."""
    texlive.provision_source(repo_root, offline)
    names = tuple(ORACLE_BUILDERS) if "all" in requested else requested
    environment = os.environ.copy()
    environment["CARGO_TARGET_DIR"] = str(target_dir)
    for name in names:
        builder = ORACLE_BUILDERS.get(name)
        if builder is None:
            raise ProvisionError(f"unknown oracle: {name}")
        command = [str(repo_root / "scripts" / builder)]
        if offline:
            command.append("--offline")
        _run(command, repo_root, environment)


def build_snapshot(args: argparse.Namespace, repo_root: Path) -> None:
    texmf_dist = args.texmf_dist.resolve()
    if not texmf_dist.is_dir():
        raise ProvisionError(f"missing texmf-dist root: {texmf_dist}")
    distribution, expected_tree = texlive.verify_runtime_tree(
        texmf_dist, repo_root / SNAPSHOT_LOCK
    )
    format_source_distribution = next(
        (
            fields[1]
            for raw_line in (repo_root / "tests/latex-source.lock").read_text().splitlines()
            if (fields := raw_line.split())[:1] == ["distribution"] and len(fields) == 2
        ),
        "",
    )
    if format_source_distribution != distribution:
        raise ProvisionError(
            f"format source distribution {format_source_distribution or '<missing>'} "
            f"differs from snapshot distribution {distribution}"
        )
    format_distribution_ahash64 = next(
        (
            fields[1]
            for raw_line in (repo_root / "tests/latex-source.lock").read_text().splitlines()
            if (fields := raw_line.split())[:1] == ["distribution_ahash64"]
            and len(fields) == 2
        ),
        "",
    )
    if not texlive.valid_digest(format_distribution_ahash64, 16):
        raise ProvisionError(
            "format source lock has no published distribution aHash64; see umber2-66p0.27"
        )
    if args.format_distribution_ahash64 is not None:
        format_distribution_ahash64 = args.format_distribution_ahash64
    if not texlive.valid_digest(format_distribution_ahash64, 16):
        raise ProvisionError("invalid format distribution aHash64")
    format_distribution = (
        args.format_distribution or repo_root / "target/texlive-snapshot"
    ).resolve()
    package_database = args.package_database
    if not args.without_package_database:
        package_database = package_database or texmf_dist.parent / "tlpkg/texlive.tlpdb"
        if not package_database.is_file():
            raise ProvisionError(f"missing TeX Live package database: {package_database}")
    elif package_database is not None:
        raise ProvisionError("--without-package-database conflicts with --package-database")
    pdftex_map = args.pdftex_map
    if pdftex_map is None:
        candidates = (
            texmf_dist.parent / "texmf-var/fonts/map/pdftex/updmap/pdftex.map",
            texmf_dist / "fonts/map/pdftex/updmap/pdftex.map",
        )
        pdftex_map = next((path for path in candidates if path.is_file()), None)
    if pdftex_map is None or not pdftex_map.is_file():
        raise ProvisionError("missing generated pdfTeX map")
    if not args.objects_base_url.startswith("https://") or not args.objects_base_url.endswith("/"):
        raise ProvisionError("objects base URL must use HTTPS and end with /")
    environment = os.environ.copy()
    with tempfile.TemporaryDirectory(prefix="umber-texlive-snapshot.") as raw_temporary:
        temporary = Path(raw_temporary)
        format_root = temporary / "formats"
        for engine in ("latex", "pdflatex"):
            _run(
                [
                    str(repo_root / "scripts/build-latex-format.sh"),
                    "--engine",
                    engine,
                    "--publish-input-closure",
                    "--texmf-dist",
                    str(texmf_dist),
                    "--distribution",
                    str(format_distribution),
                    "--distribution-ahash64",
                    format_distribution_ahash64,
                    "--output-dir",
                    str(format_root / engine),
                ],
                repo_root,
                environment,
            )
        format_input_root = temporary / "format-construction-inputs"
        _stage_format_input_root(repo_root, texmf_dist, format_input_root)
        generated_root = temporary / "generated-runtime"
        generated_map = generated_root / "fonts/map/pdftex/updmap/pdftex.map"
        generated_map.parent.mkdir(parents=True)
        shutil.copyfile(pdftex_map, generated_map)
        _run(
            ["cargo", "build", "-q", "--release", "--manifest-path", "tools/texlive-wasm-publish/Cargo.toml"],
            repo_root,
            environment,
        )
        publisher = _publisher_path(repo_root, environment)

        def tree_hash(path: Path) -> str:
            return subprocess.run(
                [str(publisher), "--tree-ahash64", str(path)],
                cwd=repo_root,
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()

        actual_tree = tree_hash(texmf_dist)
        if actual_tree != expected_tree:
            raise ProvisionError(
                f"texmf-dist tree differs from lock: expected {expected_tree}, got {actual_tree}"
            )
        config = {
            "schema": 6,
            "distribution": distribution,
            "objectsBaseUrl": args.objects_base_url,
            "shardBits": args.shard_bits,
            "roots": [
                {
                    "name": "format-construction-inputs",
                    "path": str(format_input_root),
                    "treeAhash64": tree_hash(format_input_root),
                },
                {"name": "texlive-runtime", "path": str(texmf_dist), "treeAhash64": actual_tree},
                {"name": "texlive-generated-runtime", "path": str(generated_root), "treeAhash64": tree_hash(generated_root)},
            ],
            "inventory": {"minimumLogicalFiles": 100000, "minimumObjects": 50000, "minimumBytes": 1000000000},
            "formats": [
                {
                    "path": str(format_root / engine / f"{engine}.fmt"),
                    "metadata": str(format_root / engine / f"{engine}-format.json"),
                    "inputIdentities": str(format_root / engine / f"{engine}-input-identities.json"),
                }
                for engine in ("latex", "pdflatex")
            ],
        }
        if package_database is not None:
            config["packageDatabase"] = str(package_database.resolve())
        config_path = temporary / "publish.json"
        config_path.write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")
        first = temporary / "first"
        _run([str(publisher), str(config_path), str(first)], repo_root)
        _run([str(publisher), str(config_path), str(args.output_dir.resolve())], repo_root)
        if not _trees_equal(first, args.output_dir.resolve()):
            raise ProvisionError("two clean snapshot publications differ")


def _add_materialize_options(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--root-url", default=texlive.DEFAULT_ROOT_URL)
    parser.add_argument("--root-ahash64", default=texlive.DEFAULT_ROOT_AHASH64)
    parser.add_argument("--root-path", type=Path)
    parser.add_argument("--object-root", action="append", type=Path, default=[])
    parser.add_argument("--texmf-root", action="append", type=Path, default=[])
    parser.add_argument("--output-dir", type=Path, default=Path("target/texlive-snapshot"))
    parser.add_argument("--format", action="append", default=[])
    parser.add_argument("--key", action="append", default=[])
    parser.add_argument("--keys-from", action="append", type=Path, default=[])
    parser.add_argument("--offline", action="store_true")


def parse_args(arguments: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    source = commands.add_parser("source", help="provision the shared 2026 source tree")
    source.add_argument("path", type=Path, nargs="?", default=Path.cwd())
    source.add_argument("--offline", action="store_true")
    runtime_source = commands.add_parser(
        "runtime-source", help="provision the complete pinned 2026 runtime source"
    )
    runtime_source.add_argument("path", type=Path, nargs="?", default=Path.cwd())
    runtime_source.add_argument(
        "--mirror",
        action="append",
        default=[],
        help="mirror directory containing a pinned release artifact; may be repeated",
    )
    runtime_source.add_argument("--offline", action="store_true")
    worktree = commands.add_parser("worktree", help="provision primary or linked-worktree tests")
    worktree.add_argument("path", type=Path)
    worktree.add_argument("--target-dir", type=Path)
    worktree.add_argument("--offline", action="store_true")
    materialize = commands.add_parser(
        "materialize", help="materialize a metadata-complete execution mirror"
    )
    _add_materialize_options(materialize)
    oracle = commands.add_parser("oracle", help="build pinned TeX Live 2026 reference engines")
    oracle.add_argument(
        "name",
        choices=(*ORACLE_BUILDERS, "all"),
        nargs="+",
        help="reference engine(s) to build",
    )
    oracle.add_argument("--target-dir", type=Path, default=Path("target"))
    oracle.add_argument("--offline", action="store_true")
    snapshot = commands.add_parser("snapshot", help="build the publisher snapshot")
    snapshot.add_argument("--texmf-dist", type=Path, required=True)
    snapshot.add_argument("--pdftex-map", type=Path)
    snapshot.add_argument("--package-database", type=Path)
    snapshot.add_argument("--without-package-database", action="store_true")
    snapshot.add_argument("--format-distribution", type=Path)
    snapshot.add_argument("--format-distribution-ahash64")
    snapshot.add_argument("--output-dir", type=Path, default=Path("target/texlive-snapshot"))
    snapshot.add_argument("--objects-base-url", default="https://example.invalid/umber/texlive/objects/")
    snapshot.add_argument("--shard-bits", type=int, choices=range(17), default=8)
    return parser.parse_args(arguments)


def main(arguments: list[str] | None = None) -> int:
    args = parse_args(arguments)
    repo_root = texlive.repository_root(getattr(args, "path", Path.cwd()))
    if args.command == "source":
        print(texlive.provision_source(repo_root, args.offline))
    elif args.command == "runtime-source":
        source_root = texlive_release.ensure_runtime_source(
            repo_root, tuple(args.mirror), args.offline
        )
        print(f"provision: runtime source ready at {source_root}")
    elif args.command == "worktree":
        copied = provision_worktree(args.path, args.target_dir, args.offline)
        print(f"provision: PASS: {copied} asset(s) provisioned into {repo_root}")
    elif args.command == "materialize":
        result = texlive.materialize_snapshot(
            args.output_dir,
            root_url=args.root_url,
            root_ahash64=args.root_ahash64,
            source_root_path=args.root_path,
            object_roots=tuple(args.object_root),
            texmf_roots=tuple(args.texmf_root),
            formats_requested=tuple(args.format),
            keys=tuple(args.key),
            lock_paths=tuple(args.keys_from),
            offline=args.offline,
        )
        print("TeX Live execution mirror: " + " ".join(f"{key}={value}" for key, value in result.items()))
    elif args.command == "oracle":
        provision_oracles(repo_root, tuple(args.name), args.target_dir.resolve(), args.offline)
        print(f"provision: built {', '.join(args.name)} oracle(s)")
    else:
        build_snapshot(args, repo_root)
        print(f"provision: snapshot built at {args.output_dir.resolve()}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, texlive.TexliveError, ProvisionError) as error:
        print(f"provision.py: {error}", file=sys.stderr)
        sys.exit(1)
