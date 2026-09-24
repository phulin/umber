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
cp "${repo_root}/scripts/verify-latex-format-inputs.py" "${fixture_repo}/scripts/"
cp "${repo_root}/scripts/verify-latex-corpus-inputs.py" "${fixture_repo}/scripts/"
cp "${repo_root}/scripts/latex_input_admissions.py" "${fixture_repo}/scripts/"
printf '\\dump\n' > "${fixture_repo}/texmf-dist/tex/latex-dev/base/latex.ltx"
printf '\\end\n' > "${fixture_repo}/tests/latex/format-equivalence.tex"
printf '\\end\n' > "${fixture_repo}/tests/latex/pdflatex-smoke.tex"
printf 'pdf configuration\n' > "${fixture_repo}/tests/latex/pdftexconfig.tex"
printf '\\input latex.ltx\n' > "${fixture_repo}/texmf-dist/tex/latex/tex-ini-files/pdflatex.ini"
printf '{"schema":8}\n' > "${fixture_repo}/distribution/manifest.json"

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
{
  printf 'umber-input-admissions-v1\n'
  printf 'main\t6\tffffffffffffffff\n'
  printf 'file\tadmitted\ttex:extra-prefetch.tex\t23\tffffffffffffffff\n'
} > "$expected_receipt"
pdflatex_expected_receipt="${tmp_root}/pdflatex-expected.inputs"
{
  printf 'umber-input-admissions-v1\n'
  printf 'main\t17\tffffffffffffffff\n'
  printf 'file\tused\ttex:latex.ltx\t6\tffffffffffffffff\n'
  printf 'file\tused\ttex:pdftexconfig.tex\t18\tffffffffffffffff\n'
} > "$pdflatex_expected_receipt"
invocations="${tmp_root}/run-invocations.jsonl"
captured_build_configuration="${tmp_root}/build-configuration.txt"

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
if arguments[:2] in (["format-cache", "restore"], ["format-cache", "store"]):
    Path(os.environ["UMBER_TEST_BUILD_CONFIGURATION"]).write_bytes(
        Path(arguments[arguments.index("--build-configuration") + 1]).read_bytes()
    )
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
  manifest.json) printf '%s\n' dddddddddddddddd ;;
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
printf 'distribution_ahash64 %s\n' "$distribution_ahash64" >> "${fixture_repo}/tests/latex-source.lock"
expect_failure 'source lock must not pin a packaging root digest' \
  "$builder" \
    --distribution "${fixture_repo}/distribution" \
    --distribution-ahash64 "$distribution_ahash64"
sed '$d' "${fixture_repo}/tests/latex-source.lock" > "${tmp_root}/source-lock"
mv "${tmp_root}/source-lock" "${fixture_repo}/tests/latex-source.lock"
expect_failure 'distribution root digest mismatch' \
  env PATH="${tmp_root}/bin:${PATH}" "$builder" \
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
UMBER_TEST_BUILD_CONFIGURATION="$captured_build_configuration" \
  "$builder" \
    --texmf-dist "${fixture_repo}/texmf-dist" \
    --distribution "${fixture_repo}/distribution" \
    --distribution-ahash64 "$distribution_ahash64" \
    --output-dir "${fixture_repo}/output" \
    --force >/dev/null

grep -Fx 'schema=2' "$captured_build_configuration" >/dev/null
grep -Fx "producer-sha256=$(sha256_file "${fixture_repo}/target/release/umber")" \
  "$captured_build_configuration" >/dev/null
grep -Fx "builder-sha256=$(sha256_file "$builder")" \
  "$captured_build_configuration" >/dev/null

python3 - "$invocations" "${fixture_repo}/distribution" "$distribution_ahash64" <<'PY'
import json
from pathlib import Path
import sys

rows = [json.loads(line) for line in Path(sys.argv[1]).read_text().splitlines()]
assert len(rows) == 1, rows
expected_path = str(Path(sys.argv[2]).resolve())
expected_digest = sys.argv[3]
for row in rows:
    assert row.count("--distribution") == 1, row
    assert row[row.index("--distribution") + 1] == expected_path, row
    assert row.count("--distribution-ahash64") == 1, row
    assert row[row.index("--distribution-ahash64") + 1] == expected_digest, row
    assert row.count("--offline") == 1, row
assert sum("--format-out" in row for row in rows) == 1, rows
assert sum("--format" in row for row in rows) == 0, rows
assert sum("--format-out" not in row and "--format" not in row for row in rows) == 0, rows
PY

: > "$invocations"
PATH="${tmp_root}/bin:${PATH}" \
XDG_CACHE_HOME="${tmp_root}/cache" \
UMBER_OFFLINE=1 \
UMBER_TEST_INPUT_RECEIPT="$pdflatex_expected_receipt" \
UMBER_TEST_INVOCATIONS="$invocations" \
UMBER_TEST_BUILD_CONFIGURATION="$captured_build_configuration" \
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
assert len(rows) == 1, rows
assert "--format-out" in rows[0], rows
assert "--format" not in rows[0], rows
PY

