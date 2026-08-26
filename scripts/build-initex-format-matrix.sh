#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
plain_builder="${UMBER_PLAIN_FORMAT_BUILDER:-${repo_root}/scripts/build-wasm-plain-format.sh}"
latex_builder="${UMBER_LATEX_FORMAT_BUILDER:-${repo_root}/scripts/build-latex-format.sh}"
plain_texmf_dist="${UMBER_PLAIN_TEXMF_DIST:-/usr/local/texlive/2025/texmf-dist}"
latex_texmf_dist="${UMBER_TEXMF_DIST:-${repo_root}/target/texlive-snapshot/texmf-dist}"
latex_distribution="${UMBER_LATEX_FORMAT_DISTRIBUTION:-${repo_root}/target/texlive-snapshot}"
latex_distribution_ahash64="${UMBER_LATEX_FORMAT_DISTRIBUTION_AHASH64:-$(awk '$1 == "distribution_ahash64" { print $2 }' "${repo_root}/tests/latex-source.lock")}"
output_root="${repo_root}/target/initex-format-matrix"

usage() {
  cat <<'EOF'
usage: scripts/build-initex-format-matrix.sh
       [--plain-texmf-dist PATH] [--latex-texmf-dist PATH]
       [--latex-distribution PATH] [--latex-distribution-ahash64 AHASH64]
       [--output-root PATH]

Builds and verifies the supported INITEX format matrix serially. Plain is
reproduced against its committed artifact; LaTeX and pdfLaTeX are each rebuilt
twice and checked for source-versus-loaded DVI/PDF equivalence. The delegated
builders apply the shared Umber process watchdog to every engine invocation.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --plain-texmf-dist)
      [[ $# -ge 2 ]] || { printf '%s\n' 'missing path after --plain-texmf-dist' >&2; exit 2; }
      plain_texmf_dist="$2"
      shift 2
      ;;
    --latex-texmf-dist)
      [[ $# -ge 2 ]] || { printf '%s\n' 'missing path after --latex-texmf-dist' >&2; exit 2; }
      latex_texmf_dist="$2"
      shift 2
      ;;
    --latex-distribution)
      [[ $# -ge 2 ]] || { printf '%s\n' 'missing path after --latex-distribution' >&2; exit 2; }
      latex_distribution="$2"
      shift 2
      ;;
    --latex-distribution-ahash64)
      [[ $# -ge 2 ]] || { printf '%s\n' 'missing digest after --latex-distribution-ahash64' >&2; exit 2; }
      latex_distribution_ahash64="$2"
      shift 2
      ;;
    --output-root)
      [[ $# -ge 2 ]] || { printf '%s\n' 'missing path after --output-root' >&2; exit 2; }
      output_root="$2"
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      printf 'build-initex-format-matrix.sh: unknown option: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

fail() {
  printf 'build-initex-format-matrix.sh: %s\n' "$*" >&2
  exit 1
}

[[ -x "$plain_builder" ]] || fail "missing Plain format builder: $plain_builder"
[[ -x "$latex_builder" ]] || fail "missing LaTeX format builder: $latex_builder"
[[ -d "$plain_texmf_dist" ]] || fail "missing Plain texmf-dist root: $plain_texmf_dist"
[[ -d "$latex_texmf_dist" ]] || fail "missing LaTeX texmf-dist root: $latex_texmf_dist"
mkdir -p "$output_root"

"$plain_builder" --texmf-dist "$plain_texmf_dist" --check
"$latex_builder" \
  --engine latex \
  --texmf-dist "$latex_texmf_dist" \
  --distribution "$latex_distribution" \
  --distribution-ahash64 "$latex_distribution_ahash64" \
  --output-dir "${output_root}/latex" \
  --force
"$latex_builder" \
  --engine pdflatex \
  --texmf-dist "$latex_texmf_dist" \
  --distribution "$latex_distribution" \
  --distribution-ahash64 "$latex_distribution_ahash64" \
  --output-dir "${output_root}/pdflatex" \
  --force

printf '%s\n' 'Umber INITEX format matrix: PASS (plain, latex, pdflatex)'
