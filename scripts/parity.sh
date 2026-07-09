#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

cargo test -p umber --test it run_typeset_corpus_matches_pdftex_box_dumps

if [[ -f third_party/hyphen/hyphen.tex ]]; then
  cargo test -p umber --test it run_hyphen_showhyphens_corpus_matches_pdftex
else
  printf 'skipping hyphen parity: third_party/hyphen/hyphen.tex is absent; run scripts/fetch-hyphen-corpus.sh\n' >&2
fi

cargo build -p umber -p refexec

target_dir="${CARGO_TARGET_DIR:-target}"
umber_bin="${repo_root}/${target_dir}/debug/umber"
refexec_bin="${repo_root}/${target_dir}/debug/refexec"
cmr10_tfm="${repo_root}/crates/tex-fonts/tests/fixtures/cm/cmr10.tfm"
tmp_root="$(mktemp -d)"
trap 'rm -rf "$tmp_root"' EXIT

for corpus_name in dvi page; do
  corpus_dir="${repo_root}/tests/corpus/${corpus_name}"
  for source in "${corpus_dir}"/*.tex; do
    case_name="$(basename "$source")"
    case_dir="${tmp_root}/${corpus_name}-${case_name%.tex}"
    mkdir -p "$case_dir"
    cp "$source" "${case_dir}/${case_name}"
    cp "$cmr10_tfm" "${case_dir}/cmr10.tfm"
    printf 'DVI parity: %s/%s\n' "$corpus_name" "$case_name" >&2
    (
      cd "$case_dir"
      "$umber_bin" run "$case_name" --dvi actual.dvi >/dev/null
      "$refexec_bin" "$case_name" --compare-dvi actual.dvi --extra-input cmr10.tfm
    )
  done
done
