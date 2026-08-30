#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

corpus_root="tests/corpus/command-semantic"
target_dir="${CARGO_TARGET_DIR:-target}"
[[ "$target_dir" == /* ]] || target_dir="${repo_root}/${target_dir}"
oracle_dir="${target_dir}/pdftex14029-oracle"
executable="${UMBER_MINIFIXTURE_ORACLE_EXECUTABLE:-${oracle_dir}/bin/umber-pdftex14029-oracle-instrumented}"
out_root="${target_dir}/minifixture-oracle"
# Dumped formats for the two profiles built past INITEX (see
# `ensure_format` below). One format per profile, shared by every case that
# declares it, staged into each case's run directory the same way a
# declared `font_inputs` TFM is.
format_source_dir="${out_root}/_formats"

# The instrumented pdfTeX 1.40.29 executable is a Web2C program: it needs a
# kpathsea configuration (texmf.cnf) to find *any* file, including one in its
# own working directory (verified: without TEXMFCNF it cannot even open the
# fixture given on its command line). The oracle is built from a pinned
# TeX Live source checkout that is too large to duplicate per git worktree,
# so `provision.py source` symlinks it from the primary checkout into linked
# worktrees rather than allowing an ambient source selection.
cache_root="${repo_root}/third_party/texlive-source"
source_dir="${cache_root}/src"
texmfcnf_dir="${source_dir}/texk/kpathsea"

source_date_epoch="${SOURCE_DATE_EPOCH:-1783604160}"

usage() {
  cat <<'EOF'
usage: scripts/run-minifixture-oracle.sh (--case DOMAIN/CASE-ID)... | --all
       scripts/run-minifixture-oracle.sh --profile PROFILE

Run one or more tests/corpus/command-semantic minifixture sources through the
pinned, already-built INSTRUMENTED pdfTeX 1.40.29 oracle
(target/pdftex14029-oracle/bin/umber-pdftex14029-oracle-instrumented) and
capture every channel that fixture's manifest entry can produce: terminal
text, the raw and host-clock-normalized log, the DVI/PDF page artifact,
status.txt (exit code), any writer-effect files the source itself opens, and
the schema-v1 pdftex14029-events.jsonl command trace.

This script never builds the oracle (run
python3 scripts/provision.py oracle pdftex14029 first if target/pdftex14029-oracle is
missing) and performs no network access.

Each selected case is staged and run under:
  target/minifixture-oracle/<domain>/<case-id>/

Options:
  --case DOMAIN/CASE-ID   Run one case. May be repeated.
  --all                   Run all fixture-local manifests.
  --profile PROFILE       Require every selected case to declare PROFILE.
  --profile PROFILE       Run cases whose typed capture policy selects PROFILE.
  --help, -h              Show this message.

Environment:
  CARGO_TARGET_DIR          Relocates target/pdftex14029-oracle and the output
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
required_profile=""

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --case)
      [[ "${2:-}" == */* ]] || fail "--case expects DOMAIN/CASE-ID, got: ${2:-<missing>}"
      selected_cases+=("$2")
      shift 2
      ;;
    --all) run_all=1; shift ;;
    --profile)
      [[ -n "${2:-}" ]] || fail "--profile expects a profile name"
      required_profile="$2"
      shift 2
      ;;
    --help|-h) usage; exit 0 ;;
    *) printf 'run-minifixture-oracle: unknown option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ "$run_all" -eq 1 || "${#selected_cases[@]}" -gt 0 || -n "$required_profile" ]] ||
  { usage >&2; exit 2; }
[[ "$run_all" -eq 0 || "${#selected_cases[@]}" -eq 0 ]] ||
  fail "--all and --case are mutually exclusive"

command -v jq >/dev/null || fail "jq is required"
if [[ -n "$required_profile" ]]; then
  while IFS= read -r manifest; do
    [[ "$(jq -r '.profile // "initex"' "$manifest")" == "$required_profile" ]] || continue
    [[ "$(jq -r '.capture.kind // "profile"' "$manifest")" == profile ]] || continue
    relative="${manifest#"${corpus_root}/"}"
    selected_cases+=("${relative%/manifest.json}")
  done < <(LC_ALL=C find "$corpus_root" -mindepth 3 -maxdepth 3 -name manifest.json -type f | LC_ALL=C sort)
  [[ "${#selected_cases[@]}" -gt 0 ]] || fail "profile selects no cases: $required_profile"
