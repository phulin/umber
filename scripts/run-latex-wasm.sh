#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bundle_dir="${repo_root}/target/latex-wasm"
package_dir="${repo_root}/target/umber-wasm-package"
tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/umber-latex-wasm-check.XXXXXX")"
trap 'rm -rf "$tmp_root"' EXIT
native_format="${tmp_root}/format/latex.fmt"

"${repo_root}/scripts/build-wasm-latex-bundle.sh" \
  --output-dir "$bundle_dir" --format-output "$native_format"
"${repo_root}/scripts/build-wasm-package.sh" "$package_dir"

source_date_epoch="$(awk '$1 == "source_date_epoch" { print $2 }' "${repo_root}/tests/latex-source.lock")"
texmf_dist="${UMBER_TEXMF_DIST:-${repo_root}/third_party/texlive-20260301-texmf/texmf-dist}"
texinputs="${texmf_dist}/tex/latex/base:${texmf_dist}/tex/latex/l3kernel:${texmf_dist}/tex/latex/l3backend:${texmf_dist}/tex/generic/unicode-data:${texmf_dist}/tex/generic/babel:${texmf_dist}/tex/generic/hyphen"
texfonts="${texmf_dist}/fonts/tfm/public/cm:${texmf_dist}/fonts/tfm/public/latex-fonts:${texmf_dist}/fonts/tfm/jknappen/ec"
native_dir="${tmp_root}/native"
wasm_dir="${tmp_root}/wasm"
mkdir -p "$native_dir" "$wasm_dir"
cp "${repo_root}/tests/latex/article.tex" "${native_dir}/document.tex"
cp "$native_format" "${native_dir}/latex.fmt"
: > "${native_dir}/document.aux"
: > "${native_dir}/document.toc"

for pass in 1 2 3; do
  (
    cd "$native_dir"
    env SOURCE_DATE_EPOCH="$source_date_epoch" TEXINPUTS="$texinputs" TEXFONTS="$texfonts" \
      "${repo_root}/target/release/umber" run --latex document.tex --format latex.fmt \
        --dvi document.dvi > document.stdout 2> document.stderr
  )
done

node "${repo_root}/scripts/check-latex-wasm.mjs" \
  "$package_dir" "$bundle_dir" "${repo_root}/tests/latex/article.tex" "$wasm_dir"
cmp "${native_dir}/document.dvi" "${wasm_dir}/document.dvi"
for extension in aux toc lof lot out; do
  if [[ -e "${native_dir}/document.${extension}" || -e "${wasm_dir}/document.${extension}" ]]; then
    cmp "${native_dir}/document.${extension}" "${wasm_dir}/document.${extension}"
  fi
done
printf 'LaTeX native/WASM parity: article (3 passes)\n'
