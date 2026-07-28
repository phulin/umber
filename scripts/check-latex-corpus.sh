#!/usr/bin/env bash
set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# The pinned native LaTeX corpus deliberately remains outside the routine
# suite: it builds a live distribution format and needs a pinned reference
# LaTeX. This entry point supplies deferred-tier accounting and a stamp.
source "$repo_root/scripts/tier-runner.sh"

TIER_ARGS="$*" tier_begin check-latex-corpus.sh latex-corpus
tier_step_requiring "awk cargo mktemp" latex-corpus \
  "$repo_root/scripts/run-latex-corpus.sh"
tier_finish