identity_index="${tmp_root}/authorized.index"
{
  printf 'tex:latex.ltx\tffffffffffffffff\t6\n'
  printf 'tex:pdflatex.ini\tffffffffffffffff\t17\n'
  printf 'tex:pdftexconfig.tex\tffffffffffffffff\t18\n'
} > "$identity_index"
verify_inputs="${fixture_repo}/scripts/verify-latex-format-inputs.py"
python3 "$verify_inputs" --receipt "$expected_receipt" \
  --authorized "$identity_index" --main-key tex:latex.ltx
python3 "$verify_inputs" --receipt "$pdflatex_expected_receipt" \
  --authorized "$identity_index" --main-key tex:pdflatex.ini

consumed_extra="${tmp_root}/consumed-extra.inputs"
sed 's/file\tadmitted\t/file\tused\t/' "$expected_receipt" > "$consumed_extra"
expect_failure 'consumed tex:extra-prefetch.tex is outside the locked source closure' \
  python3 "$verify_inputs" --receipt "$consumed_extra" \
    --authorized "$identity_index" --main-key tex:latex.ltx

wrong_main="${tmp_root}/wrong-main.inputs"
sed 's/main\t6\t/main\t7\t/' "$expected_receipt" > "$wrong_main"
expect_failure 'main input differs from locked tex:latex.ltx' \
  python3 "$verify_inputs" --receipt "$wrong_main" \
    --authorized "$identity_index" --main-key tex:latex.ltx

wrong_identity="${tmp_root}/wrong-identity.inputs"
sed 's/tex:latex.ltx\t6\t/tex:latex.ltx\t7\t/' \
  "$pdflatex_expected_receipt" > "$wrong_identity"
expect_failure 'consumed tex:latex.ltx is outside the locked source closure' \
  python3 "$verify_inputs" --receipt "$wrong_identity" \
    --authorized "$identity_index" --main-key tex:pdflatex.ini

corpus_index="${tmp_root}/corpus.index"
printf 'tfm:cmr10.tfm\tffffffffffffffff\t1296\n' > "$corpus_index"
corpus_receipt="${tmp_root}/corpus.inputs"
{
  printf 'umber-input-admissions-v1\n'
  printf 'main\t21\tffffffffffffffff\n'
  printf 'file\tused\ttex:document.aux\t8\tffffffffffffffff\n'
  printf 'file\tadmitted\ttex:unused.sty\t10\tffffffffffffffff\n'
  printf 'file\tused\ttfm:cmr10.tfm\t1296\tffffffffffffffff\n'
} > "$corpus_receipt"
prior_generated="${tmp_root}/prior-generated.index"
printf 'tex:document.aux\tffffffffffffffff\t8\n' > "$prior_generated"
corpus_used="${tmp_root}/corpus-used.index"
corpus_verifier="${fixture_repo}/scripts/verify-latex-corpus-inputs.py"
corpus_args=(
  --authorized "$corpus_index"
  --main-bytes 21
  --main-ahash64 ffffffffffffffff
  --prior-generated "$prior_generated"
  --used-out "$corpus_used"
)
python3 "$corpus_verifier" --receipt "$corpus_receipt" "${corpus_args[@]}"
cmp "$corpus_index" "$corpus_used"

corpus_extra="${tmp_root}/corpus-extra.inputs"
sed 's/file\tadmitted\ttex:unused/file\tused\ttex:unused/' \
  "$corpus_receipt" > "$corpus_extra"
expect_failure 'consumed tex:unused.sty is outside the locked runtime closure' \
  python3 "$corpus_verifier" --receipt "$corpus_extra" "${corpus_args[@]}"

empty_prior_generated="${tmp_root}/empty-prior-generated.index"
: > "$empty_prior_generated"
expect_failure 'consumed tex:document.aux is outside the locked runtime closure' \
  python3 "$corpus_verifier" --receipt "$corpus_receipt" \
    --authorized "$corpus_index" --main-bytes 21 --main-ahash64 ffffffffffffffff \
    --prior-generated "$empty_prior_generated" --used-out "$corpus_used"

wrong_prior_generated="${tmp_root}/wrong-prior-generated.index"
printf 'tex:document.aux\tffffffffffffffff\t9\n' > "$wrong_prior_generated"
expect_failure 'consumed tex:document.aux differs from prior-pass generated output' \
  python3 "$corpus_verifier" --receipt "$corpus_receipt" \
    --authorized "$corpus_index" --main-bytes 21 --main-ahash64 ffffffffffffffff \
    --prior-generated "$wrong_prior_generated" --used-out "$corpus_used"

corpus_wrong_tfm="${tmp_root}/corpus-wrong-tfm.inputs"
sed 's/tfm:cmr10.tfm\t1296\t/tfm:cmr10.tfm\t1297\t/' \
  "$corpus_receipt" > "$corpus_wrong_tfm"
expect_failure 'consumed tfm:cmr10.tfm differs from locked runtime identity' \
  python3 "$corpus_verifier" --receipt "$corpus_wrong_tfm" "${corpus_args[@]}"

printf '%s\n' 'build-latex-format tests: PASS'
