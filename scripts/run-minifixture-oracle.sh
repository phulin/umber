#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

corpus_root="tests/corpus/command-semantic"
target_dir="${CARGO_TARGET_DIR:-target}"
[[ "$target_dir" == /* ]] || target_dir="${repo_root}/${target_dir}"
oracle_dir="${target_dir}/pdftex14027-oracle"
executable="${oracle_dir}/bin/umber-pdftex14027-oracle-instrumented"
out_root="${target_dir}/minifixture-oracle"

# The instrumented pdfTeX 1.40.27 executable is a Web2C program: it needs a
# kpathsea configuration (texmf.cnf) to find *any* file, including one in its
# own working directory (verified: without TEXMFCNF it cannot even open the
# fixture given on its command line). The oracle is built from a pinned
# TeX Live source checkout that is too large to duplicate per git worktree,
# so, exactly like scripts/build-pdftex14027-oracle.sh, the checkout is
# selected by UMBER_REF_TEXLIVE_SOURCE and defaults to third_party/texlive-source
# relative to this checkout.
cache_root="${UMBER_REF_TEXLIVE_SOURCE:-third_party/texlive-source}"
[[ "$cache_root" == /* ]] || cache_root="${repo_root}/${cache_root}"
source_dir="${cache_root}/src"
texmfcnf_dir="${source_dir}/texk/kpathsea"

source_date_epoch="${SOURCE_DATE_EPOCH:-1783604160}"

usage() {
  cat <<'EOF'
usage: scripts/run-minifixture-oracle.sh (--case DOMAIN/CASE-ID)... | --all

Run one or more tests/corpus/command-semantic minifixture sources through the
pinned, already-built INSTRUMENTED pdfTeX 1.40.27 oracle
(target/pdftex14027-oracle/bin/umber-pdftex14027-oracle-instrumented) and
capture every channel that fixture's manifest entry can produce: terminal
text, the raw and host-clock-normalized log, the DVI/PDF page artifact,
status.txt (exit code), any writer-effect files the source itself opens, and
the schema-v1 pdftex14027-events.jsonl command trace.

This script never builds the oracle (run
scripts/build-pdftex14027-oracle.sh first if target/pdftex14027-oracle is
missing) and performs no network access.

Each selected case is staged and run under:
  target/minifixture-oracle/<domain>/<case-id>/

Options:
  --case DOMAIN/CASE-ID   Run one case. May be repeated.
  --all                   Run every case in every domain manifest.
  --help, -h              Show this message.

Environment:
  UMBER_REF_TEXLIVE_SOURCE   Selects the extracted TeX Live 2025 source
                             checkout used for TEXMFCNF (see
                             scripts/build-pdftex14027-oracle.sh). Defaults to
                             third_party/texlive-source relative to this
                             checkout; that directory is gitignored and, in a
                             fresh worktree, is normally not present, so this
                             usually needs to point at the checkout that built
                             the oracle.
  CARGO_TARGET_DIR          Relocates target/pdftex14027-oracle and the output
                             directory the same way the build script does.
EOF
}

fail() {
  printf 'run-minifixture-oracle: %s\n' "$*" >&2
  exit 1
}

warn() {
  printf 'run-minifixture-oracle: %s\n' "$*" >&2
}

selected_cases=()
run_all=0

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --case)
      [[ "${2:-}" == */* ]] || fail "--case expects DOMAIN/CASE-ID, got: ${2:-<missing>}"
      selected_cases+=("$2")
      shift 2
      ;;
    --all) run_all=1; shift ;;
    --help|-h) usage; exit 0 ;;
    *) printf 'run-minifixture-oracle: unknown option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ "$run_all" -eq 1 || "${#selected_cases[@]}" -gt 0 ]] ||
  { usage >&2; exit 2; }
[[ "$run_all" -eq 0 || "${#selected_cases[@]}" -eq 0 ]] ||
  fail "--all and --case are mutually exclusive"

