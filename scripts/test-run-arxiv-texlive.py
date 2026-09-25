#!/usr/bin/env python3
"""Hermetic declared-year PDF parity and explicit DVI diagnostic contract."""

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
from texlive_reference_runtime import prepare_reference_runtime

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

    def test_qualification_precedes_parity_and_resume_checks_evidence(self) -> None:
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
            sample_pdf = root / "sample.pdf"
            sample_pdf.write_bytes(b"%PDF-1.4\n1 0 obj<</Type/Catalog>>endobj\n%%EOF\n")
            count = root / "invocations"
            oracle = root / "oracle"
            executable(oracle, """
for arg in "$@"; do input=$arg; done
name=${input%.tex}
test -f side.bbl
printf 'reference %s %s\\n' "$name" "$TEXMFDIST" >> "$TEST_COUNTS"
if test "${TEST_OUTPUT_FORMAT:-dvi}" = pdf; then
  cp "$TEST_PDF" "$name.pdf"
  bytes=$(wc -c < "$name.pdf")
  printf 'Output written on %s.pdf (1 page, %s bytes).\n' "$name" "$bytes" > "$name.log"
else cp "$TEST_DVI" "$name.dvi"; fi
printf 'PWD %s\nINPUT %s\nINPUT %s\n' "$PWD" "$PWD/$name.tex" "$PWD/side.bbl" > "$name.fls"
mapdir=${TEXFONTMAPS%%:*}
mapdir=${mapdir%//}
printf 'INPUT %s/pdftex/updmap/pdftex.map\n' "$mapdir" >> "$name.fls"
""")
            umber = root / "umber"
            executable(umber, """
previous=
for arg in "$@"; do
  if test "$previous" = output; then output=$arg; fi
  if test "$previous" = inputs; then admissions=$arg; fi
  case "$arg" in --dvi|--pdf) previous=output;; --input-records-out) previous=inputs;; *) previous=;; esac
done
input=$arg
name=${input%.tex}
test -f side.bbl
printf 'umber %s %s\\n' "$name" "$TEXINPUTS" >> "$TEST_COUNTS"
cp "$TEST_RECEIPTS/$name.inputs" "$admissions"
if test "${TEST_OUTPUT_FORMAT:-dvi}" = pdf; then cp "$TEST_PDF" "$output";
elif test "$name" = latex23; then cp "$TEST_DIFFERENT" "$output";
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
            pdf_comparator = root / "pdf-compare"
            executable(pdf_comparator, """
if test "${1:-}" = --version; then echo umber-pdf-compare-v1; exit 0; fi
python3 - "$1" "$2" <<'PY'
import hashlib, json, os, pathlib, sys
def identity(path):
    data = pathlib.Path(path).read_bytes()
    return {'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest(),
            'pages': 1, 'projection_sha256': ('b' if 'latex23' in path and '/umber/' in path else 'a') * 64}
name = pathlib.Path(sys.argv[1]).stem
status = 'different' if name == 'latex23' else 'equal'
receipt = {'schema': 'umber-pdf-compare-v1',
           'criterion': 'hayro-structure-page-geometry-v1', 'status': status,
           'reference': identity(sys.argv[1]), 'umber': identity(sys.argv[2])}
if status == 'different':
    receipt['first_difference'] = {'line': 1, 'reference': 'a', 'umber': 'b'}
print(json.dumps(receipt))
sys.exit(1 if status == 'different' else 0)
PY
""")
            years = {}
            receipts = root / "admissions"
            receipts.mkdir()
            for year, engine in (("2023", "latex"), ("2025", "pdflatex")):
                runtime = root / year / "texmf-dist"
                (runtime / "web2c").mkdir(parents=True)
                for name in ("tex/latex/base/latex.ltx", "tex/latex/l3kernel/expl3-code.tex",
                             "tex/latex/latexconfig/latex.ini", "tex/latex/latexconfig/pdflatex.ini"):
                    path = runtime / name
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text(name + "\n")
                config = root / year / "formats/generated-config/language.dat"
                config.parent.mkdir(parents=True)
                config.write_text("=english\n")
                (root / year / "runtime.files").write_text("")
                (runtime / "web2c" / "sentinel").write_text("release input\n")
                (root / year / "acquisition.json").write_text(json.dumps({
                    "snapshot_date": f"{year}-01-01",
                    "sentinel_sha256": sha256_file(runtime / "web2c" / "sentinel")}))
                epoch = int(dt.datetime(int(year), 1, 1, tzinfo=dt.timezone.utc).timestamp())
                reference_fmt = root / year / "formats" / f"{engine}.fmt"
                reference_fmt.write_text("reference format\n")
                reference_receipt = reference_fmt.with_suffix(".json")
                reference_receipt.write_text(json.dumps({"engine": {"sha256": sha256_file(oracle),
                                                                    "arguments": [f"-progname={engine}", f"-jobname={engine}"]},
                                                         "format": {"sha256": sha256_file(reference_fmt)},
                                                         "source_date_epoch": epoch,
                                                         "inputs": [{"path": str(path),
                                                                     "bytes": path.stat().st_size,
                                                                     "sha256": sha256_file(path)} for path in
                                                                    (runtime / "tex/latex/base/latex.ltx",
                                                                     runtime / "tex/latex/l3kernel/expl3-code.tex",
                                                                     config)]}))
                umber_fmt = root / year / "formats" / f"{engine}.umberfmt"
                umber_fmt.write_text("umber format\n")
                admission = root / year / f"formats/umber-{engine}-work/build.inputs"
                admission.parent.mkdir(parents=True)
                admission.write_text("{}\n")
                umber_receipt = root / year / "formats" / f"{engine}-umber.json"
                umber_receipt.write_text(json.dumps({"format": {"sha256": sha256_file(umber_fmt)},
                                                     "engine": engine, "source_date_epoch": epoch,
                                                     "input_admissions": str(admission),
                                                     "input_admissions_sha256": sha256_file(admission)}))
                distribution = root / year / "distribution"
                distribution.mkdir()
                manifest = distribution / "manifest.json"
                manifest.write_text(json.dumps({"schema": 8, "formats": {}}))
                fontmaps = root / year / "generated-fontmaps"
                fontmaps.mkdir()
                fontmap = fontmaps / "fonts/map/pdftex/updmap/pdftex.map"
                fontmap.parent.mkdir(parents=True)
                fontmap.write_text("generated maps\n")
                years[year] = {"runtime_root": str(runtime),
                               "reference_runtime": prepare_reference_runtime(runtime.parent, root / year / "formats"),
                               "fontmaps": {"generated_root": str(fontmaps), "pdftex_map": str(fontmap),
                                            "pdftex_map_sha256": sha256_file(fontmap)},
                               "runtime_receipt": str(root / year / "acquisition.json"),
                               "formats": {engine: {"reference_binary": str(oracle),
                                                   "reference_format": str(reference_fmt),
                                                   "reference_format_receipt": str(reference_receipt),
                                                   "umber_distribution": str(distribution),
                                                   "umber_format": str(umber_fmt),
                                                   "umber_format_sha256": sha256_file(umber_fmt),
                                                   "umber_format_receipt": str(umber_receipt),
                                                   "source_date_epoch": epoch,
                                                   "distribution_manifest_sha256": sha256_file(manifest),
                                                   "distribution_ahash64": ahash64_file(manifest)}}}
                paper = "latex23" if year == "2023" else "pdf25"
                source_view = archives / f"{paper}-source"
                main = source_view / f"{paper}.tex"
                side = source_view / "side.bbl"
                (receipts / f"{paper}.inputs").write_text(
                    "umber-input-admissions-v1\n"
                    f"main\t{main.stat().st_size}\t{ahash64_file(main)}\n"
                    f"file\tused\ttex:side.bbl\t{side.stat().st_size}\t{ahash64_file(side)}\n")
            preparation = root / "preparation.json"
            preparation.write_text(json.dumps({"schema": 1, "years": years}))
            results = root / "results"
            args = [str(SCRIPT), "--source-lock", str(lock), "--archives", str(archives),
                    "--preparation", str(preparation), "--umber", str(umber),
                    "--output-format", "dvi", "--parity-harness", str(parity),
                    "--results", str(results),
                    "--expected-rows", "3", "--timeout-seconds", "10", "--max-rss-mib", "128"]
            environment = {"TEST_DVI": str(expected), "TEST_DIFFERENT": str(different),
                           "TEST_PDF": str(sample_pdf),
                           "TEST_COUNTS": str(count), "TEST_RECEIPTS": str(receipts)}
            def verify_snapshot(year: int, cache_root: Path):
                year_root = cache_root / str(year)
                receipt = year_root / "acquisition.json"
                expected_hash = json.loads(receipt.read_text())["sentinel_sha256"]
                if sha256_file(year_root / "texmf-dist/web2c/sentinel") != expected_hash:
                    raise ValueError("runtime inventory changed")
                return types.SimpleNamespace(root=year_root, receipt=receipt)

            fontmap_module = types.ModuleType("texlive_fontmaps")
            def verify_fontmaps(snapshot, record):
                if sha256_file(Path(record["pdftex_map"])) != record["pdftex_map_sha256"]:
                    raise ValueError("generated font map changed")
                return Path(record["generated_root"])
            fontmap_module.verify_fontmaps = verify_fontmaps
            snapshot_module = types.ModuleType("texlive_snapshot")
            snapshot_module.verify_snapshot = verify_snapshot
            with patch.object(runner.sys, "argv", args), patch.dict(os.environ, environment), \
                 patch.dict(runner.sys.modules, {"texlive_snapshot": snapshot_module, "texlive_fontmaps": fontmap_module}):
                self.assertEqual(runner.main(), 1)
                observed = count.read_text().splitlines()
                self.assertTrue(observed[0].startswith(f"reference latex23 {Path(years['2023']['reference_runtime']['root']) / 'texmf-dist'}"))
                self.assertTrue(observed[1].startswith(f"reference pdf25 {Path(years['2025']['reference_runtime']['root']) / 'texmf-dist'}"))
                self.assertTrue(observed[2].startswith("umber latex23 "))
                self.assertEqual(observed[3], "compare latex23")
                self.assertEqual(observed[5], "compare pdf25")
                self.assertEqual(len(observed), 6)
                self.assertEqual(json.loads((results / "summary.json").read_text())["counts"], {
                    "DVI-diverged": 1, "DVI-exact": 1, "unsupported-xelatex": 1})
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
                last_receipt = results / "rows/pdf25/result.json"
                last_receipt.unlink()
                self.assertEqual(runner.main(), 1)
                self.assertTrue((results / "rows/incomplete-reference/pdf25-0001/reference/pdf25.dvi").is_file())
                self.assertTrue(last_receipt.is_file())
                # Rebuilt native formats can retain reference formats in another
                # output directory; their input receipt remains content-bound.
                native_path = Path(years["2023"]["formats"]["latex"]["umber_format_receipt"])
                native_bytes = native_path.read_bytes()
                native = json.loads(native_bytes)
                rebuilt_inputs = root / "rebuilt-native.inputs"
                rebuilt_inputs.write_bytes(Path(native["input_admissions"]).read_bytes())
                native["input_admissions"] = str(rebuilt_inputs)
                native_path.write_text(json.dumps(native))
                rows = runner.read_sources(lock, archives, 3)
                runner.authority(preparation, rows)
                rebuilt_inputs.write_text("corrupt")
                with self.assertRaisesRegex(SystemExit, "Umber input admissions differ"):
                    runner.authority(preparation, rows)
                native_path.write_bytes(native_bytes)
                prepared_bytes = preparation.read_bytes()
                unrelated_year = json.loads(prepared_bytes)
                unrelated_year["years"]["2024"] = {"not_selected": True}
                preparation.write_text(json.dumps(unrelated_year))
                self.assertEqual(runner.main(), 1)
                preparation.write_bytes(prepared_bytes)

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
                with self.assertRaisesRegex(SystemExit, "reference format engine differs"):
                    runner.main()
                preparation.write_bytes(prepared_bytes)
                altered_catalog = json.loads(prepared_bytes)
                altered_catalog["years"]["2023"]["formats"]["latex"]["distribution_ahash64"] = "0" * 16
                preparation.write_text(json.dumps(altered_catalog))
                with self.assertRaisesRegex(SystemExit, "distribution digest changed"):
                    runner.main()
                preparation.write_bytes(prepared_bytes)
                wrong_engine = json.loads(prepared_bytes)
                other_fmt = root / "2023/formats/pdflatex.fmt"
                other_fmt.write_bytes((root / "2023/formats/latex.fmt").read_bytes())
                other_receipt = root / "2023/formats/pdflatex.json"
                other_data = json.loads((root / "2023/formats/latex.json").read_text())
                other_data["engine"]["arguments"] = ["-progname=pdflatex", "-jobname=pdflatex"]
                other_receipt.write_text(json.dumps(other_data))
                wrong_engine["years"]["2023"]["formats"]["latex"]["reference_format"] = str(other_fmt)
                wrong_engine["years"]["2023"]["formats"]["latex"]["reference_format_receipt"] = str(other_receipt)
                preparation.write_text(json.dumps(wrong_engine))
                with self.assertRaisesRegex(SystemExit, "reference format engine differs"):
                    runner.main()
                preparation.write_bytes(prepared_bytes)
                resume_results = root / "resume-results"
                resume_args = args.copy()
                resume_args[resume_args.index("--results") + 1] = str(resume_results)
                with patch.object(runner.sys, "argv", resume_args + ["--qualify-only"]):
                    self.assertEqual(runner.main(), 2)
                stale_umber = resume_results / "rows/latex23/umber"
                stale_umber.mkdir()
                (stale_umber / "partial.txt").write_text("interrupted\n")
                with patch.object(runner.sys, "argv", resume_args):
                    self.assertEqual(runner.main(), 1)
                self.assertEqual((resume_results / "rows/latex23/incomplete-umber/0001/umber/partial.txt").read_text(),
                                 "interrupted\n")
                audit_results = root / "audit-results"
                audit_args = args.copy()
                audit_args[audit_args.index("--results") + 1] = str(audit_results)
                admission_file = receipts / "latex23.inputs"
                valid_admission = admission_file.read_text()
                admission_file.write_text(valid_admission.replace(ahash64_file(archives / "latex23-source/side.bbl"),
                                                                "0" * 16))
                with patch.object(runner.sys, "argv", audit_args):
                    with self.assertRaisesRegex(SystemExit, "bytes different from reference"):
                        runner.main()
                    admission_file.write_text(valid_admission)
                    self.assertEqual(runner.main(), 1)
                self.assertTrue((audit_results / "rows/latex23/incomplete-umber/0001/umber/latex23.dvi").is_file())
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
                pdf_results = root / "pdf-results"
                pdf_args = [value for value in args if value != "dvi"]
                del pdf_args[pdf_args.index("--output-format")]
                index = pdf_args.index("--parity-harness")
                del pdf_args[index:index + 2]
                pdf_args.extend(("--pdf-comparator", str(pdf_comparator)))
                pdf_args[pdf_args.index("--results") + 1] = str(pdf_results)
                with patch.dict(os.environ, {"TEST_OUTPUT_FORMAT": "pdf"}), \
                     patch.object(runner.sys, "argv", pdf_args):
                    self.assertEqual(runner.main(), 1)
                    pdf_summary = json.loads((pdf_results / "summary.json").read_text())
                    self.assertEqual(pdf_summary["output_format"], "pdf")
                    self.assertEqual(pdf_summary["counts"], {
                        "PDF-diverged": 1, "PDF-equal": 1, "unsupported-xelatex": 1})
                    self.assertEqual(pdf_summary["pdf_eligible_rows"], 2)
                    self.assertEqual(runner.main(), 1)
                    with patch.object(runner.sys, "argv", pdf_args + ["--verify-only"]):
                        self.assertEqual(runner.main(), 1)
                    receipt_path = pdf_results / "rows/latex23/result.json"
                    receipt_bytes = receipt_path.read_bytes()
                    tampered = json.loads(receipt_bytes)
                    tampered["umber"]["comparison_receipt"]["status"] = "equal"
                    receipt_path.write_text(json.dumps(tampered))
                    with self.assertRaisesRegex(SystemExit, "PDF comparator receipt changed"):
                        runner.main()
                    receipt_path.write_bytes(receipt_bytes)
                    with patch.object(runner.sys, "argv", pdf_args + ["--output-format", "dvi",
                                                                  "--parity-harness", str(parity)]):
                        with self.assertRaisesRegex(SystemExit, "corpus run authority changed"):
                            runner.main()
                    pdf_comparator.write_text(pdf_comparator.read_text() + "# changed\n")
                    with self.assertRaisesRegex(SystemExit, "corpus run authority changed"):
                        runner.main()
                error_comparator = root / "error-compare"
                executable(error_comparator, """
if test "${1:-}" = --version; then echo umber-pdf-compare-v1; exit 0; fi
python3 - "$1" "$2" <<'PY'
import hashlib, json, pathlib, sys
def identity(path):
    data = pathlib.Path(path).read_bytes()
    return {'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}
print(json.dumps({'schema': 'umber-pdf-compare-v1',
                  'criterion': 'hayro-structure-page-geometry-v1',
                  'status': 'error', 'error': 'invalid structure',
                  'reference': identity(sys.argv[1]), 'umber': identity(sys.argv[2])}))
sys.exit(2)
PY
""")
                error_pdf_args = pdf_args.copy()
                error_pdf_args[error_pdf_args.index("--pdf-comparator") + 1] = str(error_comparator)
                error_pdf_args[error_pdf_args.index("--results") + 1] = str(root / "pdf-error-results")
                with patch.dict(os.environ, {"TEST_OUTPUT_FORMAT": "pdf"}), \
                     patch.object(runner.sys, "argv", error_pdf_args):
                    self.assertEqual(runner.main(), 3)
                    self.assertEqual(json.loads((root / "pdf-error-results/summary.json").read_text())["counts"], {
                        "comparison-error": 2, "unsupported-xelatex": 1})
                (root / "2023/texmf-dist/web2c/sentinel").write_text("changed\n")
                with self.assertRaisesRegex(ValueError, "runtime inventory changed"):
                    runner.main()
                (root / "2023/texmf-dist/web2c/sentinel").write_text("release input\n")
                (root / "2023/formats/latex.fmt").write_text("modified\n")
                with self.assertRaisesRegex(SystemExit, "reference format receipt differs"):
                    runner.main()


if __name__ == "__main__":
    unittest.main()
