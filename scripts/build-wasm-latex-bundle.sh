#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
texmf_dist="${UMBER_TEXMF_DIST:-/usr/local/texlive/2025/texmf-dist}"
runtime_lock="${repo_root}/tests/latex-runtime.lock"
output_dir="${repo_root}/target/latex-wasm"
objects_base_url="https://example.invalid/umber/latex/objects/"

usage() {
  cat <<'EOF'
usage: scripts/build-wasm-latex-bundle.sh [--texmf-dist PATH] [--output-dir PATH] [--objects-base-url URL]

Builds the deterministic Umber-native LaTeX format, stages the exact pinned
base-corpus runtime closure, and publishes both through the content-addressed
WASM manifest. The URL must identify the deployment's immutable objects/
directory; the default is a non-deployable example URL for local verification.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --texmf-dist)
      [[ $# -ge 2 ]] || { printf '%s\n' 'missing path after --texmf-dist' >&2; exit 2; }
      texmf_dist="$2"
      shift 2
      ;;
    --output-dir)
      [[ $# -ge 2 ]] || { printf '%s\n' 'missing path after --output-dir' >&2; exit 2; }
      output_dir="$2"
      shift 2
      ;;
    --objects-base-url)
      [[ $# -ge 2 ]] || { printf '%s\n' 'missing URL after --objects-base-url' >&2; exit 2; }
      objects_base_url="$2"
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      printf 'build-wasm-latex-bundle.sh: unknown option: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

fail() {
  printf 'build-wasm-latex-bundle.sh: %s\n' "$*" >&2
  exit 1
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

[[ -d "$texmf_dist" ]] || fail "missing pinned texmf-dist root: $texmf_dist"
[[ -f "$runtime_lock" ]] || fail "missing runtime closure: $runtime_lock"
[[ "$objects_base_url" == */ ]] || fail "objects base URL must end with /"

tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/umber-latex-wasm.XXXXXX")"
cleanup() {
  local status=$?
  if [[ $status -eq 0 ]]; then
    rm -rf "$tmp_root"
  else
    printf 'build-wasm-latex-bundle.sh: failed artifacts: %s\n' "$tmp_root" >&2
  fi
}
trap cleanup EXIT

format_dir="${tmp_root}/format"
runtime_root="${tmp_root}/texmf-dist"
mkdir -p "$runtime_root"
"${repo_root}/scripts/build-latex-format.sh" --texmf-dist "$texmf_dist" --output-dir "$format_dir"

distribution="$(awk '$1 == "distribution" { print $2 }' "$runtime_lock")"
[[ -n "$distribution" ]] || fail "runtime closure is missing its distribution"
while read -r record kind relative expected_bytes expected_hash extra; do
  [[ -z "${record:-}" || "$record" == \#* ]] && continue
  [[ "$record" == source ]] || continue
  [[ -z "${extra:-}" ]] || fail "invalid runtime closure entry for $relative"
  [[ "$kind" == tex || "$kind" == tfm ]] || fail "invalid resource kind for $relative"
  [[ "$relative" != /* && "$relative" != *..* && "$relative" != *\\* ]] || \
    fail "unsafe runtime path: $relative"
  case "$kind:$relative" in
    tex:*.tex|tex:*.cls|tex:*.clo|tex:*.def|tex:*.sty|tfm:*.tfm) ;;
    *) fail "resource kind does not match $relative" ;;
  esac
  source="${texmf_dist}/${relative}"
  [[ -f "$source" ]] || fail "missing pinned runtime input: $source"
  actual_bytes="$(wc -c < "$source" | tr -d ' ')"
  [[ "$actual_bytes" == "$expected_bytes" ]] || \
    fail "length mismatch for $relative: expected $expected_bytes, got $actual_bytes"
  actual_hash="$(sha256 "$source")"
  [[ "$actual_hash" == "$expected_hash" ]] || \
    fail "hash mismatch for $relative: expected $expected_hash, got $actual_hash"
  destination="${runtime_root}/${relative}"
  mkdir -p "$(dirname "$destination")"
  cp "$source" "$destination"
done < "$runtime_lock"

cd "$repo_root"
cargo build --manifest-path tools/texlive-wasm-publish/Cargo.toml
publisher="${CARGO_TARGET_DIR:-${repo_root}/tools/texlive-wasm-publish/target}/debug/texlive-wasm-publish"
tree_hash="$($publisher --tree-sha256 "$runtime_root")"

config="${tmp_root}/publish.json"
cat > "$config" <<EOF
{
  "schema": 1,
  "distribution": "${distribution}",
  "objectsBaseUrl": "${objects_base_url}",
  "roots": [
    {
      "name": "latex-base-runtime",
      "path": "${runtime_root}",
      "treeSha256": "${tree_hash}"
    }
  ],
  "dependencies": {
    "tex:article.cls": ["tex:size10.clo", "tex:l3backend-dvips.def"],
    "tex:book.cls": ["tex:bk10.clo", "tex:l3backend-dvips.def"],
    "tex:letter.cls": ["tex:size10.clo", "tex:l3backend-dvips.def"],
    "tex:report.cls": ["tex:size10.clo", "tex:l3backend-dvips.def"]
  },
  "formats": [
    {
      "path": "${format_dir}/latex.fmt",
      "metadata": "${format_dir}/latex-format.json"
    }
  ]
}
EOF

first="${tmp_root}/first"
second="${tmp_root}/second"
"$publisher" "$config" "$first"
"$publisher" "$config" "$second"
diff -qr "$first" "$second" >/dev/null || fail "two clean publications differ"
"$publisher" "$config" "$output_dir"

printf 'Umber LaTeX WASM bundle: format=%s files=%s output=%s\n' \
  "$(sha256 "${format_dir}/latex.fmt")" \
  "$(find "${runtime_root}" -type f | wc -l | tr -d ' ')" \
  "$output_dir"