command -v jq >/dev/null || fail "jq is required"
[[ -x "$executable" ]] ||
  fail "instrumented oracle not built: $executable (run scripts/build-pdftex14027-oracle.sh)"
[[ -f "${texmfcnf_dir}/texmf.cnf" ]] ||
  fail "missing kpathsea config at ${texmfcnf_dir}/texmf.cnf; set UMBER_REF_TEXLIVE_SOURCE to a checkout with an extracted texlive-source/src tree"

# Every profile the corpus declares, and how (or whether) this runner can
# reproduce it with the executable above:
#
#   initex        (default) -> `-ini`             plain INITEX, no e-TeX.
#   etex-initex              -> `-ini -etex`       INITEX with e-TeX extensions
#                                                   active; this is the exact
#                                                   invocation the rest of
#                                                   scripts/build-pdftex14027-oracle.sh
#                                                   already uses.
#   etex-loaded               -> unsupported. Umber builds this profile by
#                                                   dumping and reloading an
#                                                   in-memory e-TeX format
#                                                   (tools/tex-command-stream's
#                                                   `SessionProfile::EtexLoaded`
#                                                   calls `dump_format` /
#                                                   `from_format`). Reproducing
#                                                   it here needs a real `\dump`
#                                                   format file plus a `-fmt`
#                                                   invocation; this runner does
#                                                   not build one yet.
#   production                -> unsupported. Same gap: Umber's `Production`
#                                                   profile runs
#                                                   `CanonicalMainControl::new()`
#                                                   after INITEX, i.e. a
#                                                   post-format engine state,
#                                                   which likewise needs a real
#                                                   loaded format this runner
#                                                   does not build.
#
# 5 of 130 cases (3 etex-loaded, 2 production) are therefore skipped with a
# clear message rather than silently misrun under the wrong profile.
engine_args_for_profile() {
  case "$1" in
    initex) printf -- '-ini' ;;
    etex-initex) printf -- '-ini\n-etex' ;;
    *) return 1 ;;
  esac
}

# Interaction mode.
#
# scripts/build-pdftex14027-oracle.sh uses -interaction=batchmode for most of
# its own fixed fixtures: that suppresses nearly all terminal text (measured
# ~126 bytes per run), which would make this runner's terminal channel
# vacuous for comparison purposes.
#
# 8 of the 130 minifixtures carry a `terminal_lines` manifest field: answers
# fed to interactive prompts (`\read`-from-terminal, `\pausing`, or an
# ordinary error's `?` prompt). tex.web section 7593 (firm_up_the_line) and
# the `\read`-from-terminal case in read_toks (section ~9487) both gate on
# `interaction>nonstop_mode`: under -interaction=batchmode or =nonstopmode,
# `\read` from a negative stream is a *fatal* "cannot \read from terminal in
# nonstop modes" error and `\pausing` never fires at all, so either mode would
# misrun those 8 cases outright rather than exercise the behavior the
# minifixture is testing.
#
# -interaction=errorstopmode (tex82's actual default when no -interaction flag
# is given) stops and prompts at *every* error, not only the ones a case's
# `terminal_lines` anticipates. Verified directly: main-control/font-definition
# has no `terminal_lines` but does trigger a real error ("Illegal
# magnification..."); run with no -interaction flag and empty stdin it prints
# the error, gets an unanswered `?` prompt, and immediately follows with
# "! Emergency stop." before reaching the rest of the source (`\count0=1`
# never executes). The same source under scrollmode reaches `\end` normally.
# Since only 8 of 130 cases are known to need terminal interaction,
# errorstopmode-by-default would make the other 122 fragile to *any*
# undeclared error the reference engine raises that Umber's simulation
# doesn't (which is precisely the kind of divergence this whole effort exists
# to surface).
#
# -interaction=scrollmode is the one mode that satisfies both constraints at
# once: it is `>nonstop_mode` (verified: \read-from-terminal and \pausing both
# work under it, unlike batch/nonstopmode), and it "omits error stops"
# (tex.web section 1749) exactly like nonstopmode/batchmode do, so an
# undeclared error still just prints and the run completes instead of
# demanding an answer this runner has no way to supply. It is used
# unconditionally below, with each case's `terminal_lines` (empty by default)
# piped in as answers to whatever prompts scrollmode *does* still honor.
#
# Known gap: main-control/show-completion specifically exercises the
# errorstopmode-only "? " prompt after `\showthe` (its `terminal_lines` answer
# "s" is meant to switch interaction to \scrollmode from that prompt). Under
# -interaction=scrollmode the `\showthe` diagnostic still prints, but no `? `
# prompt appears and the fed "s" line is simply unused stdin. That one case's
# `terminal-check:? ` projection cannot be reproduced under this single-mode
# choice; every other terminal_lines case (7 read/pausing-based) is
# reproduced faithfully. This is reported rather than special-cased per-case
# to keep the runner's behavior uniform and easy to reason about.
interaction_mode=scrollmode

