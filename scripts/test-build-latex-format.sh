#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp_root="$(mktemp -d)"
trap 'rm -rf "$tmp_root"' EXIT

fixture_repo="${tmp_root}/repo"
mkdir -p \
  "${fixture_repo}/scripts" \
  "${fixture_repo}/tests/latex" \
  "${fixture_repo}/texmf-dist/tex/latex/tex-ini-files" \
  "${fixture_repo}/texmf-dist/tex/latex-dev/base" \
  "${fixture_repo}/distribution" \
  "${fixture_repo}/target/release" \
  "${fixture_repo}/tools/texlive-wasm-publish/target/release" \
  "${tmp_root}/bin" \
  "${tmp_root}/cache"
cp "${repo_root}/scripts/build-latex-format.sh" "${fixture_repo}/scripts/"
printf '\\dump\n' > "${fixture_repo}/texmf-dist/tex/latex-dev/base/latex.ltx"
printf '\\end\n' > "${fixture_repo}/tests/latex/format-equivalence.tex"
printf '\\end\n' > "${fixture_repo}/tests/latex/pdflatex-smoke.tex"
printf 'pdf configuration\n' > "${fixture_repo}/tests/latex/pdftexconfig.tex"
printf '\\input latex.ltx\n' > "${fixture_repo}/texmf-dist/tex/latex/tex-ini-files/pdflatex.ini"
printf '{"schema":6}\n' > "${fixture_repo}/distribution/manifest-v6.json"

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

distribution_ahash64=dddddddddddddddd
source_sha256="$(sha256_file "${fixture_repo}/texmf-dist/tex/latex-dev/base/latex.ltx")"
pdflatex_source_sha256="$(sha256_file "${fixture_repo}/texmf-dist/tex/latex/tex-ini-files/pdflatex.ini")"
pdftexconfig_sha256="$(sha256_file "${fixture_repo}/tests/latex/pdftexconfig.tex")"

cat > "${fixture_repo}/tests/latex-source.lock" <<EOF
distribution fixture
distribution_ahash64 ${distribution_ahash64}
format_schema 12
source_date_epoch 1
source tex/latex-dev/base/latex.ltx 6 ${source_sha256}
pdflatex-source tex/latex/tex-ini-files/pdflatex.ini 17 ${pdflatex_source_sha256}
pdflatex-local tests/latex/pdftexconfig.tex 18 ${pdftexconfig_sha256}
EOF

cat > "${fixture_repo}/tests/latex/pdflatex-representative.lock" <<'EOF'
source tex tex/runtime-a.tex 9 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
source tfm fonts/runtime-b.tfm 10 bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
EOF

expected_receipt="${tmp_root}/expected.inputs"
printf '6\t%s\n' \
  "${fixture_repo}/texmf-dist/tex/latex-dev/base/latex.ltx" > "$expected_receipt"
pdflatex_expected_receipt="${tmp_root}/pdflatex-expected.inputs"
{
  printf '6\t%s\n' "${fixture_repo}/texmf-dist/tex/latex-dev/base/latex.ltx"
  printf '17\t%s\n' "${fixture_repo}/texmf-dist/tex/latex/tex-ini-files/pdflatex.ini"
  printf '18\t%s\n' "${fixture_repo}/tests/latex/pdftexconfig.tex"
} | LC_ALL=C sort > "$pdflatex_expected_receipt"
invocations="${tmp_root}/run-invocations.jsonl"

cat > "${tmp_root}/bin/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "${1:-}" in
  build) exit 0 ;;
  pkgid) printf '%s\n' 'path+file:///fixture#umber@0.1.0' ;;
  *) exit 2 ;;
esac
EOF
cat > "${tmp_root}/bin/rustc" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' 'rustc 1.93.0 (fixture)' 'host: x86_64-unknown-linux-gnu'
EOF
cat > "${fixture_repo}/scripts/run-umber-guarded.py" <<'EOF'
#!/usr/bin/env python3
import os
import subprocess
import sys

separator = sys.argv.index("--")
raise SystemExit(subprocess.run(sys.argv[separator + 1 :], env=os.environ).returncode)
EOF
cat > "${fixture_repo}/target/release/umber" <<'EOF'
#!/usr/bin/env python3
import json
import os
from pathlib import Path
import struct
import sys

arguments = sys.argv[1:]
if arguments[:2] == ["format-cache", "restore"]:
    print("miss")
    raise SystemExit(0)
if arguments[:2] == ["format-cache", "store"]:
    raise SystemExit(0)
if arguments[:1] != ["run"]:
    raise SystemExit(2)
with Path(os.environ["UMBER_TEST_INVOCATIONS"]).open("a", encoding="utf-8") as output:
    output.write(json.dumps(arguments) + "\n")
if "--format-out" in arguments:
    output = Path(arguments[arguments.index("--format-out") + 1])
    output.write_bytes(b"UMBRFMT\0" + struct.pack("<I", 12) + b"fixture")
if "--input-records-out" in arguments:
    output = Path(arguments[arguments.index("--input-records-out") + 1])
    output.write_bytes(Path(os.environ["UMBER_TEST_INPUT_RECEIPT"]).read_bytes())
