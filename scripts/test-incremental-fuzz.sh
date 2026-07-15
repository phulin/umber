#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "$repo_root"
cargo test --tests -p tex-incr \
  tests::thousand_edit_scripted_fuzz_matches_cold_every_revision -- \
  --ignored --exact
