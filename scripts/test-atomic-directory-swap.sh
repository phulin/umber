#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
swap="${repo_root}/scripts/atomic-directory-swap.sh"
tmp_root="$(mktemp -d)"
trap 'rm -rf "$tmp_root"' EXIT

make_tree() {
  local root="$1"
  mkdir -p "$root/live" "$root/candidate"
  printf 'live\n' >"$root/live/authority"
  printf 'candidate\n' >"$root/candidate/authority"
}

success_root="${tmp_root}/success"
make_tree "$success_root"
"$swap" "$success_root/live" "$success_root/candidate" "$success_root/backup"
grep -Fqx candidate "$success_root/live/authority"
grep -Fqx live "$success_root/backup/authority"
[[ ! -e "$success_root/candidate" ]]

mock_bin="${tmp_root}/mock-bin"
mkdir -p "$mock_bin"
real_mv="$(command -v mv)"
sed 's/^+//' >"$mock_bin/mv" <<'EOF'
+#!/usr/bin/env bash
+set -euo pipefail
+count_file="${MOCK_MV_COUNT_FILE:?}"
+count=0
+[[ ! -f "$count_file" ]] || read -r count <"$count_file"
+count=$((count + 1))
+printf '%s\n' "$count" >"$count_file"
+case "${MOCK_MV_FAIL_CALLS:-}" in
+  *",$count,"*)
+    printf 'injected mv failure %s\n' "$count" >&2
+    exit 71
+    ;;
+esac
+exec "${REAL_MV:?}" "$@"
EOF
chmod +x "$mock_bin/mv"

rollback_root="${tmp_root}/rollback"
make_tree "$rollback_root"
if rollback_output="$(
  MOCK_MV_COUNT_FILE="${tmp_root}/rollback-count" \
    MOCK_MV_FAIL_CALLS=",2," REAL_MV="$real_mv" PATH="${mock_bin}:$PATH" \
    "$swap" "$rollback_root/live" "$rollback_root/candidate" \
      "$rollback_root/backup" 2>&1
)"; then
  printf '%s\n' 'atomic directory swap unexpectedly succeeded' >&2
  exit 1
fi
grep -Fq 'candidate installation failed; restored live directory: injected mv failure 2' \
  <<<"$rollback_output"
grep -Fqx live "$rollback_root/live/authority"
grep -Fqx candidate "$rollback_root/candidate/authority"
[[ ! -e "$rollback_root/backup" ]]

failed_restore_root="${tmp_root}/failed-restore"
make_tree "$failed_restore_root"
if failed_restore_output="$(
  MOCK_MV_COUNT_FILE="${tmp_root}/failed-restore-count" \
    MOCK_MV_FAIL_CALLS=",2,3," REAL_MV="$real_mv" PATH="${mock_bin}:$PATH" \
    "$swap" "$failed_restore_root/live" "$failed_restore_root/candidate" \
      "$failed_restore_root/backup" 2>&1
)"; then
  printf '%s\n' 'atomic directory swap unexpectedly succeeded' >&2
  exit 1
fi
grep -Fq 'candidate installation failed: injected mv failure 2' \
  <<<"$failed_restore_output"
grep -Fq 'backup restoration also failed: injected mv failure 3' \
  <<<"$failed_restore_output"
grep -Fq "authoritative backup remains recoverable at: $failed_restore_root/backup" \
  <<<"$failed_restore_output"
grep -Fqx live "$failed_restore_root/backup/authority"
grep -Fqx candidate "$failed_restore_root/candidate/authority"
[[ ! -e "$failed_restore_root/live" ]]

printf '%s\n' 'atomic directory swap tests passed'
