#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

trip_root="${repo_root}/third_party/trip"
texmfcnf="${repo_root}/third_party/texlive-source/src/texk/web2c/triptrap"
target_dir="${CARGO_TARGET_DIR:-target}"
[[ "$target_dir" == /* ]] || target_dir="${repo_root}/${target_dir}"
oracle_bin="${target_dir}/tex82-oracle/bin"

[[ -f "${trip_root}/trip.tex" && -f "${trip_root}/trip.tfm" ]] || {
  printf 'test-tex82-trip-observer: missing pinned TRIP inputs; run scripts/fetch-conformance-inputs.sh\n' >&2
  exit 1
}

scripts/build-tex82-oracle.sh --offline >/dev/null

work_root="$(mktemp -d "${TMPDIR:-/tmp}/umber-tex82-trip-observer.XXXXXX")"
trap 'rm -rf "$work_root"' EXIT

run_phase() {
  local executable="$1" directory="$2" prefix="$3"
  cp "${trip_root}/trip.tex" "${trip_root}/trip.tfm" "$directory/"
  (
    cd "$directory"
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C \
      SOURCE_DATE_EPOCH=1783604160 FORCE_SOURCE_DATE=1 TEXMFCNF="$texmfcnf" \
      "$executable" -ini -interaction=nonstopmode trip.tex >"${prefix}-initex-terminal.txt" 2>&1
    printf '%s\n' "$?" >"${prefix}-initex-status.txt"
    cp tex82-events.jsonl "${prefix}-initex-events.jsonl" 2>/dev/null || :
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C \
      SOURCE_DATE_EPOCH=1783604160 FORCE_SOURCE_DATE=1 TEXMFCNF="$texmfcnf" \
      "$executable" -interaction=nonstopmode '&trip' trip.tex >"${prefix}-trip-terminal.txt" 2>&1
    printf '%s\n' "$?" >"${prefix}-trip-status.txt"
    cp tex82-events.jsonl "${prefix}-trip-events.jsonl" 2>/dev/null || :
  ) || true
}

normalize_output() {
  sed -E 's/preloaded format=[^)]*/preloaded format=<FORMAT>/' "$1"
}

dvi_byte_at() {
  od -An -v -tu1 -j "$2" -N 1 "$1" | tr -d '[:space:]'
}

compare_normalized_dvi() {
  local clean_dvi="$1" profile_dvi="$2"
  local clean_comment_len profile_comment_len comment_end
  local clean_size profile_size

  [[ "$(dvi_byte_at "$clean_dvi" 0)" == 247 ]] || {
    printf 'invalid clean DVI preamble: %s\n' "$clean_dvi" >&2
    return 1
  }
  [[ "$(dvi_byte_at "$profile_dvi" 0)" == 247 ]] || {
    printf 'invalid profiled DVI preamble: %s\n' "$profile_dvi" >&2
    return 1
  }
  clean_comment_len="$(dvi_byte_at "$clean_dvi" 14)"
  profile_comment_len="$(dvi_byte_at "$profile_dvi" 14)"
  [[ -n "$clean_comment_len" && "$clean_comment_len" == "$profile_comment_len" ]] || {
    printf 'DVI preamble comment lengths differ\n' >&2
    return 1
  }
  comment_end=$((15 + clean_comment_len))
  clean_size="$(wc -c <"$clean_dvi" | tr -d '[:space:]')"
  profile_size="$(wc -c <"$profile_dvi" | tr -d '[:space:]')"
  ((comment_end <= clean_size && comment_end <= profile_size)) || {
    printf 'truncated DVI preamble comment\n' >&2
    return 1
  }

  # The DVI banner is the sole permitted output normalization. This compares
  # the complete preamble through its length byte, skips only its payload, and
  # then compares every remaining byte (including the postamble and pointers).
  cmp -n 15 "$clean_dvi" "$profile_dvi"
  cmp -s \
    <(dd if="$clean_dvi" bs=1 skip="$comment_end" status=none) \
    <(dd if="$profile_dvi" bs=1 skip="$comment_end" status=none)
}

mkdir -p "$work_root/clean" "$work_root/profile-a" "$work_root/profile-b"
run_phase "${oracle_bin}/umber-tex82-oracle" "$work_root/clean" clean
run_phase "${oracle_bin}/umber-tex82-oracle-trip-profile" "$work_root/profile-a" profile
run_phase "${oracle_bin}/umber-tex82-oracle-trip-profile" "$work_root/profile-b" profile

for profile in profile-a profile-b; do
  cargo run -q -p tex-oracle --bin tex-oracle-validate -- \
    --tex82-trip-profile "$work_root/$profile/profile-initex-events.jsonl"
  cargo run -q -p tex-oracle --bin tex-oracle-validate -- \
    --tex82-trip-profile "$work_root/$profile/profile-trip-events.jsonl"
  compare_normalized_dvi "$work_root/clean/trip.dvi" "$work_root/$profile/trip.dvi"
  cmp "$work_root/clean/clean-initex-status.txt" "$work_root/$profile/profile-initex-status.txt"
  cmp "$work_root/clean/clean-trip-status.txt" "$work_root/$profile/profile-trip-status.txt"
  for channel in initex-terminal.txt trip-terminal.txt trip.log; do
    if [[ "$channel" == trip.log ]]; then
      clean_channel="$work_root/clean/trip.log"
      profile_channel="$work_root/$profile/trip.log"
    else
      clean_channel="$work_root/clean/clean-$channel"
      profile_channel="$work_root/$profile/profile-$channel"
    fi
    normalize_output "$clean_channel" >"$work_root/clean-$channel"
    normalize_output "$profile_channel" >"$work_root/$profile-$channel"
    cmp "$work_root/clean-$channel" "$work_root/$profile-$channel"
  done
done
cmp "$work_root/profile-a/profile-initex-events.jsonl" "$work_root/profile-b/profile-initex-events.jsonl"
cmp "$work_root/profile-a/profile-trip-events.jsonl" "$work_root/profile-b/profile-trip-events.jsonl"
printf 'TeX82 bounded TRIP observer passed\n'
