#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"
if [[ "$target_dir" != /* ]]; then
  target_dir="$repo_root/$target_dir"
fi
iterations="${GENTLE_PROFILE_ITERATIONS:-50}"
warmups="${GENTLE_PROFILE_WARMUPS:-1}"
output="${GENTLE_PROFILE_OUTPUT:-$target_dir/profiles/gentle.json.gz}"

if ! command -v samply >/dev/null 2>&1; then
  printf '%s\n' 'profile-gentle: samply is required; install it with cargo install samply' >&2
  exit 1
fi

mkdir -p "$(dirname "$output")"
printf '%s\n' 'Building instrumented Gentle runner with full debug information' >&2
cargo build --profile profiling -p umber --bin gentle-profile --features profiling-stats

runner="$target_dir/profiling/gentle-profile"
printf 'Recording %s measured Gentle runs to %s\n' "$iterations" "$output" >&2
samply record \
  --save-only \
  --main-thread-only \
  --reuse-threads \
  --unstable-presymbolicate \
  --profile-name 'Umber Gentle in-process' \
  --output "$output" \
  "$runner" \
  --repo-root "$repo_root" \
  --iterations "$iterations" \
  --warmups "$warmups" \
  "$@"

printf 'Gentle profile written to %s\n' "$output"
