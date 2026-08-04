#!/usr/bin/env bash
set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Explicit gate for host-side regeneration, profiling, and triage tools that are
# intentionally absent from the routine native correctness build.
#
# This check stays out of `scripts/check.sh` and the routine `cargo test` suite
# because it needs ripgrep, the pinned oracle builds, and three dependency trees
# the workspace lockfile does not cover. What changed in umber2-johp.213 is that
# it now says what it ran: every step is named, and a step whose tool is absent
# reports BLOCKED instead of aborting at the first missing prerequisite.
#
# Naming steps on the command line runs exactly those, with byte-identical
# commands, and is reported as a partial run.

# shellcheck source=scripts/optional-check-runner.sh
source "$repo_root/scripts/optional-check-runner.sh"

OPTIONAL_CHECK_ARGS="$*" optional_check_begin check-tools.sh \
  arxiv-corpus arxiv-census oracle-regeneration \
  parity-harness corpus-sync fixturegen texlive-wasm-publish \
  clippy-reference-tools clippy-profiling-runner clippy-dvi-tools

optional_check_step_requiring "python3 tar gzip" arxiv-corpus \
  scripts/test-arxiv-corpus.sh
optional_check_step_requiring "python3 tar gzip" arxiv-census \
  scripts/test-stepwise-arxiv-census.sh
optional_check_step oracle-regeneration scripts/test-oracle-regeneration.sh

# `profile-analyzer` and `refexec` are tested by the routine `cargo test` suite
# with everything else, so re-running them here would only thrash the shared
# target directory with a narrower feature resolution. `parity-harness` stays
# because `reference-tools` is a resolution no other gate builds.
optional_check_step parity-harness \
  cargo test -q -p parity-harness --tests --features reference-tools

# The `[workspace] exclude` directories: each is its own workspace with its own
# lockfile, so `--workspace` cannot reach them and the routine suite
# requires them to name a gate that does. This is that gate; the 23 tests here
# ran nowhere at all before umber2-johp.211.
check_corpus_sync() {
  cargo test -q --tests --manifest-path tools/corpus-sync/Cargo.toml &&
    python3 scripts/test-provision.py
}
optional_check_step corpus-sync check_corpus_sync
optional_check_step fixturegen \
  cargo test -q --tests --manifest-path tools/fixturegen/Cargo.toml
optional_check_step texlive-wasm-publish \
  cargo test -q --tests --manifest-path tools/texlive-wasm-publish/Cargo.toml

# The opt-in feature resolutions `scripts/check-lint-passes.py` records as
# covered here rather than by the routine clippy gate.
tools_clippy() {
  CARGO_TARGET_DIR="${TOOLS_TARGET_DIR:-target/tools}" cargo clippy -q "$@"
}
optional_check_step clippy-reference-tools tools_clippy \
  -p profile-analyzer -p refexec -p parity-harness \
  --all-targets --features parity-harness/reference-tools -- -D warnings
optional_check_step clippy-profiling-runner tools_clippy \
  -p umber --bin gentle-profile \
  --features profiling-runner,profiling -- -D warnings
optional_check_step clippy-dvi-tools tools_clippy \
  -p tex-out --bin texout-dvitype --features dvi-tools -- -D warnings

optional_check_finish
