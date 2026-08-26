#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
lock_file="${repo_root}/tests/latex-source.lock"
engine="latex"
output_dir=""
texmf_dist="${UMBER_TEXMF_DIST:-${repo_root}/third_party/texlive-20260301-texmf/texmf-dist}"
distribution_path=""
distribution_ahash64=""
publish_input_closure=0
force_regeneration=0
check_only=0
guard="${repo_root}/scripts/run-umber-guarded.py"
guard_timeout="${UMBER_LATEX_FORMAT_TIMEOUT_SECONDS:-600}"
guard_rss_mib="${UMBER_LATEX_FORMAT_MAX_RSS_MIB:-2048}"
engine_fuel="${UMBER_LATEX_FORMAT_ENGINE_FUEL:-10000000000}"

usage() {
  cat <<'EOF'
usage: scripts/build-latex-format.sh [--engine latex|pdflatex]
                                     --distribution PATH
                                     --distribution-ahash64 AHASH64
                                     [--texmf-dist PATH] [--output-dir PATH]
                                     [--publish-input-closure] [--force|--check]

Restores a validated pinned Umber-native LaTeX format from the generated-format
cache, or verifies the exact mode-specific locked input closure, builds once,
validates the resulting image through the cache codec, and atomically publishes
the miss. --force always regenerates. --check regenerates and compares the cache
and output without changing either. Every engine run uses the same authenticated
local distribution in offline mode. The default output is target/<engine>-format.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --engine)
      [[ $# -ge 2 ]] || { printf '%s\n' 'missing mode after --engine' >&2; exit 2; }
      engine="$2"
      shift 2
      ;;
    --texmf-dist)
      [[ $# -ge 2 ]] || { printf '%s\n' 'missing path after --texmf-dist' >&2; exit 2; }
      texmf_dist="$2"
      shift 2
      ;;
    --distribution)
      [[ $# -ge 2 ]] || { printf '%s\n' 'missing path after --distribution' >&2; exit 2; }
      distribution_path="$2"
      shift 2
      ;;
    --distribution-ahash64)
      [[ $# -ge 2 ]] || { printf '%s\n' 'missing digest after --distribution-ahash64' >&2; exit 2; }
      distribution_ahash64="$2"
      shift 2
      ;;
    --output-dir)
      [[ $# -ge 2 ]] || { printf '%s\n' 'missing path after --output-dir' >&2; exit 2; }
      output_dir="$2"
      shift 2
      ;;
    --publish-input-closure)
      publish_input_closure=1
      shift
      ;;
    --force)
      force_regeneration=1
      shift
      ;;
    --check)
      check_only=1
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      printf 'build-latex-format.sh: unknown option: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

case "$engine" in
  latex)
    format_input="${texmf_dist}/tex/latex-dev/base/latex.ltx"
    ;;
  pdflatex)
    format_input="${texmf_dist}/tex/latex/tex-ini-files/pdflatex.ini"
    ;;
  *)
    printf 'build-latex-format.sh: unsupported engine: %s\n' "$engine" >&2
    usage >&2
    exit 2
    ;;
esac
format_name="$engine"
output_dir="${output_dir:-${repo_root}/target/${format_name}-format}"
[[ "$force_regeneration" -eq 0 || "$check_only" -eq 0 ]] || {
  printf '%s\n' 'build-latex-format.sh: --force and --check are mutually exclusive' >&2
  exit 2
}

[[ -n "$distribution_path" ]] || {
  printf '%s\n' 'build-latex-format.sh: --distribution PATH is required' >&2
  exit 2
}
[[ -n "$distribution_ahash64" ]] || {
  printf '%s\n' 'build-latex-format.sh: --distribution-ahash64 AHASH64 is required' >&2
  exit 2
}
[[ "$distribution_ahash64" =~ ^[0-9a-f]{16}$ ]] || {
  printf '%s\n' 'build-latex-format.sh: --distribution-ahash64 must be 16 lowercase hexadecimal characters' >&2
  exit 2
}

