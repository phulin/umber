#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
validator="${repo_root}/scripts/audit-etex26-extension-primitives.sh"
work_root="$(mktemp -d "${TMPDIR:-/tmp}/umber-etex26-audit-test.XXXXXX")"
trap 'rm -rf "$work_root"' EXIT

cat >"${work_root}/etex.ch" <<'EOF'
primitive("alpha",call,0);
primitive("beta",call,1);
EOF
cat >"${work_root}/matrix" <<'EOF'
alpha family|alpha boundary|fixture|seam|pattern|absent
EOF
cat >"${work_root}/audit" <<'EOF'
alpha|command-core|alpha boundary|delivery
beta|executor|focused beta test|execution
EOF

"$validator" "${work_root}/etex.ch" "${work_root}/audit" "${work_root}/matrix" \
  >/dev/null

expect_failure() {
  local expected="$1"
  if "$validator" "${work_root}/etex.ch" "${work_root}/audit" \
    "${work_root}/matrix" >"${work_root}/stdout" 2>"${work_root}/stderr"; then
    printf 'primitive audit unexpectedly accepted invalid fixture\n' >&2
    exit 1
  fi
  grep -Fq "$expected" "${work_root}/stderr" || {
    printf 'primitive audit did not report %q; stderr follows:\n' "$expected" >&2
    cat "${work_root}/stderr" >&2
    exit 1
  }
}

sed -i '/^beta|/d' "${work_root}/audit"
expect_failure 'audit is missing canonical primitives: beta'

cat >>"${work_root}/audit" <<'EOF'
beta|executor|focused beta test|execution
gamma|executor|focused gamma test|execution
EOF
expect_failure 'audit contains noncanonical primitives: gamma'

sed -i '/^gamma|/d; s/alpha boundary/missing boundary/' "${work_root}/audit"
expect_failure \
  'command-core primitive alpha has no extension matrix boundary: missing boundary'

printf '%s\n' 'e-TeX extension primitive audit tests passed'