fi
[[ -x "$executable" ]] ||
  fail "instrumented oracle not built: $executable (run python3 scripts/provision.py oracle pdftex14029)"
[[ -f "${texmfcnf_dir}/texmf.cnf" ]] ||
  fail "missing kpathsea config at ${texmfcnf_dir}/texmf.cnf; run python3 scripts/provision.py source ."
if [[ -n "$required_profile" ]]; then
  format_name="$required_profile"
  [[ "$required_profile" == raw-tex82-loaded ]] && format_name=production
  rm -f "${format_source_dir}/${format_name}.fmt"
fi

# Every profile the corpus declares, and how this runner reproduces it with
# the executable above:
#
#   initex        (default) -> `-ini`             plain INITEX, no e-TeX.
#   etex-initex              -> `-ini -etex`       INITEX with e-TeX extensions
#                                                   active; this is the exact
#                                                   invocation the rest of
#                                                   provision.py oracle pdftex14029
#                                                   already uses.
#   etex-loaded              -> `-fmt=etex-loaded` A real e-TeX INITEX job
#                                                   dumps `etex-loaded.fmt`
#                                                   once (see `ensure_format`),
#                                                   then every case in this
#                                                   profile loads it. Mirrors
#                                                   `tools/tex-command-stream`'s
#                                                   `SessionProfile::EtexLoaded`,
#                                                   which builds the same state
#                                                   in memory via
#                                                   `dump_format`/`from_format`.
#   production                -> `-fmt=production`  Same idea with a bare
#                                                   TeX82 INITEX dump: mirrors
#                                                   `SessionProfile::Production`,
#                                                   which runs
#                                                   `CanonicalMainControl::new()`
#                                                   (a non-INITEX session) atop
#                                                   TeX82-initialized state.
#
# Real pdfTeX only tells INITEX and a loaded-format session apart by whether
# `\dump` actually ran -- there is no flag that says "install these
# primitives but behave as if a format had been loaded" -- so reproducing
# `EtexLoaded`/`Production` needs a genuine two-phase job: `-ini` (optionally
# `-etex`) dumps a format, and a second, non-INITEX invocation loads it with
# `-fmt=` and runs the case source. `scripts/regen-fixtures.sh`'s
# `regen_etrip_pdftex_fixture` already runs exactly this shape for e-TRIP;
# `ensure_format` below reuses its `-ini [-etex] -jobname=… \dump` /
# `-fmt=… -jobname=…` idiom rather than inventing a second one.
engine_args_for_profile() {
  case "$1" in
    initex) printf -- '-ini' ;;
    etex-initex) printf -- '-ini\n-etex' ;;
    etex-loaded)
      ensure_format etex-loaded 1>&2 || return 1
      printf -- '-fmt=etex-loaded'
      ;;
    production)
      ensure_format production 1>&2 || return 1
      printf -- '-fmt=production'
      ;;
    raw-tex82-loaded)
      ensure_format production 1>&2 || return 1
      printf -- '-fmt=production'
      ;;
    *) return 1 ;;
  esac
}

format_name_for_profile() {
  case "$1" in
    raw-tex82-loaded) printf 'production\n' ;;
    etex-loaded|production) printf '%s\n' "$1" ;;
    *) return 1 ;;
  esac
}

# INITEX flags and priming source for the one-time job that dumps each
# fmt-based profile's format.
#
# `production` dumps bare TeX82 INITEX state (`Universe`'s primitives with no
# e-TeX extensions), matching `SessionProfile::Production`'s
# `CanonicalMainControl::tex82_initex`. `etex-loaded` additionally sets
# `\TeXXeTstate=1` immediately before `\dump`, matching
# `SessionProfile::EtexLoaded`'s own comment: it exercises etex.ch change
# [50.1307], which resets that and other optional e-TeX state cells to their
# defaults *during* the dump, so the reloaded format must show the reset
# value regardless of what was set beforehand (`etex-diagnostics/etex-loaded-
# state-reset` is the case that checks exactly this).
dump_engine_args_for_profile() {
  case "$1" in
    etex-loaded) printf -- '-ini\n-etex' ;;
    production) printf -- '-ini' ;;
    *) return 1 ;;
  esac
}