fail() {
  printf 'build-latex-format.sh: %s\n' "$*" >&2
  exit 1
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

[[ -f "$lock_file" ]] || fail "missing source lock: $lock_file"

distribution="$(awk '$1 == "distribution" { print $2 }' "$lock_file")"
locked_distribution_ahash64="$(awk '$1 == "distribution_ahash64" { print $2 }' "$lock_file")"
format_schema="$(awk '$1 == "format_schema" { print $2 }' "$lock_file")"
source_date_epoch="$(awk '$1 == "source_date_epoch" { print $2 }' "$lock_file")"
[[ -n "$distribution" && -n "$locked_distribution_ahash64" && \
  -n "$format_schema" && -n "$source_date_epoch" ]] || \
  fail "source lock is missing required metadata"
[[ "$locked_distribution_ahash64" =~ ^[0-9a-f]{16}$ ]] || \
  fail "source lock has an invalid distribution aHash64"
[[ "$distribution_ahash64" == "$locked_distribution_ahash64" ]] || \
  fail "distribution aHash64 does not match the source lock: expected $locked_distribution_ahash64, got $distribution_ahash64"

if [[ -d "$distribution_path" ]]; then
  distribution_path="$(cd "$distribution_path" && pwd -P)"
  if [[ -f "$distribution_path/manifest-v7.json" ]]; then
    distribution_manifest="$distribution_path/manifest-v7.json"
  elif [[ -f "$distribution_path/manifest-v6.json" ]]; then
    distribution_manifest="$distribution_path/manifest-v6.json"
  elif [[ -f "$distribution_path/manifest-v5.json" ]]; then
    distribution_manifest="$distribution_path/manifest-v5.json"
  elif [[ -f "$distribution_path/manifest.json" ]]; then
    distribution_manifest="$distribution_path/manifest.json"
  else
    fail "distribution directory has no manifest-v7.json, manifest-v6.json, manifest-v5.json, or manifest.json: $distribution_path"
  fi
elif [[ -f "$distribution_path" ]]; then
  distribution_directory="$(cd "$(dirname "$distribution_path")" && pwd -P)"
  distribution_path="${distribution_directory}/$(basename "$distribution_path")"
  distribution_manifest="$distribution_path"
else
  fail "distribution path is not a local file or directory: $distribution_path"
fi
cd "$repo_root"
cargo build -q --release --manifest-path tools/texlive-wasm-publish/Cargo.toml
publisher="${CARGO_TARGET_DIR:-${repo_root}/tools/texlive-wasm-publish/target}/release/texlive-wasm-publish"
[[ -x "$publisher" ]] || fail "publisher was not built at $publisher"
actual_distribution_ahash64="$($publisher --file-ahash64 "$distribution_manifest")"
[[ "$actual_distribution_ahash64" == "$distribution_ahash64" ]] || \
  fail "distribution root digest mismatch for $distribution_manifest: expected $distribution_ahash64, got $actual_distribution_ahash64"

scratch_parent="${UMBER_LATEX_FORMAT_WORK_ROOT:-${output_dir}/work}"
mkdir -p "$scratch_parent"
tmp_root="$(mktemp -d "${scratch_parent}/build.XXXXXX")"
cleanup() {
  local status=$?
  if [[ $status -eq 0 ]]; then
    rm -rf "$tmp_root"
  else
    printf 'build-latex-format.sh: failed artifacts: %s\n' "$tmp_root" >&2
  fi
}
trap cleanup EXIT
expected_receipt="${tmp_root}/expected.inputs"
expected_index="${tmp_root}/expected.index"
closure_index="${tmp_root}/input-closure.index"
source_index="${tmp_root}/sources.index"
identity_index="${tmp_root}/input-identities.index"
: > "$expected_index"
: > "$closure_index"
: > "$source_index"
: > "$identity_index"

while read -r kind relative expected_bytes expected_hash extra; do
  [[ -z "${kind:-}" || "$kind" == \#* ]] && continue
  case "$kind" in
    source)
      source="${texmf_dist}/${relative}"
      ;;
    local)
      source="${repo_root}/${relative}"
      ;;
    pdflatex-source)
      [[ "$engine" == pdflatex ]] || continue
      source="${texmf_dist}/${relative}"
      ;;
    pdflatex-local)
      [[ "$engine" == pdflatex ]] || continue
      source="${repo_root}/${relative}"
      ;;
    *)
      continue
      ;;
  esac
  [[ -z "${extra:-}" ]] || fail "invalid source lock entry for $relative"
  [[ "$relative" != /* && "$relative" != *..* && "$relative" != *\\* ]] || \
    fail "unsafe source path in lock: $relative"
  printf '%s\t%s\n' "$source" "$expected_bytes" >> "$expected_index"
  printf '%s\t%s\t%s\n' "$source" "$expected_bytes" "$expected_hash" >> "$source_index"
  request_name="${relative##*/}"
  [[ "$request_name" =~ ^[A-Za-z0-9._/-]+$ ]] || \
    fail "source lock path has no canonical request key: $relative"
  if [[ "$request_name" == *.tfm ]]; then
    request_key="tfm:${request_name}"
  else
    request_key="tex:${request_name}"
  fi
  printf '%s\n' "$request_key" >> "$closure_index"
  printf '%s\t%s\t%s\n' "$request_key" "$($publisher --file-ahash64 "$source")" "$expected_bytes" >> "$identity_index"
done < "$lock_file"
LC_ALL=C sort -k1,1 "$expected_index" | awk -F '\t' '{ print $2 "\t" $1 }' | LC_ALL=C sort > "$expected_receipt"
LC_ALL=C sort -u "$closure_index" -o "$closure_index"
LC_ALL=C sort -k1,1 "$identity_index" -o "$identity_index"

prefetch_source_closure() {
  local source expected_bytes expected_hash actual_bytes actual_hash
  [[ -d "$texmf_dist" ]] || fail "missing pinned texmf-dist root: $texmf_dist"
  [[ -f "$format_input" ]] || fail "missing format entry point: $format_input"
  while IFS=$'\t' read -r source expected_bytes expected_hash; do
    [[ -f "$source" ]] || fail "missing pinned source: $source"
    actual_bytes="$(wc -c < "$source" | tr -d ' ')"
    [[ "$actual_bytes" == "$expected_bytes" ]] || \
      fail "length mismatch for $source: expected $expected_bytes, got $actual_bytes"
    actual_hash="$(sha256 "$source")"
    [[ "$actual_hash" == "$expected_hash" ]] || \
      fail "hash mismatch for $source: expected $expected_hash, got $actual_hash"
  done < "$source_index"
  printf 'build-latex-format.sh: prefetched and verified %s pinned format inputs\n' \
    "$(wc -l < "$source_index" | tr -d ' ')" >&2
}

texinputs="${repo_root}/tests/latex:${texmf_dist}/tex/latex-dev/base:${texmf_dist}/tex/latex-dev/l3kernel:${texmf_dist}/tex/latex/l3backend:${texmf_dist}/tex/latex/atveryend:${texmf_dist}/tex/latex-dev/firstaid:${texmf_dist}/tex/generic/unicode-data:${texmf_dist}/tex/generic/atbegshi:${texmf_dist}/tex/generic/babel:${texmf_dist}/tex/generic/babel-english:${texmf_dist}/tex/generic/hyphen:${texmf_dist}/tex/generic/knuth-lib:${texmf_dist}/tex/generic/pdftex"
texfonts="${texmf_dist}/fonts/tfm/public/cm:${texmf_dist}/fonts/tfm/public/latex-fonts:${texmf_dist}/fonts/tfm/jknappen/ec"
prefetch_args=()
while IFS= read -r request_key; do
  prefetch_args+=(--prefetch-input "$request_key")
done < "$closure_index"

cd "$repo_root"
cargo build --release -p umber
umber_bin="${CARGO_TARGET_DIR:-${repo_root}/target}/release/umber"
[[ -x "$umber_bin" ]] || fail "Umber binary was not built at $umber_bin"
[[ -x "$guard" ]] || fail "missing shared Umber watchdog: $guard"

run_umber() {
  python3 "$guard" \
    --timeout-seconds "$guard_timeout" \
    --max-rss-mib "$guard_rss_mib" \
    --term-grace-seconds 5 \
    -- env UMBER_ENGINE_FUEL="$engine_fuel" "$umber_bin" "$@"
}

run_engine() {
  local directory="$1"
  shift
  (
    cd "$directory"
    SOURCE_DATE_EPOCH="$source_date_epoch" TEXINPUTS="$texinputs" TEXFONTS="$texfonts" \
      run_umber run "--${engine}" \
        --distribution "$distribution_path" \
        --distribution-ahash64 "$distribution_ahash64" \
        --offline \
        "$@"
  )
}

build_one() {
  local directory="$1"
  mkdir -p "$directory"
  run_engine "$directory" "$format_input" "${prefetch_args[@]}" --format-out "${format_name}.fmt" \
    --input-records-out build.inputs > "${directory}/build.stdout" 2> "${directory}/build.stderr"
  if grep -q '^! ' "${directory}/build.stdout"; then
    grep -m1 '^! ' "${directory}/build.stdout" >&2
    fail "LaTeX format build emitted a diagnostic"
  fi
  LC_ALL=C sort "${directory}/build.inputs" > "${directory}/build.inputs.sorted"
  cmp "$expected_receipt" "${directory}/build.inputs.sorted" || \
    fail "LaTeX format build opened inputs outside the locked closure"
}

package_id="$(cargo pkgid -p umber)"
engine_version="${package_id##*#}"
build_configuration="${tmp_root}/build-configuration.txt"
{
  printf 'schema=1\nprofile=release\nfeatures=default\npackage=umber@%s\n' "$engine_version"
  rustc -Vv
} > "$build_configuration"
cache_args=(
  --engine "$engine"
  --distribution "$distribution"
  --closure "$closure_index"
  --source-lock "$lock_file"
  --build-configuration "$build_configuration"
)
if [[ -n "${UMBER_FORMAT_CACHE_ROOT:-}" ]]; then
  cache_args+=(--cache-root "$UMBER_FORMAT_CACHE_ROOT")
fi
cached_format="${tmp_root}/cached/${format_name}.fmt"
mkdir -p "$(dirname "$cached_format")"
cache_state="$(
  SOURCE_DATE_EPOCH="$source_date_epoch" \
    run_umber format-cache restore "${cache_args[@]}" --format-out "$cached_format"
)"
[[ "$cache_state" == hit || "$cache_state" == miss ]] || \
  fail "unexpected generated format cache result: $cache_state"
