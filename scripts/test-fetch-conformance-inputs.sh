#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
script="$repo_root/scripts/fetch-conformance-inputs.sh"
tmp_root="$(mktemp -d)"
trap 'rm -rf "$tmp_root"' EXIT

mkdir -p "$tmp_root/repo/scripts" "$tmp_root/repo/tests" "$tmp_root/bin"
cp "$script" "$tmp_root/repo/scripts/"
printf 'font-or-hyphen\n' > "$tmp_root/system-input"

cat > "$tmp_root/bin/kpsewhich" <<EOF
#!/usr/bin/env bash
printf '%s\n' '$tmp_root/system-input'
EOF
chmod +x "$tmp_root/bin/kpsewhich"

cat > "$tmp_root/bin/curl" <<'EOF'
#!/usr/bin/env bash
output=
url=
while (( $# > 0 )); do
  case "$1" in
    -o) output="$2"; shift 2 ;;
    -*) shift ;;
    *) url="$1"; shift ;;
  esac
done
case "$url" in
  https://fail.example/*) exit 22 ;;
  https://wrong.example/*) printf 'wrong\n' > "$output" ;;
  https://good.example/*) printf 'expected\n' > "$output" ;;
  *) exit 23 ;;
esac
EOF
chmod +x "$tmp_root/bin/curl"

run_fetch() {
  (
    cd "$tmp_root/repo"
    PATH="$tmp_root/bin:$PATH" CURL="$tmp_root/bin/curl" \
      scripts/fetch-conformance-inputs.sh "$@"
  )
}

expected_hash="$(printf 'expected\n' | sha256sum | awk '{print $1}')"
wrong_hash="$(printf 'wrong\n' | sha256sum | awk '{print $1}')"

printf 'sample.tex %s https://fail.example/sample https://good.example/sample\n' \
  "$expected_hash" > "$tmp_root/repo/tests/trip-manifest.txt"
run_fetch >/dev/null 2>"$tmp_root/stderr"
cmp "$tmp_root/repo/third_party/trip/sample.tex" <(printf 'expected\n')

run_fetch --offline >/dev/null 2>"$tmp_root/stderr"

rm "$tmp_root/repo/third_party/trip/sample.tex"
printf 'sample.tex %s https://wrong.example/one https://wrong.example/two\n' \
  "$expected_hash" > "$tmp_root/repo/tests/trip-manifest.txt"
if run_fetch >"$tmp_root/stdout" 2>"$tmp_root/stderr"; then
  printf '%s\n' 'expected all-locator digest failure' >&2
  exit 1
fi
expected="fetch-conformance-inputs.sh: all 2 locators failed for sample.tex: https://wrong.example/one: SHA-256 mismatch (expected $expected_hash, got $wrong_hash); https://wrong.example/two: SHA-256 mismatch (expected $expected_hash, got $wrong_hash); not writing third_party/trip/sample.tex"
[[ "$(tail -n 1 "$tmp_root/stderr")" == "$expected" ]]
[[ ! -e "$tmp_root/repo/third_party/trip/sample.tex" ]]
[[ ! -e "$tmp_root/repo/third_party/trip/sample.tmp" ]]

for case_name in duplicate unsafe; do
  if [[ "$case_name" == duplicate ]]; then
    row="sample.tex $expected_hash https://good.example/sample https://good.example/sample"
    expected="fetch-conformance-inputs.sh: duplicate URL for sample.tex: https://good.example/sample"
  else
    row="sample.tex $expected_hash file:///tmp/sample"
    expected="fetch-conformance-inputs.sh: unsafe URL for sample.tex: file:///tmp/sample"
  fi
  printf '%s\n' "$row" > "$tmp_root/repo/tests/trip-manifest.txt"
  if run_fetch >"$tmp_root/stdout" 2>"$tmp_root/stderr"; then
    printf '%s\n' "expected manifest rejection: $row" >&2
    exit 1
  fi
  [[ "$(tail -n 1 "$tmp_root/stderr")" == "$expected" ]]
done

printf 'sample.tex %s https://good.example/sample\n' "$expected_hash" \
  > "$tmp_root/repo/tests/trip-manifest.txt"
if run_fetch --offline >"$tmp_root/stdout" 2>"$tmp_root/stderr"; then
  printf '%s\n' 'expected missing offline cache failure' >&2
  exit 1
fi
[[ "$(tail -n 1 "$tmp_root/stderr")" == \
  "fetch-conformance-inputs.sh: missing third_party/trip/sample.tex while running --offline" ]]

printf '%s\n' 'fetch-conformance-inputs fallback tests passed'
