#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/umber-arxiv-corpus-test.XXXXXX")
trap 'rm -rf "$work"' EXIT HUP INT TERM

mkdir -p "$work/input"
printf '\\documentclass{article}\n' >"$work/input/main.tex"
printf 'binary\000bytes\n' >"$work/input/asset.dat"
tar -czf "$work/source.src" -C "$work/input" main.tex asset.dat

python3 "$root/scripts/arxiv_corpus.py" materialize "$work/source.src" "$work/view" >"$work/manifest.json"
python3 "$root/scripts/arxiv_corpus.py" verify "$work/source.src" "$work/view" >"$work/verified.json"
cmp "$work/manifest.json" "$work/verified.json"
test "$(python3 -c 'import json,sys; print(len(json.load(open(sys.argv[1]))))' "$work/manifest.json")" -eq 2

printf 'mutation\n' >>"$work/view/main.tex"
if python3 "$root/scripts/arxiv_corpus.py" verify "$work/source.src" "$work/view" >/dev/null 2>&1; then
  echo 'mutated archive member passed verification' >&2
  exit 1
fi
rm -rf "$work/view"
python3 "$root/scripts/arxiv_corpus.py" materialize "$work/source.src" "$work/view" >/dev/null
printf 'derived\n' >"$work/view/main.aux"
if python3 "$root/scripts/arxiv_corpus.py" verify "$work/source.src" "$work/view" >/dev/null 2>&1; then
  echo 'extra extracted-view artifact passed verification' >&2
  exit 1
fi
python3 "$root/scripts/arxiv_corpus.py" replace "$work/source.src" "$work/view" \
  "$work/backup" "$work/replacement.json"
python3 "$root/scripts/arxiv_corpus.py" verify "$work/source.src" "$work/view" >/dev/null
test -f "$work/backup/main.aux"
python3 -c 'import json,sys; value=json.load(open(sys.argv[1])); assert value["extra_paths"] == ["main.aux"]; assert value["missing_paths"] == []; assert value["changed_paths"] == []' "$work/replacement.json"

identity=$(python3 "$root/scripts/arxiv_corpus.py" identity "$work/source.src" main.tex)
python3 -c 'import json,sys; value=json.loads(sys.argv[1]); assert value["member_count"] == 2; assert value["entrypoint"] == "main.tex"; assert len(value["archive_sha256"]) == len(value["member_manifest_sha256"]) == 64' "$identity"

python3 - "$work/case-distinct.src" <<'PY'
import io, sys, tarfile
with tarfile.open(sys.argv[1], "w:gz") as archive:
    for name, data in (("main.tex", b"\\documentclass{article}\n"),
                       ("figures/Intro/Drone.png", b"upper"),
                       ("figures/Intro/drone.png", b"lower")):
        member = tarfile.TarInfo(name)
        member.size = len(data)
        archive.addfile(member, io.BytesIO(data))
PY
python3 "$root/scripts/arxiv_corpus.py" stage -- \
  python3 - "$root" "$work/case-distinct.src" <<'PY'
import os, subprocess, sys
from pathlib import Path
root, archive = map(Path, sys.argv[1:])
view = Path(os.environ["UMBER_ARXIV_STAGE"]) / "view"
subprocess.run([sys.executable, str(root / "scripts/arxiv_corpus.py"),
                "materialize", str(archive), str(view)], check=True,
               stdout=subprocess.DEVNULL)
assert (view / "figures/Intro/Drone.png").read_bytes() == b"upper"
assert (view / "figures/Intro/drone.png").read_bytes() == b"lower"
PY
