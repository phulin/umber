#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/umber-r2-publish-test.XXXXXX")"
trap 'rm -rf "$tmp_root"' EXIT

fail() {
  printf 'test-publish-texlive-r2.sh: %s\n' "$*" >&2
  exit 1
}

bundle="$tmp_root/bundle"
mkdir -p "$bundle/objects" "$tmp_root/bin"
printf 'alpha' > "$bundle/objects/ahash64-v1-aaaaaaaaaaaaaaaa"
printf 'omega!' > "$bundle/objects/ahash64-v1-bbbbbbbbbbbbbbbb"
printf '{"schema":1}\n' > "$bundle/manifest.json"
manifest_ahash64="cccccccccccccccc"

env_file="$tmp_root/.env"
cat > "$env_file" <<'EOF'
R2_S3_ACCOUNT_ID=test-account
R2_S3_ACCESS_KEY_ID=test-access-key
R2_S3_SECRET_ACCESS_KEY=secret-must-not-leak
EOF

log="$tmp_root/rclone.log"
remote="$tmp_root/remote"
cat > "$tmp_root/bin/rclone" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$MOCK_RCLONE_LOG"
command="$1"
shift
case "$command" in
  copy)
    [[ " $* " != *' --dry-run '* ]] || exit 0
    if [[ "${MOCK_FAIL_COPY:-0}" == 1 ]]; then
      exit 19
    fi
    mkdir -p "$MOCK_REMOTE/objects"
    cp "$1"/* "$MOCK_REMOTE/objects/"
    ;;
  check)
    [[ -f "$MOCK_REMOTE/objects/ahash64-v1-aaaaaaaaaaaaaaaa" ]]
    ;;
  lsf)
    number=0
    while IFS= read -r object; do
      number=$((number + 1))
      printf '%s\tobject%s\n' "$(wc -c < "$object" | tr -d ' ')" "$number"
    done < <(find "$MOCK_REMOTE/objects" -type f)
    ;;
  copyto)
    cp "$1" "$MOCK_REMOTE/manifest.json"
    ;;
  *) exit 2 ;;
esac
EOF
chmod +x "$tmp_root/bin/rclone"

cat > "$tmp_root/bin/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
output=""
headers=""
url=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --output) output="$2"; shift 2 ;;
    --dump-header) headers="$2"; shift 2 ;;
    --header) shift 2 ;;
    --*) shift ;;
    *) url="$1"; shift ;;
  esac
done
printf 'HTTP/2 200\r\nAccess-Control-Allow-Origin: *\r\n\r\n' > "$headers"
if [[ "$url" == */manifest-v6*.json || "$url" == */manifest-v7.json ]]; then
  cp "$MOCK_REMOTE/manifest.json" "$output"
else
  cp "$MOCK_REMOTE/objects/${url##*/}" "$output"
fi
EOF
chmod +x "$tmp_root/bin/curl"

cat > "$tmp_root/bin/publisher" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "$1" == --verify-sharded ]]; then
  [[ -d "$2/objects" && -f "$2/manifest.json" ]]
elif [[ "$1" == --verify-successor ]]; then
  [[ -d "$2/objects" && -f "$2/manifest.json" && "$3" == --base-ahash64 && "$4" =~ ^[0-9a-f]{16}$ && -d "$5/objects" && -f "$5/manifest.json" ]]
elif [[ "$1" == --file-ahash64 ]]; then
  case "${2##*/}" in
    manifest.json) printf '%s\n' cccccccccccccccc ;;
    ahash64-v1-aaaaaaaaaaaaaaaa) printf '%s\n' aaaaaaaaaaaaaaaa ;;
    ahash64-v1-bbbbbbbbbbbbbbbb) printf '%s\n' bbbbbbbbbbbbbbbb ;;
    *) exit 2 ;;
  esac
else
  exit 2
fi
EOF
chmod +x "$tmp_root/bin/publisher"

export MOCK_RCLONE_LOG="$log"
export MOCK_REMOTE="$remote"
common=(
  --staging "$bundle"
  --snapshot texlive/test-snapshot
  --env-file "$env_file"
  --expected-objects 2
  --expected-bytes 11
  --expected-manifest-ahash64 "$manifest_ahash64"
  --transfers 3
  --checkers 4
  --retries 2
  --rclone "$tmp_root/bin/rclone"
  --curl "$tmp_root/bin/curl"
  --publisher "$tmp_root/bin/publisher"
)

