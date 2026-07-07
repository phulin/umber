---
name: parallel
description: Manage multiple umber implementation agents at once using separate git worktrees, then merge, tear down, and resolve conflicts through a dedicated conflict-resolution subagent. Use when coordinating parallel subagents, worktree branches, branch merges, or merge conflicts.
---

# Parallel

Use this skill only when coordinating multiple umber subagents at once.
Parallel work is allowed only when the issues touch disjoint subsystems. If the
work overlaps, serialize it with the `coordinate` skill instead.

Parallel dispatch requires a separate worktree per subagent. Never let two
subagents edit the same checkout. The coordinator dispatches worktree
instructions with those parallel jobs and handles merge/teardown after each
job's writeback passes.

## Worktree Setup Block

When running parallel subagents, add this block to each subagent prompt, filling
in `{ISSUE_SLUG}`, `{BASE_REF}`, and optional WIP-import notes:

```markdown
## Worktree setup (required first step; do before reading docs or editing)

1. Main repo: /Users/phulin/Documents/Projects/umber
2. Ensure the worktree parent exists:
   `mkdir -p /Users/phulin/Documents/Projects/umber/.worktrees`
3. Create a dedicated worktree and branch:
   `git -C /Users/phulin/Documents/Projects/umber worktree add /Users/phulin/Documents/Projects/umber/.worktrees/umber-{ISSUE_SLUG} -b umber-{ISSUE_SLUG} {BASE_REF}`
   If the branch already exists without a worktree, attach with:
   `git -C /Users/phulin/Documents/Projects/umber worktree add /Users/phulin/Documents/Projects/umber/.worktrees/umber-{ISSUE_SLUG} umber-{ISSUE_SLUG}`
4. {OPTIONAL: import partial WIP from a prior wave; list files/stashes}
5. `cd` into the worktree; all edits, tests, and commits happen there only.
   Do not modify the main checkout.
```

`{BASE_REF}` is usually `main`. Use a more specific ref only when the issue
depends on an unmerged branch, and note that in bd before dispatch.

## After Writeback Verification

For parallel worktree jobs only, merge after the subagent's writeback passes.
On the main checkout and `main` branch, run:

```bash
git -C /Users/phulin/Documents/Projects/umber checkout main
git -C /Users/phulin/Documents/Projects/umber merge umber-{ISSUE_SLUG} -m "Merge {ISSUE_ID}: {ISSUE_TITLE}."
git -C /Users/phulin/Documents/Projects/umber worktree remove /Users/phulin/Documents/Projects/umber/.worktrees/umber-{ISSUE_SLUG}
git -C /Users/phulin/Documents/Projects/umber branch -d umber-{ISSUE_SLUG}
```

Record the merge commit on the relevant bd issue or epic. If `main` has
diverged, such as math on `main` and alignments on `origin/main`, merge
prerequisites in order and note the sequence in bd before starting.

## Merge Conflicts

Do not resolve conflicts yourself. If `git merge` fails:

1. Leave the merge in progress. Run `git merge --abort` only if abandoning the
   merge entirely.
2. Dispatch a conflict-resolution subagent with context about both sides:
   the issue whose branch is being merged (`{ISSUE_ID}`, title, subsystems,
   acceptance criteria) and what is already on `main` from recently merged
   issues, listing issue ids, branch names, and subsystems touched.
3. The conflict-resolution subagent works in the main checkout, not a worktree.
   It resolves conflicts preserving intent of both sides, runs
   `cargo test --tests`, commits the merge, and reports back.
4. After a clean merge commit, remove the worktree and delete the branch as
   described above.

## Conflict-Resolution Subagent Prompt

Dispatch this prompt when `git merge` of a completed parallel worktree branch
fails. The subagent works in the main repo checkout on `main` with the merge in
progress.

```markdown
You are resolving a git merge conflict for umber. Do not change scope
beyond what is required to complete the merge correctly.

**Branch being merged:** {BRANCH} from worktree {WORKTREE_PATH}
**Issue:** {ISSUE_ID} -- {ISSUE_TITLE}
{ISSUE_DESCRIPTION; subsystems and acceptance criteria}

**Already on main (conflicting side):** {LIST merged issue ids, branch
names, subsystems, and one-line intent for each}

1. Inspect `git status` and conflict markers; understand both sides.
2. Resolve conflicts preserving the intent of both issues. Prefer
   integrating both behaviors over discarding either side.
3. `cargo test --tests` must pass; clippy and rustfmt clean.
4. Complete the merge commit with message:
   `Merge {ISSUE_ID}: {ISSUE_TITLE}.`
5. Comment on {ISSUE_ID} in bd noting conflict resolution approach.

Report in <=15 lines: conflicts resolved (file paths), tests, merge
commit hash. No diffs.
```

