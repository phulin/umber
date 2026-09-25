#!/usr/bin/env python3
"""Hermetic execution-mirror contract for ``provision.py materialize``."""

import hashlib
import http.server
import json
import subprocess
import tempfile
import threading
from pathlib import Path

import texlive
from texlive_test_fixtures import packed_fixture_shard


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def key_for_shard(stem: str, index: int) -> str:
    for suffix in range(1000):
        key = f"tex:{stem}-{suffix}.tex"
        if int(texlive.ahash64_bytes(key.encode(), 2), 16) >> 63 == index:
            return key
    raise AssertionError(f"could not construct a key for shard {index}")


def canonical_json(value: object) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode() + b"\n"


root = Path(__file__).resolve().parents[1]
script = root / "scripts/provision.py"
library = root / "scripts/texlive.py"

with tempfile.TemporaryDirectory() as temporary:
    work = Path(temporary)
    hosted = work / "hosted"
    objects = hosted / "objects"
    objects.mkdir(parents=True)

    selected = b"selected fixture payload\n"
    selected_digest = sha(selected)
    selected_hash = texlive.ahash64_bytes(selected)
    selected_name = f"ahash64-v1-{selected_hash}"
    (objects / selected_name).write_bytes(selected)
    unselected = b"unselected payload must remain remote\n"
    unselected_digest = sha(unselected)
    unselected_hash = texlive.ahash64_bytes(unselected)
    unselected_name = f"ahash64-v1-{unselected_hash}"
    (objects / unselected_name).write_bytes(unselected)

    selected_key = "tex:selected.tex"
    selected_index = int(texlive.ahash64_bytes(selected_key.encode(), 2), 16) >> 63
    other_index = 1 - selected_index
    unavailable_key = key_for_shard("unavailable", other_index)
    unselected_key = key_for_shard("unselected", other_index)
    shard_files = [{}, {}]
    shard_files[selected_index][selected_key] = {
        "virtualPath": "/texlive/tex/selected.tex",
        "object": selected_name,
        "ahash64": selected_hash,
        "bytes": len(selected),
        "dependencies": [],
    }
    shard_files[other_index][unselected_key] = {
        "virtualPath": "/texlive/tex/unselected.tex",
        "object": unselected_name,
        "ahash64": unselected_hash,
        "bytes": len(unselected),
        "dependencies": [],
    }
    shard_digests = []
    for index, files in enumerate(shard_files):
        shard = packed_fixture_shard("fixture", files, index=index)
        digest = texlive.ahash64_bytes(shard)
        shard_digests.append(digest)
        (objects / f"ahash64-v1-{digest}").write_bytes(shard)

    class Quiet(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *_arguments) -> None:
            pass

    server = http.server.ThreadingHTTPServer(
        ("127.0.0.1", 0),
        lambda *arguments: Quiet(*arguments, directory=hosted),
    )
    threading.Thread(target=server.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{server.server_port}/"
    manifest = canonical_json(
        {
            "schema": 8,
            "distribution": "fixture",
            "objectsBaseUrl": base + "objects/",
            "shardBits": 1,
            "shardCount": 2,
            "shards": shard_digests,
            "formats": {
                "latex": {
                    "object": selected_name,
                    "ahash64": selected_hash,
                    "bytes": len(selected),
                    "inputClosure": {"schema": 1, "keys": [selected_key]},
                }
            },
        }
    )
    (hosted / "manifest.json").write_bytes(manifest)

    fixture = work / "provision.py"
    fixture.write_text(script.read_text())
    (work / "texlive.py").write_text(library.read_text())
    for module in ("pdftex_reference_format.py", "texlive_release.py"):
        (work / module).write_text((root / "scripts" / module).read_text())
    common = [
        "python3",
        str(fixture),
        "materialize",
        "--root-url",
        base + "manifest.json",
        "--root-ahash64",
        texlive.ahash64_bytes(manifest),
    ]

    destination = work / "mirror"
    command = common + ["--output-dir", str(destination), "--format", "latex"]
    subprocess.run(command, check=True, capture_output=True, text=True)
    subprocess.run(command + ["--offline"], check=True, capture_output=True, text=True)
    assert (destination / "manifest-v8.json").read_bytes() == manifest
    assert (destination / "objects" / selected_name).read_bytes() == selected
    assert not (destination / "objects" / unselected_name).exists()
    assert (destination / "texmf-dist/tex/selected.tex").read_bytes() == selected
    for digest in shard_digests:
        assert (destination / "objects" / f"ahash64-v1-{digest}").is_file()

    key_file = work / "keys.txt"
    key_file.write_text(selected_key + "\n")
    keyed_destination = work / "keyed-mirror"
    keyed_command = common + [
        "--output-dir",
        str(keyed_destination),
        "--keys-from",
        str(key_file),
    ]
    subprocess.run(keyed_command, check=True, capture_output=True, text=True)
    assert (keyed_destination / "texmf-dist/tex/selected.tex").read_bytes() == selected
    assert not (keyed_destination / "objects" / unselected_name).exists()

    representative_lock = work / "representative.lock"
    representative_lock.write_text(
        "distribution fixture\n"
        "source_date_epoch 1\n"
        f"pdflatex-source tex/selected.tex {len(selected)} {selected_digest}\n"
    )
    locked_destination = work / "locked-mirror"
    locked_command = common + [
        "--output-dir",
        str(locked_destination),
        "--keys-from",
        str(representative_lock),
    ]
    subprocess.run(locked_command, check=True, capture_output=True, text=True)
    assert (locked_destination / "texmf-dist/tex/selected.tex").read_bytes() == selected

    local_shadow_lock = work / "local-shadow.lock"
    local_shadow_lock.write_text(
        f"pdflatex-local tests/selected.tex {len(selected) + 1} {'0' * 64}\n"
    )
    local_shadow_destination = work / "local-shadow-mirror"
    subprocess.run(
        common
        + [
            "--output-dir",
            str(local_shadow_destination),
            "--keys-from",
            str(local_shadow_lock),
        ],
        check=True,
        capture_output=True,
        text=True,
    )
    assert (
        local_shadow_destination / "texmf-dist/tex/selected.tex"
    ).read_bytes() == selected

    mismatched_lock = work / "mismatched.lock"
    mismatched_lock.write_text(
        f"source tex tex/selected.tex {len(selected)} {'0' * 64}\n"
    )
    mismatched = subprocess.run(
        common
        + [
            "--output-dir",
            str(work / "mismatched-mirror"),
            "--keys-from",
            str(mismatched_lock),
        ],
        capture_output=True,
        text=True,
    )
    assert mismatched.returncode != 0
    assert "differs from pinned lock identity" in mismatched.stderr
    # A valid transport cache must not bypass the independent source lock.
    cached_mismatch = subprocess.run(
        common + ["--output-dir", str(work / "mismatched-mirror"),
                  "--keys-from", str(mismatched_lock), "--offline"],
        capture_output=True, text=True,
    )
    assert cached_mismatch.returncode != 0
    assert "differs from pinned lock identity" in cached_mismatch.stderr
    assert not (work / "mismatched-mirror/texmf-dist/tex/selected.tex").exists()

    receipt = work / "font-closure.tsv"
    receipt.write_text(
        "umber-pdf-font-closure-v1\n"
        + f"unavailable\tvf\tmissing.vf\t{unavailable_key}\n"
        + f"resolved\tfont-program\tselected.tex\t{selected_key}\t"
        + f"/texlive/tex/selected.tex\t{len(selected)}\t{selected_digest}\n"
    )
    receipt_destination = work / "receipt-mirror"
    receipt_command = common + [
        "--output-dir",
        str(receipt_destination),
        "--keys-from",
        str(receipt),
    ]
    first_receipt = subprocess.run(
        receipt_command, check=True, capture_output=True, text=True
    ).stdout
    seed_free_receipt = subprocess.run(
        receipt_command + ["--offline"], check=True, capture_output=True, text=True
    ).stdout
    assert seed_free_receipt == first_receipt
    assert "shards=2 keys=1 unavailable_keys=1 payload_objects=1" in seed_free_receipt
    assert (receipt_destination / "texmf-dist/tex/selected.tex").read_bytes() == selected
    assert not (receipt_destination / "objects" / unselected_name).exists()

    false_absence = work / "false-absence.tsv"
    false_absence.write_text(
        "umber-pdf-font-closure-v1\n"
        + f"unavailable\tvf\tselected.tex\t{selected_key}\n"
    )
    failed_absence = subprocess.run(
        common
        + [
            "--output-dir",
            str(work / "false-absence-mirror"),
            "--keys-from",
            str(false_absence),
        ],
        capture_output=True,
        text=True,
    )
    assert failed_absence.returncode != 0
    assert "declares a key unavailable" in failed_absence.stderr

    (destination / "objects" / selected_name).write_bytes(b"corrupt")
    failed_payload = subprocess.run(
        command + ["--offline"], capture_output=True, text=True
    )
    assert failed_payload.returncode != 0
    assert "cached snapshot object" in failed_payload.stderr
    server.shutdown()

print("provision.py materialize contract: PASS")
