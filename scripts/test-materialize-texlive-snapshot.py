#!/usr/bin/env python3
"""Hermetic execution-mirror contract for ``provision.py materialize``."""

import hashlib
import http.server
import json
import subprocess
import tempfile
import threading
from pathlib import Path


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def key_for_shard(stem: str, index: int) -> str:
    for suffix in range(1000):
        key = f"tex:{stem}-{suffix}.tex"
        if hashlib.sha256(key.encode()).digest()[0] >> 7 == index:
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
    selected_name = f"sha256-{selected_digest}"
    (objects / selected_name).write_bytes(selected)
    unselected = b"unselected payload must remain remote\n"
    unselected_digest = sha(unselected)
    unselected_name = f"sha256-{unselected_digest}"
    (objects / unselected_name).write_bytes(unselected)

    selected_key = key_for_shard("selected", 0)
    unavailable_key = key_for_shard("unavailable", 1)
    unselected_key = key_for_shard("unselected", 1)
    shard_files = [
        {
            selected_key: {
                "virtualPath": "tex/selected.tex",
                "object": selected_name,
                "sha256": selected_digest,
                "bytes": len(selected),
                "dependencies": [],
            }
        },
        {
            unselected_key: {
                "virtualPath": "tex/unselected.tex",
                "object": unselected_name,
                "sha256": unselected_digest,
                "bytes": len(unselected),
                "dependencies": [],
            }
        },
    ]
    shard_digests = []
    for index, files in enumerate(shard_files):
        shard = canonical_json(
            {
                "schema": 1,
                "distribution": "fixture",
                "index": index,
                "files": files,
            }
        )
        digest = sha(shard)
        shard_digests.append(digest)
        (objects / f"sha256-{digest}").write_bytes(shard)

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
            "schema": 3,
            "distribution": "fixture",
            "objectsBaseUrl": base + "objects/",
            "shardBits": 1,
            "shardCount": 2,
            "shards": shard_digests,
            "formats": {
                "latex": {
                    "object": selected_name,
                    "sha256": selected_digest,
                    "bytes": len(selected),
                    "inputClosure": {"schema": 1, "keys": [selected_key]},
                }
            },
        }
    )
    (hosted / "manifest-v3.json").write_bytes(manifest)

    fixture = work / "provision.py"
    fixture.write_text(script.read_text())
    (work / "texlive.py").write_text(library.read_text())
    common = [
        "python3",
        str(fixture),
        "materialize",
        "--root-url",
        base + "manifest-v3.json",
        "--root-sha256",
        sha(manifest),
    ]

    destination = work / "mirror"
    command = common + ["--output-dir", str(destination), "--format", "latex"]
    subprocess.run(command, check=True, capture_output=True, text=True)
    subprocess.run(command + ["--offline"], check=True, capture_output=True, text=True)
    assert (destination / "manifest-v3.json").read_bytes() == manifest
    assert (destination / "objects" / selected_name).read_bytes() == selected
    assert not (destination / "objects" / unselected_name).exists()
    assert (destination / "texmf-dist/tex/selected.tex").read_bytes() == selected
    for digest in shard_digests:
        assert (destination / "objects" / f"sha256-{digest}").is_file()

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
