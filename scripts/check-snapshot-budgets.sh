#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$repo_root/benchmarks/tex-state/Cargo.toml"
target_dir="${CARGO_TARGET_DIR:-$repo_root/benchmarks/tex-state/target}"

cargo run --release --manifest-path "$manifest" --target-dir "$target_dir" \
  --bin snapshot_gate -- --enforce "$@"
