#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

target_dir="${CARGO_TARGET_DIR:-target}"
if [[ "$target_dir" != /* ]]; then
  target_dir="${repo_root}/${target_dir}"
fi
corpus_sync_bin="${target_dir}/debug/corpus-sync"

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

printf '%s\n' 'Building corpus-sync' >&2
cargo build --manifest-path tools/corpus-sync/Cargo.toml --target-dir "$target_dir"

printf '%s\n' 'Fetching/verifying external corpus' >&2
"$corpus_sync_bin"
printf '%s\n' 'External corpus acquisition complete.' >&2

scripts/fetch-conformance-inputs.sh

for case in story gentle trip etrip; do
  scripts/regen-fixtures.sh --case "e2e/${case}"
done

printf '%s\n' 'End-to-end conformance tests are set up for offline use.' >&2
