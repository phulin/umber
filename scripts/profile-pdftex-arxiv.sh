#!/usr/bin/env bash
set -euo pipefail

readonly ROOT="${PDFTEX_PROFILE_ROOT:-/tmp/umber-pdftex-primitive-trace}"
readonly SOURCE="$ROOT/source"
readonly BUILD="$ROOT/build"
readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly PATCH="$SCRIPT_DIR/pdftex-primitive-trace.patch"
# Uniform reservoir sample from 3,100,507 metadata records, seed 0x554D424552.
# The first 100 source bundles containing a LaTeX entrypoint were retained.
readonly SAMPLE="${PDFTEX_PROFILE_SAMPLE:-$SCRIPT_DIR/pdftex-arxiv-sample-100.tsv}"
readonly UPSTREAM=https://github.com/TeX-Live/texlive-source.git
readonly REVISION=1664cf0ab3f6ce3b80db649bc6723f54ab12016c
readonly PDFTEX="$BUILD/texk/web2c/pdftex"
readonly FORMAT="$ROOT/formats/pdflatex.fmt"

require() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

tex_env() {
  local texmfroot texmflocal texmfsysvar texmfsysconfig
  texmfroot="$(kpsewhich -var-value=TEXMFROOT)"
  texmflocal="$(kpsewhich -var-value=TEXMFLOCAL)"
  texmfsysvar="$(kpsewhich -var-value=TEXMFSYSVAR)"
  texmfsysconfig="$(kpsewhich -var-value=TEXMFSYSCONFIG)"
  env \
    TEXMFCNF="$texmfroot/texmf-dist/web2c" \
    TEXMFROOT="$texmfroot" \
    TEXMFLOCAL="$texmflocal" \
    TEXMFSYSVAR="$texmfsysvar" \
    TEXMFSYSCONFIG="$texmfsysconfig" \
    "$@"
}

setup() {
  mkdir -p "$ROOT"
  if [[ ! -d "$SOURCE/.git" ]]; then
    git init "$SOURCE"
    git -C "$SOURCE" fetch --depth 1 "$UPSTREAM" "$REVISION"
    git -C "$SOURCE" checkout --detach FETCH_HEAD
  fi
  if [[ -n "$(git -C "$SOURCE" status --porcelain)" ]]; then
    echo "refusing to replace changes in $SOURCE" >&2
    exit 1
  fi
  git -C "$SOURCE" checkout --detach "$REVISION"
  if ! git -C "$SOURCE" apply --check "$PATCH" >/dev/null 2>&1; then
    echo "trace patch does not apply cleanly to $REVISION" >&2
    exit 1
  fi
  git -C "$SOURCE" apply "$PATCH"

  mkdir -p "$BUILD"
  (
    cd "$BUILD"
    ../source/configure \
      --without-x \
      --disable-shared \
      --disable-all-pkgs \
      --enable-pdftex \
      --disable-synctex \
      --disable-xetex \
      --enable-missing \
      -C CFLAGS=-O2 CXXFLAGS=-O2
    make -j"$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)"
    make -C texk/web2c pdftex
  )

  mkdir -p "$ROOT/formats"
  (
    cd "$ROOT/formats"
    tex_env "$PDFTEX" \
      -ini -etex \
      -jobname=pdflatex \
      -progname=pdflatex \
      -translate-file=cp227.tcx \
      '*pdflatex.ini' >pdflatex-init.stdout 2>&1
  )
  "$PDFTEX" --version | head -1
}

unpack_source() {
  local archive=$1 destination=$2
  mkdir -p "$destination"
  if tar tzf "$archive" >/dev/null 2>&1; then
    tar xzf "$archive" -C "$destination"
  elif gzip -t "$archive" >/dev/null 2>&1; then
    gzip -dc "$archive" >"$destination/main.tex"
  else
    echo "arXiv returned a non-source payload: $archive" >&2
    return 1
  fi
}

entrypoint() {
  local directory=$1 candidate
  for candidate in main.tex manuscript.tex arxiv_version.tex paper.tex; do
    if [[ -f "$directory/$candidate" ]] && rg -q -F '\documentclass' "$directory/$candidate"; then
      printf '%s\n' "$directory/$candidate"
      return
    fi
  done
  rg -l -F '\documentclass' "$directory" -g '*.tex' \
    | rg -v '/(supp|supplement|appendix)[^/]*\.tex$' \
    | sort \
    | head -1
}

process_sample() {
    local id=$1 category=$2
    local key=${id//\//_}
    local archive="$ROOT/samples/$key.src"
    local directory="$ROOT/samples/$key"
    local result="$ROOT/results/$key"
    if [[ ! -f "$archive" ]]; then
      curl -L --fail --show-error --silent --retry 3 \
        -o "$archive" "https://export.arxiv.org/e-print/$id"
    fi
    if [[ ! -d "$directory" ]]; then
      unpack_source "$archive" "$directory"
    fi
    local main
    main="$(entrypoint "$directory")"
    [[ -n "$main" ]] || {
      echo "no TeX entrypoint found for $id" >&2
      return
    }
    mkdir -p "$result"
    set +e
    (
      cd "$(dirname "$main")"
      tex_env "$PDFTEX" \
        --progname=pdflatex \
        -fmt="$FORMAT" \
        -interaction=nonstopmode \
        -halt-on-error \
        "$(basename "$main")"
    ) >"$result/pdftex.stdout" 2>&1
    local rc=$?
    set -e
    rg '^PDFTEX_PRIMITIVE_USED \\' "$result/pdftex.stdout" \
      | sed 's/^PDFTEX_PRIMITIVE_USED //' \
      | sort -u >"$result/primitives.txt" || true
    local count
    count="$(wc -l <"$result/primitives.txt" | tr -d ' ')"
    printf '%s\t%s\t%s\t%s\t%s\n' \
      "$id" "$category" "${main#"$directory/"}" "$rc" "$count" \
      | tee -a "$ROOT/results/summary.tsv"
}

smoke() {
  [[ -x "$PDFTEX" && -f "$FORMAT" ]] || {
    echo "run '$0 setup' first" >&2
    exit 1
  }
  [[ -f "$SAMPLE" ]] || {
    echo "sample manifest not found: $SAMPLE" >&2
    exit 1
  }
  mkdir -p "$ROOT/samples" "$ROOT/results"
  printf 'id\tcategory\tentrypoint\texit\tprimitive_count\n' >"$ROOT/results/summary.tsv"

  local jobs=${PDFTEX_PROFILE_JOBS:-8}
  while IFS=$'\t' read -r id category; do
    while (( $(jobs -pr | wc -l) >= jobs )); do
      sleep 0.1
    done
    process_sample "$id" "$category" &
  done < <(sed '1d' "$SAMPLE")
  wait

  awk '{ seen[$0]++ } END { for (primitive in seen) print seen[primitive], primitive }' \
    "$ROOT"/results/*/primitives.txt \
    | sort -k1,1nr -k2,2 >"$ROOT/results/prevalence.txt"
  echo "results: $ROOT/results"
}

for command in git curl make rg tar gzip kpsewhich; do
  require "$command"
done

case "${1:-all}" in
  setup) setup ;;
  smoke) smoke ;;
  all) setup; smoke ;;
  *) echo "usage: $0 [setup|smoke|all]" >&2; exit 2 ;;
esac
