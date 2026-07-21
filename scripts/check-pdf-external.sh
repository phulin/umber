#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

mode="local"
if (( $# > 1 )); then
  printf 'usage: scripts/check-pdf-external.sh [--local|--ci]\n' >&2
  exit 2
fi
case "${1:---local}" in
  --local) ;;
  --ci) mode="ci" ;;
  *)
    printf 'usage: scripts/check-pdf-external.sh [--local|--ci]\n' >&2
    exit 2
    ;;
esac

required_tool() {
  local variable="$1"
  local fallback="$2"
  local selected="${!variable:-}"
  if [[ -n "$selected" ]]; then
    if [[ ! -x "$selected" ]]; then
      printf 'PDF external gate: %s does not name an executable: %s\n' "$variable" "$selected" >&2
      exit 1
    fi
    printf '%s\n' "$selected"
    return
  fi
  if command -v "$fallback" >/dev/null 2>&1; then
    command -v "$fallback"
    return
  fi
  if [[ "$mode" == "ci" ]]; then
    printf 'PDF external gate: required tool %s is missing\n' "$fallback" >&2
    exit 1
  fi
  printf 'PDF external gate: SKIP %s checks (tool is missing; --local mode)\n' "$fallback" >&2
}

require_version() {
  local tool="$1"
  local argument="$2"
  local expected="$3"
  local output
  output="$("$tool" "$argument" 2>&1)" || {
    printf 'PDF external gate: could not query %s version\n' "$tool" >&2
    exit 1
  }
  if ! grep -Fq "$expected" <<<"$output"; then
    printf 'PDF external gate: %s must report %q; got %q\n' \
      "$tool" "$expected" "${output%%$'\n'*}" >&2
    exit 1
  fi
}

qpdf="$(required_tool UMBER_PDF_VALIDATOR qpdf)"
if [[ -n "$qpdf" ]]; then
  require_version "$qpdf" --version 'qpdf version 12.3.2'
  artifact_dir="$(mktemp -d)"
  trap 'rm -rf "$artifact_dir"' EXIT

  for test_name in \
    raster_png_ximage_is_reused_and_emitted_through_typed_xobjects \
    rgba_png_ximage_uses_a_typed_soft_mask \
    jpeg_bytes_are_preserved_behind_a_typed_dct_filter \
    object_compression_levels_one_through_three_emit_type_two_xrefs
  do
    UMBER_PDF_EXTERNAL_GATE_DIR="$artifact_dir" \
      cargo test -q -p umber --lib "pdf_output::tests::$test_name" -- --exact
  done

  committed_cases=(
    minimal_rule
    object_dictionaries
    external_pdf_page
    embedded_type1
    embedded_truetype
    pk_bitmap_300
    embedded_subset_omit
    embedded_tagged_spacing
    annotations_running
    form_xobjects
    navigation_structures
  )
  generated_cases=(
    object-compression-1
    object-compression-2
    object-compression-3
    raster-png
    raster-alpha
    dct-jpeg
  )
  for case_name in "${committed_cases[@]}"; do
    pdf="tests/corpus/pdf/${case_name}.expected.umber.pdf"
    [[ -f "$pdf" ]] || { printf 'PDF external gate: missing %s\n' "$pdf" >&2; exit 1; }
    "$qpdf" --check "$pdf"
  done
  for case_name in "${generated_cases[@]}"; do
    pdf="$artifact_dir/${case_name}.pdf"
    [[ -f "$pdf" ]] || { printf 'PDF external gate: missing %s\n' "$pdf" >&2; exit 1; }
    "$qpdf" --check "$pdf"
  done
  printf 'PDF external gate: qpdf structural matrix passed\n' >&2
fi

renderer="$(required_tool UMBER_PDF_RENDERER pdftoppm)"
extractor="$(required_tool UMBER_PDF_EXTRACTOR pdftotext)"
if [[ -n "$renderer" && -n "$extractor" ]]; then
  UMBER_PDF_RENDERER="$renderer" UMBER_PDF_EXTRACTOR="$extractor" \
    cargo run -q --manifest-path tools/fixturegen/Cargo.toml -- --check-pdf-raster
  printf 'PDF external gate: Poppler render and extraction matrix passed\n' >&2
elif [[ "$mode" == "ci" ]]; then
  printf 'PDF external gate: both Poppler tools are required in --ci mode\n' >&2
  exit 1
else
  printf 'PDF external gate: SKIP Poppler checks (incomplete tool pair; --local mode)\n' >&2
fi