if [[ "$check_only" -eq 1 && "$cache_state" != hit ]]; then
  fail "--check requires an existing validated generated format cache entry"
fi

generated=0
if [[ "$cache_state" == miss || "$force_regeneration" -eq 1 || "$check_only" -eq 1 ]]; then
  prefetch_source_closure
  build_one "${tmp_root}/first"
  format_file="${tmp_root}/first/${format_name}.fmt"
  generated=1
else
  format_file="$cached_format"
fi

magic="$(od -An -t x1 -N 8 "$format_file" | tr -d ' \n')"
actual_schema="$(od -An -t u4 -j 8 -N 4 "$format_file" | tr -d ' \n')"
[[ "$magic" == 554d4252464d5400 ]] || fail "format image lacks Umber format magic"
[[ "$actual_schema" == "$format_schema" ]] || \
  fail "format schema $actual_schema does not match locked schema $format_schema"
if [[ "$generated" -eq 1 && "$cache_state" == hit ]]; then
  cmp "$format_file" "$cached_format" || \
    fail "regenerated ${format_name} format differs from the validated cache entry"
elif [[ "$generated" -eq 1 ]]; then
  SOURCE_DATE_EPOCH="$source_date_epoch" \
    run_umber format-cache store "${cache_args[@]}" --format "$format_file" >/dev/null
