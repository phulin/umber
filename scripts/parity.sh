#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

target_dir="${CARGO_TARGET_DIR:-target}"
if [[ "$target_dir" != /* ]]; then
  target_dir="${repo_root}/${target_dir}"
fi
corpus_sync_bin="${target_dir}/debug/corpus-sync"
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-1783604160}"
export FORCE_SOURCE_DATE="${FORCE_SOURCE_DATE:-1}"

usage() {
  cat <<'EOF'
usage:
  scripts/parity.sh [fetch] [--offline]
  scripts/parity.sh e2e [--offline] [--doc NAME] [--keep-triage]
  scripts/parity.sh self-test

Fetches and verifies the pinned external TeX corpus declared in
tests/corpus-manifest.txt. The e2e mode then runs reference TeX and Umber on
each manifest entry and writes mismatch bundles under target/parity-triage/.
The script pins SOURCE_DATE_EPOCH and FORCE_SOURCE_DATE by default so
date-sensitive documents have stable reference output; set them explicitly to
override the defaults.
EOF
}

mode="fetch"
offline=0
doc_filter=""

if [[ "$#" -gt 0 ]]; then
  case "$1" in
    fetch|e2e|self-test)
      mode="$1"
      shift
      ;;
  esac
fi

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --offline)
      offline=1
      shift
      ;;
    --doc)
      if [[ "$#" -lt 2 ]]; then
        printf '%s\n' 'parity.sh: missing value after --doc' >&2
        exit 2
      fi
      doc_filter="$2"
      shift 2
      ;;
    --keep-triage)
      # Compatibility flag: individually selected Cargo tests preserve triage
      # for the other conformance cases automatically.
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      printf 'parity.sh: unknown option: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ "$mode" == "self-test" ]]; then
  cargo test -p parity-harness self_test_bundle_pinpoints_page_and_opcode
  exit 0
fi

printf '%s\n' 'Building corpus-sync' >&2
cargo build --manifest-path tools/corpus-sync/Cargo.toml --target-dir "$target_dir"

printf '%s\n' 'Fetching/verifying external corpus' >&2
if [[ "$offline" -eq 1 ]]; then
  "$corpus_sync_bin" --offline
else
  "$corpus_sync_bin"
fi

printf '%s\n' 'External corpus acquisition complete.' >&2

if [[ "$mode" == "e2e" ]]; then
  test_names=()
  if [[ -z "$doc_filter" ]]; then
    test_names=(e2e_conformance_story e2e_conformance_gentle)
  else
    case "$doc_filter" in
      story.tex) test_names=(e2e_conformance_story) ;;
      gentle.tex) test_names=(e2e_conformance_gentle) ;;
      *)
        printf 'parity.sh: no Cargo conformance test for %s\n' "$doc_filter" >&2
        exit 2
        ;;
    esac
  fi
  for test_name in "${test_names[@]}"; do
    cargo test -p umber --test it "$test_name" -- --ignored --nocapture
  done
fi
