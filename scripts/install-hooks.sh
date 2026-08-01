#!/usr/bin/env bash
set -euo pipefail

git config core.hooksPath scripts/hooks

printf 'install-hooks: core.hooksPath = %s\n' "$(git config core.hooksPath)"
printf 'install-hooks: installed: %s\n' \
  "$(cd "$(git rev-parse --show-toplevel)/scripts/hooks" && echo *)"