fi

format_ahash64="$($publisher --file-ahash64 "$format_file")"
format_bytes="$(wc -c < "$format_file" | tr -d ' ')"
source_manifest_ahash64="$distribution_ahash64"
metadata_schema=3
closure_metadata=""
if [[ "$publish_input_closure" -eq 1 ]]; then
  metadata_schema=4
  input_closure_json="$(awk '
    BEGIN { print "    \"keys\": [" }
    {
      if (NR > 1) printf ",\n"
      printf "      \"%s\"", $0
    }
    END { print "\n    ]" }
  ' "$closure_index")"
  closure_metadata="$(printf ',\n  "inputClosure": {\n    "schema": 1,\n%s\n  }' "$input_closure_json")"
  awk -F '\t' '
    BEGIN { print "{\n  \"schema\": 1,\n  \"inputs\": [" }
    {
      if (NR > 1) printf ",\n"
      printf "    {\"key\": \"%s\", \"ahash64\": \"%s\", \"bytes\": %s}", $1, $2, $3
    }
    END { print "\n  ]\n}" }
  ' "$identity_index" > "${tmp_root}/${format_name}-input-identities.json"
fi

cat > "${tmp_root}/${format_name}-format.json" <<EOF
{
  "schema": ${metadata_schema},
  "name": "${format_name}",
  "object": "ahash64-v1-${format_ahash64}",
  "ahash64": "${format_ahash64}",
  "bytes": ${format_bytes},
  "engine": "umber",
  "engineVersion": "${engine_version}",
  "formatSchema": ${format_schema},
  "sourceDistribution": "${distribution}",
  "sourceManifestAhash64": "${source_manifest_ahash64}",
  "sourceDateEpoch": ${source_date_epoch}${closure_metadata}
}
EOF

