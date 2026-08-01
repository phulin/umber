#!/usr/bin/env bash
set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# The pinned native LaTeX corpus deliberately remains outside the routine
# suite: it builds a live distribution format and needs a pinned reference
# LaTeX. This entry point provides one explicit opt-in check.
source "$repo_root/scripts/optional-check-runner.sh"

OPTIONAL_CHECK_ARGS="$*" optional_check_begin check-latex-corpus.sh latex-corpus
optional_check_step_requiring "awk cargo mktemp" latex-corpus \
  "$repo_root/scripts/run-latex-corpus.sh"
optional_check_finish
