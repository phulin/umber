#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

manifest="tests/trip-manifest.txt"
download_dir="third_party/trip"
target_dir="${CARGO_TARGET_DIR:-target}"
if [[ "$target_dir" != /* ]]; then
  target_dir="${repo_root}/${target_dir}"
fi
work_root="${target_dir}/trip"
diff_dir="${work_root}/diffs"
umber_bin="${target_dir}/debug/umber"

mode="all"
offline=0
keep_work=0

usage() {
  cat <<'EOF'
usage:
  scripts/trip.sh [all|fetch|reference|umber|self-test] [--offline] [--keep-work]

Runs the official Knuth TeX82 TRIP conformance harness outside cargo tests.
The harness fetches the pinned CTAN TRIP materials into third_party/trip/,
verifies SHA-256 hashes, rebuilds trip.tfm via PLtoTF/TFtoPL, compares the
INITEX and format-run transcripts, runs DVItype, and runs Umber against the
same official input.

Reference tools are discovered on PATH unless overridden:
  UMBER_TRIP_TOOLS=/path/to/pinned/trip-tool-directory
  UMBER_TRIP_INITEX=/path/to/special-initex
  UMBER_REF_TEX=/path/to/pdftex-or-tex
  UMBER_REF_PLTOTF=/path/to/pltotf
  UMBER_REF_TFTOPL=/path/to/tftopl
  UMBER_REF_DVITYPE=/path/to/dvitype

TRIP requires Knuth's special INITEX build with mem_top/mem_max=3000,
mem_min/mem_bot=1, error_line=64, half_error_line=32, max_print_line=72,
and the other settings documented in tripman.tex Appendix A.
Build all pinned reference tools with scripts/build-trip-initex.sh.
EOF
}

if [[ "$#" -gt 0 ]]; then
  case "$1" in
    all|fetch|reference|umber|self-test)
      mode="$1"
      shift
      ;;
  esac
fi

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --offline)
      offline=1
      shift
      ;;
    --keep-work)
      keep_work=1
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      printf 'trip.sh: unknown option: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

fail() {
  printf 'trip.sh: %s\n' "$*" >&2
  exit 1
}

sha256_file() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    fail "need shasum or sha256sum on PATH"
  fi
}

tool_path() {
  local env_name="$1"
  local default_name="$2"
  local value="${!env_name:-}"
  if [[ -n "$value" ]]; then
    [[ -x "$value" ]] || fail "$env_name is not executable: $value"
    printf '%s\n' "$value"
    return
  fi
  local tools="${UMBER_TRIP_TOOLS:-${target_dir}/trip-initex/bin}"
  if [[ -x "${tools}/umber-trip-${default_name}" ]]; then
    printf '%s\n' "${tools}/umber-trip-${default_name}"
    return
  fi
  command -v "$default_name" || fail "could not locate $default_name; set $env_name=/absolute/path/to/$default_name"
}

trip_initex() {
  local value="${UMBER_TRIP_INITEX:-${UMBER_REF_TEX:-}}"
  if [[ -n "$value" ]]; then
    [[ -x "$value" ]] || fail "configured INITEX is not executable: $value"
    printf '%s\n' "$value"
    return
  fi
  local tools="${UMBER_TRIP_TOOLS:-${target_dir}/trip-initex/bin}"
  if [[ -x "${tools}/umber-trip-tex" ]]; then
    printf '%s\n' "${tools}/umber-trip-tex"
    return
  fi
  fail "missing pinned special TRIP INITEX; run scripts/build-trip-initex.sh, or set UMBER_TRIP_INITEX"
}

tex_dvi_args() {
  case "$(basename "$1")" in
    *pdftex*|*pdfTeX*)
      printf '%s\n' '-output-format=dvi'
      ;;
  esac
}

fetch_materials() {
  mkdir -p "$download_dir"
  while read -r name url expected extra; do
    [[ -z "${name:-}" || "$name" == \#* ]] && continue
    [[ -z "${extra:-}" ]] || fail "malformed manifest line for $name"
    local path="${download_dir}/${name}"
    if [[ -f "$path" ]]; then
      verify_hash "$path" "$expected" "$name"
      printf 'verified %s\n' "$name" >&2
      continue
    fi
    [[ "$offline" -eq 0 ]] || fail "missing $path while running --offline"
    local tmp="${path}.tmp"
    printf 'fetching %s\n' "$name" >&2
    curl -fsSL "$url" -o "$tmp"
    verify_hash "$tmp" "$expected" "$name"
    mv "$tmp" "$path"
  done < "$manifest"
}

verify_hash() {
  local path="$1"
  local expected="$2"
  local name="$3"
  local actual
  actual="$(sha256_file "$path")"
  if [[ "$actual" != "$expected" ]]; then
    fail "sha256 mismatch for $name at $path: expected $expected, got $actual"
  fi
}

prepare_work() {
  if [[ "$keep_work" -eq 0 && -d "$work_root" ]]; then
    rm -rf "$work_root"
  fi
  mkdir -p "$work_root" "$diff_dir"
}

copy_trip_inputs() {
  local dest="$1"
  mkdir -p "$dest"
  cp \
    "${download_dir}/trip.tex" \
    "${download_dir}/trip.pl" \
    "${download_dir}/trip.tfm" \
    "${download_dir}/tripos.tex" \
    "$dest/"
}

compare_text() {
  local label="$1"
  local expected="$2"
  local actual="$3"
  local diff_path="${diff_dir}/${label}.diff"
  if [[ ! -f "$actual" ]]; then
    printf 'missing actual artifact for %s: %s\n' "$label" "$actual" > "$diff_path"
    printf 'FAIL %s: missing %s; see %s\n' "$label" "$actual" "$diff_path" >&2
    return 1
  fi
  if diff -u "$expected" "$actual" > "$diff_path"; then
    rm -f "$diff_path"
    printf 'ok %s\n' "$label" >&2
    return 0
  fi
  printf 'FAIL %s; see %s\n' "$label" "$diff_path" >&2
  return 1
}

compare_binary() {
  local label="$1"
  local expected="$2"
  local actual="$3"
  local diff_path="${diff_dir}/${label}.diff"
  if [[ ! -f "$actual" ]]; then
    printf 'missing actual artifact for %s: %s\n' "$label" "$actual" > "$diff_path"
    printf 'FAIL %s: missing %s; see %s\n' "$label" "$actual" "$diff_path" >&2
    return 1
  fi
  if cmp -s "$expected" "$actual"; then
    rm -f "$diff_path"
    printf 'ok %s\n' "$label" >&2
    return 0
  fi
  {
    printf '%s differs\n' "$label"
    printf 'expected: %s\nactual:   %s\n' "$expected" "$actual"
    cmp -l "$expected" "$actual" | sed -n '1,40p'
  } > "$diff_path"
  printf 'FAIL %s; see %s\n' "$label" "$diff_path" >&2
  return 1
}

normalize_trip_log() {
  sed -E \
    -e '1s/[[:space:]]+[0-9]{1,2} [A-Z]{3} [0-9]{4} [0-9]{2}:[0-9]{2}$/  <TRIP-DATE>/' \
    -e '1s/ \(TeX Live [^)]*\)//' \
    -e '1s/ \(Web2C [^)]*\)//' \
    -e 's@\(\./trip\.tex@\(trip.tex@g' \
    -e 's@\([^()[:space:]]*/trip\.tex@\(trip.tex@g' \
    -e 's@\(\./tripos\.tex@\(tripos.tex@g' \
    -e 's@\([^()[:space:]]*/tripos\.tex@\(tripos.tex@g' \
    -e 's/\(preloaded format=trip [0-9]{4}\.[0-9]{1,2}\.[0-9]{1,2}\)/\(preloaded format=trip <TRIP-FORMAT-DATE>\)/' \
    -e 's/^[0-9]+ strings of total length [0-9]+$/<TRIP-STRING-TOTALS>/' \
    -e 's/^ [0-9]+ strings out of [0-9]+$/ <TRIP-STRINGS>/' \
    -e 's/^ [0-9]+ string characters out of [0-9]+$/ <TRIP-STRING-CHARACTERS>/' \
    -e 's/^( [0-9]+ multiletter control sequences out of )[0-9]+(\+[0-9]+)?$/\1<TRIP-HASH-CAPACITY>/' \
    -e 's/^ [0-9]+ hyphenation exceptions? out of [0-9]+$/ <TRIP-HYPHEN-EXCEPTIONS>/' \
    -e 's/(Hyphenation trie of length [0-9]+ has [0-9]+ ops out of )[0-9]+/\1<TRIP-TRIE-OP-CAPACITY>/' \
    "$1" > "$2"
}

normalize_trip_fot() {
  sed -E \
    -e 's@^\*\*\(\./trip\.tex ##@** \&trip  trip \
(trip.tex ##@' \
    -e '1s/ \(TeX Live [^)]*\)//' \
    -e '1s/ \(Web2C [^)]*\)//' \
    -e '1s/ \(preloaded format=tex\)/ \(INITEX\)/' \
    -e 's@\(\./trip\.tex@\(trip.tex@g' \
    -e 's@\([^()[:space:]]*/trip\.tex@\(trip.tex@g' \
    "$1" > "$2"
}

normalize_trip_typ() {
  sed -E \
    -e '1s/ \(.*\)$//' \
    -e "s/^' TeX output .*'$/' TeX output <TRIP-DATE>'/" \
    "$1" > "$2"
}

reconcile_trip_rounding() {
  local expected="$1" actual="$2" output="$3"
  python3 - "$expected" "$actual" "$output" <<'PY'
import decimal
import pathlib
import re
import sys

expected_path, actual_path, output_path = map(pathlib.Path, sys.argv[1:])
expected = expected_path.read_text().splitlines(keepends=True)
actual = actual_path.read_text().splitlines(keepends=True)
if len(expected) != len(actual):
    output_path.write_text("".join(actual))
    raise SystemExit(0)
pattern = re.compile(r"^(\\[hv]box\([^\n]*, glue set (?:- )?)(-?[0-9]+(?:\.[0-9]+)?)(fil(?:l|ll)?[^\n]*\n?)$")
out = []
for wanted, got in zip(expected, actual):
    if wanted == got:
        out.append(got)
        continue
    wm = pattern.match(wanted)
    gm = pattern.match(got)
    if wm and gm and wm.group(1) == gm.group(1) and wm.group(3) == gm.group(3):
        delta = abs(decimal.Decimal(wm.group(2)) - decimal.Decimal(gm.group(2)))
        if delta <= decimal.Decimal("0.001"):
            out.append(wanted)
            continue
    out.append(got)
output_path.write_text("".join(out))
PY
}

reconcile_trip_typ_rounding() {
  local expected="$1" actual="$2" output="$3"
  python3 - "$expected" "$actual" "$output" <<'PY'
import pathlib
import re
import sys

expected_path, actual_path, output_path = map(pathlib.Path, sys.argv[1:])
expected = expected_path.read_text().splitlines(keepends=True)
actual = actual_path.read_text().splitlines(keepends=True)
if len(expected) != len(actual):
    output_path.write_text("".join(actual))
    raise SystemExit(0)
movement = re.compile(r"^(\d+: (?:right|w|x|down|y|z)[0-4] )(-?\d+)( .*)$")
out = []
for wanted, got in zip(expected, actual):
    if wanted == got:
        out.append(got)
        continue
    wm = movement.match(wanted)
    gm = movement.match(got)
    if wm and gm and wm.group(1) == gm.group(1) and wm.group(3) == gm.group(3):
        if abs(int(wm.group(2)) - int(gm.group(2))) <= 64:
            out.append(wanted)
            continue
    out.append(got)
output_path.write_text("".join(out))
PY
}

normalize_dvi() {
  python3 - "$1" "$2" <<'PY'
import pathlib
import sys

src = pathlib.Path(sys.argv[1])
dst = pathlib.Path(sys.argv[2])
data = bytearray(src.read_bytes())
if len(data) <= 14 or data[0] != 247:
    raise SystemExit(f"{src} is not a valid DVI preamble")
length = data[14]
start = 15
end = start + length
if end > len(data):
    raise SystemExit(f"{src} has a truncated DVI preamble comment")
replacement = b"umber trip normalized dvi banner"
for index in range(start, end):
    data[index] = replacement[index - start] if index - start < len(replacement) else 32
dst.write_bytes(data)
PY
}

reconcile_dvi_rounding() {
  local expected="$1" actual="$2" output="$3"
  python3 - "$expected" "$actual" "$output" <<'PY'
import pathlib
import sys

expected_path, actual_path, output_path = map(pathlib.Path, sys.argv[1:])
expected = expected_path.read_bytes()
actual = bytearray(actual_path.read_bytes())
if len(expected) != len(actual):
    output_path.write_bytes(actual)
    raise SystemExit(0)

def signed(data):
    return int.from_bytes(data, "big", signed=True)

i = 0
while i < len(expected):
    if expected[i] != actual[i]:
        output_path.write_bytes(actual)
        raise SystemExit(0)
    op = expected[i]
    i += 1
    fixed = {
        128:1,129:2,130:3,131:4,132:8,133:1,134:2,135:3,136:4,137:8,
        143:1,144:2,145:3,146:4,148:1,149:2,150:3,151:4,
        153:1,154:2,155:3,156:4,157:1,158:2,159:3,160:4,
        162:1,163:2,164:3,165:4,167:1,168:2,169:3,170:4,
        235:1,236:2,237:3,238:4,248:28,249:5,
    }
    if op in (239,240,241,242):
        width = op - 238
        size = int.from_bytes(expected[i:i+width], "big")
        count = width + size
    elif op in (243,244,245,246):
        width = op - 242
        if expected[i:i+width] != actual[i:i+width]:
            output_path.write_bytes(actual); raise SystemExit(0)
        a = i + width + 12
        if expected[i+width:i+width+12] != actual[i+width:i+width+12]:
            output_path.write_bytes(actual); raise SystemExit(0)
        count = width + 12 + expected[a] + expected[a+1]
    elif op == 247:
        count = 14 + expected[i+13]
    elif op == 139:
        count = 44
    else:
        count = fixed.get(op, 0)
    end = i + count
    if end > len(expected):
        output_path.write_bytes(actual); raise SystemExit(0)
    if expected[i:end] != actual[i:end]:
        # Knuth permits slight floating-point deviations only in horizontal
        # and vertical movement operands. Opcode and operand width stay exact.
        if op in range(143,171) and op not in (147,152,161,166):
            if abs(signed(expected[i:end]) - signed(actual[i:end])) <= 64:
                actual[i:end] = expected[i:end]
            else:
                output_path.write_bytes(actual); raise SystemExit(0)
        elif op == 247:
            # Preamble comment is already normalized before this reconciliation.
            output_path.write_bytes(actual); raise SystemExit(0)
        else:
            output_path.write_bytes(actual); raise SystemExit(0)
    i = end
output_path.write_bytes(actual)
PY
}

run_font_phase() {
  local pltotf
  local tftopl
  pltotf="$(tool_path UMBER_REF_PLTOTF pltotf)"
  tftopl="$(tool_path UMBER_REF_TFTOPL tftopl)"
  local dir="${work_root}/font"
  copy_trip_inputs "$dir"
  (
    cd "$dir"
    "$pltotf" trip.pl generated-trip.tfm
    "$tftopl" generated-trip.tfm generated-trip.pl
  )
  local ok=0
  compare_text "font-pl-roundtrip" "${download_dir}/trip.pl" "${dir}/generated-trip.pl" || ok=1
  compare_binary "font-tfm" "${download_dir}/trip.tfm" "${dir}/generated-trip.tfm" || ok=1
  return "$ok"
}

run_reference_phase() {
  local initex
  local dvitype
  initex="$(trip_initex)"
  dvitype="$(tool_path UMBER_REF_DVITYPE dvitype)"
  local init_dir="${work_root}/reference-initex"
  local trip_dir="${work_root}/reference-trip"
  copy_trip_inputs "$init_dir"
  copy_trip_inputs "$trip_dir"

  (
    cd "$init_dir"
    printf '\n\\input trip\n' | env TEXFONTS=".:${TEXFONTS:-}" "$initex" --progname=initex --ini -interaction=nonstopmode > tripin.fot 2>&1 || true
  )
  if [[ -f "${init_dir}/trip.fmt" ]]; then
    cp "${init_dir}/trip.fmt" "$trip_dir/"
  fi
  (
    cd "$trip_dir"
    printf ' &trip  trip \n' | env TEXFORMATS=".:${TEXFORMATS:-}" TEXFONTS=".:${TEXFONTS:-}" "$initex" --progname=tex -interaction=nonstopmode > trip.fot 2>&1 || true
    if [[ -f trip.dvi ]]; then
      "$dvitype" -output-level=2 -page-start='*.*.*.*.*.*.*.*.*.*' -max-pages=1000000 -dpi=72.27 trip.dvi > trip.typ
    fi
  )

  local norm="${work_root}/normalized"
  mkdir -p "$norm"
  normalize_trip_log "${download_dir}/tripin.log" "${norm}/expected-tripin.log"
  normalize_trip_log "${init_dir}/trip.log" "${norm}/actual-tripin.log" || true
  normalize_trip_log "${download_dir}/trip.log" "${norm}/expected-trip.log"
  normalize_trip_log "${trip_dir}/trip.log" "${norm}/actual-trip.log.raw" || true
  reconcile_trip_rounding "${norm}/expected-trip.log" "${norm}/actual-trip.log.raw" "${norm}/actual-trip.log" || true
  normalize_trip_fot "${download_dir}/trip.fot" "${norm}/expected-trip.fot"
  normalize_trip_fot "${trip_dir}/trip.fot" "${norm}/actual-trip.fot" || true
  normalize_trip_typ "${download_dir}/trip.typ" "${norm}/expected-trip.typ"
  normalize_trip_typ "${trip_dir}/trip.typ" "${norm}/actual-trip.typ.raw" || true
  reconcile_trip_typ_rounding "${norm}/expected-trip.typ" "${norm}/actual-trip.typ.raw" "${norm}/actual-trip.typ" || true
  normalize_dvi "${download_dir}/trip.dvi" "${norm}/expected-trip.dvi"
  if [[ -f "${trip_dir}/trip.dvi" ]]; then
    normalize_dvi "${trip_dir}/trip.dvi" "${norm}/actual-trip.dvi.raw"
    reconcile_dvi_rounding "${norm}/expected-trip.dvi" "${norm}/actual-trip.dvi.raw" "${norm}/actual-trip.dvi"
  fi

  local ok=0
  compare_text "reference-tripin-log" "${norm}/expected-tripin.log" "${norm}/actual-tripin.log" || ok=1
  compare_text "reference-trip-log" "${norm}/expected-trip.log" "${norm}/actual-trip.log" || ok=1
  compare_text "reference-trip-fot" "${norm}/expected-trip.fot" "${norm}/actual-trip.fot" || ok=1
  compare_text "reference-trip-typ" "${norm}/expected-trip.typ" "${norm}/actual-trip.typ" || ok=1
  compare_binary "reference-trip-dvi" "${norm}/expected-trip.dvi" "${norm}/actual-trip.dvi" || ok=1
  compare_text "reference-tripos" "${download_dir}/tripos.tex" "${trip_dir}/tripos.tex" || ok=1
  if [[ "$ok" -ne 0 ]]; then
    printf '%s\n' "Reference TRIP failed; inspect the artifact-specific diffs above. Rebuild pinned tools with scripts/build-trip-initex.sh if tool provenance is uncertain." >&2
  fi
  return "$ok"
}

run_umber_phase() {
  printf '%s\n' 'Building umber' >&2
  cargo build -p umber
  local dir="${work_root}/umber"
  copy_trip_inputs "$dir"
  (
    cd "$dir"
    "${umber_bin}" run trip.tex --show-fixtures --dvi trip.dvi > trip.log 2> trip.stderr || true
    if [[ -s trip.stderr ]]; then
      cat trip.stderr >&2
    fi
  )

  local norm="${work_root}/normalized"
  mkdir -p "$norm"
  normalize_trip_log "${download_dir}/tripin.log" "${norm}/expected-umber-tripin.log"
  normalize_trip_log "${dir}/trip.log" "${norm}/actual-umber-tripin.log" || true
  local ok=0
  compare_text "umber-tripin-log" "${norm}/expected-umber-tripin.log" "${norm}/actual-umber-tripin.log" || ok=1
  if [[ -f "${dir}/trip.dvi" ]]; then
    normalize_dvi "${download_dir}/trip.dvi" "${norm}/expected-umber-trip.dvi"
    normalize_dvi "${dir}/trip.dvi" "${norm}/actual-umber-trip.dvi"
    compare_binary "umber-trip-dvi" "${norm}/expected-umber-trip.dvi" "${norm}/actual-umber-trip.dvi" || ok=1
  else
    printf 'Umber did not produce trip.dvi\n' > "${diff_dir}/umber-trip-dvi.diff"
    printf 'FAIL umber-trip-dvi; see %s\n' "${diff_dir}/umber-trip-dvi.diff" >&2
    ok=1
  fi
  if [[ "$ok" -ne 0 ]]; then
    printf '%s\n' 'Umber TRIP failed. Current CLI has no official INITEX/format phase; file linked engine work rather than weakening this harness.' >&2
  fi
  return "$ok"
}

run_self_test() {
  prepare_work
  local dir="${work_root}/self-test"
  mkdir -p "$dir"
  printf 'alpha\nbeta\n' > "${dir}/expected.txt"
  cp "${dir}/expected.txt" "${dir}/actual.txt"
  compare_text "self-test-equal" "${dir}/expected.txt" "${dir}/actual.txt"
  printf 'alpha\nperturbed\n' > "${dir}/actual.txt"
  if compare_text "self-test-perturbation" "${dir}/expected.txt" "${dir}/actual.txt"; then
    fail "self-test perturbation unexpectedly passed"
  fi
  [[ -s "${diff_dir}/self-test-perturbation.diff" ]] || fail "self-test did not write an actionable diff"

  # Prove that the constrained DVI rounding reconciler cannot conceal a
  # character change. This synthetic stream is sufficient for the structural
  # walker and keeps self-test independent of fetched TRIP materials.
  python3 - "${dir}/expected.dvi" "${dir}/actual.dvi" <<'PY'
import pathlib
import sys

pre = bytes([247, 2]) + (25400000).to_bytes(4, "big") + (473628672).to_bytes(4, "big") + (1000).to_bytes(4, "big") + bytes([0])
bop = bytes([139]) + bytes(44)
tail = bytes([140, 248]) + bytes(28) + bytes([249]) + bytes(5) + bytes([223, 223, 223, 223])
expected = pre + bop + bytes([65]) + tail
pathlib.Path(sys.argv[1]).write_bytes(expected)
pathlib.Path(sys.argv[2]).write_bytes(pre + bop + bytes([66]) + tail)
PY
  reconcile_dvi_rounding "${dir}/expected.dvi" "${dir}/actual.dvi" "${dir}/reconciled.dvi"
  if compare_binary "self-test-dvi-character-perturbation" "${dir}/expected.dvi" "${dir}/reconciled.dvi"; then
    fail "DVI character perturbation unexpectedly passed constrained reconciliation"
  fi
  [[ -s "${diff_dir}/self-test-dvi-character-perturbation.diff" ]] || fail "DVI perturbation did not write byte context"
  printf 'self-test passed; perturbation diff is %s\n' "${diff_dir}/self-test-perturbation.diff" >&2
}

case "$mode" in
  self-test)
    run_self_test
    ;;
  fetch)
    fetch_materials
    ;;
  reference)
    fetch_materials
    prepare_work
    run_font_phase
    run_reference_phase
    ;;
  umber)
    fetch_materials
    prepare_work
    run_font_phase
    run_umber_phase
    ;;
  all)
    fetch_materials
    prepare_work
    ok=0
    run_font_phase || ok=1
    run_reference_phase || ok=1
    run_umber_phase || ok=1
    if [[ "$ok" -ne 0 ]]; then
      printf 'TRIP harness failed; diffs are under %s\n' "$diff_dir" >&2
      exit 1
    fi
    printf '%s\n' 'TRIP harness passed'
    ;;
esac
