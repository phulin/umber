#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

pdftex="${UMBER_REF_PDFTEX:-}"
if [[ -z "$pdftex" ]]; then
  pdftex="$(command -v pdftex || true)"
fi
if [[ -z "$pdftex" || ! -x "$pdftex" ]]; then
  cat >&2 <<'EOF'
setup-conformance-tests: could not locate pdftex.

Install a TeX distribution such as TeX Live or MacTeX, or set
UMBER_REF_PDFTEX=/absolute/path/to/pdftex.
EOF
  exit 2
fi

# Story and Gentle use refexec's generic reference-engine selector; pin it to
# the same pdfTeX executable used by the specialized TRIP regeneration path.
export UMBER_REF_TEX="$pdftex"
export UMBER_REF_PDFTEX="$pdftex"

scripts/parity.sh fetch
scripts/fetch-hyphen-corpus.sh
if [[ ! -f third_party/hyphen/hyphen.tex ]]; then
  printf '%s\n' \
    'setup-conformance-tests: hyphen.tex is unavailable; ensure kpsewhich can locate it' >&2
  exit 2
fi
scripts/fetch-font-corpus.sh
scripts/trip.sh fetch

for case in story gentle trip etrip; do
  scripts/regen-fixtures.sh --case "e2e/${case}"
done

printf '%s\n' 'End-to-end conformance tests are set up.' >&2
