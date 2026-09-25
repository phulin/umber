#!/usr/bin/env python3
"""Hermetic contracts for dated TeX Live package acquisition."""

from __future__ import annotations

import hashlib
import io
import json
import lzma
import tarfile
import tempfile
import threading
import unittest
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import texlive
import texlive_snapshot as snapshot


def package_archive(name: str, *, kind: str = "regular", extra: bool = False) -> bytes:
    payload = io.BytesIO()
    with tarfile.open(fileobj=payload, mode="w:xz") as output:
        member = tarfile.TarInfo("tex/latex/demo.sty")
        member.size = len(b"dated demo\n")
        if kind == "symlink":
            member.type = tarfile.SYMTYPE
            member.linkname = "/etc/passwd"
            member.size = 0
        output.addfile(member, io.BytesIO(b"dated demo\n") if kind == "regular" else None)
        if extra:
            unexpected = tarfile.TarInfo("tex/latex/unlisted.sty")
            unexpected.size = 1
            output.addfile(unexpected, io.BytesIO(b"x"))
        metadata = tarfile.TarInfo(f"tlpkg/tlpobj/{name}.tlpobj")
        metadata.size = 0
        output.addfile(metadata, io.BytesIO())
    return payload.getvalue()


def database(name: str, archive: bytes, *, duplicate_path: bool = False) -> bytes:
    record = (
        f"name {name}\n"
        "category Package\n"
        "revision 7\n"
        "relocated 1\n"
        f"containersize {len(archive)}\n"
        f"containerchecksum {hashlib.sha512(archive).hexdigest()}\n"
        "runfiles size=1\n"
        " RELOC/tex/latex/demo.sty\n"
    )
    return lzma.compress((record + ("\n" + record.replace(f"name {name}", "name another") if duplicate_path else "")).encode())


class QuietHandler(SimpleHTTPRequestHandler):
    def log_message(self, format, *args):
        pass


class SnapshotTests(unittest.TestCase):
    def test_download_receipt_offline_and_tamper_detection(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            origin = base / "origin"
            (origin / "tlpkg").mkdir(parents=True)
            (origin / "archive").mkdir()
            archive = package_archive("demo")
            (origin / "archive/demo.r7.tar.xz").write_bytes(archive)
            (origin / "tlpkg/texlive.tlpdb.xz").write_bytes(database("demo", archive))
            handler = lambda *args, **kwargs: QuietHandler(*args, directory=str(origin), **kwargs)
            server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            try:
                cache = base / "cache"
                url = f"http://127.0.0.1:{server.server_port}/"
                result = snapshot.ensure_snapshot(2025, cache, mirror=url, workers=2)
                self.assertEqual(result.package_count, 1)
                self.assertEqual(result.file_count, 1)
                self.assertEqual((result.root / "texmf-dist/tex/latex/demo.sty").read_bytes(), b"dated demo\n")
                receipt = json.loads(result.receipt.read_text())
                self.assertEqual(receipt["snapshot_date"], "2025-08-03")
                self.assertEqual(receipt["selected_archive_bytes"], len(archive))
                snapshot.ensure_snapshot(2025, cache, offline=True)
                (result.root / "texmf-dist/tex/latex/demo.sty").write_bytes(b"changed\n")
                with self.assertRaisesRegex(texlive.TexliveError, "runtime file changed"):
                    snapshot.verify_snapshot(2025, cache)
                result.receipt.unlink()  # Simulate an interrupted final publication.
                restored = snapshot.ensure_snapshot(2025, cache, offline=True)
                self.assertEqual((restored.root / "texmf-dist/tex/latex/demo.sty").read_bytes(), b"dated demo\n")
                (restored.root / "texmf-dist/tex/latex/unlisted.sty").write_bytes(b"injected\n")
                with self.assertRaisesRegex(texlive.TexliveError, "unlisted runtime file"):
                    snapshot.verify_snapshot(2025, cache)
                restored.receipt.unlink()
                (restored.root / "tlpkg/texlive.tlpdb.xz").write_bytes(b"modified database")
                with self.assertRaisesRegex(texlive.TexliveError, "length mismatch"):
                    snapshot.ensure_snapshot(2025, cache, offline=True)
            finally:
                server.shutdown()
                server.server_close()
                thread.join(timeout=5)

    def test_tlpdb_duplicate_installed_path_is_rejected(self):
        archive = package_archive("demo")
        with self.assertRaisesRegex(texlive.TexliveError, "duplicate installed runfile"):
            snapshot.parse_tlpdb(database("demo", archive, duplicate_path=True))

    def test_archive_rejects_links_and_unlisted_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for kind, extra, message in (("symlink", False, "nonregular member"), ("regular", True, "unlisted archive member")):
                archive_bytes = package_archive("demo", kind=kind, extra=extra)
                archive = root / "demo.tar.xz"
                archive.write_bytes(archive_bytes)
                package = snapshot.parse_tlpdb(database("demo", archive_bytes))[0]
                with self.assertRaisesRegex(texlive.TexliveError, message):
                    snapshot._extract_package(archive, root / "installed", package)

    def test_cached_archive_identity_is_checked_before_extraction(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = package_archive("demo")
            destination = root / "demo.r7.tar.xz"
            destination.write_bytes(archive + b"tamper")
            identity = snapshot.parse_tlpdb(database("demo", archive))[0].identity
            with self.assertRaisesRegex(texlive.TexliveError, "length mismatch"):
                snapshot._download("http://127.0.0.1:1/unused", destination, expected=identity, limit=identity.bytes, offline=True)

    def test_declared_non_runtime_runfile_is_validated_but_not_installed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            payload = io.BytesIO()
            with tarfile.open(fileobj=payload, mode="w:xz") as output:
                for name, content in (("texmf-dist/tex/latex/demo.sty", b"demo"), ("tlpkg/installer.pl", b"script")):
                    member = tarfile.TarInfo(name)
                    member.size = len(content)
                    output.addfile(member, io.BytesIO(content))
            archive_bytes = payload.getvalue()
            archive = root / "mixed.tar.xz"
            archive.write_bytes(archive_bytes)
            record = (
                "name mixed\ncategory TLCore\nrevision 1\n"
                f"containersize {len(archive_bytes)}\n"
                f"containerchecksum {hashlib.sha512(archive_bytes).hexdigest()}\n"
                "runfiles size=1\n texmf-dist/tex/latex/demo.sty\n tlpkg/installer.pl\n"
            )
            package = snapshot.parse_tlpdb(lzma.compress(record.encode()))[0]
            snapshot._extract_package(archive, root / "installed", package)
            self.assertEqual((root / "installed/texmf-dist/tex/latex/demo.sty").read_bytes(), b"demo")
            self.assertFalse((root / "installed/tlpkg/installer.pl").exists())
            binary = record.replace("name mixed", "name mixed.windows").replace(
                "runfiles size=1", "binfiles arch=windows size=1\n bin/windows/helper.exe\nrunfiles size=1"
            )
            self.assertEqual(
                [item.name for item in snapshot.parse_tlpdb(lzma.compress((record + "\n" + binary).encode()))],
                ["mixed"],
            )


if __name__ == "__main__":
    unittest.main()
