#!/usr/bin/env python3
"""Hermetic source selection and provenance contracts for annual formats."""

from __future__ import annotations

import datetime as dt
import lzma
import tempfile
import unittest
from pathlib import Path

import texlive_formats as formats


class AnnualFormatsTest(unittest.TestCase):
    def test_clock_comes_from_selected_acquisition_receipt(self) -> None:
        self.assertEqual(
            formats.source_epoch({"snapshot_date": "2024-03-11"}),
            int(dt.datetime(2024, 3, 11, tzinfo=dt.timezone.utc).timestamp()),
        )
        with self.assertRaisesRegex(formats.FormatPreparationError, "snapshot_date"):
            formats.source_epoch({"year": 2024})

    def test_language_dat_uses_selected_tlpdb_and_dat_directives(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            texmf = root / "texmf-dist"
            static = texmf / "tex/generic/config/language.us"
            static.parent.mkdir(parents=True)
            static.write_text("english hyphen.tex\n=american\n", encoding="utf-8")
            tlpdb = root / "texlive.tlpdb.xz"
            with lzma.open(tlpdb, "wt", encoding="utf-8") as stream:
                stream.write(
                    "name hyphen-a\n"
                    "execute AddHyphen name=afrikaans synonyms=af,afrikaans-alt file=loadhyph-af.tex databases=dat,def\n"
                    "execute AddHyphen name=other file=other.tex databases=lua\n"
                    "name hyphen-b\n"
                    "execute AddHyphen name=basque file=loadhyph-eu.tex\n"
                )
            output = root / "generated/language.dat"
            receipt = formats.language_dat(texmf, tlpdb, output)
            text = output.read_text(encoding="utf-8")
            self.assertIn("english hyphen.tex\n=american", text)
            self.assertIn("% from hyphen-a:\nafrikaans loadhyph-af.tex\n=af\n=afrikaans-alt", text)
            self.assertIn("% from hyphen-b:\nbasque loadhyph-eu.tex", text)
            self.assertNotIn("other.tex", text)
            self.assertEqual(receipt["addhyphen_count"], 2)
            self.assertEqual(receipt["sha256"], formats.sha256(output))

    def test_stable_sources_are_required(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            texmf = Path(raw)
            for relative in (
                "tex/latex-dev/base/latex.ltx",
                "tex/latex/l3kernel/expl3-code.tex",
                "tex/latex/tex-ini-files/latex.ini",
                "tex/latex/tex-ini-files/pdflatex.ini",
            ):
                path = texmf / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("test", encoding="utf-8")
            with self.assertRaisesRegex(formats.FormatPreparationError, "stable format source"):
                formats.stable_paths(texmf)

    def test_recorder_rejects_latex_dev_and_foreign_inputs(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            run = root / "run"
            run.mkdir()
            texmf = root / "texmf-dist"
            config = root / "config"
            config.mkdir()
            recorder = run / "latex.fls"
            for bad in (texmf / "tex/latex-dev/base/latex.ltx", root / "other.tex"):
                recorder.write_text(f"INPUT {bad}\n", encoding="utf-8")
                with self.assertRaises(formats.FormatPreparationError):
                    formats.recorder_inputs(run, texmf, config, recorder)


if __name__ == "__main__":
    unittest.main()
