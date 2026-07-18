#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
lock_file="${repo_root}/tests/latex-source.lock"
engine="latex"
output_dir=""
texmf_dist="${UMBER_TEXMF_DIST:-/usr/local/texlive/2026/texmf-dist}"
publish_input_closure=0

usage() {
  cat <<'EOF'
usage: scripts/build-latex-format.sh [--engine latex|pdflatex]
                                     [--texmf-dist PATH] [--output-dir PATH]
                                     [--publish-input-closure]

Builds one pinned Umber-native LaTeX format twice, requires byte identity and
the exact mode-specific locked input closure, then compares a source-initialized
representative document with the format-loaded job. The default output is
target/<engine>-format.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --engine)
      [[ $# -ge 2 ]] || { printf '%s\n' 'missing mode after --engine' >&2; exit 2; }
      engine="$2"
      shift 2
      ;;
    --texmf-dist)
      [[ $# -ge 2 ]] || { printf '%s\n' 'missing path after --texmf-dist' >&2; exit 2; }
      texmf_dist="$2"
      shift 2
      ;;
    --output-dir)
      [[ $# -ge 2 ]] || { printf '%s\n' 'missing path after --output-dir' >&2; exit 2; }
      output_dir="$2"
      shift 2
      ;;
    --publish-input-closure)
      publish_input_closure=1
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      printf 'build-latex-format.sh: unknown option: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

case "$engine" in
  latex)
    fixture="${repo_root}/tests/latex/format-equivalence.tex"
    format_input="${texmf_dist}/tex/latex/base/latex.ltx"
    output_extension=dvi
    ;;
  pdflatex)
    fixture="${repo_root}/tests/latex/pdflatex-smoke.tex"
    format_input="${texmf_dist}/tex/latex/tex-ini-files/pdflatex.ini"
    output_extension=pdf
    ;;
  *)
    printf 'build-latex-format.sh: unsupported engine: %s\n' "$engine" >&2
    usage >&2
    exit 2
    ;;
esac
format_name="$engine"
output_dir="${output_dir:-${repo_root}/target/${format_name}-format}"

