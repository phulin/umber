---
name: parallel
description: Coordinate multiple disjoint Umber issues in bounded persistent Git worktree slots with checkout-local Cargo caches, explicit slot ownership, linear in-slot rebases, fast-forward integration, and delegated conflict resolution. Use when dispatching parallel subagents, allocating or recycling worktree slots, preserving build caches across issues, integrating parallel branches, or resolving rebase conflicts.
---

# Parallel

Use this skill only when coordinating multiple Umber subagents at once. Parallel
work is allowed only when the issues touch disjoint subsystems. Serialize
overlapping work with the `coordinate` skill.

Give every active agent its own worktree. Use a bounded pool of persistent
worker slots by default: keep each slot at a stable filesystem path with its own
`target/` and `target/clippy`, but give every issue a fresh branch at an explicit
base commit. Never share one `CARGO_TARGET_DIR` across concurrent agents.

The coordinator owns slot allocation, integration, and release. An
implementation agent never creates, removes, reassigns, cleans, or switches its
slot.

## Non-Negotiable Rules

- Pin every wave to an explicit base commit, not a moving branch name.
- Give one slot to exactly one issue until writeback and integration finish.
- Write every coordinator Git command with an explicit `git -C <path>`.
- Keep all edits, tests, commits, and rebase conflict resolution in the assigned
  slot. Do not let a parallel agent modify the integration checkout.
- Namespace shared scratch files by issue id. Use `johp-208-after.txt`, not
  `after.txt`.
- Keep history linear: rebase the issue branch in its slot, then fast-forward
  the integration branch. Never create a merge commit.
- Do not regenerate, purge, or mutate shared corpus, distribution, oracle, or
  cache state while another agent may be reading it.

## Before A Wave

1. Confirm the issues are unblocked, claimed in Beads, and touch disjoint
   subsystems.
2. Choose at most the available concurrency and run:

   ```bash
   scripts/build-cache-policy.py --jobs N
   ```

   The current preflight deliberately reserves 12 GiB per concurrent job plus
   4 GiB. Reduce the wave or reclaim an idle slot if it refuses.
3. Resolve the integration branch to one explicit `{BASE_REF}` commit shared by
   the wave.
4. Allocate one idle persistent slot per issue. If the bounded pool has no idle
   slot, wait or serialize; do not grow it without another capacity check.

## Persistent Slot Allocation

Use stable paths such as `{REPO_ROOT}/.worktrees/slot-1`. Slot paths must not
contain issue ids because their path identity is what preserves Cargo's local
crate and dep-info cache across issues.

The coordinator allocates each slot before dispatch:

1. Ensure `{REPO_ROOT}/.worktrees` and
   `{REPO_ROOT}/.worktrees/.locks` exist.
2. Acquire the slot by atomically creating
   `{REPO_ROOT}/.worktrees/.locks/{SLOT_NAME}`. If it already exists, treat the
   slot as owned until Beads, the worktree registry, live agents, and
   Cargo-family processes prove the lock stale. Never clear it merely because
   no agent is visible in the current thread.
3. Create a missing slot once, detached at the pinned base:

   ```bash
   git -C {REPO_ROOT} worktree add {SLOT_PATH} --detach {BASE_REF}
   ```

4. For an existing slot, require a clean worktree, no active owner or build
   process, and a detached idle HEAD. Resolve any abandoned issue through its
   Beads record before reuse.
5. Create the issue branch inside the slot:

   ```bash
   git -C {SLOT_PATH} switch -c umber-{ISSUE_SLUG} {BASE_REF}
   ```

   To resume an existing issue, attach only its recorded branch after proving
   no other worktree has it checked out.
6. Append the slot path, issue branch, and pinned base to the issue's Beads
   notes. Beads plus the lock directory are the durable ownership record.

Prewarm a newly created or explicitly reclaimed slot before starting the
parallel wave. Warm new slots sequentially so their cold builds do not contend:

```bash
cargo test -q --tests --no-run
scripts/check.sh clippy
```

Run those commands with the slot as the command working directory. Normal slot
reuse needs no prewarm and no clean.

## Dispatch Prompt Block

Add this block to each parallel implementation prompt after the standard
`coordinate` prompt, filling every placeholder:

```markdown
## Assigned persistent worktree

- Main repository: {REPO_ROOT}
- Assigned slot: {SLOT_PATH}
- Issue branch: umber-{ISSUE_SLUG}
- Pinned base: {BASE_REF}

The coordinator has already allocated this slot and checked out the issue
branch. Before reading docs or editing, `cd` to {SLOT_PATH} and verify the
branch and base. Do not create or remove a worktree, switch branches, change
`CARGO_TARGET_DIR`, clean build caches, or modify the main checkout. Keep every
edit, test, and commit in this slot. Use full in-slot paths for patch tools.

Namespace every shared scratch artifact with {ISSUE_ID}. If a required
gitignored asset is missing, check the primary checkout and the owning workflow
before calling it a repository failure; do not improvise symlinks or regenerate
shared evidence.
```

## Gitignored Assets And Specialized Workflows

Routine native conformance assets provision themselves on first use through
`test_support::native_assets::provision`. It copies only the SHA-256 allowlist
in `tests/native-test-assets.lock` from the primary checkout. Do not symlink or
broadly copy `third_party/`, DVI oracles, or TRIP inputs into a slot. If the
primary checkout lacks an allowlisted asset, run
`scripts/setup-conformance-tests.sh` there once, never in each slot.

