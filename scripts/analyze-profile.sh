#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if [[ $# -eq 0 ]]; then
  set -- target/profiles/gentle.json.gz
fi

cargo run --quiet -p profile-analyzer -- "$@"
