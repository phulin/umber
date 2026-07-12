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
verifies SHA-256 hashes, rebuilds trip.tfm via PLtoTF/TFtoPL, runs the official
two-phase workload, and compares the resulting DVI and DVItype output. Text
transcripts remain in target/trip/ for diagnostics but do not gate this tier.

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

phase_failure() {
  local label="$1"
  local message="$2"
  local diff_path="${diff_dir}/${label}.diff"
  mkdir -p "$diff_dir"
  printf '%s\n' "$message" > "$diff_path"
  printf 'FAIL %s: %s; see %s\n' "$label" "$message" "$diff_path" >&2
  return 1
}

run_required_artifact_command() {
  local label="$1"
  local artifact="$2"
  local status_policy="$3"
  shift 3

  # --keep-work retains transcripts and diffs for diagnosis, but a producer
  # must never inherit the artifact that proves its current invocation worked.
  rm -f "$artifact"
  local status
  if "$@"; then
    status=0
  else
    status=$?
  fi

  case "$status_policy" in
    trip-errors)
      # Appendix A deliberately drives both engines through errors; TeX82 and
      # Umber therefore return 1 on successful TRIP execution.
      if [[ "$status" -ne 0 && "$status" -ne 1 ]]; then
        phase_failure "$label" "producer exited with status ${status} (expected 0 or 1 for the intentional TRIP errors)"
        return 1
      fi
      ;;
    strict)
      if [[ "$status" -ne 0 ]]; then
        phase_failure "$label" "producer exited with status ${status}"
        return 1
      fi
      ;;
    *)
      fail "internal error: unknown status policy ${status_policy}"
      ;;
  esac

  if [[ ! -s "$artifact" ]]; then
    phase_failure "$label" "current invocation did not produce a nonempty ${artifact}"
    return 1
  fi
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