# The dumped format's contents must match what
# `tex_command_stream::semantic`'s `execute` builds for the same profile, or a
# case that probes the format's contents compares two differently-populated
# formats and reports a fixture defect as an engine divergence.
# `\formatmacro` is exactly that: `execute` defines it in the `EtexLoaded`
# universe as a bounded format-loaded macro identity probe (TeX82 sections
# 341/1221 expose the `def_ref` head after section 1309's format memory
# compaction), and until it was defined here too,
# `etex-diagnostics/etex-loaded-macro-call` pinned an
# `! Undefined control sequence.` that only the reference engine could raise
# (umber2-sy8o).
#
# The catcode dance is required, not decorative. INITEX starts every character
# at catcode 12 except the few tex.web section 232 names, so `{` and `}` are
# *not* grouping characters here and `\def\formatmacro{\relax}` would be a
# runaway definition rather than a definition. They are restored to 12
# immediately afterwards so the dumped format still carries INITEX's own
# catcode table -- `execute` sets no catcodes before its dump either, and a
# format that silently shipped grouping characters would be a fresh asymmetry
# in place of the one this fixes.
dump_source_for_profile() {
  case "$1" in
    etex-loaded)
      printf '%s%s%s\n' \
        '\catcode`\{=1 \catcode`\}=2 \def\formatmacro{\relax}' \
        '\catcode`\{=12 \catcode`\}=12 ' \
        '\TeXXeTstate=1 \dump'
      ;;
    production) printf '\\dump\n' ;;
    *) return 1 ;;
  esac
}

# Builds `${format_source_dir}/<profile>.fmt` once and reuses it for every
# case that declares `<profile>`. `web2c/texmf.cnf`'s `TEXFORMATS` already
# begins with `$TEXMFDOTDIR` (`.`), so a case's own run directory finds a
# format staged beside it with no extra `-fmt` search-path configuration --
# the same reason a staged `font_inputs` TFM needs no `TEXFONTS` override.
ensure_format() {
  local profile="$1"
  local fmt_path="${format_source_dir}/${profile}.fmt"
  [[ -s "$fmt_path" ]] && return 0

  local dump_args
  dump_args="$(dump_engine_args_for_profile "$profile")" || return 1
  readarray -t dump_args <<<"$dump_args"
  local dump_source
  dump_source="$(dump_source_for_profile "$profile")" || return 1

  mkdir -p "$format_source_dir"
  local build_dir="${format_source_dir}/.build-${profile}"
  rm -rf "$build_dir"
  mkdir -p "$build_dir"
  printf '%s' "$dump_source" >"${build_dir}/dump.tex"

  local status=0
  (
    cd "$build_dir"
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C TZ=UTC \
      SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
      TEXMFCNF="$texmfcnf_dir" \
      "$executable" "${dump_args[@]}" -jobname="$profile" \
      -interaction=batchmode "${job_flags[@]}" dump.tex >dump.fot 2>&1
  ) || status="$?"

  if [[ ! -s "${build_dir}/${profile}.fmt" ]]; then
    warn "priming job for profile '$profile' did not produce ${profile}.fmt" \
      "(exit $status; see ${build_dir}/dump.fot)"
    return 1
  fi
  cp "${build_dir}/${profile}.fmt" "$fmt_path"
  rm -rf "$build_dir"
}

# Interaction mode.
#
# The pdftex14029 oracle builder uses -interaction=batchmode for most of
# its own fixed fixtures: that suppresses nearly all terminal text (measured
# ~126 bytes per run), which would make this runner's terminal channel
# vacuous for comparison purposes.
#
# 11 of the 207 minifixtures carry a `terminal_lines` manifest field: answers
# fed to interactive prompts (`\read`-from-terminal, `\pausing`, or an
# ordinary error's `?` prompt). tex.web section 7593 (firm_up_the_line) and
# the `\read`-from-terminal case in read_toks (section ~9487) both gate on
# `interaction>nonstop_mode`: under -interaction=batchmode or =nonstopmode,
# `\read` from a negative stream is a *fatal* "cannot \read from terminal in
# nonstop modes" error and `\pausing` never fires at all, so either mode would
# misrun those 11 cases outright rather than exercise the behavior the
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
# Since only 11 of 207 cases are known to need terminal interaction,
# errorstopmode-by-default would make the other 196 fragile to *any*
# undeclared error the reference engine raises that Umber's simulation
# doesn't (which is precisely the kind of divergence this whole effort exists
# to surface).
#
# -interaction=scrollmode is the one mode that satisfies both constraints at
# once: it is `>nonstop_mode` (verified: \read-from-terminal and \pausing both
# work under it, unlike batch/nonstopmode), and it "omits error stops"
# (tex.web section 1749) exactly like nonstopmode/batchmode do, so an
# undeclared error still just prints and the run completes instead of
# demanding an answer this runner has no way to supply. It is therefore this
# runner's default, with each case's `terminal_lines` (empty by default) piped
# in as answers to whatever prompts scrollmode *does* still honor.
#
# A case can declare a different `interaction_mode` in its manifest entry
# instead (`Case::interaction_mode` on the `tools/tex-command-stream` side,
# `CaseInteractionMode::engine_mode` for what it selects on the engine). Doing
# so is that case's declaration that its channels are not comparable to this
# runner's standard scrollmode sweep, and the corpus requires a nonempty
# `interaction_mode_note` alongside it saying why (enforced by `validate_case`
# in `tools/tex-command-stream/src/semantic.rs`). `main-control/show-completion`
# is the one committed case that does this: it exists specifically to exercise
# the errorstopmode-only "? " prompt after `\showthe` (its `terminal_lines`
# answer "s" switches interaction to \scrollmode from that prompt, exactly
# as tex.web section 1298 describes), which no scrollmode run -- oracle or
# Umber's own -- can ever produce. Reading the declared mode per case below is
# what lets this one case run under the mode its channels actually need
# instead of being permanently unreproducible against this runner, which used
# to be this comment's "Known gap".
case_interaction_mode() {
  jq -r '.interaction_mode // "scrollmode"' <<<"$1"
}

