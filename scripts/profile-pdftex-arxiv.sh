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

usage() {
  cat <<EOF
usage: $0 [setup|smoke|all|check-sample|check-entrypoint|select-entrypoint DIRECTORY]

Build a pinned instrumented pdfTeX and profile the committed 100-paper sample.
Each result preserves primitive usage, the raw recorder inputs.fls, and a
host-independent files.txt containing normalized /texlive paths.

  setup         build pdfTeX and the pdflatex format under PDFTEX_PROFILE_ROOT
  smoke         download/profile the sample with the existing build
  all           run setup followed by smoke (default)
  check-sample  validate the sample header, row count, and unique identifiers
  check-entrypoint
                test entrypoint selection against live and commented declarations
  select-entrypoint DIRECTORY
                print the entrypoint selected for an extracted source bundle

Environment:
  PDFTEX_PROFILE_ROOT    disposable build/cache root (default: /tmp/umber-pdftex-primitive-trace)
  PDFTEX_PROFILE_SAMPLE  alternate TSV sample manifest
  PDFTEX_PROFILE_JOBS    concurrent profiling jobs (default: 8)
  PDFTEX_PROFILE_LIMIT   deterministic sample prefix to profile (default: 100)
EOF
}

validate_sample() {
  [[ -f "$SAMPLE" ]] || {
    echo "sample manifest not found: $SAMPLE" >&2
    return 1
  }
  awk -F '\t' '
    NR == 1 {
      if ($0 != "id\tcategories") {
        print "invalid sample header: " $0 > "/dev/stderr"
        exit 1
      }
      next
    }
    NF != 2 || $1 == "" || $2 == "" {
      print "invalid sample row " NR > "/dev/stderr"
      exit 1
    }
    seen[$1]++ {
      print "duplicate sample identifier at row " NR ": " $1 > "/dev/stderr"
      exit 1
    }
    { rows++ }
    END {
      if (rows != 100) {
        print "sample must contain exactly 100 rows; found " rows > "/dev/stderr"
        exit 1
      }
    }
  ' "$SAMPLE"
  echo "sample ok: 100 unique papers ($SAMPLE)"
}

sample_rows() {
  awk -v limit="$1" 'NR > 1 && NR <= limit + 1' "$SAMPLE"
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
  python3 "$SCRIPT_DIR/arxiv_corpus.py" materialize "$archive" "$destination"
}

entrypoint() {
  local directory=$1 candidate
  # A live declaration can span physical lines (most commonly its option
  # list), so identify the control sequence rather than requiring its opening
  # class-name brace on this line. Requiring the command at the start after
  # whitespace still excludes commented examples.
  local documentclass='^[[:space:]]*\\documentclass([[:space:]]|\[|\{|$)'
  for candidate in main.tex manuscript.tex arxiv_version.tex paper.tex; do
    if [[ -f "$directory/$candidate" ]] && rg -q "$documentclass" "$directory/$candidate"; then
      printf '%s\n' "$directory/$candidate"
      return
    fi
  done
  rg -l "$documentclass" "$directory" -g '*.tex' \
    | rg -v '/(supp|supplement|appendix)[^/]*\.tex$' \
    | sort \
    | head -1
}

check_entrypoint() {
  local fixture selected
  fixture="$(mktemp -d "${TMPDIR:-/tmp}/umber-arxiv-entrypoint.XXXXXX")"
  printf '%% \\documentclass{standalone}\nfragment\n' >"$fixture/a-fragment.tex"
  printf '  %% \\documentclass{article}\ncommented\n' >"$fixture/main.tex"
  printf '\\documentclass{article}\n\\begin{document}\n\\end{document}\n' \
    >"$fixture/z-document.tex"
  selected="$(entrypoint "$fixture")"
  if [[ "$selected" != "$fixture/z-document.tex" ]]; then
    rm -rf -- "$fixture"
    echo "entrypoint check selected ${selected:-nothing}, expected z-document.tex" >&2
    return 1
  fi
  printf '\\documentclass[\n  twocolumn,\n  draft\n]{article}\n' \
    >"$fixture/z-document.tex"
  selected="$(entrypoint "$fixture")"
  if [[ "$selected" != "$fixture/z-document.tex" ]]; then
    rm -rf -- "$fixture"
    echo "entrypoint check rejected a live multiline declaration" >&2
    return 1
  fi
  printf '\\documentclass{article}\n' >"$fixture/paper.tex"
  selected="$(entrypoint "$fixture")"
  rm -rf -- "$fixture"
  if [[ "$selected" != */paper.tex ]]; then
    echo "entrypoint check did not preserve preferred paper.tex precedence" >&2
    return 1
  fi
  echo 'entrypoint selection ok'
}