domains() {
  LC_ALL=C find "$corpus_root" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' | LC_ALL=C sort
}

case_json_for() {
  local domain="$1" case_id="$2"
  local manifest="${corpus_root}/${domain}/manifest.json"
  [[ -f "$manifest" ]] || fail "no such domain manifest: $manifest"
  local json
  json="$(jq -c --arg id "$case_id" '.cases[] | select(.id == $id)' "$manifest")"
  [[ -n "$json" ]] || fail "no such case in $domain: $case_id"
  printf '%s\n' "$json"
}

# Copies the case's declared `inputs` (exact byte content, no added newline)
# and `font_inputs` (TFM bytes copied from the repository path the manifest
# names) into the run directory, alongside the source itself.
stage_case_files() {
  local case_json="$1" domain="$2" run_dir="$3" source_name="$4"
  cp "${corpus_root}/${domain}/${source_name}" "${run_dir}/${source_name}"
  local key
  while IFS= read -r key; do
    [[ -n "$key" ]] || continue
    jq -j --arg k "$key" '.inputs[$k]' <<<"$case_json" >"${run_dir}/${key}"
  done < <(jq -r '(.inputs // {}) | keys[]' <<<"$case_json")
  while IFS= read -r key; do
    [[ -n "$key" ]] || continue
    local source_path
    source_path="$(jq -r --arg k "$key" '.font_inputs[$k]' <<<"$case_json")"
    [[ -f "${repo_root}/${source_path}" ]] ||
      fail "font_inputs source is missing: $source_path"
    cp "${repo_root}/${source_path}" "${run_dir}/${key}"
  done < <(jq -r '(.font_inputs // {}) | keys[]' <<<"$case_json")
}

byte_size() {
  [[ -f "$1" ]] && stat -c '%s' "$1" || printf 'absent'
}

