#!/usr/bin/env bash
set -euo pipefail

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p tex-state --features testing --test replay
cargo test -p tex-state --features shadow --tests
cargo test --workspace --tests
