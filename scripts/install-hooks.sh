#!/usr/bin/env bash
set -euo pipefail

git config core.hooksPath scripts/hooks

# Say which hooks this actually installed. `git config` succeeding proves only
# that a path was written; it is not evidence that anything now runs, and this
# repository has spent umber2-johp.121, .168, .201, .210, .211, and .213 on the
# difference between those two statements.
printf 'install-hooks: core.hooksPath = %s\n' "$(git config core.hooksPath)"
printf 'install-hooks: installed: %s\n' \
  "$(cd "$(git rev-parse --show-toplevel)/scripts/hooks" && echo *)"
