#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/sync-github-issues.sh [options] [-- bd-github-sync-args...]

Sync GitHub issues with local Beads state.

The script first runs `bd github sync`, then:
  - closes GitHub issues whose local bead status is `closed`
  - reopens GitHub issues whose local bead status is not `closed`
  - creates one GitHub label per epic and adds it to the epic and descendants
  - creates one GitHub Projects v2 project per epic and adds epic issues to it

Options:
  --repo OWNER/REPO          GitHub repo for issues. Defaults to git remote
                             origin, with `gh repo view` as a fallback.
  --project-owner OWNER      Owner for GitHub Projects v2. Defaults to repo owner.
  --epic-label-prefix TEXT   Prefix for epic labels. Default: epic:
  --project-prefix TEXT      Prefix for project titles. Default: Epic
  --dry-run                  Print commands without mutating GitHub or Beads.
  --no-status                Skip GitHub issue close/reopen propagation.
  --no-labels                Skip epic label creation/application.
  --no-projects              Skip GitHub project creation/item membership.
  -h, --help                 Show this help.

Arguments after `--` are passed to `bd github sync`.

Prerequisites:
  bd, gh, jq, and GitHub CLI authentication. If GH_TOKEN is unset but
  GITHUB_TOKEN is set, this script exports GH_TOKEN=GITHUB_TOKEN for gh.
USAGE
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

repo=""
project_owner=""
epic_label_prefix="epic:"
project_prefix="Epic"
dry_run=0
sync_status=1
sync_labels=1
sync_projects=1
bd_sync_args=()

while (($# > 0)); do
  case "$1" in
    --repo)
      repo="${2:?missing value for --repo}"
      shift 2
      ;;
    --project-owner)
      project_owner="${2:?missing value for --project-owner}"
      shift 2
      ;;
    --epic-label-prefix)
      epic_label_prefix="${2:?missing value for --epic-label-prefix}"
      shift 2
      ;;
    --project-prefix)
      project_prefix="${2:?missing value for --project-prefix}"
      shift 2
      ;;
    --dry-run)
      dry_run=1
      shift
      ;;
    --no-status)
      sync_status=0
      shift
      ;;
    --no-labels)
      sync_labels=0
      shift
      ;;
    --no-projects)
      sync_projects=0
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      bd_sync_args=("$@")
      break
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: required command not found: $1" >&2
    exit 1
  fi
}

run() {
  if ((dry_run)); then
    printf '+'
    printf ' %q' "$@"
    printf '\n'
  else
    "$@"
  fi
}

capture_or_empty() {
  if ((dry_run)); then
    return 0
  fi
  "$@" 2>/dev/null || true
}

need_cmd bd
need_cmd gh
need_cmd jq

if [[ -z "${GH_TOKEN:-}" && -n "${GITHUB_TOKEN:-}" ]]; then
  export GH_TOKEN="$GITHUB_TOKEN"
fi

if [[ -z "$repo" ]]; then
  origin_url="$(git remote get-url origin 2>/dev/null || true)"
  repo="$(sed -E \
    -e 's#^git@github.com:##' \
    -e 's#^ssh://git@github.com/##' \
    -e 's#^https://github.com/##' \
    -e 's#\.git$##' \
    <<<"$origin_url")"
fi

if [[ -z "$repo" ]]; then
  repo="$(capture_or_empty gh repo view --json owner,name --jq '.owner.login + "/" + .name')"
fi

if [[ ! "$repo" =~ ^[^/]+/[^/]+$ ]]; then
  echo "error: could not determine GitHub repo; pass --repo OWNER/REPO" >&2
  exit 1
fi

repo_owner="${repo%%/*}"
if [[ -z "$project_owner" ]]; then
  project_owner="$repo_owner"
fi

if ! ((dry_run)) && ! gh auth status >/dev/null 2>&1; then
  echo "error: gh is not authenticated. Run 'gh auth login' or set GH_TOKEN." >&2
  exit 1
fi

workdir="$(mktemp -d "${TMPDIR:-/tmp}/beads-github-sync.XXXXXX")"
trap 'rm -rf "$workdir"' EXIT

