#!/usr/bin/env python3
"""Reconcile explicitly selected Rust suites with Cargo and libtest discovery."""

import argparse
import json
import re
import shlex
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
INVENTORY = ROOT / "scripts/selected-rust-suite-inventory.tsv"
CLASSES = {
    "local-rules",
    "reference-compatibility",
    "state-lifecycle",
    "formats-outputs",
    "product-platform",
    "limits-performance",
    "test-tool-integrity",
}


@dataclass(frozen=True)
class Suite:
    mode: str
    package: str
    target: str
    selector: str
    match: str
    feature: str
    case_class: str
    lane: str
    command: str
    prerequisites: str
    status: str

    def selects(self, name: str) -> bool:
        if self.match == "all":
            return True
        if self.match == "prefix":
            return name.startswith(self.selector)
        return name == self.selector


def parse_inventory(source: str) -> list[Suite]:
    rows = []
    for line_number, raw in enumerate(source.splitlines(), 1):
        if not raw or raw.startswith("#"):
            continue
        fields = raw.split("\t")
        if len(fields) != 11:
            raise ValueError(f"inventory line {line_number}: expected 11 tab-separated fields")
        row = Suite(*fields)
        if row.mode not in {"host-ignored", "feature", "wasm"}:
            raise ValueError(f"inventory line {line_number}: unknown mode {row.mode}")
        if row.match not in {"exact", "prefix", "all"} or (
            row.match == "all" and row.selector != "*"
        ) or (row.match != "all" and not row.selector):
            raise ValueError(f"inventory line {line_number}: invalid selector/match")
        if row.case_class not in CLASSES:
            raise ValueError(f"inventory line {line_number}: unknown class {row.case_class}")
        if not row.prerequisites or not row.status:
            raise ValueError(f"inventory line {line_number}: missing prerequisite/status")
        expected_lane = "manual" if row.mode == "host-ignored" else "subsystem"
        if row.lane != expected_lane:
            raise ValueError(f"inventory line {line_number}: invalid lane {row.lane}")
        if row.mode == "feature" and row.feature == "-":
            raise ValueError(f"inventory line {line_number}: missing feature")
        if row.mode != "feature" and row.feature != "-":
            raise ValueError(f"inventory line {line_number}: unexpected feature")
        if row.mode == "feature" and row.match != "exact":
            raise ValueError(f"inventory line {line_number}: feature test must use an exact selector")
        if row.mode == "wasm" and (row.match != "all" or row.selector != "*"):
            raise ValueError(f"inventory line {line_number}: WASM target must select all tests")
        if row.mode == "host-ignored":
            target_args = target_arguments(row.target)
            expected = [
                "cargo", "test", "-q", "-p", row.package, *target_args,
                row.selector, "--", *(["--exact"] if row.match == "exact" else []),
                "--ignored",
            ]
            command = shlex.split(row.command)
            if command != expected and command != [*expected, "--nocapture"]:
                raise ValueError(f"inventory line {line_number}: command does not select its row")
            if row.status not in {"ignored-manual", "ignored-helper"}:
                raise ValueError(f"inventory line {line_number}: invalid ignored status")
        elif row.status != "selected-only":
            raise ValueError(f"inventory line {line_number}: invalid optional status")
        rows.append(row)
    if not rows:
        raise ValueError("selected Rust suite inventory is empty")
    if len(set(rows)) != len(rows):
        raise ValueError("selected Rust suite inventory has duplicate rows")
    return rows


def target_arguments(target: str) -> list[str]:
    if target == "lib":
        return ["--lib"]
    if target.startswith("test:") and target[5:]:
        return ["--test", target[5:]]
    if target.startswith("bin:") and target[4:]:
        return ["--bin", target[4:]]
    raise ValueError(f"unsupported selected Rust target {target}")


def metadata() -> dict:
    result = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=ROOT, capture_output=True, text=True, check=True,
    )
    return json.loads(result.stdout)


