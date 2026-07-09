#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Local fast gate: consume committed fixtures; live reference TeX work belongs
# only to scripts/regen-fixtures.sh.

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p tex-state --features testing replay
cargo test -p tex-state --features shadow --tests
cargo test --workspace --tests