compare_dvi() {
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
  python3 - "$label" "$expected" "$actual" "$diff_path" <<'PY'
import pathlib
import sys

label = sys.argv[1]
expected_path, actual_path, diff_path = map(pathlib.Path, sys.argv[2:])
expected = expected_path.read_bytes()
actual = actual_path.read_bytes()
common = min(len(expected), len(actual))
offset = next((i for i in range(common) if expected[i] != actual[i]), common)

def command_length(data, start):
    op = data[start]
    fixed = {
        128: 2, 129: 3, 130: 4, 131: 5, 132: 9,
        133: 2, 134: 3, 135: 4, 136: 5, 137: 9,
        139: 45,
        143: 2, 144: 3, 145: 4, 146: 5,
        148: 2, 149: 3, 150: 4, 151: 5,
        153: 2, 154: 3, 155: 4, 156: 5,
        157: 2, 158: 3, 159: 4, 160: 5,
        162: 2, 163: 3, 164: 4, 165: 5,
        167: 2, 168: 3, 169: 4, 170: 5,
        235: 2, 236: 3, 237: 4, 238: 5,
        248: 29, 249: 6,
    }
    if op in range(239, 243):
        width = op - 238
        end = start + 1 + width
        if end > len(data):
            return len(data) - start
        return 1 + width + int.from_bytes(data[start + 1:end], "big")
    if op in range(243, 247):
        width = op - 242
        lengths = start + 1 + width + 12
        if lengths + 2 > len(data):
            return len(data) - start
        return 1 + width + 12 + data[lengths] + data[lengths + 1]
    if op == 247:
        if start + 15 > len(data):
            return len(data) - start
        return 15 + data[start + 14]
    return fixed.get(op, 1)

def opcode_name(op):
    if op is None:
        return "EOF"
    if op <= 127:
        return f"set_char_{op}"
    names = {
        128: "set1", 129: "set2", 130: "set3", 131: "set4",
        132: "set_rule", 133: "put1", 134: "put2", 135: "put3",
        136: "put4", 137: "put_rule", 138: "nop", 139: "bop",
        140: "eop", 141: "push", 142: "pop", 147: "w0", 152: "x0",
        161: "y0", 166: "z0", 239: "xxx1", 240: "xxx2",
        241: "xxx3", 242: "xxx4", 247: "pre", 248: "post",
        249: "post_post",
    }
    if op in names:
        return names[op]
    ranges = (
        (143, 146, "right"), (148, 151, "w"), (153, 156, "x"),
        (157, 160, "down"), (162, 165, "y"), (167, 170, "z"),
        (171, 234, "fnt_num_"), (235, 238, "fnt"),
        (243, 246, "fnt_def"),
    )
    for lo, hi, name in ranges:
        if lo <= op <= hi:
            suffix = op - lo if lo == 171 else op - lo + 1
            return f"{name}{suffix}"
    return f"undefined_{op}"

def context(data, target):
    start = 0
    page = 0
    command = None
    while start < len(data) and start <= target:
        op = data[start]
        if op == 139:
            page += 1
        command = (start, page, op)
        length = command_length(data, start)
        if length <= 0:
            break
        start += length
    return command

def byte(data, at):
    return "EOF" if at >= len(data) else f"{data[at]} (0x{data[at]:02x})"

expected_context = context(expected, offset)
actual_context = context(actual, offset)
start = max(0, offset - 8)
end = offset + 9
lines = [
    f"{label} differs",
    f"expected: {expected_path}",
    f"actual:   {actual_path}",
    f"first_divergent_byte_offset: {offset}",
    f"expected_byte: {byte(expected, offset)}",
    f"actual_byte: {byte(actual, offset)}",
]
for prefix, item in (("expected", expected_context), ("actual", actual_context)):
    if item is None:
        lines.append(f"{prefix}_page: none")
        lines.append(f"{prefix}_opcode: EOF")
    else:
        command_offset, page, op = item
        lines.append(f"{prefix}_page: {page if page else 'outside-page'}")
        lines.append(f"{prefix}_opcode: {opcode_name(op)} at byte {command_offset}")
lines.extend((
    f"expected_context_hex[{start}:{min(end, len(expected))}]: {expected[start:end].hex(' ')}",
    f"actual_context_hex[{start}:{min(end, len(actual))}]: {actual[start:end].hex(' ')}",
))
diff_path.write_text("\n".join(lines) + "\n")
PY
  printf 'FAIL %s; see %s\n' "$label" "$diff_path" >&2
  return 1
}

