#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp_root="$(mktemp -d)"
trap 'rm -rf "$tmp_root"' EXIT

mkdir -p "${tmp_root}/plain-root" "${tmp_root}/latex-root" "${tmp_root}/distribution"
distribution_sha256=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef

cat > "${tmp_root}/plain-builder" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'plain' > "${UMBER_MATRIX_TEST_ROOT}/plain.called"
printf '%s\n' "$@" > "${UMBER_MATRIX_TEST_ROOT}/plain.args"
EOF

cat > "${tmp_root}/latex-builder" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
engine=""
for ((index = 1; index <= $#; index++)); do
  if [[ "${!index}" == --engine ]]; then
    next=$((index + 1))
    engine="${!next}"
  fi
done
[[ -n "$engine" ]]
printf 'latex' > "${UMBER_MATRIX_TEST_ROOT}/${engine}.called"
printf '%s\n' "$@" > "${UMBER_MATRIX_TEST_ROOT}/${engine}.args"
EOF
chmod +x "${tmp_root}/plain-builder" "${tmp_root}/latex-builder"

UMBER_MATRIX_TEST_ROOT="$tmp_root" \
UMBER_PLAIN_FORMAT_BUILDER="${tmp_root}/plain-builder" \
UMBER_LATEX_FORMAT_BUILDER="${tmp_root}/latex-builder" \
  "${repo_root}/scripts/build-initex-format-matrix.sh" \
    --plain-texmf-dist "${tmp_root}/plain-root" \
    --latex-texmf-dist "${tmp_root}/latex-root" \
    --latex-distribution "${tmp_root}/distribution" \
    --latex-distribution-sha256 "$distribution_sha256" \
    --output-root "${tmp_root}/output"

[[ -f "${tmp_root}/plain.called" ]]
[[ -f "${tmp_root}/latex.called" ]]
[[ -f "${tmp_root}/pdflatex.called" ]]
cmp -s "${tmp_root}/plain.args" <(printf '%s\n' \
  --texmf-dist "${tmp_root}/plain-root" --check)
cmp -s "${tmp_root}/latex.args" <(printf '%s\n' \
  --engine latex --texmf-dist "${tmp_root}/latex-root" \
  --distribution "${tmp_root}/distribution" \
  --distribution-sha256 "$distribution_sha256" \
  --output-dir "${tmp_root}/output/latex" --force)
cmp -s "${tmp_root}/pdflatex.args" <(printf '%s\n' \
  --engine pdflatex --texmf-dist "${tmp_root}/latex-root" \
  --distribution "${tmp_root}/distribution" \
  --distribution-sha256 "$distribution_sha256" \
  --output-dir "${tmp_root}/output/pdflatex" --force)

printf '%s\n' 'build-initex-format-matrix tests: PASS'
