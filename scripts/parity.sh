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
