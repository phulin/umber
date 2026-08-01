#!/usr/bin/env bash
set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Preserve established parity options. Options are passed to the implementation
# rather than interpreted as step selections.
source "$repo_root/scripts/optional-check-runner.sh"

OPTIONAL_CHECK_ARGS="" optional_check_begin check-latex-parity.sh latex2e-dvi-parity
optional_check_step_requiring "awk cargo mktemp perl" latex2e-dvi-parity \
  "$repo_root/scripts/run-latex-parity.sh" "$@"
optional_check_finish
