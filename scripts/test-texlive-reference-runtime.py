#!/usr/bin/env python3
"""Filename-index provenance and immutable-source contracts."""

import json
import tempfile
import unittest
from pathlib import Path

from texlive_reference_runtime import (ReferenceRuntimeError, index_bytes,
                                       prepare_reference_runtime, verify_reference_runtime)


class ReferenceRuntimeTests(unittest.TestCase):
    def test_index_reuse_and_corruption_rejection(self):
        with tempfile.TemporaryDirectory() as raw:
            base = Path(raw)
            snapshot = base / "snapshot"
            source = snapshot / "texmf-dist/tex/latex/base/article.cls"
            source.parent.mkdir(parents=True)
            source.write_text("class")
            (snapshot / "runtime.files").write_text("texmf-dist/tex/latex/base/article.cls\t5\tignored\n")
            (snapshot / "acquisition.json").write_text(json.dumps({"year": 2025}))
            before = sorted(str(path.relative_to(snapshot)) for path in snapshot.rglob("*"))
            record = prepare_reference_runtime(snapshot, base / "generated")
            root = verify_reference_runtime(snapshot, record)
            self.assertIn(b"./texmf-dist/tex/latex/base:\narticle.cls\n", (root / "ls-R").read_bytes())
            self.assertEqual((root / "texmf-dist/tex/latex/base/article.cls").read_text(), "class")
            self.assertEqual(record, prepare_reference_runtime(snapshot, base / "generated"))
            self.assertEqual(before, sorted(str(path.relative_to(snapshot)) for path in snapshot.rglob("*")))
            (root / "ls-R").write_text("corrupt")
            with self.assertRaisesRegex(ReferenceRuntimeError, "database changed"):
                prepare_reference_runtime(snapshot, base / "generated")
            (root / "ls-R").write_bytes(index_bytes(snapshot))
            (root / "texmf-dist").unlink()
            (root / "texmf-dist").symlink_to(base, target_is_directory=True)
            with self.assertRaisesRegex(ReferenceRuntimeError, "link changed"):
                verify_reference_runtime(snapshot, record)

    def test_inventory_paths_cannot_escape_index(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            for name in ("/etc/passwd", "texmf-dist/../secret", "texmf-dist//name"):
                (root / "runtime.files").write_text(f"{name}\t1\thash\n")
                with self.assertRaisesRegex(ReferenceRuntimeError, "unsafe"):
                    index_bytes(root)


if __name__ == "__main__":
    unittest.main()
