#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
checker="${repo_root}/scripts/check-latex-representative-resources.sh"
tmp_root="$(mktemp -d)"
trap 'rm -rf "$tmp_root"' EXIT
fixture_repo="${tmp_root}/repo"
mkdir -p "${fixture_repo}/scripts" "${fixture_repo}/tests/latex" \
  "${fixture_repo}/distribution" "${fixture_repo}/bin"
cp "$checker" "${fixture_repo}/scripts/"
cp "${repo_root}/scripts/run-umber-guarded.py" "${fixture_repo}/scripts/"
printf '{}\n' > "${fixture_repo}/distribution/manifest-v3.json"
printf 'format fixture\n' > "${fixture_repo}/pdflatex.fmt"

cat > "${fixture_repo}/tests/latex-source.lock" <<'EOF'
distribution fixture
distribution_sha256 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
format_schema 11
source_date_epoch 1
source tex/base.tex 1 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
local tests/local.tex 2 bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
pdflatex-source fonts/font.tfm 3 cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc
EOF
cat > "${fixture_repo}/tests/latex/pdflatex-representative.lock" <<'EOF'
source tex tex/runtime.tex 4 dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd
source tfm fonts/runtime.tfm 5 eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee
EOF

cat > "${fixture_repo}/bin/umber" <<'EOF'
#!/usr/bin/env python3
import json
import os
from pathlib import Path
import sys

cache = Path(os.environ["XDG_CACHE_HOME"])
row = {
    "args": sys.argv[1:],
    "cache": str(cache),
    "cache_entries": len(list(cache.iterdir())),
    "texinputs": os.environ.get("TEXINPUTS"),
    "texfonts": os.environ.get("TEXFONTS"),
}
with Path(os.environ["UMBER_TEST_INVOCATIONS"]).open("a") as output:
    output.write(json.dumps(row) + "\n")
EOF
chmod +x "${fixture_repo}/bin/umber"

invocations="${tmp_root}/invocations.jsonl"
receipt="${tmp_root}/receipt.txt"
UMBER_TEST_INVOCATIONS="$invocations" \
  "${fixture_repo}/scripts/check-latex-representative-resources.sh" \
    --distribution "${fixture_repo}/distribution" \
    --distribution-sha256 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
    --format "${fixture_repo}/pdflatex.fmt" \
    --umber "${fixture_repo}/bin/umber" \
    --receipt "$receipt" >/dev/null

python3 - "$invocations" "$receipt" "${fixture_repo}/pdflatex.fmt" <<'PY'
import hashlib
import json
from pathlib import Path
import sys

rows = [json.loads(line) for line in Path(sys.argv[1]).read_text().splitlines()]
assert len(rows) == 2, rows

def prefetch(row):
    args = row["args"]
    return {args[index + 1] for index, value in enumerate(args) if value == "--prefetch-input"}

source = next(row for row in rows if "--format" not in row["args"])
loaded = next(row for row in rows if "--format" in row["args"])
runtime = {"tex:runtime.tex", "tfm:runtime.tfm"}
assert prefetch(source) == {"tex:base.tex", "tex:local.tex", "tfm:font.tfm"} | runtime, source
assert prefetch(loaded) == runtime, loaded
assert source["cache"] != loaded["cache"]
assert source["cache_entries"] == loaded["cache_entries"] == 0
assert source["texinputs"] is loaded["texinputs"] is None
assert source["texfonts"] is loaded["texfonts"] is None

receipt = dict(line.split("=", 1) for line in Path(sys.argv[2]).read_text().splitlines())
assert receipt["schema"] == "1"
assert receipt["source_prefetch_keys"] == "5"
assert receipt["loaded_prefetch_keys"] == "2"
assert receipt["source_smoke"] == receipt["loaded_smoke"] == "pass"
format_bytes = Path(sys.argv[3]).read_bytes()
assert receipt["format_sha256"] == hashlib.sha256(format_bytes).hexdigest()
assert receipt["format_bytes"] == str(len(format_bytes))
PY

printf '%s\n' 'check-latex-representative-resources tests: PASS'
