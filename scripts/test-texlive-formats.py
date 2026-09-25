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

    def test_upstream_release_marker_prevents_mislabelled_snapshot(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            tlpdb = Path(raw) / "texlive.tlpdb.xz"
            with lzma.open(tlpdb, "wt", encoding="utf-8") as stream:
                stream.write("name 00texlive.config\ndepend minrelease/2016\ndepend release/2023\nname unrelated\ndepend release/2025\n")
            self.assertEqual(formats.tlpdb_release_year(tlpdb), 2023)
            with lzma.open(tlpdb, "wt", encoding="utf-8") as stream:
                stream.write("name 00texlive.config\ndepend minrelease/2016\nname unrelated\ndepend release/2025\n")
            with self.assertRaisesRegex(formats.FormatPreparationError, "release year"):
                formats.tlpdb_release_year(tlpdb)

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

    def test_ini_directory_is_discovered_from_selected_tree(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            texmf = Path(raw)
            for relative in (
                "tex/latex/base/latex.ltx",
                "tex/latex/l3kernel/expl3-code.tex",
                "tex/latex/latexconfig/latex.ini",
                "tex/latex/latexconfig/pdflatex.ini",
            ):
                path = texmf / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("test", encoding="utf-8")
            self.assertEqual(formats.stable_paths(texmf)[2], texmf / "tex/latex/latexconfig")
            other = texmf / "tex/latex/another/latex.ini"
            other.parent.mkdir(parents=True)
            other.write_text("test", encoding="utf-8")
            (other.parent / "pdflatex.ini").write_text("test", encoding="utf-8")
            with self.assertRaisesRegex(formats.FormatPreparationError, "expected one"):
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

    def test_umber_consumed_inputs_must_match_reference_closure(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            texmf = root / "texmf-dist"
            ini = texmf / "tex/latex/tex-ini-files/latex.ini"
            stable = texmf / "tex/latex/base/latex.ltx"
            extra = texmf / "tex/latex/base/omsenc.dfu"
            companion = texmf / "tex/latex/tex-ini-files/pdflatex.ini"
            kernel = texmf / "tex/latex/l3kernel/expl3-code.tex"
            for path, data in ((ini, b"entry"), (stable, b"stable"), (extra, b"extra"), (companion, b"other"), (kernel, b"kernel")):
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
            publisher = root / "publisher"
            publisher.write_text(
                "#!/usr/bin/env python3\n"
                "import sys\n"
                "from pathlib import Path\n"
                "print({'entry':'0000000000000001','stable':'0000000000000002','extra':'0000000000000004'}[Path(sys.argv[2]).read_text()])\n",
                encoding="utf-8",
            )
            publisher.chmod(0o755)
            reference = {"inputs": [
                {"path": str(ini), "bytes": 5},
                {"path": str(stable), "bytes": 6},
            ]}
            admission = root / "build.inputs"
            admission.write_text(
                "umber-input-admissions-v1\n"
                "main\t5\t0000000000000001\n"
                "file\tused\ttex:latex.ltx\t6\t0000000000000002\n"
                "file\tused\ttex:omsenc.dfu\t5\t0000000000000004\n"
                "file\tadmitted\ttex:unused.sty\t1\t0000000000000003\n",
                encoding="utf-8",
            )
            extras = formats.verify_umber_inputs(reference, admission, publisher, texmf, root / "config", "latex")
            self.assertEqual([record["path"] for record in extras], [str(extra)])
            admission.write_text(
                "umber-input-admissions-v1\n"
                "main\t5\t0000000000000001\n"
                "file\tused\ttex:latex.ltx\t6\t0000000000000003\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(formats.FormatPreparationError, "different from clean reference"):
                formats.verify_umber_inputs(reference, admission, publisher, texmf, root / "config", "latex")


if __name__ == "__main__":
    unittest.main()
