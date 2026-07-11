#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
test_dir="$(mktemp -d "${TMPDIR:-/tmp}/sync-github-issues-test.XXXXXX")"
cleanup() {
  local status=$?
  if ((status != 0)) && [[ -f "${output:-}" ]]; then
    cat "$output" >&2
  fi
  rm -rf "$test_dir"
  exit "$status"
}
trap cleanup EXIT

mkdir -p "$test_dir/bin" "$test_dir/state"

cat >"$test_dir/bin/bd" <<'MOCK_BD'
#!/usr/bin/env bash
set -euo pipefail

if [[ "$1 $2" == "github sync" ]]; then
  count_file="$MOCK_STATE/bd-sync-count"
  count="$(cat "$count_file" 2>/dev/null || printf '0')"
  count=$((count + 1))
  printf '%s\n' "$count" >"$count_file"
  if ((count == 1)); then
    echo "mock transient bd sync failure" >&2
    exit 1
  fi
  exit 0
fi

if [[ "$1" == "list" ]]; then
  cat <<'JSON'
[
  {"id":"epic-1","title":"Test epic","type":"epic","status":"open","github_number":1},
  {"id":"child-1","title":"Test child","type":"task","status":"closed","parent_id":"epic-1","github_number":2}
]
JSON
  exit 0
fi

echo "unexpected bd invocation: $*" >&2
exit 1
MOCK_BD

cat >"$test_dir/bin/gh" <<'MOCK_GH'
#!/usr/bin/env bash
set -euo pipefail

if [[ "$1 $2" == "auth status" ]]; then
  exit 0
fi

if [[ "${MOCK_PERMANENT_PROJECT_ERROR:-0}" == "1" && "$1 $2" == "project list" ]]; then
  count_file="$MOCK_STATE/permanent-project-count"
  count="$(cat "$count_file" 2>/dev/null || printf '0')"
  printf '%s\n' "$((count + 1))" >"$count_file"
  echo "error: your authentication token is missing required scopes [read:project]" >&2
  exit 1
fi

key="$(printf '%s' "$*" | cksum | awk '{print $1}')"
count_file="$MOCK_STATE/gh-$key-count"
count="$(cat "$count_file" 2>/dev/null || printf '0')"
count=$((count + 1))
printf '%s\n' "$count" >"$count_file"
if ((count == 1)); then
  echo "PARTIAL-OUTPUT-THAT-MUST-BE-DISCARDED"
  echo "mock transient gh failure" >&2
  exit 1
fi

case "$1 $2" in
  "issue view")
    echo "OPEN"
    ;;
  "project list")
    echo '{"projects":[]}'
    ;;
  "project create")
    echo "7"
    ;;
esac
MOCK_GH

chmod +x "$test_dir/bin/bd" "$test_dir/bin/gh"

output="$test_dir/output"
PATH="$test_dir/bin:$PATH" \
  MOCK_STATE="$test_dir/state" \
  GITHUB_API_RETRY_ATTEMPTS=3 \
  GITHUB_API_RETRY_DELAY_SECONDS=0 \
  "$repo_root/scripts/sync-github-issues.sh" --repo owner/repo \
  >"$output" 2>&1

grep -q "warning: GitHub API command failed (attempt 1/3)" "$output"
grep -q "Status phase complete: changed 1, already aligned 1" "$output"
grep -q "GitHub issue status, epic labels, and epic projects are synced" "$output"
if grep -q "github=PARTIAL" "$output"; then
  echo "failed attempt stdout leaked into a successful result" >&2
  exit 1
fi
if [[ "$(cat "$test_dir/state/bd-sync-count")" != "2" ]]; then
  echo "bd github sync was not retried exactly once" >&2
  exit 1
fi

mkdir -p "$test_dir/permanent-state"
permanent_output="$test_dir/permanent-output"
if PATH="$test_dir/bin:$PATH" \
  MOCK_STATE="$test_dir/permanent-state" \
  MOCK_PERMANENT_PROJECT_ERROR=1 \
  GITHUB_API_RETRY_ATTEMPTS=3 \
  GITHUB_API_RETRY_DELAY_SECONDS=0 \
  "$repo_root/scripts/sync-github-issues.sh" --repo owner/repo \
  >"$permanent_output" 2>&1; then
  echo "permanent GitHub scope error unexpectedly succeeded" >&2
  exit 1
fi

grep -q "authentication token is missing required scopes \[read:project\]" "$permanent_output"
grep -q "GitHub API command failed permanently: project_number_for_title" "$permanent_output"
grep -q "gh auth refresh -s read:project -s project" "$permanent_output"
if grep -q "retrying.*project_number_for_title" "$permanent_output"; then
  echo "permanent GitHub scope error was retried" >&2
  exit 1
fi
if [[ "$(cat "$test_dir/permanent-state/permanent-project-count")" != "1" ]]; then
  echo "permanent GitHub scope error was not attempted exactly once" >&2
  exit 1
fi

echo "sync-github-issues retry test passed"
