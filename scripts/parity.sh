#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

target_dir="${CARGO_TARGET_DIR:-target}"
if [[ "$target_dir" != /* ]]; then
  target_dir="${repo_root}/${target_dir}"
fi
corpus_sync_bin="${target_dir}/debug/corpus-sync"

usage() {
  cat <<'EOF'
usage:
  scripts/parity.sh [--offline]

Fetches and verifies the pinned external TeX corpus declared in
tests/corpus-manifest.toml before long-running parity checks. Later parity
harness stages are tracked separately; this script currently owns the
reproducible acquisition step.
EOF
}

offline=0
while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --offline)
      offline=1
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

printf '%s\n' 'Building corpus-sync' >&2
cargo build --manifest-path tools/corpus-sync/Cargo.toml --target-dir "$target_dir"

printf '%s\n' 'Fetching/verifying external corpus' >&2
if [[ "$offline" -eq 1 ]]; then
  "$corpus_sync_bin" --offline
else
  "$corpus_sync_bin"
fi

printf '%s\n' 'External corpus acquisition complete.' >&2
