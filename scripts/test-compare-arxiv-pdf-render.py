#!/usr/bin/env python3
"""Hermetic admission and independent-channel contracts; no live PDF dependency."""
import importlib.util
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
