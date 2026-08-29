#!/usr/bin/env bash
set -uo pipefail

if [[ $# -ne 9 ]]; then
  printf '%s\n' \
    'usage: run-pinned-pdflatex-50m-authority-row.sh BINARY SOURCE_ROOT INPUT FORMAT DISTRIBUTION DISTRIBUTION_AHASH64 PREFETCH_KEYS OUTPUT SOURCE_DATE_EPOCH' >&2
  exit 2
fi

binary=$1
source_root=$2
input=$3
format=$4
distribution=$5
distribution_ahash64=$6
prefetch_keys=$7
output=$8
source_date_epoch=$9

expansion_fuel_cap=50000000
execution_steps_cap=100000000

for file in "$binary" "$source_root/$input" "$format" "$distribution/manifest-v8.json" "$prefetch_keys"; do
  if [[ ! -f $file ]]; then
    printf 'authority input is not a regular file: %s\n' "$file" >&2
    exit 2
  fi
done
if [[ ! $distribution_ahash64 =~ ^[0-9a-f]{16}$ ]]; then
  printf '%s\n' 'DISTRIBUTION_AHASH64 must be exactly 16 lowercase hexadecimal digits' >&2
  exit 2
fi
if [[ ! $source_date_epoch =~ ^[0-9]+$ ]]; then
  printf '%s\n' 'SOURCE_DATE_EPOCH must be an unsigned integer' >&2
  exit 2
fi

absolute_file() {
  local directory
  directory=$(CDPATH= cd -- "$(dirname -- "$1")" && pwd -P)
  printf '%s/%s\n' "$directory" "$(basename -- "$1")"
}

binary=$(absolute_file "$binary")
source_root=$(CDPATH= cd -- "$source_root" && pwd -P)
format=$(absolute_file "$format")
distribution=$(CDPATH= cd -- "$distribution" && pwd -P)
prefetch_keys=$(absolute_file "$prefetch_keys")

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

mkdir -p "$output"
output=$(CDPATH= cd -- "$output" && pwd -P)
prefetch=()
prefetch_count=0
while IFS= read -r key; do
  if [[ -z $key ]]; then
    printf '%s\n' 'PREFETCH_KEYS must not contain empty lines' >&2
    exit 2
  fi
  prefetch+=(--prefetch-input "$key")
  prefetch_count=$((prefetch_count + 1))
done < "$prefetch_keys"

receipt_tmp="$output/authority.receipt.tmp"
{
  printf '%s\n' 'schema=1'
  printf '%s\n' 'authority=umber-pdflatex-50m'
  printf 'expansion_fuel_cap=%s\n' "$expansion_fuel_cap"
  printf '%s\n' 'expansion_fuel_unit=canonical-command-fuel'
  printf 'execution_steps_cap=%s\n' "$execution_steps_cap"
  printf '%s\n' 'execution_steps_unit=committed-executor-steps'
  printf '%s\n' 'offline=1'
  printf 'source_date_epoch=%s\n' "$source_date_epoch"
  printf 'distribution_ahash64=%s\n' "$distribution_ahash64"
  printf 'prefetch_keys=%s\n' "$prefetch_count"
  printf 'binary_sha256=%s\n' "$(sha256_file "$binary")"
  printf 'input_sha256=%s\n' "$(sha256_file "$source_root/$input")"
  printf 'format_sha256=%s\n' "$(sha256_file "$format")"
  printf 'distribution_root_sha256=%s\n' "$(sha256_file "$distribution/manifest-v8.json")"
  printf 'prefetch_keys_sha256=%s\n' "$(sha256_file "$prefetch_keys")"
} > "$receipt_tmp"
mv "$receipt_tmp" "$output/authority.receipt"

cache="$output/cache"
mkdir -p "$cache"
cd "$source_root"
LC_ALL=C.UTF-8 \
SOURCE_DATE_EPOCH="$source_date_epoch" \
FORCE_SOURCE_DATE=1 \
XDG_CACHE_HOME="$cache" \
  "$binary" run --pdflatex \
  --format "$format" \
  --distribution "$distribution" \
  --distribution-ahash64 "$distribution_ahash64" \
  --offline \
  "${prefetch[@]}" \
  --expansion-fuel "$expansion_fuel_cap" \
  --execution-steps "$execution_steps_cap" \
  --pdf "$output/output.pdf" \
  --input-records-out "$output/inputs.receipt" \
  "$input" > "$output/stdout" 2> "$output/stderr"
status=$?
printf '%s\n' "$status" > "$output/status"
exit "$status"
