#!/usr/bin/env bash
set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Preserve established parity options while making a normal full run a
# registered tier. Options are implementation options, not step selections.
source "$repo_root/scripts/tier-runner.sh"

TIER_ARGS="" tier_begin check-latex-parity.sh latex2e-dvi-parity
tier_step_requiring "awk cargo mktemp perl" latex2e-dvi-parity \
  "$repo_root/scripts/run-latex-parity.sh" "$@"
tier_finish
