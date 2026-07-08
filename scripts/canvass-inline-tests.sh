#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if ! command -v rg >/dev/null 2>&1; then
  printf 'error: rg is required\n' >&2
  exit 1
fi

rust_sources="$(mktemp)"
trap 'rm -f "$rust_sources"' EXIT
find crates \
  -path '*/src/*' \
  -name '*.rs' \
  ! -name 'tests.rs' \
  ! -path '*/src/tests/*' \
  ! -path '*/src/*/tests/*' \
  -print0 >"$rust_sources"

inline_pattern='^[[:space:]]*mod[[:space:]]+tests[[:space:]]*\{'
external_pattern='^[[:space:]]*mod[[:space:]]+tests[[:space:]]*;'

source_count="$(tr -cd '\0' <"$rust_sources" | wc -c | tr -d ' ')"
inline_matches="$(xargs -0 rg -n "$inline_pattern" <"$rust_sources" || true)"
external_matches="$(xargs -0 rg -n "$external_pattern" <"$rust_sources" || true)"

inline_count="$(
  if [[ -n "$inline_matches" ]]; then
    printf '%s\n' "$inline_matches" | wc -l | tr -d ' '
  else
    printf '0'
  fi
)"
inline_file_count="$(
  if [[ -n "$inline_matches" ]]; then
    printf '%s\n' "$inline_matches" | cut -d: -f1 | sort -u | wc -l | tr -d ' '
  else
    printf '0'
  fi
)"
external_count="$(
  if [[ -n "$external_matches" ]]; then
    printf '%s\n' "$external_matches" | wc -l | tr -d ' '
  else
    printf '0'
  fi
)"
external_file_count="$(
  if [[ -n "$external_matches" ]]; then
    printf '%s\n' "$external_matches" | cut -d: -f1 | sort -u | wc -l | tr -d ' '
  else
    printf '0'
  fi
)"

printf 'Production Rust source files scanned: %s\n' "$source_count"
printf 'Inline test module declarations: %s\n' "$inline_count"
printf 'Files with inline test modules: %s\n' "$inline_file_count"
printf 'Separate test module declarations: %s\n' "$external_count"
printf 'Files with separate test modules: %s\n' "$external_file_count"

if [[ -n "$inline_matches" ]]; then
  printf '\nInline test modules:\n'
  printf '%s\n' "$inline_matches"
fi

if [[ -n "$external_matches" ]]; then
  printf '\nSeparate test module declarations:\n'
  printf '%s\n' "$external_matches"
fi
