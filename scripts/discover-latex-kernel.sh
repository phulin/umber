#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
texmf_dist=/usr/local/texlive/2026/texmf-dist
keep_work=false
expected_first_error=

usage() {
  echo "usage: $0 [--texmf-dist PATH] [--expect-first-error TEXT] [--keep-work]" >&2
}

while (($#)); do
  case "$1" in
    --texmf-dist)
      (($# >= 2)) || { usage; exit 2; }
      texmf_dist=$2
      shift 2
      ;;
    --expect-first-error)
      (($# >= 2)) || { usage; exit 2; }
      expected_first_error=$2
      shift 2
      ;;
    --keep-work)
      keep_work=true
      shift
      ;;
    *)
      usage
      exit 2
      ;;
  esac
done

latex_ltx="$texmf_dist/tex/latex/base/latex.ltx"
expl3_ltx="$texmf_dist/tex/latex/l3kernel/expl3.ltx"
expl3_code="$texmf_dist/tex/latex/l3kernel/expl3-code.tex"
font_area="$texmf_dist/fonts/tfm/public/cm"
latex_font_area="$texmf_dist/fonts/tfm/public/latex-fonts"
unicode_data="$texmf_dist/tex/generic/unicode-data"
babel_area="$texmf_dist/tex/generic/babel"
hyphen_area="$texmf_dist/tex/generic/hyphen"

hash_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

check_hash() {
  local path=$1
  local expected=$2
  [[ -f "$path" ]] || { echo "missing pinned input: $path" >&2; exit 2; }
  local actual
  actual=$(hash_file "$path")
  [[ "$actual" == "$expected" ]] || {
    echo "pinned input hash mismatch: $path" >&2
    echo "expected $expected" >&2
    echo "actual   $actual" >&2
    exit 2
  }
}

check_hash "$latex_ltx" 19d3b75884681539e1ef7a427af472bfe80d9ff214b84b9ca45e790d3e89c5c4
check_hash "$expl3_ltx" 822839097244afbd63ee1bfdf17f079717e35c349a77aa13437aaf7b7f62b31b
check_hash "$expl3_code" f32e3b68513ff880ad4aaa2822df00526f73b28bc008fa0d4ae11aa74c53945d
check_hash "$unicode_data/UnicodeData.txt" 2e1efc1dcb59c575eedf5ccae60f95229f706ee6d031835247d843c11d96470c
check_hash "$unicode_data/CaseFolding.txt" ff8d8fefbf123574205085d6714c36149eb946d717a0c585c27f0f4ef58c4183
check_hash "$unicode_data/SpecialCasing.txt" efc25faf19de21b92c1194c111c932e03d2a5eaf18194e33f1156e96de4c9588
check_hash "$unicode_data/GraphemeBreakProperty.txt" d6b51d1d2ae5c33b451b7ed994b48f1f4dc62b2272a5831e7fd418514a6bae89
check_hash "$unicode_data/WordBreakProperty.txt" 72274cac1e6b919507db35655c3e175aa27274668a1ece95c28d2069f2ad9852
check_hash "$babel_area/hyphen.cfg" 0451f25065b15542fb2703281ac8442739eb5d0658fb8c1a3be41a01e5c8be1b
check_hash "$babel_area/switch.def" 38b68dcf48519643a5bd7e09c20fbf1307a92c2470bd129397db7c11c0bf6c19
check_hash "$hyphen_area/hyphen.tex" 2c18acdc04c1a066aeb1759905e7ca449f0616c314b5ed6aebe55b9d4a89b8d4
[[ -d "$font_area" ]] || { echo "missing pinned font area: $font_area" >&2; exit 2; }
[[ -d "$latex_font_area" ]] || { echo "missing pinned font area: $latex_font_area" >&2; exit 2; }

work=$(mktemp -d "${TMPDIR:-/tmp}/umber-latex-discovery.XXXXXX")
cleanup() {
  if [[ "$keep_work" == true ]]; then
    echo "discovery artifacts: $work" >&2
  else
    rm -rf "$work"
  fi
}
trap cleanup EXIT

if [[ -z "${UMBER_BIN:-}" ]]; then
  cargo build --manifest-path "$root/Cargo.toml" --profile test -p umber
  umber_bin=$root/target/debug/umber
else
  umber_bin=$UMBER_BIN
fi

set +e
(
  cd "$work"
  env \
    SOURCE_DATE_EPOCH=1784066400 \
    TEXINPUTS="$texmf_dist/tex/latex/base:$texmf_dist/tex/latex/l3kernel:$unicode_data:$babel_area:$hyphen_area" \
    TEXFONTS="$font_area:$latex_font_area" \
    "$umber_bin" run --latex "$latex_ltx" --format-out latex.fmt
) >"$work/umber.stdout" 2>"$work/umber.stderr"
status=$?
set -e

first_error=$(grep -m1 '^! ' "$work/umber.stdout" || true)
if [[ -n "$first_error" ]]; then
  echo "LaTeX discovery first diagnostic: $first_error" >&2
  if [[ -n "$expected_first_error" && "$first_error" == *"$expected_first_error"* ]]; then
    exit 0
  fi
  exit 1
fi
if ((status != 0)); then
  echo "LaTeX discovery failed before a recoverable TeX diagnostic:" >&2
  sed -n '1,40p' "$work/umber.stderr" >&2
  exit "$status"
fi
if [[ ! -f "$work/latex.fmt" ]]; then
  echo "LaTeX discovery finished without producing latex.fmt" >&2
  exit 1
fi

echo "LaTeX kernel bootstrap completed without diagnostics"
