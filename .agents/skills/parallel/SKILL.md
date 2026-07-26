---
name: parallel
description: Manage multiple umber implementation agents at once using separate git worktrees, then rebase-integrate, tear down, and resolve conflicts through a dedicated conflict-resolution subagent. Use when coordinating parallel subagents, worktree branches, branch integration, or merge conflicts.
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

1. Main repo: {REPO_ROOT} (the coordinator's primary working directory; do not
   hardcode a path here when dispatching -- substitute the real one)
2. Ensure the worktree parent exists:
   `mkdir -p {REPO_ROOT}/.worktrees`
3. Create a dedicated worktree and branch:
   `git -C {REPO_ROOT} worktree add {REPO_ROOT}/.worktrees/umber-{ISSUE_SLUG} -b umber-{ISSUE_SLUG} {BASE_REF}`
   If the branch already exists without a worktree, attach with:
   `git -C {REPO_ROOT} worktree add {REPO_ROOT}/.worktrees/umber-{ISSUE_SLUG} umber-{ISSUE_SLUG}`
4. {OPTIONAL: import partial WIP from a prior wave; list files/stashes}
5. `cd` into the worktree; all edits, tests, and commits happen there only.
   Do not modify the main checkout. If you have an `apply_patch` skill, it
   needs the full path to the in-worktree file every time you call it.
```

`{BASE_REF}` is the current tip of the branch the work integrates onto. Pin it
to an explicit commit hash rather than a branch name so concurrent jobs share a
known base. Use `main` only when that is genuinely the integration branch;
long-running epics integrate onto their own feature branch instead.

### Disk cost

Each worktree carries its own `target/`, roughly 7 GB once built. Check free
space before dispatching a wave (`df -h`) and do not start a new worktree job
below about 8 GB free. Tear worktrees down promptly after merge; an abandoned
wave can exhaust the disk and stall every running agent at once.

## After Writeback Verification

For parallel worktree jobs only, integrate after the subagent's writeback
passes. **Keep history linear -- rebase, never `git merge`.** From the main
checkout on the integration branch:

```bash
git -C {REPO_ROOT} rebase {INTEGRATION_BRANCH} umber-{ISSUE_SLUG}
git -C {REPO_ROOT} checkout {INTEGRATION_BRANCH}
git -C {REPO_ROOT} merge --ff-only umber-{ISSUE_SLUG}
git -C {REPO_ROOT} worktree remove {REPO_ROOT}/.worktrees/umber-{ISSUE_SLUG}
git -C {REPO_ROOT} branch -d umber-{ISSUE_SLUG}
```

If the branch is checked out in its worktree and so cannot be rebased in place,
cherry-pick its commits onto the integration branch instead, then remove the
worktree and force-delete the branch.

Record the resulting commit range on the relevant bd issue or epic. When a
rebase changes commits that a prior green test run covered, confirm the tree is
unchanged (`git diff {PRE_REBASE_REF} HEAD` empty) before reusing that result.

## Merge Conflicts

Do not resolve conflicts yourself. If the rebase or fast-forward fails:

1. Leave the rebase in progress. Run `git rebase --abort` only if abandoning it
   entirely.
2. Dispatch a conflict-resolution subagent with context about both sides:
   the issue whose branch is being integrated (`{ISSUE_ID}`, title, subsystems,
   acceptance criteria) and what is already on the integration branch from
   recently landed issues, listing issue ids, branch names, and subsystems
   touched.
3. The conflict-resolution subagent works in the main checkout, not a worktree.
   It resolves conflicts preserving intent of both sides, runs
   `cargo test --tests`, completes the rebase, and reports back.
4. Once the branch fast-forwards cleanly, remove the worktree and delete the
   branch as described above.

## Conflict-Resolution Subagent Prompt

Dispatch this prompt when `git merge` of a completed parallel worktree branch
fails. The subagent works in the main repo checkout on `main` with the merge in
progress.

```markdown
You are resolving a git rebase conflict for umber. Do not change scope
beyond what is required to complete the rebase correctly.

**Branch being integrated:** {BRANCH} from worktree {WORKTREE_PATH}
**Integration branch:** {INTEGRATION_BRANCH}
**Issue:** {ISSUE_ID} -- {ISSUE_TITLE}
{ISSUE_DESCRIPTION; subsystems and acceptance criteria}

**Already on the integration branch (conflicting side):** {LIST landed
issue ids, branch names, subsystems, and one-line intent for each}

1. Inspect `git status` and conflict markers; understand both sides.
2. Resolve conflicts preserving the intent of both issues. Prefer
   integrating both behaviors over discarding either side.
3. `cargo test -q --tests` must pass; clippy and rustfmt clean.
4. Complete the rebase; keep history linear and introduce no merge commit.
5. Comment on {ISSUE_ID} in bd noting conflict resolution approach.

Report in <=15 lines: conflicts resolved (file paths), tests, resulting
commit range. No diffs.
```
