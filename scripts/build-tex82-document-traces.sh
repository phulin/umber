#!/usr/bin/env bash
set -euo pipefail

# Generate the full-document TeX82 semantic traces the differential tracer
# (tools/tex-command-stream) replays for plain bootstrap, Story, and Gentle.
#
# Unlike the committed fixtures under tests/corpus/command/tex82, these traces
# are NOT versioned: one plain-bootstrap trace is roughly 17 MB and Gentle's is
# roughly 156 MB. They are regenerated on demand into a gitignored tree, and
# their identity is pinned by tests/tex82-document-trace-manifest.txt. See
# "Full-Document Trace Registry" in docs/testing_infrastructure.md.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

target_dir="${CARGO_TARGET_DIR:-target}"
[[ "$target_dir" == /* ]] || target_dir="${repo_root}/${target_dir}"
oracle_dir="${target_dir}/tex82-oracle"
instrumentable_executable="${oracle_dir}/bin/umber-tex82-oracle-instrumentable"
clean_executable="${oracle_dir}/bin/umber-tex82-oracle"
build_record="${oracle_dir}/build-record.txt"
texmfcnf="${UMBER_REF_TEXLIVE_SOURCE:-${repo_root}/third_party/texlive-source}/src/texk/web2c/triptrap"
document_root="${repo_root}/tests/tex82-documents"
fixture_root="${repo_root}/tests/corpus/command/tex82-documents"
contract="${repo_root}/tests/tex82-document-trace-manifest.txt"
publication_root="${oracle_dir}/documents/publish-candidate"
corpus="${repo_root}/third_party/corpus"
hyphen="${repo_root}/third_party/hyphen/hyphen.tex"
fonts="${repo_root}/third_party/fonts"
source_date_epoch="${SOURCE_DATE_EPOCH:-1783604160}"
all_documents=(plain story gentle)
selected=()

usage() {
  cat <<'EOF'
usage: scripts/build-tex82-document-traces.sh [--document NAME]...

Regenerate the gitignored full-document TeX82 semantic traces under
tests/corpus/command/tex82-documents and rewrite the pinned identities in
tests/tex82-document-trace-manifest.txt. Without --document every registered
document (plain, story, gentle) is regenerated; the contract is only rewritten
when every document is regenerated, so a partial run must not be published.

Requires scripts/build-tex82-oracle.sh to have produced
target/tex82-oracle/bin, and scripts/setup-conformance-tests.sh (or
scripts/fetch-conformance-inputs.sh) to have populated third_party/corpus,
third_party/hyphen, and third_party/fonts. It performs no network I/O.

Exit statuses. No status means "nothing to do": a run that generates no trace
never exits 0, because a caller that reads only the status must not be able to
confuse "regenerated every document" with "regenerated none".

  0  every selected document was regenerated (and, for a full run, the pinned
     contract was rewritten)
  1  generation ran and failed: an oracle run, a determinism comparison, or a
     fixture bootstrap did not hold
  2  the command line is wrong
  3  a prerequisite is absent, so nothing ran. This is the expected status on
     a fresh checkout; run scripts/build-tex82-oracle.sh and
     scripts/setup-conformance-tests.sh, then rerun. It is separated from 1
     so a caller can tell "not set up yet" from "set up, and broken"
EOF
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --document)
      [[ "$#" -ge 2 ]] || { printf 'build-tex82-document-traces: --document requires a name\n' >&2; exit 2; }
      selected+=("$2"); shift ;;
    --help|-h) usage; exit 0 ;;
    *) printf 'build-tex82-document-traces: unknown option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

fail() {
  printf 'build-tex82-document-traces: %s\n' "$*" >&2
  exit 1
}

# A prerequisite this script cannot supply is absent, so no trace was
# generated. Reported as its own status rather than as success: the caller's
# worklist silently shrinks by however many documents this run did not
# produce, and the only place that shrinkage is visible before the tracer runs
# is here.
missing_prerequisite() {
  printf 'build-tex82-document-traces: %s\n' "$*" >&2
  printf 'build-tex82-document-traces: no document trace was generated (exit 3).\n' >&2
  exit 3
}

sha256_of() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    fail 'need shasum or sha256sum on PATH'
  fi
}

sha256_of_stdin() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 - | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum - | awk '{print $1}'
  else
    fail 'need shasum or sha256sum on PATH'
  fi
}

# Aggregate identity over the staged TFM set, matching
# tools/tex-command-stream's `font_set_identity`: one "name sha256" line per
# font in bytewise name order.
font_set_identity() {
  local directory="$1" font
  (
    cd "$directory"
    for font in $(LC_ALL=C ls -1 *.tfm); do
      printf '%s %s\n' "$font" "$(sha256_of "$font")"
    done
  ) | sha256_of_stdin
}

stage_document() {
  local name="$1" run_dir="$2"
  rm -rf "$run_dir"
  mkdir -p "$run_dir"
  cp "${document_root}/${name}/root.tex" "${run_dir}/root.tex"
  cp "${corpus}/plain.tex" "${hyphen}" "$run_dir/"
  [[ "$name" == plain ]] || cp "${corpus}/${name}.tex" "$run_dir/"
  cp "${fonts}"/*.tfm "$run_dir/"
}

run_document() {
  local executable="$1" run_dir="$2" status=0
  (
    cd "$run_dir"
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C \
      SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
      TEXMFCNF="$texmfcnf" TEXINPUTS=".:" TEXFONTS=".:" \
      "$executable" -ini -interaction=nonstopmode root.tex >terminal.txt 2>&1
  ) || status="$?"
  [[ "$status" -le 1 ]] ||
    fail "document run in ${run_dir} exited with unexpected status $status"
  printf '%s\n' "$status" >"${run_dir}/status.txt"
  [[ -f "${run_dir}/root.log" ]] || fail "document run in ${run_dir} wrote no log"
  sed '1s/)  .*$/) <HOST-CLOCK>/' "${run_dir}/root.log" >"${run_dir}/ordinary.log"
}

# The live run directory holds every staged input; only the manifest-declared
# ordinary channels and .tex sources become fixture artifacts, so the staged
# TFMs and the raw log are compared here rather than committed through the
# manifest.
compare_variants() {
  local name="$1" clean_dir="$2" instrumented_dir="$3" repeat_dir="$4" channel
  for channel in terminal.txt ordinary.log status.txt; do
    cmp "${clean_dir}/${channel}" "${instrumented_dir}/${channel}" >/dev/null ||
      fail "instrumentable oracle changed ${name} ${channel}"
    cmp "${instrumented_dir}/${channel}" "${repeat_dir}/${channel}" >/dev/null ||
      fail "repeated instrumentable oracle changed ${name} ${channel}"
  done
  if [[ -f "${clean_dir}/root.dvi" ]]; then
    cmp "${clean_dir}/root.dvi" "${instrumented_dir}/root.dvi" >/dev/null ||
      fail "instrumentable oracle changed ${name} DVI"
    cmp "${instrumented_dir}/root.dvi" "${repeat_dir}/root.dvi" >/dev/null ||
      fail "repeated instrumentable oracle changed ${name} DVI"
  fi
  cmp "${instrumented_dir}/tex82-events.jsonl" "${repeat_dir}/tex82-events.jsonl" >/dev/null ||
    fail "instrumentable oracle emitted a nondeterministic ${name} trace"
}

build_document() {
  local name="$1"
  [[ -f "${document_root}/${name}/root.tex" ]] ||
    fail "unregistered document ${name}: ${document_root}/${name}/root.tex is absent"
  local base="${oracle_dir}/documents/${name}"
  local clean_dir="${base}/clean"
  local instrumented_dir="${base}/instrumentable"
  local repeat_dir="${base}/instrumentable-repeat"

  stage_document "$name" "$clean_dir"
  run_document "$clean_executable" "$clean_dir"
  stage_document "$name" "$instrumented_dir"
  run_document "$instrumentable_executable" "$instrumented_dir"
  stage_document "$name" "$repeat_dir"
  run_document "$instrumentable_executable" "$repeat_dir"
  compare_variants "$name" "$clean_dir" "$instrumented_dir" "$repeat_dir"
  # The repeat run exists only to prove determinism; its ~17-156 MB trace is
  # dropped immediately so three copies never coexist on disk.
  rm -rf "$repeat_dir"

  local font_identity
  font_identity="$(font_set_identity "$instrumented_dir")"

  # The document tier's "distribution" is exactly the staged metric set, so
  # the manifest's distribution identity is that set's aggregate digest rather
  # than a copied constant.
  local template_dir="${base}/template"
  rm -rf "$template_dir"
  mkdir -p "$template_dir"
  sed "s/\"distribution_sha256\":\"[0-9a-f]*\"/\"distribution_sha256\":\"${font_identity}\"/" \
    "${document_root}/${name}/template-manifest.json" >"${template_dir}/manifest.json"
  local source source_names=()
  for source in "${instrumented_dir}"/*.tex; do
    source_names+=("${source##*/}")
  done
  local source_closure="${source_names[0]}"
  for source in "${source_names[@]:1}"; do
    source_closure+=" and ${source}"
  done
  printf 'document|source closure|%s|full-document trace|\"event\":\n' \
    "$source_closure" >"${template_dir}/semantic-matrix.txt"

  local candidate="${base}/candidate"
  rm -rf "$candidate"
  cargo run -q -p tex-oracle --bin tex-oracle-bootstrap -- \
    --template "$template_dir" \
    --live-directory "$instrumented_dir" \
    --semantic-matrix "${template_dir}/semantic-matrix.txt" \
    --event-stream "${instrumented_dir}/tex82-events.jsonl" \
    --build-record "$build_record" \
    --output "$candidate"

  local published
  if [[ "$publish_contract" -eq 1 ]]; then
    published="${publication_root}/${name}"
  else
    published="${fixture_root}/${name}"
  fi
  rm -rf "$published"
  mkdir -p "$fixture_root"
  mv "$candidate" "$published"
  mkdir -p "${published}/fonts"
  cp "${fonts}"/*.tfm "${published}/fonts/"
  rm -rf "$clean_dir" "$instrumented_dir" "$template_dir"

  local manifest_identity events
  manifest_identity="$(sha256_of "${published}/manifest.json")"
  events="$(($(wc -l <"${published}/events.jsonl") - 1))"
  # Published through a global rather than stdout: `set -e` does not abort on
  # a failed command substitution, so capturing this function's output would
  # turn a failed oracle run into a silently empty record.
  document_record="$(printf 'document %s root.tex %s %s %s' \
    "$name" "$manifest_identity" "$events" "$font_identity")"
}

[[ -x "$instrumentable_executable" && -x "$clean_executable" ]] ||
  missing_prerequisite "missing pinned oracle executables; run scripts/build-tex82-oracle.sh first"
[[ -f "$build_record" ]] ||
  missing_prerequisite "missing ${build_record}; run scripts/build-tex82-oracle.sh first"
[[ -d "$texmfcnf" ]] || missing_prerequisite "missing pinned web2c configuration: ${texmfcnf}"
[[ -f "$hyphen" && -d "$fonts" ]] ||
  missing_prerequisite 'missing external inputs; run scripts/setup-conformance-tests.sh first'

publish_contract=1
if [[ "${#selected[@]}" -eq 0 ]]; then
  selected=("${all_documents[@]}")
else
  publish_contract=0
fi

records=()
document_record=""
if [[ "$publish_contract" -eq 1 ]]; then
  rm -rf "$publication_root"
fi
for document in "${selected[@]}"; do
  printf 'building %s document trace\n' "$document" >&2
  build_document "$document"
  [[ -n "$document_record" ]] || fail "${document} produced no pinned record"
  records+=("$document_record")
  document_record=""
done

# The only path to exit 0. Every earlier exit either names a missing
# prerequisite (3) or a failure (1), so reaching here is a positive statement
# that every selected document was regenerated -- not merely that nothing
# raised an objection.
[[ "${#records[@]}" -eq "${#selected[@]}" ]] ||
  fail "generated ${#records[@]} of ${#selected[@]} selected document trace(s)"

if [[ "$publish_contract" -eq 1 ]]; then
  contract_candidate="${oracle_dir}/documents/trace-manifest-candidate.txt"
  {
    printf '# Pinned identities for generated-on-demand TeX82 full-document semantic traces.\n'
    printf '# Regenerate with scripts/build-tex82-document-traces.sh; the traces themselves are\n'
    printf '# too large to commit (17-156 MB each) and live in the gitignored\n'
    printf '# tests/corpus/command/tex82-documents tree.\n'
    printf '# Formats:\n'
    printf '#   contract-schema VERSION\n'
    printf '#   document NAME ROOT-SOURCE FIXTURE-MANIFEST-SHA256 EVENTS FONT-SET-SHA256\n'
    printf 'contract-schema 1\n'
    printf '%s\n' "${records[@]}"
  } >"$contract_candidate"
  previous_fixture_root="${oracle_dir}/documents/published-previous"
  previous_contract="${oracle_dir}/documents/trace-manifest-previous.txt"
  rm -rf "$previous_fixture_root"
  rm -f "$previous_contract"
  mv "$fixture_root" "$previous_fixture_root"
  if ! mv "$publication_root" "$fixture_root"; then
    mv "$previous_fixture_root" "$fixture_root"
    fail "could not publish the staged document trace tree"
  fi
  cp "$contract" "$previous_contract"
  if ! mv "$contract_candidate" "$contract"; then
    rm -rf "$publication_root"
    mv "$fixture_root" "$publication_root"
    mv "$previous_fixture_root" "$fixture_root"
    cp "$previous_contract" "$contract"
    fail "could not publish the staged document trace contract"
  fi
  rm -rf "$previous_fixture_root"
  rm -f "$previous_contract"
  printf 'regenerated %s document trace(s); wrote %s\n' \
    "${#records[@]}" "${contract#"${repo_root}/"}" >&2
else
  printf 'regenerated %s of %s document trace(s) (--document selection); %s left unchanged.\n' \
    "${#records[@]}" "${#all_documents[@]}" "${contract#"${repo_root}/"}" >&2
  printf 'Records:\n' >&2
  printf '%s\n' "${records[@]}" >&2
fi
