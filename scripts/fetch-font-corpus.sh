#!/usr/bin/env bash
set -euo pipefail

fonts=(
  cmbsy10 cmbx10 cmbx5 cmbx6 cmbx7 cmbx8 cmbx9 cmcsc10 cmdunh10
  cmex10 cmmi10 cmmi5 cmmi6 cmmi7 cmmi8 cmmi9 cmmib10
  cmr10 cmr5 cmr6 cmr7 cmr8 cmr9
  cmsl10 cmsl8 cmsl9 cmsltt10 cmss10 cmssbx10 cmssi10 cmssq8 cmssqi8
  cmsy10 cmsy5 cmsy6 cmsy7 cmsy8 cmsy9
  cmti10 cmti7 cmti8 cmti9 cmtt10 cmtt8 cmtt9 cmu10 manfnt
)
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
