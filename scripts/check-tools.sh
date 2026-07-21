#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

scripts/profile-pdftex-arxiv.sh check-entrypoint

# Explicit gate for host-side regeneration, profiling, and triage tools that
# are intentionally absent from the routine native correctness build.

cargo test -q -p profile-analyzer --tests
cargo test -q -p refexec --tests
cargo test -q -p parity-harness --tests --features reference-tools

CARGO_TARGET_DIR="${TOOLS_TARGET_DIR:-target/tools}" \
  cargo clippy -q -p profile-analyzer -p refexec -p parity-harness \
    --all-targets --features parity-harness/reference-tools -- -D warnings
CARGO_TARGET_DIR="${TOOLS_TARGET_DIR:-target/tools}" \
  cargo clippy -q -p umber --bin gentle-profile \
    --features profiling-runner,profiling-stats -- -D warnings
CARGO_TARGET_DIR="${TOOLS_TARGET_DIR:-target/tools}" \
  cargo clippy -q -p tex-out --bin texout-dvitype --features dvi-tools -- -D warnings
