#!/usr/bin/env python3
"""Negative contracts for selected Rust suite inventory reconciliation."""

import importlib.util
import sys
from dataclasses import replace
from pathlib import Path


source = Path(__file__).with_name("check-selected-rust-suites.py")
spec = importlib.util.spec_from_file_location("selected_rust_suites", source)
module = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = module
spec.loader.exec_module(module)


def row(selector="tests::manual", match="exact"):
    return module.Suite(
        "host-ignored", "example", "lib", selector, match, "-",
        "reference-compatibility", "manual",
        f"cargo test -q -p example --lib {selector} -- "
        + ("--exact " if match == "exact" else "") + "--ignored",
        "pinned-fixtures", "ignored-manual",
    )


def rejects(callback, expected):
    try:
        callback()
    except ValueError as error:
        assert expected in str(error), (expected, str(error))
    else:
        raise AssertionError(f"accepted invalid inventory: {expected}")


declared = row()
assert module.parse_inventory("\t".join(declared.__dict__.values())) == [declared]
rejects(
    lambda: module.parse_inventory("\t".join(declared.__dict__.values()) + "\n" + "\t".join(declared.__dict__.values())),
    "duplicate rows",
)
rejects(
    lambda: module.parse_inventory("\t".join((*list(declared.__dict__.values())[:-3], "cargo test -q -p example", "pinned-fixtures", "ignored-manual"))),
    "command does not select",
)
feature_prefix = replace(
    declared, mode="feature", feature="profiling", match="prefix", lane="subsystem",
    command="scripts/check-tools.sh profiling-command-tests", status="selected-only",
)
rejects(lambda: module.parse_inventory("\t".join(feature_prefix.__dict__.values())), "exact selector")
wasm_subset = replace(
    declared, mode="wasm", lane="subsystem",
    command="scripts/check-wasm.sh wasm-bindgen", status="selected-only",
)
rejects(lambda: module.parse_inventory("\t".join(wasm_subset.__dict__.values())), "select all tests")

actual = {("example", "lib"): {"tests::manual"}}
assert module.reconcile_ignored([declared], actual) == 1
rejects(lambda: module.reconcile_ignored([], actual), "found 0")
rejects(lambda: module.reconcile_ignored([row("tests::", "prefix"), declared], actual), "found 2")
rejects(lambda: module.reconcile_ignored([row("tests::stale")], actual), "found 0")
rejects(lambda: module.reconcile_ignored([declared], {}), "stale inventory selector")
assert module.test_names("tests::manual: test\n1 test, 0 benchmarks\n") == {"tests::manual"}

print("selected Rust suite inventory contracts: PASS")
