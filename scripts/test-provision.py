#!/usr/bin/env python3
"""Hermetic contracts for the unified provisioning entry point."""

from __future__ import annotations

import hashlib
import http.server
import importlib.util
import json
import struct
import subprocess
import tarfile
import tempfile
import threading
from pathlib import Path

MODULE_PATH = Path(__file__).with_name("provision.py")
SPEC = importlib.util.spec_from_file_location("provision", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
provision = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(provision)


def packed_fixture_shard(distribution: str, files: dict[str, dict[str, object]]) -> bytes:
    records = sorted(files.items())
    objects: list[tuple[int, int]] = []
    object_indexes: dict[tuple[str, int], int] = {}
    paths: list[str] = []
    path_indexes: dict[str, int] = {}
    key_blob = bytearray()
    encoded_records: list[tuple[int, int, int, int]] = []
    for key, record in records:
        key_offset = len(key_blob)
        key_blob.extend(key.encode())
        object_key = (str(record["ahash64"]), int(record["bytes"]))
        object_index = object_indexes.setdefault(object_key, len(objects))
        if object_index == len(objects):
            objects.append((int(object_key[0], 16), object_key[1]))
        path = str(record["virtualPath"])
        path_index = path_indexes.setdefault(path, len(paths))
        if path_index == len(paths):
            paths.append(path)
        encoded_records.append((key_offset, len(key), object_index, path_index))
    bucket_count = 2
    while len(records) * 5 > bucket_count * 4:
        bucket_count *= 2
    buckets_offset = 80
    records_offset = buckets_offset + bucket_count * 16
    objects_offset = records_offset + len(records) * 32
    paths_offset = objects_offset + len(objects) * 16
    dependencies_offset = paths_offset + len(paths) * 8
    keys_offset = dependencies_offset
    strings_offset = keys_offset + len(key_blob)
    strings = bytearray(distribution.encode())
    path_spans = []
    for path in paths:
        path_spans.append((len(strings), len(path)))
        strings.extend(path.encode())
    total_len = strings_offset + len(strings)
    output = bytearray(total_len)
    output[:8] = b"UMBRPKS1"
    struct.pack_into(
        "<HH17I",
        output,
        8,
        1,
        0,
        3,
        0,
        0,
        len(distribution),
        bucket_count,
        len(records),
        len(objects),
        len(paths),
        0,
        buckets_offset,
        records_offset,
        objects_offset,
        paths_offset,
        dependencies_offset,
        keys_offset,
        strings_offset,
        total_len,
    )
    for bucket in range(bucket_count):
        struct.pack_into("<QII", output, buckets_offset + bucket * 16, 0, 0xFFFFFFFF, 0)
    for index, ((key, _), (key_offset, key_len, object_index, path_index)) in enumerate(
        zip(records, encoded_records, strict=True)
    ):
        struct.pack_into(
            "<IHBBIIIHHII",
            output,
            records_offset + index * 32,
            key_offset,
            key_len,
            1,
            0,
            object_index,
            path_index,
            0,
            0,
            0,
            0,
            0,
        )
        key_hash = int(provision.texlive.ahash64_bytes(key.encode(), 2), 16)
        bucket = key_hash & (bucket_count - 1)
        while struct.unpack_from("<I", output, buckets_offset + bucket * 16 + 8)[0] != 0xFFFFFFFF:
            bucket = (bucket + 1) & (bucket_count - 1)
        struct.pack_into("<QII", output, buckets_offset + bucket * 16, key_hash, index, 0)
    for index, (digest, length) in enumerate(objects):
        struct.pack_into("<QQ", output, objects_offset + index * 16, digest, length)
    for index, span in enumerate(path_spans):
        struct.pack_into("<II", output, paths_offset + index * 8, *span)
    output[keys_offset:strings_offset] = key_blob
    output[strings_offset:] = strings
    return bytes(output)


def expect_error(action, fragment: str) -> None:
    try:
        action()
    except provision.ProvisionError as error:
        assert fragment in str(error), (fragment, str(error))
    else:
        raise AssertionError(f"expected ProvisionError containing {fragment!r}")


def expect_texlive_error(action, fragment: str) -> None:
    try:
        action()
    except provision.texlive.TexliveError as error:
        assert fragment in str(error), (fragment, str(error))
    else:
        raise AssertionError(f"expected TexliveError containing {fragment!r}")


def main() -> None:
    with tempfile.TemporaryDirectory() as raw_directory:
        root = Path(raw_directory)
        subprocess.run(["git", "init", "-q", str(root)], check=True)
        hosted = root / "hosted"
        hosted.mkdir()
        payload = b"expected\n"
        (hosted / "sample.tex").write_bytes(payload)
        range_requests: list[str] = []

        class Quiet(http.server.SimpleHTTPRequestHandler):
            def log_message(self, *_arguments) -> None:
                pass

            def do_GET(self) -> None:
                requested_range = self.headers.get("Range")
                if requested_range is None:
                    super().do_GET()
                    return
                data = Path(self.translate_path(self.path)).read_bytes()
                unit, raw_range = requested_range.split("=", 1)
                raw_start, raw_end = raw_range.split("-", 1)
                assert unit == "bytes"
                start, end = int(raw_start), int(raw_end)
                assert 0 <= start <= end < len(data)
                range_requests.append(requested_range)
                self.send_response(206)
                self.send_header("Content-Type", "application/octet-stream")
                self.send_header("Content-Range", f"bytes {start}-{end}/{len(data)}")
                self.send_header("Content-Length", str(end - start + 1))
                self.end_headers()
                self.wfile.write(data[start : end + 1])

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

        runtime_payload = root / "runtime-payload"
        locked_runtime = b"runtime input\n"
        (runtime_payload / "texmf-dist/tex").mkdir(parents=True)
        (runtime_payload / "texmf-dist/tex/locked.tex").write_bytes(locked_runtime)
        (runtime_payload / "texmf-dist/ls-R").write_bytes(b"locked.tex\n")
        runtime_archive = hosted / "fixture-runtime.tar.xz"
        with tarfile.open(runtime_archive, "w:xz") as output:
            output.add(runtime_payload, arcname="fixture-runtime")
        package_database = b"name 00texlive.config\ndepend release/2026\n"
        iso_prefix = b"fixture ISO prefix"
        iso_suffix = b"fixture ISO suffix"
        iso = hosted / "fixture-runtime.iso"
        iso.write_bytes(iso_prefix + package_database + iso_suffix)
        runtime_lock = tests / "texlive-runtime-source.lock"
        runtime_lock.write_text(
            "distribution fixture-runtime\n"
            f"archive {runtime_archive.name} {runtime_archive.stat().st_size} "
            f"{hashlib.sha512(runtime_archive.read_bytes()).hexdigest()}\n"
            f"iso-slice {iso.name} {iso.stat().st_size} {len(iso_prefix)} "
            f"{len(package_database)} {hashlib.sha512(package_database).hexdigest()}\n",
            encoding="utf-8",
        )
        runtime_tree_lock = tests / "texlive-snapshot.lock"
        runtime_tree_lock.write_text(
            "distribution fixture-runtime\n"
            "tree_ahash64 0123456789abcdef\n"
            f"source tex/locked.tex {len(locked_runtime)} "
            f"{hashlib.sha256(locked_runtime).hexdigest()}\n",
            encoding="utf-8",
        )
        runtime_cache = root / "third_party"
        runtime_cache.mkdir(exist_ok=True)
        partial_archive = runtime_archive.read_bytes()
        partial_length = len(partial_archive) // 2
        (runtime_cache / f".{runtime_archive.name}.part").write_bytes(
            partial_archive[:partial_length]
        )
        runtime_root = provision.texlive_release.ensure_runtime_source(
            root,
            (base,),
            release_lock=runtime_lock,
            snapshot_lock=runtime_tree_lock,
        )
        assert (runtime_root / "texmf-dist/tex/locked.tex").read_bytes() == locked_runtime
        assert (runtime_root / "tlpkg/texlive.tlpdb").read_bytes() == package_database
        assert range_requests == [
            f"bytes={partial_length}-{len(partial_archive) - 1}",
            f"bytes={len(iso_prefix)}-{len(iso_prefix) + len(package_database) - 1}"
        ]
        (runtime_root / "texmf-dist/tex/locked.tex").write_bytes(b"corrupt\n")
        provision.texlive_release.ensure_runtime_source(
            root,
            (),
            offline=True,
            release_lock=runtime_lock,
            snapshot_lock=runtime_tree_lock,
        )
        assert (runtime_root / "texmf-dist/tex/locked.tex").read_bytes() == locked_runtime
        parsed = provision.parse_args(["runtime-source", "--mirror", base])
        assert parsed.command == "runtime-source" and parsed.mirror == [base]

        texmf_dist = root / "texmf-dist"
        (texmf_dist / "tex/latex-dev/base").mkdir(parents=True)
        (texmf_dist / "tex/latex-dev/base/latex.ltx").write_bytes(b"dev kernel\n")
        (texmf_dist / "fonts/tfm/public/cm").mkdir(parents=True)
        (texmf_dist / "fonts/tfm/public/cm/cmr10.tfm").write_bytes(b"metric\n")
        (tests / "latex").mkdir()
        language_configuration = (
            b"english hyphen.tex\n=usenglish\n=USenglish\n=american\n"
        )
        (tests / "latex/language.dat").write_bytes(language_configuration)
        format_lock = tests / "latex-source.lock"
        format_lock.write_text(
            "distribution fixture-runtime\n"
            "format_schema 11\n"
            "source_date_epoch 1\n"
            "source tex/latex-dev/base/latex.ltx 11 "
            f"{hashlib.sha256(b'dev kernel\n').hexdigest()}\n"
            "source fonts/tfm/public/cm/cmr10.tfm 7 "
            f"{hashlib.sha256(b'metric\n').hexdigest()}\n"
            f"local tests/latex/language.dat {len(language_configuration)} "
            f"{hashlib.sha256(language_configuration).hexdigest()}\n",
            encoding="utf-8",
        )
        staged = root / "staged-format-inputs"
        assert provision._stage_format_input_root(root, texmf_dist, staged) == 3
        assert (staged / "tex/latex-dev/base/latex.ltx").read_bytes() == b"dev kernel\n"
        assert (staged / "fonts/tfm/public/cm/cmr10.tfm").read_bytes() == b"metric\n"
        assert (staged / "tex/language.dat").read_bytes() == language_configuration

        seed = root / "snapshot-seed"
        seed_objects = seed / "objects"
        seed_texmf = seed / "texmf-dist"
        seed_objects.mkdir(parents=True)
        (seed_texmf / "tex").mkdir(parents=True)
        from_object = b"from object root\n"
        from_texmf = b"from texmf root\n"
        object_digest = provision.texlive.ahash64_bytes(from_object)
        texmf_digest = provision.texlive.ahash64_bytes(from_texmf)
        (seed_objects / f"ahash64-v1-{object_digest}").write_bytes(from_object)
        (seed_texmf / "tex/from-texmf.tex").write_bytes(from_texmf)
        shard = packed_fixture_shard(
            "fixture-snapshot",
            {
                        "tex:from-object.tex": {
                            "virtualPath": "/texlive/tex/from-object.tex",
                            "object": f"ahash64-v1-{object_digest}",
                            "ahash64": object_digest,
                            "bytes": len(from_object),
                        },
                        "tex:from-texmf.tex": {
                            "virtualPath": "/texlive/tex/from-texmf.tex",
                            "object": f"ahash64-v1-{texmf_digest}",
                            "ahash64": texmf_digest,
                            "bytes": len(from_texmf),
                        },
            },
        )
        shard_digest = provision.texlive.ahash64_bytes(shard)
        (seed_objects / f"ahash64-v1-{shard_digest}").write_bytes(shard)
        root_manifest = (
            json.dumps(
                {
                    "schema": 8,
                    "distribution": "fixture-snapshot",
                    "objectsBaseUrl": "https://example.invalid/objects/",
                    "formats": {},
                    "shardBits": 0,
                    "shardCount": 1,
                    "shards": [shard_digest],
                },
                separators=(",", ":"),
            )
            + "\n"
        ).encode()
        root_digest = provision.texlive.ahash64_bytes(root_manifest)
        source_root = seed / "manifest-v8.json"
        source_root.write_bytes(root_manifest)
        mirror = root / "mirror"
        result = provision.texlive.materialize_snapshot(
            mirror,
            root_ahash64=root_digest,
            source_root_path=source_root,
            object_roots=(seed_objects,),
            texmf_roots=(seed_texmf,),
            keys=("tex:from-object.tex", "tex:from-texmf.tex"),
            offline=True,
        )
        assert result["root_ahash64"] == root_digest
        assert result["shards"] == 1
        assert result["keys"] == 2
        assert result["unavailable_keys"] == 0
        assert result["payload_objects"] == 2
        assert (mirror / "texmf-dist/tex/from-object.tex").read_bytes() == from_object
        assert (mirror / "texmf-dist/tex/from-texmf.tex").read_bytes() == from_texmf
        provision.texlive.materialize_snapshot(
            mirror,
            root_ahash64=root_digest,
            keys=("tex:from-object.tex", "tex:from-texmf.tex"),
            offline=True,
        )
        (mirror / "objects" / f"ahash64-v1-{object_digest}").write_bytes(b"corrupt\n")
        expect_texlive_error(
            lambda: provision.texlive.materialize_snapshot(
                mirror,
                root_ahash64=root_digest,
                keys=("tex:from-object.tex", "tex:from-texmf.tex"),
                offline=True,
            ),
            "cached snapshot object",
        )
        server.shutdown()
    print("test-provision: PASS")


if __name__ == "__main__":
    main()
