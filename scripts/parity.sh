#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

target_dir="${CARGO_TARGET_DIR:-target}"
if [[ "$target_dir" != /* ]]; then
  target_dir="${repo_root}/${target_dir}"
fi
corpus_sync_bin="${target_dir}/debug/corpus-sync"
parity_harness_bin="${target_dir}/debug/parity-harness"
umber_bin="${target_dir}/debug/umber"

usage() {
  cat <<'EOF'
usage:
  scripts/parity.sh [fetch] [--offline]
  scripts/parity.sh e2e [--offline] [--doc NAME] [--keep-triage]
  scripts/parity.sh self-test

Fetches and verifies the pinned external TeX corpus declared in
tests/corpus-manifest.toml. The e2e mode then runs reference TeX and Umber on
each manifest entry and writes mismatch bundles under target/parity-triage/.
EOF
}

mode="fetch"
offline=0
doc_args=()
keep_triage=0

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
      doc_args+=(--doc "$2")
      shift 2
      ;;
    --keep-triage)
      keep_triage=1
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
  printf '%s\n' 'Building parity-harness' >&2
  cargo build -p parity-harness
  "$parity_harness_bin" --self-test
  exit 0
fi

printf '%s\n' 'Building corpus-sync' >&2
cargo build -p corpus-sync

printf '%s\n' 'Fetching/verifying external corpus' >&2
if [[ "$offline" -eq 1 ]]; then
  "$corpus_sync_bin" --offline
else
  "$corpus_sync_bin"
fi

printf '%s\n' 'External corpus acquisition complete.' >&2

if [[ "$mode" == "e2e" ]]; then
  printf '%s\n' 'Building e2e parity harness and umber' >&2
  cargo build -p parity-harness -p umber
  harness_args=(--umber-bin "$umber_bin")
  if [[ "$keep_triage" -eq 1 ]]; then
    harness_args+=(--keep-triage)
  fi
  if [[ "${#doc_args[@]}" -gt 0 ]]; then
    harness_args+=("${doc_args[@]}")
  fi
  "$parity_harness_bin" "${harness_args[@]}"
fi
