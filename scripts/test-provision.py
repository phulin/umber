#!/usr/bin/env python3
"""Hermetic contracts for the unified provisioning entry point."""

from __future__ import annotations

import hashlib
import http.server
import importlib.util
import tarfile
import tempfile
import threading
from pathlib import Path

MODULE_PATH = Path(__file__).with_name("provision.py")
SPEC = importlib.util.spec_from_file_location("provision", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
provision = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(provision)


def expect_error(action, fragment: str) -> None:
    try:
        action()
    except provision.ProvisionError as error:
        assert fragment in str(error), (fragment, str(error))
    else:
        raise AssertionError(f"expected ProvisionError containing {fragment!r}")


def main() -> None:
    with tempfile.TemporaryDirectory() as raw_directory:
        root = Path(raw_directory)
        hosted = root / "hosted"
        hosted.mkdir()
        payload = b"expected\n"
        (hosted / "sample.tex").write_bytes(payload)

        class Quiet(http.server.SimpleHTTPRequestHandler):
            def log_message(self, *_arguments) -> None:
                pass

        server = http.server.ThreadingHTTPServer(
            ("127.0.0.1", 0),
            lambda *arguments: Quiet(*arguments, directory=hosted),
        )
        threading.Thread(target=server.serve_forever, daemon=True).start()
        base = f"http://127.0.0.1:{server.server_port}"
        tests = root / "tests"
        tests.mkdir()
        digest = hashlib.sha256(payload).hexdigest()
        manifest = tests / "trip-manifest.txt"
        manifest.write_text(
            f"sample.tex {digest} {base}/missing {base}/sample.tex\n",
            encoding="utf-8",
        )
        provision._download_trip_inputs(root, False)
        assert (root / "third_party/trip/sample.tex").read_bytes() == payload
        provision._download_trip_inputs(root, True)

        (root / "third_party/trip/sample.tex").unlink()
        expect_error(lambda: provision._download_trip_inputs(root, True), "--offline")
        manifest.write_text(
            f"sample.tex {digest} {base}/sample.tex {base}/sample.tex\n",
            encoding="utf-8",
        )
        expect_error(lambda: provision._download_trip_inputs(root, False), "duplicate")
        manifest.write_text(
            f"sample.tex {digest} file:///tmp/sample.tex\n", encoding="utf-8"
        )
        expect_error(lambda: provision._download_trip_inputs(root, False), "unsafe")

        source_payload = root / "source-payload"
        (source_payload / "texk/web2c").mkdir(parents=True)
        (source_payload / "configure").write_bytes(b"configure\n")
        (source_payload / "texk/web2c/tex.web").write_bytes(b"tex.web\n")
        archive = hosted / "fixture-source.tar.xz"
        with tarfile.open(archive, "w:xz") as output:
            output.add(source_payload, arcname="fixture-source")
        source_lock = tests / "texlive-source.lock"
        configure_digest = hashlib.sha256(b"configure\n").hexdigest()
        tex_web_digest = hashlib.sha256(b"tex.web\n").hexdigest()
        source_lock.write_text(
            "distribution fixture-source\n"
            f"archive {archive.name} {archive.stat().st_size} "
            f"{hashlib.sha512(archive.read_bytes()).hexdigest()} {base}/{archive.name}\n"
            f"source configure 10 {configure_digest}\n"
            f"source texk/web2c/tex.web 8 {tex_web_digest}\n",
            encoding="utf-8",
        )
        source_cache = root / "third_party/texlive-source"
        provision.texlive.ensure_source_cache(source_cache, source_lock)
        assert (source_cache / "src/texk/web2c/tex.web").read_bytes() == b"tex.web\n"
        provision.texlive.ensure_source_cache(source_cache, source_lock, offline=True)
        (source_cache / "src/configure").write_bytes(b"wrong\n")
        provision.texlive.ensure_source_cache(source_cache, source_lock, offline=True)
        assert (source_cache / "src/configure").read_bytes() == b"configure\n"
        server.shutdown()
    print("test-provision: PASS")


if __name__ == "__main__":
    main()
