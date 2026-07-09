#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Local fast gate: consume committed fixtures and keep live reference TeX work
# in scripts/parity.sh or scripts/regen-fixtures.sh.
export UMBER_LIVE_REF=0
export UPDATE_FIXTURES=0

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --release -- -D warnings
cargo test -p tex-state --features testing replay
cargo test -p tex-state --features shadow --tests
cargo test --workspace --tests
