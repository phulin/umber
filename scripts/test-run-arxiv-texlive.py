#!/usr/bin/env python3
"""Hermetic declared-year routing and DVI corpus evidence contract."""

from __future__ import annotations

import importlib.util
import datetime as dt
import json
import os
import tarfile
import tempfile
import types
import unittest
from pathlib import Path
from unittest.mock import patch

from arxiv_corpus import declared_texlive, sha256_file
from texlive import ahash64_file

SCRIPT = Path(__file__).with_name("run-arxiv-texlive.py")
spec = importlib.util.spec_from_file_location("arxiv_texlive_runner", SCRIPT)
assert spec is not None and spec.loader is not None
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


def executable(path: Path, body: str) -> None:
    path.write_text("#!/bin/sh\nset -eu\n" + body)
    path.chmod(0o755)


def dvi(path: Path, marker: int) -> None:
    path.write_bytes(bytes([248]) + bytes(26) + bytes([0, 1, marker]) +
                     bytes([249, 0, 0, 0, 0, 2, 223, 223, 223, 223]))


def archive(path: Path, paper: str, compiler: str, year: str | None) -> None:
    source = path.parent / (paper + "-source")
    source.mkdir()
    (source / f"{paper}.tex").write_text("\\bye\n")
    (source / "side.bbl").write_text("archive side input\n")
    declaration = {"process": {"compiler": compiler}}
    if year is not None:
        declaration["texlive_version"] = year
    (source / "00README.json").write_text(json.dumps(declaration))
    with tarfile.open(path, "w:gz") as target:
        for member in source.iterdir():
            target.add(member, arcname=member.name)


