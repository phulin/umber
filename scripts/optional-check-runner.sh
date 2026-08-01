# shellcheck shell=bash
#
# Named-step execution shared by opt-in checks. Callers declare their steps,
# run every selected step even after failures, and finish with one verdict.

check_name=""
check_all_steps=()
check_selected_steps=()
check_ran=0
check_failed_steps=()
check_blocked_steps=()
check_blockers=()
check_transcript=()

CHECK_EXIT_PASS=0
CHECK_EXIT_FAIL=1
CHECK_EXIT_USAGE=2
CHECK_EXIT_BLOCKED=4

# optional_check_begin <check> <step>...
#
# `OPTIONAL_CHECK_ARGS` is a whitespace-separated list of steps. An empty list
# selects every declared step.
optional_check_begin() {
  check_name="$1"
  shift
  check_all_steps=("$@")
  check_selected_steps=()
  check_ran=0
  check_failed_steps=()
  check_blocked_steps=()
  check_blockers=()
  check_transcript=()

  local requested
  for requested in ${OPTIONAL_CHECK_ARGS:-}; do
    case " ${check_all_steps[*]} " in
      *" $requested "*) check_selected_steps+=("$requested") ;;
      *)
        printf '%s: unknown step %q\n' "$check_name" "$requested" >&2
        printf '%s: steps: %s\n' "$check_name" "${check_all_steps[*]}" >&2
        exit "$CHECK_EXIT_USAGE"
        ;;
    esac
  done

  if ((${#check_selected_steps[@]} == 0)); then
    check_selected_steps=("${check_all_steps[@]}")
  fi
}

optional_check_is_selected() {
  case " ${check_selected_steps[*]} " in
    *" $1 "*) return 0 ;;
    *) return 1 ;;
  esac
}

# optional_check_step <step> <command>...
optional_check_step() {
  optional_check_step_requiring "" "$@"
}

# optional_check_step_requiring "<tool>..." <step> <command>...
optional_check_step_requiring() {
  local tools="$1" step="$2"
  shift 2
  optional_check_is_selected "$step" || return 0

  local missing=() tool
  for tool in $tools; do
    command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
  done
  if ((${#missing[@]} > 0)); then
    check_blocked_steps+=("$step")
    check_blockers+=("$step needs ${missing[*]}, not installed here")
    check_transcript+=("BLOCKED $step (missing: ${missing[*]})")
    printf '\n=== %s step: %s -- BLOCKED, missing: %s\n' \
      "$check_name" "$step" "${missing[*]}" >&2
    return 0
  fi

  printf '\n=== %s step: %s\n' "$check_name" "$step"
  local status=0
  "$@" || status=$?
  check_ran=$((check_ran + 1))
  if ((status != 0)); then
    check_failed_steps+=("$step")
    check_transcript+=("FAILED  $step (exit $status)")
  else
    check_transcript+=("ok      $step")
  fi
}

optional_check_finish() {
  local total=${#check_all_steps[@]}
  local selected=${#check_selected_steps[@]}
  local status verdict

  printf '\n%s: steps:\n' "$check_name"
  if ((${#check_transcript[@]} > 0)); then
    printf '  %s\n' "${check_transcript[@]}"
  fi

  local census="$check_ran of $total steps ran"
  if ((selected < total)); then
    census="$census; $((total - selected)) not selected"
  fi

  if ((${#check_failed_steps[@]} > 0)); then
    status=$CHECK_EXIT_FAIL
    verdict="FAIL"
    census="$census, ${#check_failed_steps[@]} failed: ${check_failed_steps[*]}"
  elif ((${#check_blocked_steps[@]} > 0)); then
    status=$CHECK_EXIT_BLOCKED
    verdict="BLOCKED"
    local reasons="${check_blockers[0]}" index
    for ((index = 1; index < ${#check_blockers[@]}; index++)); do
      reasons="$reasons; ${check_blockers[index]}"
    done
    census="$census, ${#check_blocked_steps[@]} could not run: $reasons"
  elif ((selected < total)); then
    status=$CHECK_EXIT_PASS
    verdict="PARTIAL"
  else
    status=$CHECK_EXIT_PASS
    verdict="PASS"
  fi

  local line
  line="$(printf '%s: VERDICT: %s - %s' "$check_name" "$verdict" "$census")"
  if ((status == CHECK_EXIT_PASS)); then
    printf '\n%s\n' "$line"
  else
    printf '\n%s\n' "$line" >&2
  fi
  exit "$status"
}
