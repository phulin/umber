#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
selector="$repo_root/scripts/profile-pdftex-arxiv.sh"
candidates=${1:?usage: $0 CANDIDATES_TSV DESTINATION OUTPUT_TSV LOCK_TSV [EXCLUSIONS_TSV]}
destination=${2:?usage: $0 CANDIDATES_TSV DESTINATION OUTPUT_TSV LOCK_TSV [EXCLUSIONS_TSV]}
output=${3:?usage: $0 CANDIDATES_TSV DESTINATION OUTPUT_TSV LOCK_TSV [EXCLUSIONS_TSV]}
lock=${4:?usage: $0 CANDIDATES_TSV DESTINATION OUTPUT_TSV LOCK_TSV [EXCLUSIONS_TSV]}
exclusions=${5:-}
sample_size=${ARXIV_RECENT_SAMPLE_SIZE:-100}
candidate_limit=${ARXIV_RECENT_CANDIDATE_LIMIT:-150}
jobs=${ARXIV_RECENT_JOBS:-4}

[[ $sample_size =~ ^[1-9][0-9]*$ ]] || { echo "invalid sample size" >&2; exit 2; }
[[ $candidate_limit =~ ^[1-9][0-9]*$ ]] || { echo "invalid candidate limit" >&2; exit 2; }
[[ $jobs =~ ^[1-9][0-9]*$ ]] || { echo "invalid job count" >&2; exit 2; }
[[ -f $candidates ]] || { echo "candidate TSV not found: $candidates" >&2; exit 2; }
[[ $(sed -n '1p' "$candidates") == $'id\tcategories\tfirst_submitted\tshuffle_sha256' ]] || {
  echo "invalid candidate TSV header" >&2
  exit 2
}
if [[ -n $exclusions ]]; then
  [[ -f $exclusions ]] || { echo "exclusions TSV not found: $exclusions" >&2; exit 2; }
  [[ $(sed -n '1p' "$exclusions") == $'id\treason' ]] || {
    echo "invalid exclusions TSV header" >&2
    exit 2
  }
fi

mkdir -p "$destination/archives" "$destination/sources" "$destination/audit"

download() {
  local identifier=$1 archive="$destination/archives/${1//\//_}.src"
  [[ -f $archive ]] && return
  curl -L --fail --show-error --silent --retry 4 --retry-all-errors \
    -o "$archive.part" "https://export.arxiv.org/e-print/$identifier"
  mv "$archive.part" "$archive"
}

is_excluded() {
  [[ -n $exclusions ]] && awk -F '\t' -v id="$1" \
    'NR > 1 && $1 == id { found = 1 } END { exit !found }' "$exclusions"
}

export destination
seen=0
while IFS=$'\t' read -r identifier _; do
  [[ $identifier == id ]] && continue
  (( ++seen > candidate_limit )) && break
  is_excluded "$identifier" && continue
  while (( $(jobs -pr | wc -l) >= jobs )); do
    sleep 0.1
  done
  key=${identifier//\//_}
  {
    if ! download "$identifier"; then
      rm -f -- "$destination/archives/$key.src.part"
      printf '%s\tdownload failed\n' "$identifier" \
        >"$destination/audit/$key.tsv"
    fi
  } &
done <"$candidates"
wait

temporary_output="$output.part"
temporary_lock="$lock.part"
printf 'id\tcategories\n' >"$temporary_output"
printf 'id\tsource_sha256\tsource_bytes\tfirst_submitted\tshuffle_sha256\tentrypoint\n' \
  >"$temporary_lock"
accepted=0
seen=0
while IFS=$'\t' read -r identifier categories submitted digest; do
  [[ $identifier == id ]] && continue
  (( ++seen > candidate_limit )) && break
  is_excluded "$identifier" && continue
  key=${identifier//\//_}
  archive="$destination/archives/$key.src"
  source_directory="$destination/sources/$key"
  [[ -f $archive ]] || continue
  rm -rf -- "$source_directory"
  mkdir -p "$source_directory"
  if tar tzf "$archive" >/dev/null 2>&1; then
    tar xzf "$archive" -C "$source_directory"
  elif gzip -t "$archive" >/dev/null 2>&1; then
    gzip -dc "$archive" >"$source_directory/main.tex"
  else
    printf '%s\t%s\t%s\tnon-source payload\n' \
      "$identifier" "$submitted" "$digest" >"$destination/audit/$key.tsv"
    continue
  fi
  entrypoint="$($selector select-entrypoint "$source_directory" || true)"
  if [[ -z $entrypoint ]]; then
    printf '%s\t%s\t%s\tno live documentclass\n' \
      "$identifier" "$submitted" "$digest" >"$destination/audit/$key.tsv"
    continue
  fi
  printf '%s\t%s\n' "$identifier" "$categories" >>"$temporary_output"
  source_sha256="$(shasum -a 256 "$archive" | awk '{print $1}')"
  source_bytes="$(wc -c <"$archive" | tr -d ' ')"
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$identifier" "$source_sha256" "$source_bytes" "$submitted" "$digest" \
    "${entrypoint#"$source_directory/"}" >>"$temporary_lock"
  printf '%s\t%s\t%s\t%s\n' \
    "$identifier" "$submitted" "$digest" "${entrypoint#"$source_directory/"}" \
    >"$destination/audit/$key.tsv"
  (( ++accepted ))
  (( accepted == sample_size )) && break
done <"$candidates"

if (( accepted != sample_size )); then
  echo "accepted $accepted of $sample_size requested documents" >&2
  exit 1
fi
mv "$temporary_output" "$output"
mv "$temporary_lock" "$lock"
echo "materialized $accepted documents from $seen randomly ordered candidates"
