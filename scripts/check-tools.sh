#!/usr/bin/env bash
set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Explicit gate for host-side regeneration, profiling, and triage tools that are
# intentionally absent from the routine native correctness build.
#
# This tier stays out of `scripts/check.sh` and `scripts/run-native-tests.py`
# because it needs ripgrep, the pinned oracle builds, and three dependency trees
# the workspace lockfile does not cover. What changed in umber2-johp.213 is that
# it now says what it ran: every step is named, a step whose tool is absent
# reports BLOCKED instead of aborting the whole run at the first missing
# prerequisite, and the outcome is stamped where the routine gates read it.
#
# Naming steps on the command line runs exactly those, with byte-identical
# commands, and is recorded as the partial run it is.

# shellcheck source=scripts/tier-runner.sh
source "$repo_root/scripts/tier-runner.sh"

TIER_ARGS="$*" tier_begin check-tools.sh \
  arxiv-sample arxiv-entrypoint arxiv-corpus arxiv-census oracle-regeneration \
  parity-harness corpus-sync fixturegen texlive-wasm-publish \
  clippy-reference-tools clippy-profiling-runner clippy-dvi-tools

tier_step_requiring awk arxiv-sample \
  scripts/profile-pdftex-arxiv.sh check-sample
tier_step_requiring "rg mktemp" arxiv-entrypoint \
  scripts/profile-pdftex-arxiv.sh check-entrypoint
tier_step_requiring "python3 tar gzip" arxiv-corpus \
  scripts/test-arxiv-corpus.sh
tier_step_requiring "python3 tar gzip" arxiv-census \
  scripts/test-stepwise-arxiv-census.sh
tier_step oracle-regeneration scripts/test-oracle-regeneration.sh

# `profile-analyzer` and `refexec` are tested by `scripts/run-native-tests.py`
# with everything else, so re-running them here would only thrash the shared
# target directory with a narrower feature resolution. `parity-harness` stays
# because `reference-tools` is a resolution no other gate builds.
tier_step parity-harness \
  cargo test -q -p parity-harness --tests --features reference-tools

# The `[workspace] exclude` directories: each is its own workspace with its own
# lockfile, so `--workspace` cannot reach them and `scripts/run-native-tests.py`
# requires them to name a gate that does. This is that gate; the 23 tests here
# ran nowhere at all before umber2-johp.211.
tier_step corpus-sync \
  cargo test -q --tests --manifest-path tools/corpus-sync/Cargo.toml
tier_step fixturegen \
  cargo test -q --tests --manifest-path tools/fixturegen/Cargo.toml
tier_step texlive-wasm-publish \
  cargo test -q --tests --manifest-path tools/texlive-wasm-publish/Cargo.toml

# The opt-in feature resolutions `scripts/check-lint-passes.py` records as
# covered here rather than by the routine clippy gate.
tools_clippy() {
  CARGO_TARGET_DIR="${TOOLS_TARGET_DIR:-target/tools}" cargo clippy -q "$@"
}
tier_step clippy-reference-tools tools_clippy \
  -p profile-analyzer -p refexec -p parity-harness \
  --all-targets --features parity-harness/reference-tools -- -D warnings
tier_step clippy-profiling-runner tools_clippy \
  -p umber --bin gentle-profile \
  --features profiling-runner,profiling-stats -- -D warnings
tier_step clippy-dvi-tools tools_clippy \
  -p tex-out --bin texout-dvitype --features dvi-tools -- -D warnings

tier_finish