if [[ "$check_only" -eq 1 ]]; then
  cmp "$format_file" "${output_dir}/${format_name}.fmt" || \
    fail "published ${format_name}.fmt differs from the reproducible cache entry"
  cmp "${tmp_root}/${format_name}-format.json" "${output_dir}/${format_name}-format.json" || \
    fail "published ${format_name}-format.json is stale"
  if [[ "$publish_input_closure" -eq 1 ]]; then
    cmp "${tmp_root}/${format_name}-input-identities.json" \
      "${output_dir}/${format_name}-input-identities.json" || \
      fail "published ${format_name}-input-identities.json is stale"
  fi
else
  mkdir -p "$output_dir"
  staged_format="$(mktemp "${output_dir}/.${format_name}.fmt.XXXXXX")"
  staged_metadata="$(mktemp "${output_dir}/.${format_name}-format.json.XXXXXX")"
  cp "$format_file" "$staged_format"
  cp "${tmp_root}/${format_name}-format.json" "$staged_metadata"
  mv -f "$staged_format" "${output_dir}/${format_name}.fmt"
  mv -f "$staged_metadata" "${output_dir}/${format_name}-format.json"
  if [[ "$publish_input_closure" -eq 1 ]]; then
    staged_identities="$(mktemp "${output_dir}/.${format_name}-input-identities.json.XXXXXX")"
    cp "${tmp_root}/${format_name}-input-identities.json" "$staged_identities"
    mv -f "$staged_identities" "${output_dir}/${format_name}-input-identities.json"
  fi
fi

printf 'Umber %s format: ahash64-v1=%s bytes=%s schema=%s source=%s\n' \
  "$format_name" "$format_ahash64" "$format_bytes" "$format_schema" "$distribution"