process_sample_staged() (
  local id=$1 category=$2
  local key=${id//\//_}
  local archive="$ROOT/samples/$key.src"
  local result="$ROOT/results/$key"
  mkdir -p "$result"
  rm -f "$result/summary.tsv"
  if [[ ! -f "$archive" ]]; then
    curl -L --fail --show-error --silent --retry 3 \
      -o "$archive" "https://export.arxiv.org/e-print/$id"
  fi
  # The archive is the durable input. Every reference run gets a disposable
  # exact extraction so generated TeX artifacts never enter a shared view.
  local run_root directory
  run_root="${UMBER_ARXIV_STAGE:?}"
  directory="$run_root/source"
  unpack_source "$archive" "$directory" >"$result/source-members.json"
  local main
  main="$(entrypoint "$directory")"
  [[ -n "$main" ]] || {
    echo "no TeX entrypoint found for $id" >&2
    return
  }
  python3 "$SCRIPT_DIR/arxiv_corpus.py" identity "$archive" "${main#"$directory/"}" \
    >"$result/source-identity.json"
  set +e
  (
    cd "$(dirname "$main")"
    tex_env "$PDFTEX" \
      --progname=pdflatex \
      -fmt="$FORMAT" \
      -recorder \
      -interaction=nonstopmode \
      -halt-on-error \
      "$(basename "$main")"
  ) >"$result/pdftex.stdout" 2>&1
  local rc=$?
  set -e
  rg '^PDFTEX_PRIMITIVE_USED \\' "$result/pdftex.stdout" \
    | sed 's/^PDFTEX_PRIMITIVE_USED //' \
    | sort -u >"$result/primitives.txt" || true
  local fls
  fls="$(dirname "$main")/$(basename "${main%.*}").fls"
  if [[ -f "$fls" ]]; then
    cp "$fls" "$result/inputs.fls"
    # Recorder paths are host-specific. Preserve the raw trace for audit, and
    # also emit snapshot-relative paths that replay identically on every host.
    LC_ALL=C awk '
      /^INPUT / {
        path = substr($0, 7)
        marker = "/texmf-dist/"
        start = index(path, marker)
        if (start != 0) print "/texlive/" substr(path, start + length(marker))
      }
    ' "$fls" | LC_ALL=C sort -u >"$result/files.txt"
  else
    : >"$result/inputs.fls"
    : >"$result/files.txt"
  fi
  local count input_count
  count="$(wc -l <"$result/primitives.txt" | tr -d ' ')"
  input_count="$(wc -l <"$result/files.txt" | tr -d ' ')"
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$id" "$category" "${main#"$directory/"}" "$rc" "$count" "$input_count" \
    | tee "$result/summary.tsv"
)

process_sample() {
  python3 "$SCRIPT_DIR/arxiv_corpus.py" stage -- \
    "$0" process-sample-staged "$1" "$2"
}

smoke() {
  [[ -x "$PDFTEX" && -f "$FORMAT" ]] || {
    echo "run '$0 setup' first" >&2
    exit 1
  }
  validate_sample
  mkdir -p "$ROOT/samples" "$ROOT/results"

  local jobs=${PDFTEX_PROFILE_JOBS:-8}
  [[ "$jobs" =~ ^[1-9][0-9]*$ ]] || {
    echo "PDFTEX_PROFILE_JOBS must be a positive integer: $jobs" >&2
    exit 1
  }
  local limit=${PDFTEX_PROFILE_LIMIT:-100}
  [[ "$limit" =~ ^[1-9][0-9]*$ ]] && (( limit <= 100 )) || {
    echo "PDFTEX_PROFILE_LIMIT must be an integer from 1 through 100: $limit" >&2
    exit 1
  }
  while IFS=$'\t' read -r id category; do
    while (( $(jobs -pr | wc -l) >= jobs )); do
      sleep 0.1
    done
    process_sample "$id" "$category" &
  done < <(sample_rows "$limit")
  wait

  local key
  local -a primitive_files=()
  printf 'id\tcategory\tentrypoint\texit\tprimitive_count\tinput_count\n' >"$ROOT/results/summary.tsv"
  while IFS=$'\t' read -r id _; do
    key=${id//\//_}
    if [[ -f "$ROOT/results/$key/summary.tsv" ]]; then
      cat "$ROOT/results/$key/summary.tsv" >>"$ROOT/results/summary.tsv"
      primitive_files+=("$ROOT/results/$key/primitives.txt")
    fi
  done < <(sample_rows "$limit")

  if (( ${#primitive_files[@]} > 0 )); then
    awk '{ seen[$0]++ } END { for (primitive in seen) print seen[primitive], primitive }' \
      "${primitive_files[@]}" \
      | LC_ALL=C sort -k1,1nr -k2,2 >"$ROOT/results/prevalence.txt"
  else
    : >"$ROOT/results/prevalence.txt"
  fi
  echo "results: $ROOT/results"
}

readonly ACTION=${1:-all}
case "$ACTION" in
  -h|--help|help) usage; exit 0 ;;
  setup|smoke|all)
    for command in git curl make rg tar gzip kpsewhich python3; do
      require "$command"
    done
    ;;
  check-sample) require awk ;;
  check-entrypoint) require mktemp; require rg ;;
  select-entrypoint)
    require rg
    [[ $# -eq 2 && -d "${2:-}" ]] || { usage >&2; exit 2; }
    ;;
  process-sample-staged)
    [[ $# -eq 3 && -n "${UMBER_ARXIV_STAGE:-}" ]] || { usage >&2; exit 2; }
    ;;
  *) usage >&2; exit 2 ;;
esac

case "$ACTION" in
  setup) setup ;;
  smoke) smoke ;;
  all) setup; smoke ;;
  check-sample) validate_sample ;;
  check-entrypoint) check_entrypoint ;;
  select-entrypoint) entrypoint "$2" ;;
  process-sample-staged) process_sample_staged "$2" "$3" ;;
esac
