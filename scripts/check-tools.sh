#!/usr/bin/env bash
set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Explicit gate for host-side regeneration, profiling, and triage tools that are
# intentionally absent from the routine native correctness build.
#
# This check stays out of `scripts/check.sh` and the routine `cargo test` suite
# because it needs toolchain utilities, pinned oracle contracts, and two
# dependency trees the workspace lockfile does not cover. Every step is named,
# and a step whose tool is absent reports BLOCKED.
#
# Naming steps on the command line runs exactly those, with byte-identical
# commands, and is reported as a partial run.

# shellcheck source=scripts/optional-check-runner.sh
source "$repo_root/scripts/optional-check-runner.sh"

# Preserve the old selector as an alias; this step validates the regeneration
# contract and never constructs a live oracle.
selected_args=()
for requested in "$@"; do
  if [[ "$requested" == oracle-regeneration ]]; then
    requested=oracle-contract
  fi
  selected_args+=("$requested")
done
OPTIONAL_CHECK_ARGS="${selected_args[*]}" optional_check_begin check-tools.sh \
  arxiv-corpus arxiv-census texlive-parity-tools oracle-contract \
  parity-harness fixturegen texlive-wasm-publish \
  profiling-command-tests profiling-cli copy-attribution clippy-reference-tools \
  clippy-profiling-runner clippy-dvi-tools

optional_check_step_requiring "python3 tar gzip" arxiv-corpus \
  scripts/test-arxiv-corpus.sh
optional_check_step_requiring "python3 tar gzip" arxiv-census \
  scripts/test-stepwise-arxiv-census.sh
# Synthetic archives and mocked engines exercise year selection, acquisition,
# format provenance, and corpus resume without downloads or live TeX binaries.
check_texlive_parity_tools() {
  local failed=0
  python3 scripts/test-texlive-snapshot.py || failed=1
  python3 scripts/test-texlive-formats.py || failed=1
  python3 scripts/test-run-arxiv-texlive.py || failed=1
  return "$failed"
}
optional_check_step_requiring "python3" texlive-parity-tools check_texlive_parity_tools

optional_check_step_requiring "python3 openssl" oracle-contract \
  scripts/test-oracle-regeneration.sh

# `profile-analyzer` is tested by the routine `cargo test` suite
# with everything else, so re-running them here would only thrash the shared
# target directory with a narrower feature resolution. `parity-harness` stays
# because `reference-tools` is a resolution no other gate builds.
optional_check_step_requiring "cargo" parity-harness \
  cargo test -q -p parity-harness --tests --features reference-tools

# The `[workspace] exclude` directories: each is its own workspace with its own
# lockfile, so `--workspace` cannot reach them and the routine suite
# requires them to name a gate that does. This is that gate.
check_fixturegen() {
  cargo test -q --tests --manifest-path tools/fixturegen/Cargo.toml &&
    python3 scripts/test-provision.py
}
optional_check_step_requiring "cargo python3" fixturegen check_fixturegen
optional_check_step_requiring "cargo" texlive-wasm-publish \
  cargo test -q --tests --manifest-path tools/texlive-wasm-publish/Cargo.toml

# A dependent Umber target enables tex-command's profiling library resolution,
# but Cargo does not compile that dependency's feature-only unit-test bodies.
# Keep the command core's profiling test target current with its production
# input owners before testing the user-facing binary. Selecting the focused
# resident fixture still compiles the complete unit-test target without running
# unrelated allocation-budget tests.
profiling_command_tests() {
  python3 scripts/check-selected-rust-suites.py --verify-feature tex-command &&
    cargo test -q --tests -p tex-command --features profiling \
      one_and_4096_deliveries_derive_ordinary_freshness_without_a_coordinate_mirror
}

optional_check_step_requiring "cargo python3" profiling-command-tests \
  profiling_command_tests

# Build the user-facing binary in its real profiling profile and prove that
# the feature-only flag publishes a non-empty command census.
profiling_cli() {
  python3 scripts/check-selected-rust-suites.py --verify-feature umber &&
    cargo test -q --profile profiling -p umber --test it --features profiling \
      profiling_stats_flag_reports_feature_only_census
}
optional_check_step_requiring "cargo python3" profiling-cli profiling_cli
optional_check_step_requiring "cc rustc addr2line python3 cargo" copy-attribution \
  scripts/test-copy-attribution.sh

# The opt-in feature resolutions `scripts/check-lint-passes.py` records as
# covered here rather than by the routine clippy gate.
tools_clippy() {
  CARGO_TARGET_DIR="${TOOLS_TARGET_DIR:-target/tools}" cargo clippy -q "$@"
}
optional_check_step_requiring "cargo" clippy-reference-tools tools_clippy \
  -p profile-analyzer -p parity-harness \
  --all-targets --features parity-harness/reference-tools -- -D warnings
optional_check_step_requiring "cargo" clippy-profiling-runner tools_clippy \
  -p umber --bin gentle-profile \
  --features profiling-runner,profiling -- -D warnings
optional_check_step_requiring "cargo" clippy-dvi-tools tools_clippy \
  -p tex-out --bin texout-dvitype --features dvi-tools -- -D warnings

optional_check_finish
