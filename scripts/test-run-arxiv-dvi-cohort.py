#!/usr/bin/env python3
"""Hermetic full-source DVI qualification, comparison, and resume contracts."""

from __future__ import annotations

import importlib.util
import json
import os
import shutil
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from arxiv_corpus import source_identity


SCRIPT = Path(__file__).with_name("run-arxiv-dvi-cohort.py")
spec = importlib.util.spec_from_file_location("arxiv_dvi_cohort", SCRIPT)
assert spec is not None and spec.loader is not None
cohort = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cohort)


def executable(path: Path, body: str) -> None:
    path.write_text("#!/bin/sh\nset -eu\n" + body)
    path.chmod(0o755)


def one_page_dvi(path: Path, marker: int) -> None:
    data = bytes([248]) + bytes(26) + bytes([0, 1, marker])
    data += bytes([249, 0, 0, 0, 0, 2, 223, 223, 223, 223])
    path.write_bytes(data)


class DviCohortContract(unittest.TestCase):
    def test_qualification_comparison_stop_resume_and_artifact_integrity(self) -> None:
        with tempfile.TemporaryDirectory(prefix="arxiv-dvi-contract-") as temporary:
            root = Path(temporary)
            archives = root / "archives"
            archives.mkdir()
            distribution = root / "distribution"
            distribution.mkdir()
            (root / "metadata.json").write_text("{}\n")
            (root / "results.jsonl").write_text("\n")
            (root / "summary.json").write_text("{}\n")
            runtime = root / "runtime"
            runtime.mkdir()
            for name in ("source.lock", "build-record", "reference.fmt", "receipt",
                         "runtime.lock", "umber.fmt"):
                (root / name).write_text(name + "\n")
            format_digest = cohort.ahash64_file(root / "umber.fmt")
            manifest = distribution / "manifest.json"
            manifest.write_text(json.dumps({
                "schema": 8, "formats": {"pdflatex": {
                    "ahash64": format_digest, "object": "ahash64-v1-" + format_digest,
                    "bytes": (root / "umber.fmt").stat().st_size,
                }},
            }) + "\n")
            expected = root / "expected.dvi"
            different = root / "different.dvi"
            one_page_dvi(expected, 0)
            one_page_dvi(different, 1)
            count = root / "invocations"

            oracle = root / "oracle"
            executable(oracle, """
for arg in "$@"; do
  case "$arg" in --jobname*) exit 91;; esac
done
input=$arg
name=${input##*/}
name=${name%.tex}
test -f side.bbl
printf 'reference %s\\n' "$name" >> "$TEST_COUNTS"
test "$name" != nodvi || exit 1
cp "$TEST_DVI" "$name.dvi"
printf 'Output written on %s.dvi (1 page).\\n' "$name" > "$name.log"
""")
            umber = root / "umber"
            executable(umber, """
output=
previous=
for arg in "$@"; do
  test "$previous" != output || output=$arg
  case "$arg" in --dvi) previous=output;; *) previous=;; esac
done
input=$arg
name=${input##*/}
name=${name%.tex}
test -f side.bbl
printf 'umber %s\\n' "$name" >> "$TEST_COUNTS"
if test "$name" = diverge; then cp "$TEST_DIFFERENT" "$output";
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

            papers = ("pdf-fail", "exact", "nodvi", "diverge", "unreached")
            rows = []
            for order, paper in enumerate(papers, 1):
                source = root / (paper + "-source")
                source.mkdir()
                (source / f"{paper}.tex").write_text("\\relax\n")
                (source / "side.bbl").write_text("archive side file\n")
                archive = archives / f"{paper}.src"
                with tarfile.open(archive, "w:gz") as output:
                    output.add(source / f"{paper}.tex", arcname=f"{paper}.tex")
                    output.add(source / "side.bbl", arcname="side.bbl")
                rows.append(({
                    "id": paper, "lock_order": order, "archive": archive,
                    "entrypoint": f"{paper}.tex",
                    "source_identity": source_identity(archive, f"{paper}.tex"),
                }, {"terminal_status": {"classification":
                      "PDF-failure" if paper == "pdf-fail" else "PDF-success"},
                    "pdf": None}))
            metadata = {"authority": {"test": "oracle"},
                        "scope": {"sample_rows": 5,
                                  "declared_compilers": {"pdflatex": 5}}}
            results = root / "results"
            argv = [str(SCRIPT)]
            paths = {
                "source-lock": root / "source.lock", "archives": archives,
                "pdf-survey": root, "oracle": oracle,
                "oracle-build-record": root / "build-record",
                "reference-format": root / "reference.fmt", "format-receipt": root / "receipt",
                "runtime-root": runtime, "runtime-lock": root / "runtime.lock",
                "umber": umber, "umber-format": root / "umber.fmt",
                "distribution": distribution, "parity-harness": parity, "results": results,
            }
            for key, path in paths.items():
                argv.extend(("--" + key, str(path)))
            argv.extend(("--timeout-seconds", "10", "--max-rss-mib", "128",
                         "--distribution-ahash64", cohort.ahash64_file(manifest)))
            environment = {"TEST_DVI": str(expected), "TEST_DIFFERENT": str(different),
                           "TEST_COUNTS": str(count)}
            with patch.object(cohort, "verify_pdf_survey", return_value=(rows, metadata)), \
                 patch.object(sys, "argv", argv), patch.dict(os.environ, environment):
                self.assertEqual(cohort.main(), 1)
                first_count = count.read_text().splitlines()
                self.assertEqual(first_count, [
                    "reference exact", "umber exact", "compare exact",
                    "reference nodvi", "reference diverge", "umber diverge", "compare diverge",
                ])
                summary = json.loads((results / "summary.json").read_text())
                self.assertEqual(summary["first_divergence"], "diverge")
                self.assertEqual(summary["counts"], {
                    "PDF-ineligible": 1, "DVI-exact": 1,
                    "DVI-ineligible": 1, "DVI-diverged": 1,
                })
                self.assertFalse((results / "rows/unreached").exists())
                self.assertEqual(cohort.main(), 1)
                self.assertEqual(count.read_text().splitlines(), first_count)
                with patch.object(sys, "argv", argv + ["--verify-only"]):
                    self.assertEqual(cohort.main(), 1)
                self.assertEqual(count.read_text().splitlines(), first_count)

                partial = root / "partial-results"
                (partial / "rows/pdf-fail").mkdir(parents=True)
                shutil.copyfile(results / "run-identity.json", partial / "run-identity.json")
                shutil.copyfile(results / "rows/pdf-fail/result.json",
                                partial / "rows/pdf-fail/result.json")
                partial_argv = argv.copy()
                partial_argv[partial_argv.index("--results") + 1] = str(partial)
                with patch.object(sys, "argv", partial_argv + ["--verify-only"]):
                    self.assertEqual(cohort.main(), 2)
                self.assertEqual(json.loads((partial / "summary.json").read_text())["verdict"], "PARTIAL")
                self.assertEqual(count.read_text().splitlines(), first_count)

                (results / "rows/exact/reference/exact.dvi").write_bytes(b"corrupt")
                with self.assertRaisesRegex(SystemExit, "cohort artifact changed"):
                    cohort.main()
                shutil.copyfile(expected, results / "rows/exact/reference/exact.dvi")
                (results / "rows/pdf-fail/result.json").unlink()
                with patch.object(sys, "argv", argv + ["--verify-only"]):
                    with self.assertRaisesRegex(SystemExit, "missing earlier row"):
                        cohort.main()

                executable(parity, "echo 'internal comparator error' >&2\nexit 2\n")
                error_results = root / "error-results"
                error_argv = argv.copy()
                error_argv[error_argv.index("--results") + 1] = str(error_results)
                with patch.object(sys, "argv", error_argv):
                    self.assertEqual(cohort.main(), 3)
                error_summary = json.loads((error_results / "summary.json").read_text())
                self.assertEqual(error_summary["verdict"], "ERROR")
                self.assertIsNone(error_summary["first_divergence"])
                self.assertEqual(error_summary["comparison_error"], "exact")

                (root / "umber.fmt").write_text("different format\n")
                with patch.object(sys, "argv", error_argv):
                    with self.assertRaisesRegex(SystemExit, "Umber format differs"):
                        cohort.main()


if __name__ == "__main__":
    unittest.main()
