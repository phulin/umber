#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

scripts/profile-pdftex-arxiv.sh check-entrypoint

# Explicit gate for host-side regeneration, profiling, and triage tools that
# are intentionally absent from the routine native correctness build.

scripts/profile-pdftex-arxiv.sh check-sample
scripts/profile-pdftex-arxiv.sh check-entrypoint
scripts/test-arxiv-corpus.sh
scripts/test-stepwise-arxiv-census.sh
scripts/test-oracle-regeneration.sh

# `profile-analyzer` and `refexec` are tested by `scripts/run-native-tests.py`
# with everything else, so re-running them here would only thrash the shared
# target directory with a narrower feature resolution. `parity-harness` stays
# because `reference-tools` is a resolution no other gate builds.
cargo test -q -p parity-harness --tests --features reference-tools

# The `[workspace] exclude` directories: each is its own workspace with its own
# lockfile, so `--workspace` cannot reach them and `scripts/run-native-tests.py`
# requires them to name a gate that does. This is that gate; the 23 tests here
# ran nowhere at all before umber2-johp.211.
cargo test -q --tests --manifest-path tools/corpus-sync/Cargo.toml
cargo test -q --tests --manifest-path tools/fixturegen/Cargo.toml
cargo test -q --tests --manifest-path tools/texlive-wasm-publish/Cargo.toml

CARGO_TARGET_DIR="${TOOLS_TARGET_DIR:-target/tools}" \
  cargo clippy -q -p profile-analyzer -p refexec -p parity-harness \
    --all-targets --features parity-harness/reference-tools -- -D warnings
CARGO_TARGET_DIR="${TOOLS_TARGET_DIR:-target/tools}" \
  cargo clippy -q -p umber --bin gentle-profile \
    --features profiling-runner,profiling-stats -- -D warnings
CARGO_TARGET_DIR="${TOOLS_TARGET_DIR:-target/tools}" \
  cargo clippy -q -p tex-out --bin texout-dvitype --features dvi-tools -- -D warnings