# Job-startup notices this runner's texmf.cnf defaults would otherwise add.
#
# The plain `web2c/texmf.cnf` this checkout's kpathsea reads from enables the
# restricted shell escape and `%&`-line parsing by default, so an unflagged
# run prints two lines TeX82 has no analogue for:
#
#   ␣restricted␣\write18␣enabled.
#   ␣%&-line␣parsing␣enabled.
#
# `docs/job_framing.md`'s "Why the notices are configuration, not output"
# section states the rule this follows: when the two engines' output differs
# because they were *configured* differently, fix the configuration rather
# than normalize the difference away. Umber could print both lines
# unconditionally and match, but both would be lies -- the minifixture world
# runs with shell escape disabled and implements no `%&` first-line parsing
# at all -- so the honest fix is here, on the oracle side that is actually
# configured to enable them: `-no-shell-escape` and `-no-parse-first-line`
# turn both off, which is the configuration Umber actually is, and neither
# engine ends up printing either notice.
job_flags=(-no-shell-escape -no-parse-first-line)

domains() {
  LC_ALL=C find "$corpus_root" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' | LC_ALL=C sort
}

case_json_for() {
  local domain="$1" case_id="$2"
  local manifest="${corpus_root}/${domain}/${case_id}/manifest.json"
  [[ -f "$manifest" ]] || fail "no such fixture manifest: $manifest"
  local json
  json="$(jq -c --arg id "$case_id" '. + {id: $id, source: ($id + ".tex")}' "$manifest")"
  [[ "$(jq -r '.schema' <<<"$json")" == 2 ]] ||
    fail "$domain/$case_id is not a command-semantic V2 manifest"
  printf '%s\n' "$json"
}