def validate_optional_rows(rows: list[Suite], packages: dict[str, dict]) -> None:
    for row in rows:
        package = packages.get(row.package)
        if package is None:
            raise ValueError(f"{row.package}: inventory names no Cargo package")
        kind, _, name = row.target.partition(":")
        if not any(
            kind in target["kind"] and (kind == "lib" or target["name"] == name)
            for target in package["targets"]
        ):
            raise ValueError(f"{row.package} {row.target}: Cargo target is absent")
        if row.mode == "feature" and row.feature not in package["features"]:
            raise ValueError(f"{row.package}: feature {row.feature} is absent")
        if row.mode == "host-ignored":
            continue
        command = shlex.split(row.command)
        expected_script = "scripts/check-tools.sh" if row.mode == "feature" else "scripts/check-wasm.sh"
        if len(command) != 2 or command[0] != expected_script:
            raise ValueError(f"{row.package}: optional command is not a named gate step")
        script = (ROOT / command[0]).read_text()
        step = command[1]
        if not re.search(
            r"optional_check_step_requiring\s+(?:\"[^\"]+\"|\S+)\s+"
            + re.escape(step) + r"\b",
            script,
        ):
            raise ValueError(f"{row.package}: optional step {step} is absent")
        if row.mode == "feature":
            if row.selector not in script or row.feature not in script or row.package not in script:
                raise ValueError(f"{row.package}: gate step does not select target test/feature")
        else:
            package_dir = Path(package["manifest_path"]).parent
            if package_dir.relative_to(ROOT).as_posix() not in script:
                raise ValueError(f"{row.package}: WASM step does not select its package")
            target_source = next(
                target["src_path"] for target in package["targets"]
                if kind in target["kind"] and (kind == "lib" or target["name"] == name)
            )
            source = Path(target_source)
            source_text = source.read_text()
            if kind == "lib" and "mod wasm_tests;" in source_text:
                source_text += source.with_name("wasm_tests.rs").read_text()
            if "#[wasm_bindgen_test]" not in source_text:
                raise ValueError(f"{row.package} {row.target}: no WASM test in selected target")


def test_names(output: str) -> set[str]:
    return {line[:-6] for line in output.splitlines() if line.endswith(": test")}


def discover_host_ignored(metadata_value: dict) -> dict[tuple[str, str], set[str]]:
    scratch = ROOT / "target/simplify-suite-index"
    scratch.mkdir(parents=True, exist_ok=True)
    with (scratch / "cargo-stderr.log").open("w") as error_log:
        built = subprocess.run(
            ["cargo", "test", "--tests", "--no-run", "--message-format=json", "-j4"],
            cwd=ROOT, stdout=subprocess.PIPE, stderr=error_log, text=True,
        )
    if built.returncode:
        raise RuntimeError(
            "Cargo test discovery failed; see target/simplify-suite-index/cargo-stderr.log"
        )
    package_names = {package["id"]: package["name"] for package in metadata_value["packages"]}
    discovered: dict[tuple[str, str], set[str]] = {}
    for line in built.stdout.splitlines():
        record = json.loads(line)
        if record.get("reason") != "compiler-artifact" or not record.get("executable") or not record["profile"]["test"]:
            continue
        package = package_names.get(record["package_id"])
        if package is None or package.startswith("bib-"):
            continue  # Bibliography compatibility migration has a separate inventory.
        target = record["target"]
        kind = target["kind"][0]
        selector = "lib" if kind == "lib" else f"{kind}:{target['name']}"
        result = subprocess.run(
            [record["executable"], "--ignored", "--list"],
            cwd=ROOT, capture_output=True, text=True, check=True,
        )
        names = test_names(result.stdout)
        if not names:
            continue
        key = (package, selector)
        if key in discovered:
            raise ValueError(f"multiple compiled test binaries for {key}")
        discovered[key] = names
    return discovered
def reconcile_ignored(rows: list[Suite], discovered: dict[tuple[str, str], set[str]]) -> int:
    indexed = [row for row in rows if row.mode == "host-ignored"]
    count = 0
    for (package, target), names in discovered.items():
        for name in names:
            owners = [row for row in indexed if row.package == package and row.target == target and row.selects(name)]
            if len(owners) != 1:
                raise ValueError(f"{package} {target} {name}: expected one inventory owner, found {len(owners)}")
            count += 1
    for row in indexed:
        if not any(row.selects(name) for name in discovered.get((row.package, row.target), set())):
            raise ValueError(f"{row.package} {row.target} {row.selector}: stale inventory selector")
    return count


def verify_feature(row: Suite) -> None:
    result = subprocess.run(
        ["cargo", "test", "-q", "-j4", "-p", row.package, *target_arguments(row.target),
         "--features", row.feature, "--", "--list"],
        cwd=ROOT, capture_output=True, text=True, check=True,
    )
    names = test_names(result.stdout)
    matches = [name for name in names if row.selects(name) or name.endswith("::" + row.selector)]
    if len(matches) != 1:
        raise ValueError(f"{row.package} {row.target}: feature test {row.selector} resolves to {len(matches)} cases")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify-feature", metavar="PACKAGE")
    args = parser.parse_args()
    rows = parse_inventory(INVENTORY.read_text())
    meta = metadata()
    packages = {package["name"]: package for package in meta["packages"]}
    validate_optional_rows(rows, packages)
    if args.verify_feature:
        selected = [row for row in rows if row.mode == "feature" and row.package == args.verify_feature]
        if not selected:
            raise ValueError(f"{args.verify_feature}: no indexed feature suite")
        for row in selected:
            verify_feature(row)
        print(f"selected Rust feature discovery: PASS ({len(selected)} suites)")
    else:
        count = reconcile_ignored(rows, discover_host_ignored(meta))
        print(f"selected Rust suite inventory: PASS ({count} ignored host tests; {len(rows)} suite rows)")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"selected Rust suite inventory: FAIL ({error})", file=sys.stderr)
        raise SystemExit(1)