fail() {
  printf 'build-latex-format.sh: %s\n' "$*" >&2
  exit 1
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

[[ -d "$texmf_dist" ]] || fail "missing pinned texmf-dist root: $texmf_dist"
[[ -f "$lock_file" ]] || fail "missing source lock: $lock_file"
[[ -f "$fixture" ]] || fail "missing equivalence fixture: $fixture"
[[ -f "$format_input" ]] || fail "missing format entry point: $format_input"

distribution="$(awk '$1 == "distribution" { print $2 }' "$lock_file")"
format_schema="$(awk '$1 == "format_schema" { print $2 }' "$lock_file")"
source_date_epoch="$(awk '$1 == "source_date_epoch" { print $2 }' "$lock_file")"
[[ -n "$distribution" && -n "$format_schema" && -n "$source_date_epoch" ]] || \
  fail "source lock is missing required metadata"

scratch_parent="${UMBER_LATEX_FORMAT_WORK_ROOT:-${output_dir}/work}"
mkdir -p "$scratch_parent"
tmp_root="$(mktemp -d "${scratch_parent}/build.XXXXXX")"
cleanup() {
  local status=$?
  if [[ $status -eq 0 ]]; then
    rm -rf "$tmp_root"
  else
    printf 'build-latex-format.sh: failed artifacts: %s\n' "$tmp_root" >&2
  fi
}
trap cleanup EXIT
expected_receipt="${tmp_root}/expected.inputs"
expected_index="${tmp_root}/expected.index"
closure_index="${tmp_root}/input-closure.index"
: > "$expected_index"
: > "$closure_index"

while read -r kind relative expected_bytes expected_hash extra; do
  [[ -z "${kind:-}" || "$kind" == \#* ]] && continue
  case "$kind" in
    source)
      source="${texmf_dist}/${relative}"
      ;;
    local)
      source="${repo_root}/${relative}"
      ;;
    pdflatex-source)
      [[ "$engine" == pdflatex ]] || continue
      source="${texmf_dist}/${relative}"
      ;;
    pdflatex-local)
      [[ "$engine" == pdflatex ]] || continue
      source="${repo_root}/${relative}"
      ;;
    *)
      continue
      ;;
  esac
  [[ -z "${extra:-}" ]] || fail "invalid source lock entry for $relative"
  [[ "$relative" != /* && "$relative" != *..* && "$relative" != *\\* ]] || \
    fail "unsafe source path in lock: $relative"
  [[ -f "$source" ]] || fail "missing pinned source: $source"
  actual_bytes="$(wc -c < "$source" | tr -d ' ')"
  [[ "$actual_bytes" == "$expected_bytes" ]] || \
    fail "length mismatch for $relative: expected $expected_bytes, got $actual_bytes"
  actual_hash="$(sha256 "$source")"
  [[ "$actual_hash" == "$expected_hash" ]] || \
    fail "hash mismatch for $relative: expected $expected_hash, got $actual_hash"
  printf '%s\t%s\n' "$source" "$expected_bytes" >> "$expected_index"
  request_name="${relative##*/}"
  [[ "$request_name" =~ ^[A-Za-z0-9._/-]+$ ]] || \
    fail "source lock path has no canonical request key: $relative"
  if [[ "$request_name" == *.tfm ]]; then
    printf 'tfm:%s\n' "$request_name" >> "$closure_index"
  else
    printf 'tex:%s\n' "$request_name" >> "$closure_index"
  fi
done < "$lock_file"
LC_ALL=C sort -k1,1 "$expected_index" | awk -F '\t' '{ print $2 "\t" $1 }' > "$expected_receipt"
LC_ALL=C sort -u "$closure_index" -o "$closure_index"

texinputs="${repo_root}/tests/latex:${texmf_dist}/tex/latex/base:${texmf_dist}/tex/latex/l3kernel:${texmf_dist}/tex/latex/l3backend:${texmf_dist}/tex/generic/unicode-data:${texmf_dist}/tex/generic/babel:${texmf_dist}/tex/generic/hyphen:${texmf_dist}/tex/generic/pdftex"
texfonts="${texmf_dist}/fonts/tfm/public/cm:${texmf_dist}/fonts/tfm/public/latex-fonts:${texmf_dist}/fonts/tfm/jknappen/ec"
latex_ltx="${texmf_dist}/tex/latex/base/latex.ltx"

cd "$repo_root"
cargo build --release -p umber
umber_bin="${CARGO_TARGET_DIR:-${repo_root}/target}/release/umber"
[[ -x "$umber_bin" ]] || fail "Umber binary was not built at $umber_bin"

run_engine() {
  local directory="$1"
  shift
  (
    cd "$directory"
    env SOURCE_DATE_EPOCH="$source_date_epoch" TEXINPUTS="$texinputs" TEXFONTS="$texfonts" \
      "$umber_bin" run "--${engine}" "$@"
  )
}

build_one() {
  local directory="$1"
  mkdir -p "$directory"
  run_engine "$directory" "$format_input" --format-out "${format_name}.fmt" \
    --input-records-out build.inputs > "${directory}/build.stdout" 2> "${directory}/build.stderr"
  if grep -q '^! ' "${directory}/build.stdout"; then
    grep -m1 '^! ' "${directory}/build.stdout" >&2
    fail "LaTeX format build emitted a diagnostic"
  fi
  cmp "$expected_receipt" "${directory}/build.inputs" || \
    fail "LaTeX format build opened inputs outside the locked closure"
}

build_one "${tmp_root}/first"
build_one "${tmp_root}/second"
cmp "${tmp_root}/first/${format_name}.fmt" "${tmp_root}/second/${format_name}.fmt" || \
  fail "two clean ${format_name} format generations were not byte-identical"

format_file="${tmp_root}/first/${format_name}.fmt"
magic="$(od -An -t x1 -N 8 "$format_file" | tr -d ' \n')"
actual_schema="$(od -An -t u4 -j 8 -N 4 "$format_file" | tr -d ' \n')"
[[ "$magic" == 554d4252464d5400 ]] || fail "generated file lacks Umber format magic"
[[ "$actual_schema" == "$format_schema" ]] || \
  fail "generated schema $actual_schema does not match locked schema $format_schema"

source_dir="${tmp_root}/source"
loaded_dir="${tmp_root}/loaded"
mkdir -p "$source_dir" "$loaded_dir"
cp "$fixture" "${source_dir}/representative.tex"
cp "$fixture" "${loaded_dir}/representative.tex"
cp "$format_file" "${loaded_dir}/${format_name}.fmt"
awk '
  $0 == sprintf("%c%s", 92, "dump") {
    print sprintf("%c%s", 92, "input representative")
    next
  }
  { print }
' "$latex_ltx" > "${source_dir}/latex-source.ltx"
if [[ "$engine" == pdflatex ]]; then
  printf '\\input pdftexconfig.tex\n\\input latex-source.ltx\n' > "${source_dir}/document.tex"
else
  printf '\\input latex-source.ltx\n' > "${source_dir}/document.tex"
fi
printf '\input representative\n' > "${loaded_dir}/document.tex"

output_args=("--${output_extension}" "document.${output_extension}")
run_engine "$source_dir" document.tex "${output_args[@]}" \
  > "${source_dir}/document.stdout" 2> "${source_dir}/document.stderr"
run_engine "$loaded_dir" document.tex --format "${format_name}.fmt" "${output_args[@]}" \
  > "${loaded_dir}/document.stdout" 2> "${loaded_dir}/document.stderr"
for directory in "$source_dir" "$loaded_dir"; do
  if grep -q '^! ' "${directory}/document.stdout"; then
    grep -m1 '^! ' "${directory}/document.stdout" >&2
    fail "representative LaTeX job emitted a diagnostic"
  fi
done
cmp "${source_dir}/document.${output_extension}" "${loaded_dir}/document.${output_extension}" || \
  fail "source-initialized and format-loaded ${format_name} ${output_extension^^} differ"
cmp "${source_dir}/document.aux" "${loaded_dir}/document.aux" || \
  fail "source-initialized and format-loaded ${format_name} auxiliary effects differ"

format_sha256="$(sha256 "$format_file")"
format_bytes="$(wc -c < "$format_file" | tr -d ' ')"
source_manifest_sha256="$(sha256 "$lock_file")"
package_id="$(cargo pkgid -p umber)"
engine_version="${package_id##*#}"
metadata_schema=1
closure_metadata=""
if [[ "$publish_input_closure" -eq 1 ]]; then
  metadata_schema=2
  input_closure_json="$(awk '
    BEGIN { print "    \"keys\": [" }
    { printf "%s      \"%s\"", NR == 1 ? "" : ",\n", $0 }
    END { print "\n    ]" }
  ' "$closure_index")"
  closure_metadata="$(printf ',\n  "inputClosure": {\n    "schema": 1,\n%s\n  }' "$input_closure_json")"
fi

cat > "${tmp_root}/${format_name}-format.json" <<EOF
{
  "schema": ${metadata_schema},
  "name": "${format_name}",
  "object": "sha256-${format_sha256}",
  "sha256": "${format_sha256}",
  "bytes": ${format_bytes},
  "engine": "umber",
  "engineVersion": "${engine_version}",
  "formatSchema": ${format_schema},
  "sourceDistribution": "${distribution}",
  "sourceManifestSha256": "${source_manifest_sha256}",
  "sourceDateEpoch": ${source_date_epoch}${closure_metadata}
}
EOF

mkdir -p "$output_dir"
cp "$format_file" "${output_dir}/${format_name}.fmt"
cp "${tmp_root}/${format_name}-format.json" "${output_dir}/${format_name}-format.json"

printf 'Umber %s format: sha256=%s bytes=%s schema=%s source=%s\n' \
  "$format_name" "$format_sha256" "$format_bytes" "$format_schema" "$distribution"
