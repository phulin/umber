#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/umber-r2-publish-test.XXXXXX")"
trap 'rm -rf "$tmp_root"' EXIT

fail() {
  printf 'test-publish-texlive-r2.sh: %s\n' "$*" >&2
  exit 1
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

bundle="$tmp_root/bundle"
mkdir -p "$bundle/objects" "$tmp_root/bin"
printf 'alpha' > "$bundle/objects/sha256-8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8"
printf 'omega!' > "$bundle/objects/sha256-780a5fdbf446d1be41aa2c6fb8e9be3f1d65ec3b42f3e1ae833867e34fb7e5e8"
printf '{"schema":1}\n' > "$bundle/manifest.json"
manifest_sha256="$(sha256 "$bundle/manifest.json")"

env_file="$tmp_root/.env"
cat > "$env_file" <<'EOF'
CLOUDFLARE_ACCOUNT_ID=test-account
R2_ACCESS_KEY_ID=test-access-key
R2_SECRET_ACCESS_KEY=secret-must-not-leak
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
    [[ -f "$MOCK_REMOTE/objects/sha256-8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8" ]]
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
if [[ "$url" == */manifest-v3.json ]]; then
  cp "$MOCK_REMOTE/manifest.json" "$output"
else
  cp "$MOCK_REMOTE/objects/${url##*/}" "$output"
fi
EOF
chmod +x "$tmp_root/bin/curl"

cat > "$tmp_root/bin/publisher" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ "$1" == --verify-sharded && -d "$2/objects" && -f "$2/manifest.json" ]]
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
  --expected-manifest-sha256 "$manifest_sha256"
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
! grep -q 'secret-must-not-leak' "$log" || fail "rclone argv exposed a credential"
! grep -Eq '(^| )sync( |$)|delete' "$log" || fail "publication used a deleting operation"

printf 'publish-texlive-r2 shell contract tests passed\n'
