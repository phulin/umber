# shellcheck shell=bash
#
# Shared step accounting for the deferred test tiers.
#
# The tiers this file backs are the ones the routine gate cannot run: they need
# ripgrep, wasm-pack, a headless browser, HarfBuzz, or three extra dependency
# trees. Before umber2-johp.213 each of them was a bare `set -e` script whose
# only output on success was the absence of output, so a reader could not tell
# a whole run from a run that stopped at its first missing prerequisite, and
# nothing outside the terminal recorded that it had run at all.
#
# Sourcing this file gives a tier three things:
#
#   * named steps, so the run ends in a verdict line stating which steps ran
#     rather than in silence;
#   * a distinct BLOCKED outcome for a step whose tool is absent, which never
#     exits 0 -- "skipped because the tool is missing, exit 0" is the same
#     defect as a check that does not run (umber2-johp.210);
#   * a stamp under `.tier-stamps/`, written from this accounting, so the
#     routine gates can print what this tier last did instead of asserting it.
#
# Every gate runs even after one fails, as in scripts/check.sh: a tier that
# stopped at its first failure reports one problem and hides the rest.
#
# Exit status:
#
#   0  PASS     every selected step ran and passed
#   1  FAIL     a step ran and failed
#   2  usage    the command line named a step this tier does not have
#   4  BLOCKED  a step could not run because a prerequisite is absent

tier_name=""
tier_all_steps=()
tier_selected_steps=()
tier_ran=0
tier_failed_steps=()
tier_blocked_steps=()
tier_blockers=()
tier_transcript=()

TIER_EXIT_PASS=0
TIER_EXIT_FAIL=1
TIER_EXIT_USAGE=2
TIER_EXIT_BLOCKED=4

# tier_begin <tier> <step>...
#
# Declares the tier's complete step list, then resolves the selection from the
# caller's arguments in TIER_ARGS. A run that selects a subset is recorded as a
# subset, so naming steps cannot be mistaken for having run the tier.
tier_begin() {
  tier_name="$1"
  shift
  tier_all_steps=("$@")
  tier_selected_steps=()

  local requested
  for requested in ${TIER_ARGS:-}; do
    case " ${tier_all_steps[*]} " in
      *" $requested "*) tier_selected_steps+=("$requested") ;;
      *)
        printf '%s: unknown step %q\n' "$tier_name" "$requested" >&2
        printf '%s: steps: %s\n' "$tier_name" "${tier_all_steps[*]}" >&2
        exit "$TIER_EXIT_USAGE"
        ;;
    esac
  done

  if ((${#tier_selected_steps[@]} == 0)); then
    tier_selected_steps=("${tier_all_steps[@]}")
  fi
}

tier_is_selected() {
  case " ${tier_selected_steps[*]} " in
    *" $1 "*) return 0 ;;
    *) return 1 ;;
  esac
}

# tier_step <step> <command>...
tier_step() {
  tier_step_requiring "" "$@"
}

# tier_step_requiring "<tool>..." <step> <command>...
#
# Runs the step only when every named tool is on PATH. A missing tool makes the
# step BLOCKED, never skipped: the difference between "the comparison ran and
# matched" and "the comparison never happened" is the whole point of the step.
tier_step_requiring() {
  local tools="$1" step="$2"
  shift 2
  tier_is_selected "$step" || return 0

  local missing=() tool
  for tool in $tools; do
    command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
  done
  if ((${#missing[@]} > 0)); then
    tier_blocked_steps+=("$step")
    tier_blockers+=("$step needs ${missing[*]}, not installed here")
    tier_transcript+=("BLOCKED $step (missing: ${missing[*]})")
    printf '\n=== %s step: %s -- BLOCKED, missing: %s\n' \
      "$tier_name" "$step" "${missing[*]}" >&2
    return 0
  fi

  printf '\n=== %s step: %s\n' "$tier_name" "$step"
  local status=0
  "$@" || status=$?
  tier_ran=$((tier_ran + 1))
  if ((status != 0)); then
    tier_failed_steps+=("$step")
    tier_transcript+=("FAILED  $step (exit $status)")
  else
    tier_transcript+=("ok      $step")
  fi
}

# tier_finish
#
# Prints the transcript and the verdict, records the stamp, and exits.
tier_finish() {
  local total=${#tier_all_steps[@]}
  local selected=${#tier_selected_steps[@]}
  local status verdict
  local -a record_arguments=()

  printf '\n%s: steps:\n' "$tier_name"
  if ((${#tier_transcript[@]} > 0)); then
    printf '  %s\n' "${tier_transcript[@]}"
  fi

  local census="$tier_ran of $total steps ran"
  local partial=0
  if ((selected < total)); then
    partial=1
    census="$census; $((total - selected)) not selected"
  fi

  local step blocker
  for step in "${tier_failed_steps[@]}"; do record_arguments+=(--failed "$step"); done
  for step in "${tier_blocked_steps[@]}"; do record_arguments+=(--blocked "$step"); done
  for blocker in "${tier_blockers[@]}"; do record_arguments+=(--blocker "$blocker"); done

  if ((${#tier_failed_steps[@]} > 0)); then
    status=$TIER_EXIT_FAIL
    verdict="FAIL"
    census="$census, ${#tier_failed_steps[@]} failed: ${tier_failed_steps[*]}"
  elif ((${#tier_blocked_steps[@]} > 0)); then
    status=$TIER_EXIT_BLOCKED
    verdict="BLOCKED"
    local reasons="${tier_blockers[0]}" index
    for ((index = 1; index < ${#tier_blockers[@]}; index++)); do
      reasons="$reasons; ${tier_blockers[index]}"
    done
    census="$census, ${#tier_blocked_steps[@]} could not run: $reasons"
  elif ((partial)); then
    # The named steps passed, so this exits 0 as `scripts/check.sh clippy` does.
    # It is not called PASS: a run that selected a quarter of the tier is not
    # the tier passing, and the stamp records it as the partial run it is.
    status=$TIER_EXIT_PASS
    verdict="PARTIAL"
  else
    status=$TIER_EXIT_PASS
    verdict="PASS"
  fi

  # PARTIAL is a property of the selection, which the stamp already carries as
  # `steps_selected`; the recorded status stays the three-way outcome.
  local recorded="$verdict"
  [[ $recorded == PARTIAL ]] && recorded=PASS

  python3 "$(dirname "${BASH_SOURCE[0]}")/tier_stamp.py" record "$tier_name" \
    --status "$recorded" \
    --total "$total" --selected "$selected" --ran "$tier_ran" \
    "${record_arguments[@]}"

  local line
  line="$(printf '%s: VERDICT: %s - %s' "$tier_name" "$verdict" "$census")"
  if ((status == TIER_EXIT_PASS)); then
    printf '\n%s\n' "$line"
  else
    printf '\n%s\n' "$line" >&2
  fi
  exit "$status"
}
