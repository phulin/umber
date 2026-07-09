#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Explicit slow tier: run live reference checks and full DVI corpora. The
# default cargo tests and scripts/check.sh must stay fixture-only and hermetic.
export UMBER_LIVE_REF="${UMBER_LIVE_REF:-1}"

cargo test -p umber --test it run_exec_corpus_matches_pdftex_diagnostics
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
cm_tfm_dir="${repo_root}/crates/tex-fonts/tests/fixtures/cm"
cm_tfms=(
  "${cm_tfm_dir}/cmr10.tfm"
  "${cm_tfm_dir}/cmmi10.tfm"
  "${cm_tfm_dir}/cmsy10.tfm"
  "${cm_tfm_dir}/cmex10.tfm"
)
tmp_root="$(mktemp -d)"
trap 'rm -rf "$tmp_root"' EXIT

for corpus_name in dvi page math align leaders; do
  corpus_dir="${repo_root}/tests/corpus/${corpus_name}"
  for source in "${corpus_dir}"/*.tex; do
    case_name="$(basename "$source")"
    case_dir="${tmp_root}/${corpus_name}-${case_name%.tex}"
    mkdir -p "$case_dir"
    cp "$source" "${case_dir}/${case_name}"
    extra_inputs=()
    for tfm in "${cm_tfms[@]}"; do
      cp "$tfm" "${case_dir}/$(basename "$tfm")"
      extra_inputs+=(--extra-input "$(basename "$tfm")")
    done
    while IFS= read -r support; do
      support_name="$(basename "$support")"
      cp "$support" "${case_dir}/${support_name}"
      extra_inputs+=(--extra-input "$support_name")
    done < <(find "$corpus_dir" -maxdepth 1 -type f ! -name '*.tex' | sort)
    printf 'DVI parity: %s/%s\n' "$corpus_name" "$case_name" >&2
    (
      cd "$case_dir"
      "$umber_bin" run "$case_name" --dvi actual.dvi >/dev/null
      refexec_args=("$case_name" --compare-dvi actual.dvi)
      if [[ "$corpus_name" == "math" ]]; then
        refexec_args+=(--ini)
      fi
      "$refexec_bin" "${refexec_args[@]}" "${extra_inputs[@]}"
    )
  done
done