# Copies the case's declared `inputs` (exact byte content, no added newline)
# and `font_inputs` (TFM bytes copied from the repository path the manifest
# names) into the run directory, alongside the source itself.
stage_case_files() {
  local case_json="$1" domain="$2" case_id="$3" run_dir="$4" source_name="$5"
  cp "${corpus_root}/${domain}/${case_id}/${source_name}" "${run_dir}/${source_name}"
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
  local source_name profile interaction_mode
  source_name="$(jq -r '.source' <<<"$case_json")"
  profile="$(jq -r '.profile // "initex"' <<<"$case_json")"
  [[ -z "$required_profile" || "$profile" == "$required_profile" ]] ||
    fail "$domain/$case_id declares profile '$profile', required '$required_profile'"
  interaction_mode="$(case_interaction_mode "$case_json")"

  local engine_args
  if ! engine_args="$(engine_args_for_profile "$profile")"; then
    warn "skipping $domain/$case_id: profile '$profile' has no reproduction" \
      "(see engine_args_for_profile in this script)"
    return 1
  fi
  readarray -t engine_args <<<"$engine_args"

  local run_dir="${out_root}/${domain}/${case_id}"
  rm -rf "$run_dir"
  mkdir -p "$run_dir"
  stage_case_files "$case_json" "$domain" "$case_id" "$run_dir" "$source_name"
  # A fmt-based profile's format was just built (or already cached) by
  # `engine_args_for_profile`'s `ensure_format` call above; stage it beside
  # the source the same way a declared `font_inputs` TFM is staged, so
  # `TEXMFDOTDIR` (`.`) finds it.
  local format_name
  format_name="$(format_name_for_profile "$profile" 2>/dev/null || true)"
  if [[ -n "$format_name" && -f "${format_source_dir}/${format_name}.fmt" ]]; then
    cp "${format_source_dir}/${format_name}.fmt" "${run_dir}/${format_name}.fmt"
  fi

  local -a terminal_lines
  readarray -t terminal_lines < <(jq -r '(.terminal_lines // [])[]' <<<"$case_json")

  local stem="${source_name%.tex}"
  local status=0
  (
    cd "$run_dir"
    # `printf` runs its format once even with no arguments, so a case that
    # declares no terminal lines still hands the engine one empty line. That
    # is deliberate and load-bearing: it is what lets tex.web §360's `*`
    # prompt (or §83's `? ` prompt) succeed once before the next read reaches
    # end of file, which is the shape most of this corpus captured. Umber's
    # side reproduces the same stdin -- see `terminal_stdin` in
    # tools/tex-command-stream/src/semantic.rs.
    printf '%s\n' "${terminal_lines[@]}" |
      env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C TZ=UTC \
        SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
        TEXMFCNF="$texmfcnf_dir" \
        "$executable" "${engine_args[@]}" "-interaction=${interaction_mode}" \
        "${job_flags[@]}" \
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
  # The pdftex14029 oracle builder's own transitions/extensions/state
  # fixtures use (five of 207 sources call \openout, and they open names of
  # their own choosing), so effect artifacts are discovered rather
  # than assumed.
  local -a staged=("$source_name" "${stem}.log" ordinary.log "${stem}.dvi" "${stem}.pdf" \
    status.txt terminal.txt pdftex14029-events.jsonl pdftex14029-diagnostics.jsonl \
    "${format_name}.fmt")
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
  if [[ "${#effect_artifacts[@]}" -gt 0 ]]; then
    printf '%s\n' "${effect_artifacts[@]}" | LC_ALL=C sort >"${run_dir}/effect-artifacts.txt"
  else
    : >"${run_dir}/effect-artifacts.txt"
  fi

  printf 'case %s/%s profile=%s interaction=%s status=%s' \
    "$domain" "$case_id" "$profile" "$interaction_mode" "$status"
  printf ' terminal=%s' "$(byte_size "${run_dir}/terminal.txt")"
  printf ' log=%s' "$(byte_size "${run_dir}/${stem}.log")"
  printf ' ordinary.log=%s' "$(byte_size "${run_dir}/ordinary.log")"
  printf ' dvi=%s' "$(byte_size "${run_dir}/${stem}.dvi")"
  printf ' pdf=%s' "$(byte_size "${run_dir}/${stem}.pdf")"
  printf ' events=%s' "$(byte_size "${run_dir}/pdftex14029-events.jsonl")"
  printf ' diagnostics=%s' "$(byte_size "${run_dir}/pdftex14029-diagnostics.jsonl")"
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
    fixture_dir=""
    while IFS= read -r fixture_dir; do
      case_id="$(basename "$fixture_dir")"
      if run_one_case "$domain" "$case_id"; then
        ran=$((ran + 1))
      else
        skipped=$((skipped + 1))
      fi
    done < <(
      LC_ALL=C find "${corpus_root}/${domain}" -mindepth 1 -maxdepth 1 \
        -type d -exec test -f '{}/manifest.json' ';' -print | LC_ALL=C sort
    )
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

warn "ran $ran case(s), skipped $skipped case(s)"
if [[ -n "$required_profile" && "$ran" -ne "${#selected_cases[@]}" ]]; then
  fail "profile selected ${#selected_cases[@]} case(s), but $ran ran"
fi
# A skipped case captured nothing, so exiting 0 would report a run that did no
# work as a clean one -- and it did exactly that once: a priming job that
# failed to build its format skipped every case of that profile while this
# script still succeeded, and the stale captures on disk from the previous run
# made the no-op look like a successful re-capture. A skip is a failure to
# capture, so it fails the run. `docs/testing_policy.md`'s rule that a skipped
# case must never read as a pass applies to the capture step too.
if [[ "$skipped" -gt 0 ]]; then
  warn "a skipped case captured nothing; any capture already on disk for it is stale"
  overall_status=1
fi
exit "$overall_status"
