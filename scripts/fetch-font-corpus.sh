#!/usr/bin/env bash
set -euo pipefail

fonts=(cmr10 cmmi10 cmsy10 cmex10 cmtt10)
dest_dir="third_party/fonts"

if ! command -v kpsewhich >/dev/null 2>&1; then
  cat >&2 <<'EOF'
Could not locate kpsewhich on PATH.

Install a TeX distribution such as TeX Live or MacTeX, or run this script from
an environment where kpsewhich can locate Computer Modern TFM files.
EOF
  exit 2
fi

mkdir -p "$dest_dir"

missing=()
for font in "${fonts[@]}"; do
  source_path="$(kpsewhich "${font}.tfm" || true)"
  if [[ -z "$source_path" ]]; then
    missing+=("${font}.tfm")
    continue
  fi

  dest_path="${dest_dir}/${font}.tfm"
  if [[ -f "$dest_path" ]] && cmp -s "$source_path" "$dest_path"; then
    printf '%s already up to date\n' "$dest_path"
  else
    cp "$source_path" "$dest_path"
    printf 'fetched %s from %s\n' "$dest_path" "$source_path"
  fi
done

if (( ${#missing[@]} > 0 )); then
  printf 'kpsewhich could not locate required TFM files: %s\n' "${missing[*]}" >&2
  exit 1
fi
