#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
texmf_dist=/usr/local/texlive/2025/texmf-dist
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

check_hash "$latex_ltx" 8420904f06764a8cc2ec81e13215c22fec8e599c269160dcc02ad84507851f32
check_hash "$expl3_ltx" 5fe990d648915d271e08c1576f2e8f01ec72b0d769efce7f290915fab9bfbfeb
check_hash "$expl3_code" 7e765c50730451ddf9f1d4dec8a167ba6a3af567325caacb6a74cde3e1e1cab7
[[ -d "$font_area" ]] || { echo "missing pinned font area: $font_area" >&2; exit 2; }

work=$(mktemp -d "${TMPDIR:-/tmp}/umber-latex-discovery.XXXXXX")
cleanup() {
  if [[ "$keep_work" == true ]]; then
    echo "discovery artifacts: $work" >&2
  else
    rm -rf "$work"
  fi
}
trap cleanup EXIT

umber_bin=${UMBER_BIN:-$root/target/debug/umber}
if [[ ! -x "$umber_bin" ]]; then
  cargo build --manifest-path "$root/Cargo.toml" -p umber
fi

set +e
(
  cd "$work"
  env \
    SOURCE_DATE_EPOCH=1784066400 \
    TEXINPUTS="$texmf_dist/tex/latex/base:$texmf_dist/tex/latex/l3kernel" \
    TEXFONTS="$font_area" \
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
