#!/usr/bin/env python3
"""Materialize a verified subset of the pinned hosted TeX Live snapshot."""

from __future__ import annotations
import argparse, hashlib, json, os, tempfile, urllib.error, urllib.request
from pathlib import Path
from urllib.parse import urljoin

DEFAULT_ROOT_URL = "https://assets.umber.ink/texlive/texlive-20260301/manifest-v3.json"
DEFAULT_ROOT_SHA256 = "43a31da364e4607957a38da10dabff227657d607d1845d502204adfd5d002e4b"
MAX_MANIFEST_BYTES = 32 * 1024 * 1024

def fail(message): raise SystemExit(f"materialize-texlive-snapshot.py: {message}")
def digest(data): return hashlib.sha256(data).hexdigest()

def verified(path, expected, size=None):
    try: data = path.read_bytes()
    except FileNotFoundError: return None
    if digest(data) != expected or (size is not None and len(data) != size):
        fail(f"existing object failed verification: {path}")
    return data

def acquire(url, path, expected, size, offline):
    data = verified(path, expected, size)
    if data is not None: return data
    if offline: fail(f"missing {path} while running --offline")
    limit = MAX_MANIFEST_BYTES if size is None else size
    request = urllib.request.Request(url, headers={"User-Agent": "umber-snapshot-materializer/1"})
    try:
        with urllib.request.urlopen(request, timeout=60) as response: data = response.read(limit + 1)
    except urllib.error.URLError as error:
        fail(f"download failed for {url}: {error}")
    if len(data) > limit or (size is not None and len(data) != size):
        fail(f"download length mismatch for {url}")
    if digest(data) != expected: fail(f"download digest mismatch for {url}")
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as output:
            output.write(data); output.flush(); os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        try: os.unlink(temporary)
        except FileNotFoundError: pass
    return data

def document(data, label):
    try: value = json.loads(data)
    except (UnicodeDecodeError, json.JSONDecodeError) as error: fail(f"invalid {label}: {error}")
    if not isinstance(value, dict): fail(f"invalid {label}: expected an object")
    return value

def keys_from(path):
    keys = {}
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        fields = line.split()
        if not fields or fields[0].startswith("#"): continue
        if fields[0] == "distribution" and len(fields) == 2: continue
        if fields[0] == "source" and len(fields) == 5:
            keys[f"{fields[1]}:{Path(fields[2]).name}"] = (int(fields[3]), fields[4])
        elif len(fields) == 1 and ":" in fields[0]: keys[fields[0]] = None
        else: fail(f"invalid key record at {path}:{number}")
    return keys

def shard_index(key, bits):
    value = int.from_bytes(hashlib.sha256(key.encode()).digest()[:2], "big")
    return value >> (16 - bits) if bits else 0

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root-url", default=DEFAULT_ROOT_URL); parser.add_argument("--root-sha256", default=DEFAULT_ROOT_SHA256)
    parser.add_argument("--output-dir", type=Path, default=Path("target/texlive-snapshot"))
    parser.add_argument("--format", action="append", default=[]); parser.add_argument("--key", action="append", default=[])
    parser.add_argument("--keys-from", action="append", type=Path, default=[]); parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    if not args.root_url.startswith("https://"): fail("root URL must use HTTPS")
    if len(args.root_sha256) != 64 or args.root_sha256.lower() != args.root_sha256: fail("invalid root SHA-256")
    output = args.output_dir.resolve(); root_path = output / "manifest-v3.json"
    root_data = acquire(args.root_url, root_path, args.root_sha256, None, args.offline); root = document(root_data, "root manifest")
    bits, shards, formats, objects_url = root.get("shardBits"), root.get("shards"), root.get("formats"), root.get("objectsBaseUrl")
    if root.get("schema") != 3: fail("root manifest is not schema 3")
    if not isinstance(bits, int) or not 0 <= bits <= 16 or not isinstance(shards, list) or len(shards) != 1 << bits: fail("invalid shard inventory")
    if not isinstance(formats, dict) or not isinstance(objects_url, str) or not objects_url.startswith("https://"): fail("invalid formats or objectsBaseUrl")
    requested = set(args.key); expected_keys = {}
    for path in args.keys_from:
        parsed = keys_from(path); requested.update(parsed); expected_keys.update({key: value for key, value in parsed.items() if value is not None})
    records = {}; views = {}
    for name in args.format:
        record = formats.get(name); closure = record.get("inputClosure") if isinstance(record, dict) else None
        if not isinstance(closure, dict) or closure.get("schema") != 1 or not isinstance(closure.get("keys"), list): fail(f"unknown or invalid format: {name}")
        requested.update(closure["keys"]); records[record["object"]] = (record["sha256"], record["bytes"])
    shard_documents = {}
    for index in sorted({shard_index(key, bits) for key in requested}):
        expected = shards[index]; name = f"sha256-{expected}"
        data = acquire(urljoin(objects_url, name), output / "objects" / name, expected, None, args.offline)
        shard = document(data, f"shard {index}")
        if shard.get("schema") != 1 or shard.get("distribution") != root.get("distribution") or shard.get("index") != index: fail(f"shard {index} identity mismatch")
        shard_documents[index] = shard
    for key in sorted(requested):
        record = shard_documents[shard_index(key, bits)].get("files", {}).get(key)
        if not isinstance(record, dict): fail(f"requested key is absent from its canonical shard: {key}")
        if key in expected_keys and expected_keys[key] != (record.get("bytes"), record.get("sha256")):
            fail(f"requested key differs from pinned lock identity: {key}")
        records[record["object"]] = (record["sha256"], record["bytes"])
        virtual = record.get("virtualPath")
        if isinstance(virtual, str) and virtual.startswith("/texlive/"):
            virtual = virtual.removeprefix("/texlive/")
        if not isinstance(virtual, str) or not virtual or virtual.startswith("/") or ".." in Path(virtual).parts or "\\" in virtual:
            fail(f"unsafe virtual path for {key}")
        views[virtual] = record["object"]
    total = 0
    for name, (expected, size) in sorted(records.items()):
        if name != f"sha256-{expected}" or not isinstance(size, int) or size < 0: fail(f"invalid object record: {name}")
        acquire(urljoin(objects_url, name), output / "objects" / name, expected, size, args.offline); total += size
    for virtual, name in sorted(views.items()):
        source = output / "objects" / name; target = output / "texmf-dist" / virtual
        target.parent.mkdir(parents=True, exist_ok=True)
        if target.exists():
            if target.read_bytes() != source.read_bytes(): fail(f"existing TEXMF view differs: {target}")
        else:
            try: os.link(source, target)
            except OSError:
                temporary = target.with_name(f".{target.name}.tmp")
                temporary.write_bytes(source.read_bytes()); os.replace(temporary, target)
    print(f"TeX Live hosted subset: root_sha256={args.root_sha256} shards={len(shard_documents)} keys={len(requested)} payload_objects={len(records)} payload_bytes={total} texmf_files={len(views)} output={output}")

if __name__ == "__main__": main()
