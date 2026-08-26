#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_lock="${repo_root}/tests/latex-source.lock"
runtime_lock="${repo_root}/tests/latex/pdflatex-representative.lock"
guard="${repo_root}/scripts/run-umber-guarded.py"
umber_bin="${repo_root}/target/release/umber"
distribution=""
distribution_ahash64=""
format=""
receipt=""

fail() {
  printf 'check-latex-representative-resources.sh: %s\n' "$*" >&2
  exit 1
}

usage() {
  printf '%s\n' \
    'usage: scripts/check-latex-representative-resources.sh --distribution PATH --distribution-ahash64 AHASH64 --format PATH [--umber PATH] [--receipt PATH]'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --distribution | --distribution-ahash64 | --format | --umber | --receipt)
      [[ $# -ge 2 ]] || fail "missing value for $1"
      case "$1" in
        --distribution) distribution="$2" ;;
        --distribution-ahash64) distribution_ahash64="$2" ;;
        --format) format="$2" ;;
        --umber) umber_bin="$2" ;;
        --receipt) receipt="$2" ;;
      esac
      shift 2
      ;;
    --help | -h)
      usage
      exit 0
      ;;
    *) fail "unknown argument: $1" ;;
  esac
done

[[ -n "$distribution" && -d "$distribution" ]] || fail 'missing --distribution directory'
[[ "$distribution_ahash64" =~ ^[0-9a-f]{64}$ ]] || fail 'invalid --distribution-ahash64'
[[ -n "$format" && -f "$format" ]] || fail 'missing --format file'
[[ -x "$umber_bin" ]] || fail "missing Umber executable: $umber_bin"
[[ -x "$guard" ]] || fail "missing Umber guard: $guard"
[[ -f "$source_lock" && -f "$runtime_lock" ]] || fail 'missing representative resource lock'

distribution="$(realpath "$distribution")"
format="$(realpath "$format")"
umber_bin="$(realpath "$umber_bin")"
if [[ -n "$receipt" ]]; then
  mkdir -p "$(dirname "$receipt")"
  receipt="$(realpath -m "$receipt")"
fi

