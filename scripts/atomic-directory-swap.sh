#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 3 ]]; then
  printf 'usage: %s LIVE CANDIDATE BACKUP\n' "${0##*/}" >&2
  exit 2
fi

live_dir="$1"
candidate_dir="$2"
backup_dir="$3"

[[ -d "$live_dir" ]] || {
  printf 'atomic-directory-swap: live directory is missing: %s\n' "$live_dir" >&2
  exit 2
}
[[ -d "$candidate_dir" ]] || {
  printf 'atomic-directory-swap: candidate directory is missing: %s\n' "$candidate_dir" >&2
  exit 2
}
[[ ! -e "$backup_dir" ]] || {
  printf 'atomic-directory-swap: backup path already exists: %s\n' "$backup_dir" >&2
  exit 2
}

mv "$live_dir" "$backup_dir"

install_error=""
if install_error="$(mv "$candidate_dir" "$live_dir" 2>&1)"; then
  exit 0
fi

restore_error=""
if restore_error="$(mv "$backup_dir" "$live_dir" 2>&1)"; then
  printf 'atomic-directory-swap: candidate installation failed; restored live directory: %s\n' \
    "$install_error" >&2
  exit 1
fi

printf 'atomic-directory-swap: candidate installation failed: %s\n' \
  "$install_error" >&2
printf 'atomic-directory-swap: backup restoration also failed: %s\n' \
  "$restore_error" >&2
printf 'atomic-directory-swap: authoritative backup remains recoverable at: %s\n' \
  "$backup_dir" >&2
exit 1
