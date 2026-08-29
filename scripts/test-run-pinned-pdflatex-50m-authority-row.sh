#!/usr/bin/env bash
set -euo pipefail

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
tmp_root=$(mktemp -d)
trap 'rm -rf "$tmp_root"' EXIT

mkdir -p "$tmp_root/source" "$tmp_root/distribution/objects"
printf '%s\n' '\\end' > "$tmp_root/source/main.tex"
printf '%s\n' format > "$tmp_root/pdflatex.fmt"
printf '%s\n' '{"schema":8}' > "$tmp_root/distribution/manifest-v8.json"
printf '%s\n' 'tex:first.sty' 'tfm:second.tfm' > "$tmp_root/prefetch.keys"

fake="$tmp_root/fake-umber"
capture="$tmp_root/argv.txt"
cat > "$fake" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$@" > "$FAKE_UMBER_ARGV"
exit 7
SH
chmod +x "$fake"

set +e
FAKE_UMBER_ARGV="$capture" \
  "$root/scripts/run-pinned-pdflatex-50m-authority-row.sh" \
  "$fake" "$tmp_root/source" main.tex "$tmp_root/pdflatex.fmt" \
  "$tmp_root/distribution" 0123456789abcdef "$tmp_root/prefetch.keys" \
  "$tmp_root/output" 1787080434
status=$?
set -e
if [[ $status -ne 7 ]]; then
  printf 'expected fake authority status 7, got %s\n' "$status" >&2
  exit 1
fi

for expected in \
  '--expansion-fuel' '50000000' \
  '--execution-steps' '100000000' \
  '--offline' \
  '--prefetch-input' 'tex:first.sty' 'tfm:second.tfm'; do
  if ! grep -Fqx -- "$expected" "$capture"; then
    printf 'missing exact authority argument %s\n' "$expected" >&2
    exit 1
  fi
done

receipt="$tmp_root/output/authority.receipt"
for expected in \
  'authority=umber-pdflatex-50m' \
  'expansion_fuel_cap=50000000' \
  'expansion_fuel_unit=canonical-command-fuel' \
  'execution_steps_cap=100000000' \
  'execution_steps_unit=committed-executor-steps' \
  'offline=1' \
  'prefetch_keys=2'; do
  if ! grep -Fqx -- "$expected" "$receipt"; then
    printf 'missing exact authority receipt field %s\n' "$expected" >&2
    exit 1
  fi
done
if [[ $(< "$tmp_root/output/status") != 7 ]]; then
  printf '%s\n' 'authority status receipt did not preserve the terminal status' >&2
  exit 1
fi

printf '%s\n' 'pinned pdflatex 50M authority row: PASS'