class DeclaredYearCorpus(unittest.TestCase):
    def test_declaration_is_required_and_strict(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / "paper.src"
            archive(path, "paper", "pdflatex", None)
            with self.assertRaisesRegex(ValueError, "invalid 00README"):
                declared_texlive(path)
            path.unlink()
            archive(path, "paper2", "pdflatex", "2022")
            with self.assertRaisesRegex(ValueError, "unsupported declared TeX Live year"):
                declared_texlive(path)
            path.unlink()
            archive(path, "paper3", "xelatex", "2025")
            self.assertEqual(declared_texlive(path)[:2], ("xelatex", "2025"))

    def test_dvi_qualification_precedes_parity_and_resume_checks_evidence(self) -> None:
        with tempfile.TemporaryDirectory(prefix="arxiv-year-contract-") as temporary:
            root = Path(temporary)
            archives = root / "archives"
            archives.mkdir()
            papers = [("latex23", "latex", "2023"),
                      ("xe25", "xelatex", "2025"),
                      ("pdf25", "pdflatex", "2025")]
            lock = root / "sample.tsv"
            lock.write_text("id\tsource_sha256\tsource_bytes\tfirst_submitted\tshuffle_sha256\tentrypoint\n")
            for paper, engine, year in papers:
                source = archives / f"{paper}.src"
                archive(source, paper, engine, year)
                with lock.open("a") as output:
                    output.write(f"{paper}\t{sha256_file(source)}\t{source.stat().st_size}\t2025-01-01\t"
                                 f"{'0' * 64}\t{paper}.tex\n")
            expected, different = root / "expected.dvi", root / "different.dvi"
            dvi(expected, 0)
            dvi(different, 1)
            count = root / "invocations"
            oracle = root / "oracle"
            executable(oracle, """
for arg in "$@"; do input=$arg; done
name=${input%.tex}
test -f side.bbl
printf 'reference %s %s\\n' "$name" "$TEXMFDIST" >> "$TEST_COUNTS"
cp "$TEST_DVI" "$name.dvi"
printf 'PWD %s\nINPUT %s\n' "$PWD" "$PWD/$name.tex" > "$name.fls"
""")
            umber = root / "umber"
            executable(umber, """
previous=
for arg in "$@"; do
  if test "$previous" = output; then output=$arg; fi
  case "$arg" in --dvi) previous=output;; *) previous=;; esac
done
input=$arg
name=${input%.tex}
test -f side.bbl
printf 'umber %s %s\\n' "$name" "$TEXINPUTS" >> "$TEST_COUNTS"
if test "$name" = latex23; then cp "$TEST_DIFFERENT" "$output";
else cp "$TEST_DVI" "$output"; fi
""")
            parity = root / "parity"
            executable(parity, """
test "$1" = --compare-existing-dvi
printf 'compare %s\\n' "$5" >> "$TEST_COUNTS"
if ! cmp -s "$2" "$3"; then
  echo "$5 DVI mismatch at byte 10 on page 1 (a != b)" >&2
  exit 1
fi
""")
            years = {}
            for year, engine in (("2023", "latex"), ("2025", "pdflatex")):
                runtime = root / year / "texmf-dist"
                (runtime / "web2c").mkdir(parents=True)
                for name in ("tex/latex/base/latex.ltx", "tex/latex/l3kernel/expl3-code.tex"):
                    path = runtime / name
                    path.parent.mkdir(parents=True)
                    path.write_text(name + "\n")
                config = root / year / "formats/generated-config/language.dat"
                config.parent.mkdir(parents=True)
                config.write_text("=english\n")
                (runtime / "web2c" / "sentinel").write_text("release input\n")
                (root / year / "acquisition.json").write_text(json.dumps({
                    "snapshot_date": f"{year}-01-01",
                    "sentinel_sha256": sha256_file(runtime / "web2c" / "sentinel")}))
                epoch = int(dt.datetime(int(year), 1, 1, tzinfo=dt.timezone.utc).timestamp())
                reference_fmt = root / year / f"{engine}.fmt"
                reference_fmt.write_text("reference format\n")
                reference_receipt = root / year / f"{engine}-reference.json"
                reference_receipt.write_text(json.dumps({"engine": {"sha256": sha256_file(oracle)},
                                                         "format": {"sha256": sha256_file(reference_fmt)},
                                                         "source_date_epoch": epoch,
                                                         "inputs": [{"path": str(path),
                                                                     "bytes": path.stat().st_size,
                                                                     "sha256": sha256_file(path)} for path in
                                                                    (runtime / "tex/latex/base/latex.ltx",
                                                                     runtime / "tex/latex/l3kernel/expl3-code.tex",
                                                                     config)]}))
                umber_fmt = root / year / f"{engine}.umberfmt"
                umber_fmt.write_text("umber format\n")
                admission = root / year / f"formats/umber-{engine}-work/build.inputs"
                admission.parent.mkdir(parents=True)
                admission.write_text("{}\n")
                umber_receipt = root / year / f"{engine}-umber.json"
                umber_receipt.write_text(json.dumps({"format": {"sha256": sha256_file(umber_fmt)},
                                                     "engine": engine, "source_date_epoch": epoch,
                                                     "input_admissions": str(admission),
                                                     "input_admissions_sha256": sha256_file(admission)}))
                distribution = root / year / "distribution"
                distribution.mkdir()
                manifest = distribution / "manifest.json"
                manifest.write_text(json.dumps({"schema": 8, "formats": {}}))
                years[year] = {"runtime_root": str(runtime),
                               "runtime_receipt": str(root / year / "acquisition.json"),
                               "formats": {engine: {"reference_binary": str(oracle),
                                                   "reference_format": str(reference_fmt),
                                                   "reference_format_receipt": str(reference_receipt),
                                                   "umber_distribution": str(distribution),
                                                   "umber_format": str(umber_fmt),
                                                   "umber_format_sha256": sha256_file(umber_fmt),
                                                   "umber_format_receipt": str(umber_receipt),
                                                   "source_date_epoch": epoch,
                                                   "distribution_ahash64": ahash64_file(manifest)}}}
            preparation = root / "preparation.json"
            preparation.write_text(json.dumps({"schema": 1, "years": years}))
            results = root / "results"
            args = [str(SCRIPT), "--source-lock", str(lock), "--archives", str(archives),
                    "--preparation", str(preparation), "--umber", str(umber),
                    "--parity-harness", str(parity), "--results", str(results),
                    "--expected-rows", "3", "--timeout-seconds", "10", "--max-rss-mib", "128"]
            environment = {"TEST_DVI": str(expected), "TEST_DIFFERENT": str(different),
                           "TEST_COUNTS": str(count)}
            def verify_snapshot(year: int, cache_root: Path):
                year_root = cache_root / str(year)
                receipt = year_root / "acquisition.json"
                expected_hash = json.loads(receipt.read_text())["sentinel_sha256"]
                if sha256_file(year_root / "texmf-dist/web2c/sentinel") != expected_hash:
                    raise ValueError("runtime inventory changed")
                return types.SimpleNamespace(root=year_root, receipt=receipt)

            snapshot_module = types.ModuleType("texlive_snapshot")
            snapshot_module.verify_snapshot = verify_snapshot
            with patch.object(runner.sys, "argv", args), patch.dict(os.environ, environment), \
                 patch.dict(runner.sys.modules, {"texlive_snapshot": snapshot_module}):
                self.assertEqual(runner.main(), 1)
                observed = count.read_text().splitlines()
                self.assertTrue(observed[0].startswith(f"reference latex23 {root / '2023' / 'texmf-dist'}"))
                self.assertTrue(observed[1].startswith(f"reference pdf25 {root / '2025' / 'texmf-dist'}"))
                self.assertTrue(observed[2].startswith("umber latex23 "))
                self.assertEqual(observed[3], "compare latex23")
                self.assertEqual(len(observed), 4)
                self.assertEqual(json.loads((results / "summary.json").read_text())["counts"], {
                    "DVI-diverged": 1, "DVI-qualified": 1, "unsupported-xelatex": 1})
                with patch.object(runner.sys, "argv", args + ["--verify-only"]):
                    self.assertEqual(runner.main(), 1)
                self.assertEqual(count.read_text().splitlines(), observed)
                (results / "rows/pdf25/reference/pdf25.dvi").write_bytes(b"corrupt")
                with self.assertRaisesRegex(SystemExit, "corpus artifact changed"):
                    runner.main()
                dvi(results / "rows/pdf25/reference/pdf25.dvi", 0)
                first_receipt = results / "rows/latex23/result.json"
                first_bytes = first_receipt.read_bytes()
                first_receipt.unlink()
                with self.assertRaisesRegex(SystemExit, "missing earlier row receipt"):
                    runner.main()
                first_receipt.write_bytes(first_bytes)
                prepared_bytes = preparation.read_bytes()
                preparation.write_text(json.dumps({"schema": 1, "years": {"2025": years["2025"]}}))
                with self.assertRaisesRegex(SystemExit, "prepared TeX Live year is missing"):
                    runner.main()
                preparation.write_bytes(prepared_bytes)
                wrong_year = json.loads(prepared_bytes)
                wrong_year["years"]["2023"]["formats"]["latex"]["reference_format"] = \
                    years["2025"]["formats"]["pdflatex"]["reference_format"]
                wrong_year["years"]["2023"]["formats"]["latex"]["reference_format_receipt"] = \
                    years["2025"]["formats"]["pdflatex"]["reference_format_receipt"]
                preparation.write_text(json.dumps(wrong_year))
                with self.assertRaisesRegex(SystemExit, "format source clock or engine differs"):
                    runner.main()
                preparation.write_bytes(prepared_bytes)
                parity_bytes = parity.read_bytes()
                executable(parity, "echo comparator failed >&2\nexit 2\n")
                error_results = root / "error-results"
                error_args = args.copy()
                error_args[error_args.index("--results") + 1] = str(error_results)
                with patch.object(runner.sys, "argv", error_args):
                    self.assertEqual(runner.main(), 3)
                self.assertEqual(json.loads((error_results / "summary.json").read_text())["verdict"],
                                 "ERROR")
                parity.write_bytes(parity_bytes)
                (root / "2023/texmf-dist/web2c/sentinel").write_text("changed\n")
                with self.assertRaisesRegex(ValueError, "runtime inventory changed"):
                    runner.main()
                (root / "2023/texmf-dist/web2c/sentinel").write_text("release input\n")
                (root / "2023" / "latex.fmt").write_text("modified\n")
                with self.assertRaisesRegex(SystemExit, "reference format receipt differs"):
                    runner.main()


if __name__ == "__main__":
    unittest.main()
