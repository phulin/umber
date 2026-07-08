#!/usr/bin/env bash
set -euo pipefail

dest_dir="third_party/hyphen"
dest_path="${dest_dir}/hyphen.tex"

if ! command -v kpsewhich >/dev/null 2>&1; then
  cat >&2 <<'EOF'
Could not locate kpsewhich on PATH; skipping hyphen.tex fetch.

Install a TeX distribution such as TeX Live or MacTeX, or run this script from
an environment where kpsewhich can locate hyphen.tex.
EOF
  exit 0
fi

source_path="$(kpsewhich hyphen.tex || true)"
if [[ -z "$source_path" ]]; then
  printf 'kpsewhich could not locate hyphen.tex; skipping\n' >&2
  exit 0
fi

mkdir -p "$dest_dir"
if [[ -f "$dest_path" ]] && cmp -s "$source_path" "$dest_path"; then
  printf '%s already up to date\n' "$dest_path"
else
  cp "$source_path" "$dest_path"
  printf 'fetched %s from %s\n' "$dest_path" "$source_path"
fi