dry_output="$tmp_root/dry-output"
"$repo_root/scripts/publish-texlive-r2.sh" "${common[@]}" --dry-run > "$dry_output" 2>&1
grep -q '^copy ' "$log" || fail "dry run did not plan an object copy"
grep -q -- '--dry-run' "$log" || fail "dry run did not reach rclone"
! grep -q '^copyto ' "$log" || fail "dry run attempted manifest publication"
! grep -q 'secret-must-not-leak' "$dry_output" || fail "dry run exposed a credential"

: > "$log"
"$repo_root/scripts/publish-texlive-r2.sh" "${common[@]}" \
  --env-file "$tmp_root/absent.env" --rclone-remote existing_r2 --dry-run \
  > "$tmp_root/configured-remote-output" 2>&1
grep -q 'existing_r2:umber-assets/texlive/test-snapshot/objects' "$log" || \
  fail "configured remote was not used"
! grep -q -- '--config /dev/null' "$log" || \
  fail "configured remote was isolated from its config file"

: > "$log"
if MOCK_FAIL_COPY=1 "$repo_root/scripts/publish-texlive-r2.sh" "${common[@]}" > "$tmp_root/fail-output" 2>&1; then
  fail "injected object upload failure unexpectedly succeeded"
fi
! grep -q '^copyto ' "$log" || fail "manifest was published after object failure"

# A rerun is the resume mechanism: copy checks existing objects and fills misses.
: > "$log"
"$repo_root/scripts/publish-texlive-r2.sh" "${common[@]}" > "$tmp_root/resume-output" 2>&1
grep -q '^check ' "$log" || fail "remote objects were not checked"
grep -q '^lsf ' "$log" || fail "remote inventory was not counted"
copyto_line="$(grep -n '^copyto ' "$log" | cut -d: -f1)"
check_line="$(grep -n '^check ' "$log" | cut -d: -f1)"
[[ "$copyto_line" -gt "$check_line" ]] || fail "manifest was not published after verification"
grep -q -- '--transfers 3' "$log" || fail "bounded transfer count was not forwarded"
grep -q -- '--checkers 4' "$log" || fail "bounded checker count was not forwarded"
grep -q -- '--retries 2' "$log" || fail "retry count was not forwarded"
grep -q -- '--immutable' "$log" || fail "immutable copy protection was not enabled"
grep -q -- '--s3-no-check-bucket' "$log" || fail "bucket creation checks were not disabled"
! grep -q 'secret-must-not-leak' "$log" || fail "rclone argv exposed a credential"
! grep -Eq '(^| )sync( |$)|delete' "$log" || fail "publication used a deleting operation"

: > "$log"
"$repo_root/scripts/publish-texlive-r2.sh" "${common[@]}" \
  --profile html --snapshot html/test-v1 > "$tmp_root/html-output" 2>&1
grep -q 'html/test-v1/manifest-v7.json' "$log" || fail "HTML profile did not use its distinct manifest key"

successor_base="$tmp_root/successor-base"
mkdir -p "$successor_base/objects"
cp "$bundle/manifest.json" "$successor_base/manifest.json"
printf 'remote-extra' > "$remote/objects/extra"
: > "$log"
"$repo_root/scripts/publish-texlive-r2.sh" "${common[@]}" \
  --successor-base "$successor_base" \
  --successor-base-ahash64 "$manifest_ahash64" \
  --manifest-name manifest-v6-latex-dev-test.json > "$tmp_root/successor-output" 2>&1
grep -q 'manifest-v6-latex-dev-test.json' "$log" || fail "successor did not use its unique root key"

if "$repo_root/scripts/publish-texlive-r2.sh" \
  --staging "$bundle" --profile html --snapshot html/unpinned \
  --expected-objects 2 --expected-bytes 11 \
  --rclone "$tmp_root/bin/rclone" --curl "$tmp_root/bin/curl" \
  --publisher "$tmp_root/bin/publisher" > "$tmp_root/unpinned-output" 2>&1; then
  fail "HTML publication without an explicit root pin unexpectedly succeeded"
fi
grep -q 'explicit --root-ahash64 pin' "$tmp_root/unpinned-output" || fail "missing HTML root pin was not diagnosed"

printf 'publish-texlive-r2 shell contract tests passed\n'