source_date_epoch=""
source_prefetch_args=()
runtime_prefetch_args=()
declare -A source_keys=()
declare -A runtime_keys=()
while read -r record relative expected_bytes expected_hash extra; do
  [[ -z "${record:-}" || "$record" == \#* ]] && continue
  case "$record" in
    source_date_epoch)
      [[ -z "${expected_bytes:-}" ]] || fail 'invalid source_date_epoch record'
      source_date_epoch="$relative"
      ;;
    source | local | pdflatex-source | pdflatex-local)
      [[ -z "${extra:-}" ]] || fail "invalid construction record for ${relative:-<missing>}"
      [[ "$relative" != /* && "$relative" != *..* && "$relative" != *\\* ]] || \
        fail "unsafe construction path: $relative"
      [[ "$expected_bytes" =~ ^[0-9]+$ && "$expected_hash" =~ ^[0-9a-f]{64}$ ]] || \
        fail "invalid construction identity for $relative"
      request_kind=tex
      [[ "$relative" == *.tfm ]] && request_kind=tfm
      request_key="${request_kind}:${relative##*/}"
      [[ -z "${source_keys[$request_key]:-}" ]] || fail "duplicate construction key: $request_key"
      source_keys[$request_key]=1
      source_prefetch_args+=(--prefetch-input "$request_key")
      ;;
  esac
done < "$source_lock"
[[ "$source_date_epoch" =~ ^[0-9]+$ ]] || fail 'missing source_date_epoch'

while read -r record request_kind relative expected_bytes expected_hash extra; do
  [[ -z "${record:-}" || "$record" == \#* ]] && continue
  [[ "$record" == source && -z "${extra:-}" ]] || \
    fail "invalid representative runtime record for ${relative:-<missing>}"
  [[ "$request_kind" == tex || "$request_kind" == tfm ]] || \
    fail "invalid representative runtime kind: $request_kind"
  [[ "$relative" != /* && "$relative" != *..* && "$relative" != *\\* ]] || \
    fail "unsafe representative runtime path: $relative"
  [[ "$expected_bytes" =~ ^[0-9]+$ && "$expected_hash" =~ ^[0-9a-f]{64}$ ]] || \
    fail "invalid representative runtime identity for $relative"
  request_key="${request_kind}:${relative##*/}"
  [[ -z "${runtime_keys[$request_key]:-}" ]] || fail "duplicate runtime key: $request_key"
  [[ -z "${source_keys[$request_key]:-}" ]] || \
    fail "runtime key duplicates construction key: $request_key"
  runtime_keys[$request_key]=1
  runtime_prefetch_args+=(--prefetch-input "$request_key")
done < "$runtime_lock"
[[ ${#source_keys[@]} -gt 0 && ${#runtime_keys[@]} -gt 0 ]] || fail 'empty resource closure'

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/source" "$scratch/loaded" "$scratch/source-cache" "$scratch/loaded-cache"
printf '\\end\n' > "$scratch/source/smoke.tex"
cp "$scratch/source/smoke.tex" "$scratch/loaded/smoke.tex"

run_smoke() {
  local directory="$1"
  local cache="$2"
  shift 2
  (
    cd "$directory"
    python3 "$guard" \
      --timeout-seconds 60 \
      --max-rss-mib 2048 \
      --term-grace-seconds 5 \
      -- env -u TEXINPUTS -u TEXFONTS \
        XDG_CACHE_HOME="$cache" \
        UMBER_OFFLINE=1 \
        SOURCE_DATE_EPOCH="$source_date_epoch" \
        "$umber_bin" run --pdflatex \
          --distribution "$distribution" \
          --distribution-ahash64 "$distribution_ahash64" \
          --offline \
          smoke.tex "$@"
  )
}

run_smoke "$scratch/source" "$scratch/source-cache" \
  "${source_prefetch_args[@]}" "${runtime_prefetch_args[@]}" \
  > "$scratch/source.stdout" 2> "$scratch/source.stderr" || {
    cat "$scratch/source.stderr" >&2
    fail 'source-profile cold prefetch smoke failed'
  }
run_smoke "$scratch/loaded" "$scratch/loaded-cache" \
  --format "$format" "${runtime_prefetch_args[@]}" \
  > "$scratch/loaded.stdout" 2> "$scratch/loaded.stderr" || {
    cat "$scratch/loaded.stderr" >&2
    fail 'loaded-format cold prefetch smoke failed'
  }

source_lock_sha256="$(sha256sum "$source_lock" | awk '{print $1}')"
runtime_lock_sha256="$(sha256sum "$runtime_lock" | awk '{print $1}')"
format_sha256="$(sha256sum "$format" | awk '{print $1}')"
format_bytes="$(wc -c < "$format" | tr -d ' ')"
if [[ -n "$receipt" ]]; then
  {
    printf 'schema=1\n'
    printf 'distribution_root_sha256=%s\n' "$distribution_ahash64"
    printf 'source_lock_sha256=%s\n' "$source_lock_sha256"
    printf 'runtime_lock_sha256=%s\n' "$runtime_lock_sha256"
    printf 'format_sha256=%s\n' "$format_sha256"
    printf 'format_bytes=%s\n' "$format_bytes"
    printf 'source_prefetch_keys=%s\n' "$((${#source_keys[@]} + ${#runtime_keys[@]}))"
    printf 'loaded_prefetch_keys=%s\n' "${#runtime_keys[@]}"
    printf 'source_smoke=pass\n'
    printf 'loaded_smoke=pass\n'
  } > "$receipt"
fi
printf 'pdfLaTeX representative resource smoke: PASS source_keys=%s loaded_keys=%s root_sha256=%s\n' \
  "$((${#source_keys[@]} + ${#runtime_keys[@]}))" "${#runtime_keys[@]}" "$distribution_ahash64"