echo "==> Running bd github sync"
if ((dry_run)); then
  bd_sync_cmd=(bd github sync --dry-run)
  if ((${#bd_sync_args[@]})); then
    bd_sync_cmd+=("${bd_sync_args[@]}")
  fi
  run "${bd_sync_cmd[@]}"
else
  bd_sync_cmd=(bd github sync)
  if ((${#bd_sync_args[@]})); then
    bd_sync_cmd+=("${bd_sync_args[@]}")
  fi
  "${bd_sync_cmd[@]}"
fi

issues_json="$workdir/issues.json"
all_records_tsv="$workdir/all-records.tsv"
records_tsv="$workdir/records.tsv"
label_targets_tsv="$workdir/label-targets.tsv"
project_targets_tsv="$workdir/project-targets.tsv"

echo "==> Loading local Beads issues"
bd list --all --limit 0 --long --json >"$issues_json"
issue_count="$(jq '
  if type == "array" then length
  elif (.issues? | type) == "array" then .issues | length
  elif (.items? | type) == "array" then .items | length
  else 0
  end
' "$issues_json")"
echo "Loaded $issue_count Beads issues from local state"

jq -r \
  --arg repo "$repo" \
  --arg label_prefix "$epic_label_prefix" \
  --arg project_prefix "$project_prefix" \
  '
  def issues:
    if type == "array" then .
    elif (.issues? | type) == "array" then .issues
    elif (.items? | type) == "array" then .items
    else []
    end;

  def field($names):
    . as $i
    | ([$names[] as $name | $i[$name]? // empty] | first // null);

  def parent_id:
    field(["parent_id", "parentId", "parent"]) // .parent.id? // .parent.issue_id?;

  def github_number:
    ([
      field(["github_number", "githubIssueNumber", "github_issue_number"])
      , .external_ref?
      , .externalRef?
      , .github.number?
      , .github.issue_number?
      , .metadata.github.number?
      , .metadata.github.issue_number?
      , .metadata.github_number?
      , .metadata.github_issue_number?
      , .metadata["github.number"]?
      , .metadata["github.issue_number"]?
      , (.external_refs[]? | select(((.provider? // .source? // .type? // "") | ascii_downcase) == "github") | (.number? // .issue_number?))
      , (.links[]? | select(((.provider? // .source? // .type? // "") | ascii_downcase) == "github") | (.number? // .issue_number?))
    ] | map(select(. != null and . != "")) | first // null) as $n
    | if $n == null then null else ($n | tostring | capture("(?<n>[0-9]+)").n) end;

  def github_url($number):
    ([
      field(["github_url", "githubUrl"])
      , .external_ref?
      , .externalRef?
      , .github.url?
      , .metadata.github.url?
      , .metadata.github_url?
      , .metadata["github.url"]?
      , (.external_refs[]? | select(((.provider? // .source? // .type? // "") | ascii_downcase) == "github") | .url?)
      , (.links[]? | select(((.provider? // .source? // .type? // "") | ascii_downcase) == "github") | .url?)
      , if $number == null then null else "https://github.com/\($repo)/issues/\($number)" end
    ] | map(select(. != null and . != "")) | first // null);

  def norm_issue:
    . as $i
    | (github_number) as $number
    | {
        id: (($i.id // $i.issue_id // "") | tostring),
        title: (($i.title // "") | tostring),
        type: (($i.type // $i.issue_type // "") | tostring | ascii_downcase),
        status: (($i.status // "") | tostring | ascii_downcase),
        parent: (parent_id // null),
        number: $number,
        url: github_url($number)
      };

  def nearest_epic($byid; $seen):
    . as $issue
    | if $issue == null or $issue.id == null or ($issue.id == "") then null
      elif ($seen | index($issue.id)) then null
      elif $issue.type == "epic" then $issue
      elif $issue.parent == null then null
      else ($byid[$issue.parent] | nearest_epic($byid; $seen + [$issue.id]))
      end;

  [issues[] | norm_issue] as $records
  | ($records | INDEX(.id)) as $byid
  | $records[]
  | (. | nearest_epic($byid; [])) as $epic
  | (
      if .number == null then "no GitHub issue number"
      elif $epic == null then "no epic ancestry"
      else ""
      end
    ) as $skip_reason
  | [
      .id,
      (.number // ""),
      (.url // ""),
      .status,
      .title,
      ($epic.id // ""),
      ($epic.title // ""),
      (if $epic == null then "" else ($label_prefix + $epic.id) end),
      (if $epic == null then "" else ($project_prefix + " " + $epic.id + ": " + $epic.title) end),
      $skip_reason
    ]
  | @tsv
  ' "$issues_json" >"$all_records_tsv"

awk -F '\t' '$10 == "" { print }' "$all_records_tsv" >"$records_tsv"
awk -F '\t' '$10 == "" && !seen[$8]++ { print $8 "\t" $7 }' "$all_records_tsv" >"$label_targets_tsv"
awk -F '\t' '$10 == "" && !seen[$9]++ { print $9 }' "$all_records_tsv" >"$project_targets_tsv"

count_lines() {
  wc -l <"$1" | tr -d ' '
}

synced_issue_count="$(awk -F '\t' '$2 != "" { count++ } END { print count + 0 }' "$all_records_tsv")"
target_count="$(count_lines "$records_tsv")"
label_count="$(count_lines "$label_targets_tsv")"
project_count="$(count_lines "$project_targets_tsv")"
epic_count="$(awk -F '\t' '$10 == "" && !seen[$6]++ { count++ } END { print count + 0 }' "$all_records_tsv")"
missing_github_count="$(awk -F '\t' '$10 == "no GitHub issue number" { count++ } END { print count + 0 }' "$all_records_tsv")"
missing_epic_count="$(awk -F '\t' '$10 == "no epic ancestry" { count++ } END { print count + 0 }' "$all_records_tsv")"

echo "Found $synced_issue_count Beads issues with GitHub issue numbers"
echo "Targeting $target_count GitHub issues across $epic_count epics"
echo "Will ensure $label_count epic labels and $project_count epic projects"
echo "Skipped $missing_github_count Beads issues with no GitHub issue number"
echo "Skipped $missing_epic_count synced GitHub issues with no epic ancestry"

if [[ ! -s "$records_tsv" ]]; then
  echo "No synced GitHub issues with epic ancestry found."
  exit 0
fi

ensure_label() {
  local label="$1"
  local epic_title="$2"

  if ((dry_run)); then
    run gh label create "$label" \
      --repo "$repo" \
      --color "6A737D" \
      --description "Beads epic: $epic_title" \
      --force
  else
    gh label create "$label" \
      --repo "$repo" \
      --color "6A737D" \
      --description "Beads epic: $epic_title" \
      --force >/dev/null
  fi
}

project_number_for_title() {
  local title="$1"
  local number

  number="$(gh project list \
    --owner "$project_owner" \
    --limit 1000 \
    --format json 2>/dev/null \
    | jq -r --arg title "$title" 'first(.projects[] | select(.title == $title) | .number) // empty')"

  if [[ -n "$number" ]]; then
    printf '%s\n' "$number"
    return 0
  fi

  gh project create \
    --owner "$project_owner" \
    --title "$title" \
    --format json \
    --jq '.number'
}

labels_done="$workdir/labels-done"
projects_done="$workdir/projects-done.tsv"
: >"$labels_done"
: >"$projects_done"

if ((sync_status)); then
  echo "==> Syncing GitHub issue open/closed state ($target_count issues)"
  status_index=0
  status_changed=0
  status_unchanged=0
  while IFS=$'\t' read -r bead_id issue_number _issue_url bead_status title _epic_id _epic_title _epic_label _project_title _skip_reason; do
    status_index=$((status_index + 1))
    if ((dry_run)); then
      echo "[$status_index/$target_count] Status #$issue_number from $bead_id: local=$bead_status (dry-run; would inspect GitHub)"
      continue
    fi

    github_state="$(gh issue view "$issue_number" --repo "$repo" --json state --jq '.state')"
    if [[ "$bead_status" == "closed" && "$github_state" != "CLOSED" ]]; then
      echo "[$status_index/$target_count] Closing #$issue_number from $bead_id: $title"
      run gh issue close "$issue_number" --repo "$repo" --reason completed
      status_changed=$((status_changed + 1))
    elif [[ "$bead_status" != "closed" && "$github_state" == "CLOSED" ]]; then
      echo "[$status_index/$target_count] Reopening #$issue_number from $bead_id: $title"
      run gh issue reopen "$issue_number" --repo "$repo"
      status_changed=$((status_changed + 1))
    else
      echo "[$status_index/$target_count] Status already aligned for #$issue_number from $bead_id: local=$bead_status github=$github_state"
      status_unchanged=$((status_unchanged + 1))
    fi
  done <"$records_tsv"
  if ((dry_run)); then
    echo "Status phase dry-run complete: would compare $target_count issues"
  else
    echo "Status phase complete: changed $status_changed, already aligned $status_unchanged"
  fi
else
  echo "==> Skipping GitHub issue open/closed state (--no-status)"
fi

if ((sync_labels)); then
  echo "==> Ensuring epic labels ($label_count labels)"
  label_index=0
  while IFS=$'\t' read -r epic_label epic_title; do
    label_index=$((label_index + 1))
    echo "[$label_index/$label_count] Ensuring label $epic_label"
    ensure_label "$epic_label" "$epic_title"
    printf '%s\n' "$epic_label" >>"$labels_done"
  done <"$label_targets_tsv"

  echo "==> Applying epic labels to issues ($target_count issues)"
  issue_index=0
  while IFS=$'\t' read -r bead_id issue_number _issue_url _bead_status _title _epic_id _epic_title epic_label _project_title _skip_reason; do
    issue_index=$((issue_index + 1))
    echo "[$issue_index/$target_count] Adding $epic_label to #$issue_number from $bead_id"
    if ((dry_run)); then
      run gh issue edit "$issue_number" --repo "$repo" --add-label "$epic_label"
    else
      gh issue edit "$issue_number" --repo "$repo" --add-label "$epic_label" >/dev/null
    fi
  done <"$records_tsv"
  echo "Label phase complete: ensured $label_count labels, applied labels to $target_count issues"
else
  echo "==> Skipping epic labels (--no-labels)"
fi

if ((sync_projects)); then
  echo "==> Ensuring epic projects ($project_count projects)"
  project_index=0
  while IFS=$'\t' read -r project_title; do
    project_index=$((project_index + 1))
    echo "[$project_index/$project_count] Ensuring project $project_title"
    if ((dry_run)); then
      run gh project create --owner "$project_owner" --title "$project_title"
      project_number="DRY-RUN"
    else
      project_number="$(project_number_for_title "$project_title")"
    fi
    printf '%s\t%s\n' "$project_title" "$project_number" >>"$projects_done"
  done <"$project_targets_tsv"

  echo "==> Adding issues to epic projects ($target_count issues)"
  issue_index=0
  while IFS=$'\t' read -r bead_id issue_number issue_url _bead_status _title _epic_id _epic_title _epic_label project_title _skip_reason; do
    issue_index=$((issue_index + 1))
    project_number="$(awk -F '\t' -v title="$project_title" '$1 == title { print $2; exit }' "$projects_done")"
    echo "[$issue_index/$target_count] Adding #$issue_number from $bead_id to project $project_title"
    if ((dry_run)); then
      run gh project item-add "<project-number>" --owner "$project_owner" --url "$issue_url"
    else
      gh project item-add "$project_number" \
        --owner "$project_owner" \
        --url "$issue_url" >/dev/null
    fi
  done <"$records_tsv"
  echo "Project phase complete: ensured $project_count projects, added $target_count issue items"
else
  echo "==> Skipping epic projects (--no-projects)"
fi

echo "==> GitHub issue status, epic labels, and epic projects are synced"