Some opt-in workflows use additional gitignored material that the native asset
provisioner does not own. Follow the affected subsystem's current documentation
and provision only the exact paths it names. Treat a missing worktree asset as
an incomplete slot setup until the primary checkout has been checked.

For canonical-command semantic or DVI divergence work, read
`docs/canonical_divergence_workflow.md` and use its oracle, tracer, expected
total, stream, and front requirements. Do not run the command-stream tracer or
inject divergence-specific setup into unrelated parallel tasks.

## While Slots Are Active

- Keep the slot lock until the branch has been integrated or its exact
  unfinished disposition has been recorded.
- Never switch, detach, reclaim, or inspect an active slot in a way that can
  alter it. Send questions to its owner instead.
- Let each slot use its checkout-local `target/`; `scripts/check.sh` already
  isolates clippy in that slot's `target/clippy`.
- Do not copy, hard-link, reflink, or share Cargo internals between slots.
- If an agent stops unexpectedly, preserve the branch, slot, lock, and Beads
  state until recovery is complete.

The shell working directory can persist across tool calls. Never rely on it.
Use `git -C {REPO_ROOT} ...` for the integration checkout and
`git -C {SLOT_PATH} ...` for the issue branch. Do not use a pipeline such as
`git ... | head && echo CLEAN` as a cleanliness proof; a pipeline can hide the
Git command's failure status.

## Integration After Writeback

Integrate only after the `coordinate` writeback checks pass. Process completed
slots one at a time because each later branch must rebase onto the integration
tip produced by the prior one.

1. Record the tested issue tip as `{PRE_REBASE_REF}` and resolve the current
   integration tip as `{INTEGRATION_TIP}`.
2. Rebase inside the issue slot, where its branch is already checked out:

   ```bash
   git -C {SLOT_PATH} rebase {INTEGRATION_TIP}
   ```

3. If `git diff {PRE_REBASE_REF} HEAD` is not empty, the tested tree changed.
   Run the relevant focused tests and `scripts/check-and-test.sh` again in the
   slot. Reuse prior green evidence only when that tree diff is empty.
4. Fast-forward the integration branch from the primary checkout:

   ```bash
   git -C {REPO_ROOT} checkout {INTEGRATION_BRANCH}
   git -C {REPO_ROOT} merge --ff-only umber-{ISSUE_SLUG}
   ```

   If this fails because the integration tip advanced, do not force it or call
   it a conflict. Resolve the new tip and repeat the in-slot rebase and
   verification.
5. Record the resulting commit range on the issue or owning epic.

Do not cherry-pick a normal completed slot merely because its branch is checked
out there. Rebasing in the slot preserves the intended branch and makes the
fast-forward path routine.

## Release Or Retire A Slot

After fast-forward integration, recycle the slot instead of deleting it:

1. Resolve the new integration tip and detach the clean slot there:

   ```bash
   git -C {SLOT_PATH} switch --detach {INTEGRATION_TIP}
   git -C {REPO_ROOT} branch -d umber-{ISSUE_SLUG}
   ```

2. Verify clean status, no live agent, and no Cargo-family process belonging to
   the slot.
3. Append the release and integrated commit range to Beads, then remove the
   slot's lock directory. The detached worktree and both Cargo caches remain
   available for the next issue.

Reclamation is explicit and applies only to an idle locked slot. Run
`scripts/build-cache-policy.py --reclaim --jobs N` with that slot as the
working directory after checking for live owners and processes. Reclamation
makes later work colder, so choose the least-recently-used idle slot.

Retire a physical slot only to shrink the bounded pool or recover more space
than cache reclamation provides. Require it to be idle, clean, detached, and
process-free, then use `git -C {REPO_ROOT} worktree remove {SLOT_PATH}`. Never
remove an active or unresolved slot.

## Rebase Conflicts

The coordinator does not resolve conflicts. If the in-slot rebase conflicts:

1. Leave the rebase in progress in `{SLOT_PATH}`. Do not run `rebase --abort`
   unless abandoning the integration attempt.
2. Reopen the issue and dispatch a conflict-resolution subagent into the same
   locked slot. No implementation agent may use that slot concurrently.
3. Give the resolver context about the issue branch and the changes already on
   the integration branch.
4. Require the resolver to preserve both issues' intent, complete the rebase,
   run focused tests and `scripts/check-and-test.sh`, update Beads, and re-close
   the issue.
5. Resume the normal fast-forward and slot-release sequence.

Use this prompt:

```markdown
You are resolving a Git rebase conflict for Umber. Work only in the assigned
persistent slot and do not broaden scope.

**Slot:** {SLOT_PATH}
**Branch:** umber-{ISSUE_SLUG}
**Integration tip:** {INTEGRATION_TIP}
**Issue:** {ISSUE_ID} -- {ISSUE_TITLE}
{ISSUE_DESCRIPTION_AND_ACCEPTANCE_CRITERIA}

**Already integrated:** {RECENT_ISSUES_BRANCHES_SUBSYSTEMS_AND_INTENT}

1. Inspect the in-progress rebase and conflict markers in the slot.
2. Resolve conflicts while preserving both sides' intent.
3. Run the relevant focused tests and `scripts/check-and-test.sh`.
4. Complete the rebase without a merge commit.
5. Comment on {ISSUE_ID} with the resolution and validation, then re-close it.

Report in at most 15 lines: files resolved, tests, resulting commit range, and
Beads writeback. Do not include diffs or logs.
```
