#!/usr/bin/env python3
"""Hermetic admission and independent-channel contracts; no live PDF dependency."""
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("render", Path(__file__).with_name("compare-arxiv-pdf-render.py"))
render = importlib.util.module_from_spec(spec)
spec.loader.exec_module(render)


class Document(list):
    is_repaired = False

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        pass


class Page:
    mediabox = cropbox = [0, 0, 10, 10]
    rotation = 0
    rect = SimpleNamespace(width=10, height=10)

    def __init__(self, pixels=b"rgb", text="text"):
        self.pixels, self.text = pixels, text
        class Rect(list):
            width = height = 10
        self.rect = Rect([0, 0, 10, 10])

    def get_pixmap(self, **_kwargs):
        return SimpleNamespace(width=20, height=20, samples=self.pixels)

    def get_text(self):
        return self.text


class RenderContractTests(unittest.TestCase):
    def test_artifact_tampering_is_rejected_before_consumer(self):
        with tempfile.TemporaryDirectory() as raw:
            path = Path(raw) / "paper.pdf"
            path.write_bytes(b"original")
            record = {"path": str(path), **render.identity(path)}
            row = {name: {"exit_status": 0, "pdf": record} for name in ("reference", "umber")}
            self.assertEqual(render.eligible_pair(row), (path, path))
            path.write_bytes(b"changed")
            with self.assertRaisesRegex(ValueError, "differs from corpus receipt"):
                render.eligible_pair(row)

    def test_failed_compile_is_not_a_render_candidate(self):
        self.assertIsNone(render.eligible_pair({"reference": {"exit_status": 1}}))
        self.assertIsNone(render.eligible_pair({"reference": {"exit_status": 0, "pdf": {}},
                                               "umber": {"exit_status": 0, "pdf": {}}}))

    def test_first_raster_difference_counts_pixels_not_channels(self):
        report = render.pixel_difference(bytes([0, 0, 0, 10, 20, 30]),
                                         bytes([0, 0, 0, 11, 18, 30]), 2)
        self.assertEqual(report, {"changed_pixels": 1, "bounds": [1, 0, 2, 1],
                                  "max_channel_delta": 2})

    def test_verdict_uses_reference_qualified_denominator(self):
        for statuses, verdict, qualified in [
            (["ineligible", "equal"], "PASS", 1),
            (["ineligible"], "PARTIAL", 0),
            (["equal", "unavailable"], "PARTIAL", 2),
            (["ineligible", "different"], "FAIL", 1),
            (["error"], "FAIL", 1),
        ]:
            report = render.summarize([{"status": status} for status in statuses])
            self.assertEqual(report["verdict"], verdict)
            self.assertEqual(report["reference_qualified_rows"], qualified)

    def test_missing_rows_cannot_produce_a_passing_subset(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            (root / "summary.json").write_text(json.dumps({"output_format": "pdf", "sample_rows": 2,
                "reference_rows_recorded": 2, "pdf_eligible_rows": 1}))
            receipt = root / "rows" / "paper" / "result.json"
            receipt.parent.mkdir(parents=True)
            receipt.write_text(json.dumps({"reference": {"status": "PDF-success"}}))
            with self.assertRaisesRegex(ValueError, "coverage differs"):
                render.corpus_rows(root)

    def test_memory_limit_is_installed_in_child_not_parent(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            receipt = root / "rows" / "paper" / "result.json"
            receipt.parent.mkdir(parents=True)
            receipt.write_text(json.dumps({"id": "paper", "status": "PDF-diverged",
                                           "reference": {"status": "PDF-success"}}))
            (root / "summary.json").write_text(json.dumps({"output_format": "pdf", "sample_rows": 1,
                "reference_rows_recorded": 1, "pdf_eligible_rows": 1}))
            result = {"schema": render.SCHEMA, "status": "equal", "reference": {}, "umber": {},
                      "raster_equal": True, "text_equal": True}
            with patch.object(sys, "argv", ["render", "--results", str(root),
                                            "--output", str(root / "output")]), \
                 patch.object(render.importlib.util, "find_spec", return_value=True), \
                 patch.object(render, "eligible_pair", return_value=(root / "a", root / "b")), \
                 patch.object(render, "identity", return_value={}), \
                 patch.object(render, "memory_limit", side_effect=AssertionError("parent limited")) as limit, \
                 patch.object(render.subprocess, "run", return_value=SimpleNamespace(
                     stdout=json.dumps(result), returncode=0)) as run, patch("builtins.print"):
                self.assertEqual(render.main(), 0)
                self.assertIs(run.call_args.kwargs["preexec_fn"], limit)
                limit.assert_not_called()

    def test_raster_and_extracted_text_are_separate_required_channels(self):
        for page, raster_equal, text_equal in [(Page(b"changed"), False, True),
                                                (Page(text="changed"), True, False),
                                                (Page(), True, True)]:
            documents = iter([Document([Page()]), Document([page])])
            module = SimpleNamespace(version=("test", "test", "test"),
                                     TOOLS=SimpleNamespace(mupdf_display_errors=lambda _: None,
                                                           mupdf_display_warnings=lambda _: None),
                                     open=lambda _: next(documents), Matrix=lambda *_: None, csRGB=3)
            with patch.dict(sys.modules, pymupdf=module), patch.object(render, "identity", return_value={}):
                result = render.compare(Path("a"), Path("b"))
            self.assertEqual(result["raster_equal"], raster_equal)
            self.assertEqual(result["text_equal"], text_equal)
            self.assertEqual(result["status"], "equal" if raster_equal and text_equal else "different")


if __name__ == "__main__":
    unittest.main()