normalize_trip_typ() {
  sed -E \
    -e '1s/ \(.*\)$//' \
    -e "s/^'.*'$/' <TRIP-DVI-COMMENT>'/" \
    "$1" > "$2"
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

invoke_pltotf() {
  local dir="$1"
  local pltotf="$2"
  (cd "$dir" && "$pltotf" trip.pl generated-trip.tfm)
}

invoke_tftopl() {
  local dir="$1"
  local tftopl="$2"
  (cd "$dir" && "$tftopl" generated-trip.tfm generated-trip.pl)
}

invoke_reference_initex() {
  local dir="$1"
  local initex="$2"
  (
    cd "$dir"
    printf '\n\\input trip\n' | env TEXFONTS=".:${TEXFONTS:-}" "$initex" --progname=initex --ini -interaction=nonstopmode > tripin.fot 2>&1
  )
}

invoke_reference_trip() {
  local dir="$1"
  local initex="$2"
  (
    cd "$dir"
    printf ' &trip  trip \n' | env TEXFORMATS=".:${TEXFORMATS:-}" TEXFONTS=".:${TEXFONTS:-}" "$initex" --progname=tex -interaction=nonstopmode > trip.fot 2>&1
  )
}

invoke_dvitype() {
  local dir="$1"
  local dvitype="$2"
  (
    cd "$dir"
    "$dvitype" -output-level=2 -page-start='*.*.*.*.*.*.*.*.*.*' -max-pages=1000000 -dpi=72.27 trip.dvi > trip.typ
  )
}

invoke_umber_format() {
  local dir="$1"
  local format="$2"
  local stderr="$3"
  (cd "$dir" && "${umber_bin}" run trip.tex --format-out "$format" > /dev/null 2> "$stderr")
}

invoke_umber_dvi() {
  local dir="$1"
  (
    cd "$dir"
    "${umber_bin}" run trip.tex --format trip.fmt --dvi trip.dvi > /dev/null 2> trip-artifact.stderr
  )
}

run_font_phase() {
  local pltotf
  local tftopl
  pltotf="$(tool_path UMBER_REF_PLTOTF pltotf)"
  tftopl="$(tool_path UMBER_REF_TFTOPL tftopl)"
  local dir="${work_root}/font"
  copy_trip_inputs "$dir"
  local ok=0
  run_required_artifact_command "font-pltotf" "${dir}/generated-trip.tfm" strict invoke_pltotf "$dir" "$pltotf" || ok=1
  if [[ "$ok" -eq 0 ]]; then
    run_required_artifact_command "font-tftopl" "${dir}/generated-trip.pl" strict invoke_tftopl "$dir" "$tftopl" || ok=1
  else
    rm -f "${dir}/generated-trip.pl"
  fi
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

  rm -f "${init_dir}/trip.log" "${init_dir}/tripin.fot" \
    "${init_dir}/trip.dvi" "${init_dir}/trip.typ" \
    "${init_dir}/tripos.tex" "${init_dir}/8terminal.tex"
  local ok=0
  run_required_artifact_command "reference-initex-format" "${init_dir}/trip.fmt" trip-errors invoke_reference_initex "$init_dir" "$initex" || ok=1
  rm -f "${trip_dir}/trip.fmt" "${trip_dir}/trip.log" "${trip_dir}/trip.fot" \
    "${trip_dir}/trip.dvi" "${trip_dir}/trip.typ" "${trip_dir}/tripos.tex" \
    "${trip_dir}/8terminal.tex"
  if [[ "$ok" -eq 0 ]]; then
    cp "${init_dir}/trip.fmt" "$trip_dir/"
    run_required_artifact_command "reference-format-loaded-dvi" "${trip_dir}/trip.dvi" trip-errors invoke_reference_trip "$trip_dir" "$initex" || ok=1
  fi
  if [[ -s "${trip_dir}/trip.dvi" ]]; then
    run_required_artifact_command "reference-dvitype" "${trip_dir}/trip.typ" strict invoke_dvitype "$trip_dir" "$dvitype" || ok=1
  else
    rm -f "${trip_dir}/trip.typ"
  fi

  local norm="${work_root}/normalized"
  mkdir -p "$norm"
  rm -f "${norm}/actual-trip.typ.raw" "${norm}/actual-trip.typ" \
    "${norm}/actual-trip.dvi.raw" "${norm}/actual-trip.dvi"
  normalize_trip_typ "${download_dir}/trip.typ" "${norm}/expected-trip.typ"
  if [[ -s "${trip_dir}/trip.typ" ]]; then
    normalize_trip_typ "${trip_dir}/trip.typ" "${norm}/actual-trip.typ.raw"
    reconcile_trip_typ_rounding "${norm}/expected-trip.typ" "${norm}/actual-trip.typ.raw" "${norm}/actual-trip.typ"
  fi
  normalize_dvi "${download_dir}/trip.dvi" "${norm}/expected-trip.dvi"
  if [[ -s "${trip_dir}/trip.dvi" ]]; then
    normalize_dvi "${trip_dir}/trip.dvi" "${norm}/actual-trip.dvi.raw"
    reconcile_dvi_rounding "${norm}/expected-trip.dvi" "${norm}/actual-trip.dvi.raw" "${norm}/actual-trip.dvi"
  fi

  compare_text "reference-trip-typ" "${norm}/expected-trip.typ" "${norm}/actual-trip.typ" || ok=1
  compare_dvi "reference-trip-dvi" "${norm}/expected-trip.dvi" "${norm}/actual-trip.dvi" || ok=1
  if [[ "$ok" -ne 0 ]]; then
    printf '%s\n' "Reference TRIP failed; inspect the artifact-specific diffs above. Rebuild pinned tools with scripts/build-trip-initex.sh if tool provenance is uncertain." >&2
  fi
  return "$ok"
}

run_umber_phase() {
  local dvitype
  dvitype="$(tool_path UMBER_REF_DVITYPE dvitype)"
  printf '%s\n' 'Building umber' >&2
  cargo build -p umber
  local dir="${work_root}/umber"
  copy_trip_inputs "$dir"
  rm -f "${dir}/fixture-trip.fmt" "${dir}/trip.fmt" "${dir}/trip.dvi" \
    "${dir}/trip.typ" "${dir}/tripos.tex" "${dir}/8terminal.tex" \
    "${dir}/tripin.log" "${dir}/tripin.stderr" \
    "${dir}/tripin-artifact.stderr" "${dir}/trip.log" \
    "${dir}/trip.stderr" "${dir}/trip-artifact.stderr"
  local ok=0
  (
    cd "$dir"
    "${umber_bin}" run trip.tex --show-fixtures --format-out fixture-trip.fmt > tripin.log 2> tripin.stderr || true
    if [[ -s tripin.stderr ]]; then
      cat tripin.stderr >&2
    fi
  )
  run_required_artifact_command "umber-initex-format" "${dir}/trip.fmt" trip-errors invoke_umber_format "$dir" trip.fmt tripin-artifact.stderr || ok=1
  if [[ -s "${dir}/tripin-artifact.stderr" ]]; then
    cat "${dir}/tripin-artifact.stderr" >&2
  fi
  if [[ "$ok" -eq 0 ]]; then
    (
      cd "$dir"
      rm -f trip.log trip.stderr tripos.tex 8terminal.tex trip.dvi trip.typ
      "${umber_bin}" run trip.tex --format trip.fmt --show-fixtures > trip.log 2> trip.stderr || true
      if [[ -s trip.stderr ]]; then
        cat trip.stderr >&2
      fi
      rm -f trip.log tripos.tex 8terminal.tex trip.dvi trip.typ trip-artifact.stderr
    )
    run_required_artifact_command "umber-format-loaded-dvi" "${dir}/trip.dvi" trip-errors invoke_umber_dvi "$dir" || ok=1
  fi
  if [[ -s "${dir}/trip-artifact.stderr" ]]; then
    cat "${dir}/trip-artifact.stderr" >&2
  fi
  if [[ -s "${dir}/trip.dvi" ]]; then
    run_required_artifact_command "umber-dvitype" "${dir}/trip.typ" strict invoke_dvitype "$dir" "$dvitype" || ok=1
  else
    rm -f "${dir}/trip.typ"
  fi

  local norm="${work_root}/normalized"
  mkdir -p "$norm"
  rm -f "${norm}/actual-umber-trip.typ.raw" "${norm}/actual-umber-trip.typ" \
    "${norm}/actual-umber-trip.dvi.raw" "${norm}/actual-umber-trip.dvi"
  if [[ -s "${dir}/trip.dvi" && -s "${dir}/trip.typ" ]]; then
    normalize_trip_typ "${download_dir}/trip.typ" "${norm}/expected-umber-trip.typ"
    normalize_trip_typ "${dir}/trip.typ" "${norm}/actual-umber-trip.typ.raw" || true
    reconcile_trip_typ_rounding "${norm}/expected-umber-trip.typ" "${norm}/actual-umber-trip.typ.raw" "${norm}/actual-umber-trip.typ" || true
    normalize_dvi "${download_dir}/trip.dvi" "${norm}/expected-umber-trip.dvi"
    normalize_dvi "${dir}/trip.dvi" "${norm}/actual-umber-trip.dvi.raw"
    reconcile_dvi_rounding "${norm}/expected-umber-trip.dvi" "${norm}/actual-umber-trip.dvi.raw" "${norm}/actual-umber-trip.dvi"
    compare_text "umber-trip-typ" "${norm}/expected-umber-trip.typ" "${norm}/actual-umber-trip.typ" || ok=1
    compare_dvi "umber-trip-dvi" "${norm}/expected-umber-trip.dvi" "${norm}/actual-umber-trip.dvi" || ok=1
  else
    printf 'Umber did not produce trip.dvi\n' > "${diff_dir}/umber-trip-dvi.diff"
    printf 'FAIL umber-trip-dvi; see %s\n' "${diff_dir}/umber-trip-dvi.diff" >&2
    ok=1
  fi
  if [[ "$ok" -ne 0 ]]; then
    printf '%s\n' 'Umber TRIP DVI parity failed; compare the opcode context against tex.web ship_out and movement semantics rather than weakening this harness.' >&2
  fi
  return "$ok"
}

run_self_test() {
  prepare_work
  local dir="${work_root}/self-test"
  mkdir -p "$dir"
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
  if compare_dvi "self-test-dvi-character-perturbation" "${dir}/expected.dvi" "${dir}/reconciled.dvi"; then
    fail "DVI character perturbation unexpectedly passed constrained reconciliation"
  fi
  local dvi_diff="${diff_dir}/self-test-dvi-character-perturbation.diff"
  [[ -s "$dvi_diff" ]] || fail "DVI perturbation did not write byte context"
  grep -q '^first_divergent_byte_offset: ' "$dvi_diff" || fail "DVI perturbation did not identify the divergent byte"
  grep -q '^expected_page: 1$' "$dvi_diff" || fail "DVI perturbation did not identify the divergent page"
  grep -q '^expected_opcode: set_char_65 ' "$dvi_diff" || fail "DVI perturbation did not identify the expected opcode"
  grep -q '^actual_opcode: set_char_66 ' "$dvi_diff" || fail "DVI perturbation did not identify the actual opcode"

  # Retained diagnostic work must not be mistaken for output from the current
  # Appendix A producers. Exercise both the intentional-error engine policy
  # and a strict tool status with plausible stale artifacts in place.
  local stale_dir="${dir}/stale-artifacts"
  mkdir -p "$stale_dir"
  printf 'plausible stale format\n' > "${stale_dir}/trip.fmt"
  printf 'plausible stale DVI\n' > "${stale_dir}/trip.dvi"
  printf 'plausible stale DVItype output\n' > "${stale_dir}/trip.typ"
  if run_required_artifact_command "self-test-stale-engine-format" "${stale_dir}/trip.fmt" trip-errors /usr/bin/false; then
    fail "failing engine unexpectedly reused a retained format artifact"
  fi
  [[ ! -e "${stale_dir}/trip.fmt" ]] || fail "stale format artifact survived producer staging"
  local engine_diff="${diff_dir}/self-test-stale-engine-format.diff"
  grep -q 'current invocation did not produce a nonempty' "$engine_diff" || fail "stale engine failure was not actionable"

  if run_required_artifact_command "self-test-stale-engine-dvi" "${stale_dir}/trip.dvi" trip-errors /usr/bin/false; then
    fail "failing format-loaded engine unexpectedly reused a retained DVI artifact"
  fi
  [[ ! -e "${stale_dir}/trip.dvi" ]] || fail "stale DVI artifact survived producer staging"
  local dvi_engine_diff="${diff_dir}/self-test-stale-engine-dvi.diff"
  grep -q 'current invocation did not produce a nonempty' "$dvi_engine_diff" || fail "stale DVI producer failure was not actionable"

  if run_required_artifact_command "self-test-stale-dvitype" "${stale_dir}/trip.typ" strict /usr/bin/false; then
    fail "failing DVItype unexpectedly reused retained output"
  fi
  [[ ! -e "${stale_dir}/trip.typ" ]] || fail "stale DVItype artifact survived producer staging"
  local tool_diff="${diff_dir}/self-test-stale-dvitype.diff"
  grep -q 'producer exited with status 1' "$tool_diff" || fail "DVItype exit failure was not actionable"
  printf 'self-test passed; actionable DVI perturbation diff is %s\n' "$dvi_diff" >&2
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