run_one_case() {
  local domain="$1" case_id="$2"
  local case_json
  case_json="$(case_json_for "$domain" "$case_id")"
  local source_name profile
  source_name="$(jq -r '.source' <<<"$case_json")"
  profile="$(jq -r '.profile // "initex"' <<<"$case_json")"

  local engine_args
  if ! engine_args="$(engine_args_for_profile "$profile")"; then
    warn "skipping $domain/$case_id: profile '$profile' needs a loaded/dumped" \
      "format, which this runner does not build yet (see" \
      "engine_args_for_profile in this script)"
    return 1
  fi
  readarray -t engine_args <<<"$engine_args"

  local run_dir="${out_root}/${domain}/${case_id}"
  rm -rf "$run_dir"
  mkdir -p "$run_dir"
  stage_case_files "$case_json" "$domain" "$run_dir" "$source_name"

  local -a terminal_lines
  readarray -t terminal_lines < <(jq -r '(.terminal_lines // [])[]' <<<"$case_json")

  local stem="${source_name%.tex}"
  local status=0
  (
    cd "$run_dir"
    printf '%s\n' "${terminal_lines[@]}" |
      env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C TZ=UTC \
        SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
        TEXMFCNF="$texmfcnf_dir" \
        "$executable" "${engine_args[@]}" "-interaction=${interaction_mode}" \
        "$source_name" >terminal.txt 2>&1
  ) || status="$?"
  printf '%s\n' "$status" >"${run_dir}/status.txt"

  if [[ -f "${run_dir}/${stem}.log" ]]; then
    sed '1s/)  .*$/) <HOST-CLOCK>/' "${run_dir}/${stem}.log" >"${run_dir}/ordinary.log"
  fi

  # Anything left in the run directory beyond the staged inputs and the
  # channels named above is a writer-effect artifact the source itself
  # produced (e.g. `\openout`/`\write` to a file it names, as
  # page-output/open-close-effect-observation.tex does). The corpus does not
  # use the fixed "<jobname>-effects.out" convention
  # scripts/build-pdftex14027-oracle.sh's own transitions/extensions/state
  # fixtures use (only one of 130 sources calls \openout at all, and it opens
  # a name of its own choosing), so effect artifacts are discovered rather
  # than assumed.
  local -a staged=("$source_name" "${stem}.log" ordinary.log "${stem}.dvi" "${stem}.pdf" \
    status.txt terminal.txt pdftex14027-events.jsonl)
  local key
  while IFS= read -r key; do staged+=("$key"); done \
    < <(jq -r '(.inputs // {}) | keys[]' <<<"$case_json")
  while IFS= read -r key; do staged+=("$key"); done \
    < <(jq -r '(.font_inputs // {}) | keys[]' <<<"$case_json")
  local -a effect_artifacts=()
  local entry known found
  for entry in "${run_dir}"/*; do
    entry="$(basename "$entry")"
    found=0
    for known in "${staged[@]}"; do
      [[ "$entry" == "$known" ]] && { found=1; break; }
    done
    [[ "$found" -eq 1 ]] || effect_artifacts+=("$entry")
  done

  printf 'case %s/%s profile=%s status=%s' "$domain" "$case_id" "$profile" "$status"
  printf ' terminal=%s' "$(byte_size "${run_dir}/terminal.txt")"
  printf ' log=%s' "$(byte_size "${run_dir}/${stem}.log")"
  printf ' ordinary.log=%s' "$(byte_size "${run_dir}/ordinary.log")"
  printf ' dvi=%s' "$(byte_size "${run_dir}/${stem}.dvi")"
  printf ' pdf=%s' "$(byte_size "${run_dir}/${stem}.pdf")"
  printf ' events=%s' "$(byte_size "${run_dir}/pdftex14027-events.jsonl")"
  if [[ "${#effect_artifacts[@]}" -gt 0 ]]; then
    printf ' effects=%s' "${effect_artifacts[*]}"
  else
    printf ' effects=none'
  fi
  printf '\n'
  return 0
}

overall_status=0
ran=0
skipped=0

if [[ "$run_all" -eq 1 ]]; then
  domain=""
  while IFS= read -r domain; do
    [[ -f "${corpus_root}/${domain}/manifest.json" ]] || continue
    case_id=""
    while IFS= read -r case_id; do
      [[ -n "$case_id" ]] || continue
      if run_one_case "$domain" "$case_id"; then
        ran=$((ran + 1))
      else
        skipped=$((skipped + 1))
      fi
    done < <(jq -r '.cases[].id' "${corpus_root}/${domain}/manifest.json")
  done < <(domains)
else
  for selector in "${selected_cases[@]}"; do
    domain="${selector%%/*}"
    case_id="${selector#*/}"
    if run_one_case "$domain" "$case_id"; then
      ran=$((ran + 1))
    else
      skipped=$((skipped + 1))
    fi
  done
fi

warn "ran $ran case(s), skipped $skipped unsupported-profile case(s)"
exit "$overall_status"