for option in ("--dvi", "--pdf"):
    if option in arguments:
        Path(arguments[arguments.index(option) + 1]).write_bytes(b"artifact\n")
EOF
cat > "${fixture_repo}/tools/texlive-wasm-publish/target/release/texlive-wasm-publish" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ "$1" == --file-ahash64 && -f "$2" ]]
case "${2##*/}" in
  manifest-v6.json) printf '%s\n' dddddddddddddddd ;;
  *.fmt) printf '%s\n' eeeeeeeeeeeeeeee ;;
  *) printf '%s\n' ffffffffffffffff ;;
esac
EOF
chmod +x \
  "${tmp_root}/bin/cargo" \
  "${tmp_root}/bin/rustc" \
  "${fixture_repo}/scripts/build-latex-format.sh" \
  "${fixture_repo}/scripts/run-umber-guarded.py" \
  "${fixture_repo}/tools/texlive-wasm-publish/target/release/texlive-wasm-publish" \
  "${fixture_repo}/target/release/umber"

expect_failure() {
  local expected="$1"
  shift
  local output="${tmp_root}/failure.txt"
  if "$@" > "$output" 2>&1; then
    printf 'expected command to fail: %s\n' "$*" >&2
    exit 1
  fi
  grep -F -- "$expected" "$output" >/dev/null
}

builder="${fixture_repo}/scripts/build-latex-format.sh"
expect_failure '--distribution PATH is required' "$builder"
expect_failure '--distribution-ahash64 AHASH64 is required' \
  "$builder" --distribution "${fixture_repo}/distribution"
expect_failure '--distribution-ahash64 must be 16 lowercase hexadecimal characters' \
  "$builder" \
    --distribution "${fixture_repo}/distribution" \
    --distribution-ahash64 BAD
expect_failure 'distribution aHash64 does not match the source lock' \
  "$builder" \
    --distribution "${fixture_repo}/distribution" \
    --distribution-ahash64 0000000000000000
expect_failure 'distribution path is not a local file or directory' \
  "$builder" \
    --distribution "${fixture_repo}/absent" \
    --distribution-ahash64 "$distribution_ahash64"

PATH="${tmp_root}/bin:${PATH}" \
XDG_CACHE_HOME="${tmp_root}/cache" \
UMBER_OFFLINE=1 \
UMBER_TEST_INPUT_RECEIPT="$expected_receipt" \
UMBER_TEST_INVOCATIONS="$invocations" \
  "$builder" \
    --texmf-dist "${fixture_repo}/texmf-dist" \
    --distribution "${fixture_repo}/distribution" \
    --distribution-ahash64 "$distribution_ahash64" \
    --output-dir "${fixture_repo}/output" \
    --force >/dev/null

python3 - "$invocations" "${fixture_repo}/distribution" "$distribution_ahash64" <<'PY'
import json
from pathlib import Path
import sys

rows = [json.loads(line) for line in Path(sys.argv[1]).read_text().splitlines()]
assert len(rows) == 4, rows
expected_path = str(Path(sys.argv[2]).resolve())
expected_digest = sys.argv[3]
for row in rows:
    assert row.count("--distribution") == 1, row
    assert row[row.index("--distribution") + 1] == expected_path, row
    assert row.count("--distribution-ahash64") == 1, row
    assert row[row.index("--distribution-ahash64") + 1] == expected_digest, row
    assert row.count("--offline") == 1, row
assert sum("--format-out" in row for row in rows) == 2, rows
assert sum("--format" in row for row in rows) == 1, rows
assert sum("--format-out" not in row and "--format" not in row for row in rows) == 1, rows
PY

: > "$invocations"
PATH="${tmp_root}/bin:${PATH}" \
XDG_CACHE_HOME="${tmp_root}/cache" \
UMBER_OFFLINE=1 \
UMBER_TEST_INPUT_RECEIPT="$pdflatex_expected_receipt" \
UMBER_TEST_INVOCATIONS="$invocations" \
  "$builder" \
    --engine pdflatex \
    --texmf-dist "${fixture_repo}/texmf-dist" \
    --distribution "${fixture_repo}/distribution" \
    --distribution-ahash64 "$distribution_ahash64" \
    --output-dir "${fixture_repo}/pdflatex-output" \
    --force >/dev/null

python3 - "$invocations" <<'PY'
import json
from pathlib import Path
import sys

rows = [json.loads(line) for line in Path(sys.argv[1]).read_text().splitlines()]
assert len(rows) == 4, rows

def prefetch(row):
    return {
        row[index + 1]
        for index, argument in enumerate(row)
        if argument == "--prefetch-input"
    }

source_closure = {"tex:latex.ltx", "tex:pdflatex.ini", "tex:pdftexconfig.tex"}
runtime_closure = {"tex:runtime-a.tex", "tfm:runtime-b.tfm"}
source = next(row for row in rows if "--format-out" not in row and "--format" not in row)
loaded = next(row for row in rows if "--format" in row)
assert prefetch(source) == source_closure | runtime_closure, source
assert prefetch(loaded) == runtime_closure, loaded
PY

printf '%s\n' 'build-latex-format tests: PASS'
