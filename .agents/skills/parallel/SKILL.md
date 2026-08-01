---
name: parallel
description: Coordinate parallel Umber agents in persistent Git worktree slots. Use for slot allocation, cache reuse, linear integration, and rebase conflicts.
---

# Parallel

Use with `coordinate` when multiple ready issues touch disjoint subsystems.
Serialize overlapping work.

Maintain a bounded pool of fixed worktrees such as `.worktrees/slot-1`.
Stable paths preserve each slot's `target/` and `target/clippy` caches. Give
every issue a fresh branch at an explicit base commit.

## Rules

- One active issue and agent per slot. The coordinator allocates, integrates,
  and releases slots; implementation agents never manage them.
- Keep Cargo targets checkout-local. Never share `CARGO_TARGET_DIR` or copy
  Cargo internals between slots.
- Use `git -C {REPO_ROOT}` or `git -C {SLOT_PATH}` for every coordinator Git
  command. Never rely on the ambient directory.
- Rebase in the issue slot, then fast-forward the integration branch. Never
  create a merge commit.
- Namespace shared scratch files by issue id. Do not mutate shared corpus,
  distribution, oracle, or cache state during a wave.

## Dispatch A Wave

1. Claim disjoint issues in Beads and pin one explicit `{BASE_REF}`.
2. Check capacity with `scripts/build-cache-policy.py --jobs N`. If it refuses,
   reduce the wave or reclaim an idle slot.
3. Claim an idle slot by atomically creating
   `.worktrees/.locks/{SLOT_NAME}`. Treat an existing lock as live until Beads,
   active agents, Git worktrees, and Cargo processes prove it stale.
4. Require a reused slot to be clean, detached, idle, and process-free. Create
   a missing slot only once, then create the issue branch:

   ```bash
   git -C {REPO_ROOT} worktree add {SLOT_PATH} --detach {BASE_REF}
   git -C {SLOT_PATH} switch -c {ISSUE_BRANCH} {BASE_REF}
   ```

5. Record the slot, branch, and base on the Beads issue. If no slot is free,
   wait or serialize instead of growing the pool without a capacity check.

Prewarm only new or reclaimed slots, sequentially, with the slot as the working
directory:

```bash
cargo test -q --tests --no-run
scripts/check.sh clippy
```

Append this to each dispatched agent's prompt:

```markdown
## Assigned worktree

- Main repository: {REPO_ROOT}
- Slot: {SLOT_PATH}
- Branch: {ISSUE_BRANCH}
- Base: {BASE_REF}

Work only in the prepared slot. Do not create or remove worktrees, switch
branches, change `CARGO_TARGET_DIR`, clean caches, or modify the main checkout.
Use full in-slot paths for patch tools and namespace shared scratch files with
{ISSUE_ID}.

If a gitignored asset is missing, check the primary checkout and its owning
workflow. Do not improvise symlinks or regenerate shared evidence.
```

Routine conformance assets provision themselves through
`test_support::native_assets::provision`; never symlink or broadly copy
`third_party/`. Run `scripts/setup-conformance-tests.sh` only in the primary
checkout when its source assets are absent.

For canonical-command divergence work, follow
`docs/canonical_divergence_workflow.md` for extra asset, tracer, stream, and
front checks. Do not impose those checks on unrelated jobs.

## Integrate And Release

After `coordinate` writeback verification, integrate completed slots one at a
time:

1. Record the tested `{PRE_REBASE_REF}` and current `{INTEGRATION_TIP}`.
2. Rebase in the slot:

   ```bash
   git -C {SLOT_PATH} rebase {INTEGRATION_TIP}
   ```

3. If `git diff {PRE_REBASE_REF} HEAD` is nonempty, rerun focused tests and
   `scripts/check-and-test.sh`. Reuse results only for an identical tree.
4. Fast-forward, then recycle the slot:

   ```bash
   git -C {REPO_ROOT} checkout {INTEGRATION_BRANCH}
   git -C {REPO_ROOT} merge --ff-only {ISSUE_BRANCH}
   git -C {SLOT_PATH} switch --detach HEAD
   git -C {REPO_ROOT} branch -d {ISSUE_BRANCH}
   ```

5. Record the integrated range and slot release in Beads. Verify the slot is
   clean, idle, and process-free, then remove its lock—not the worktree.

If the fast-forward fails because the integration tip moved, rebase and verify
again. Do not force, merge, or use a routine cherry-pick.

## Exceptional States

- _Interrupted agent:_ preserve its branch, slot, lock, and Beads state until
  recovery records an exact disposition.
- _Rebase conflict:_ leave the rebase active in the locked slot. Reopen the
  issue and dispatch a resolver into that slot with both sides' issue context.
  Require it to preserve both behaviors, complete the rebase, run focused tests
  and `scripts/check-and-test.sh`, and re-close the issue.
- _Disk pressure:_ reclaim only the least-recently-used idle slot by running
  `scripts/build-cache-policy.py --reclaim --jobs N` from that slot. Remove a
  physical worktree only when deliberately shrinking the pool and only after
  proving it clean, detached, idle, and process-free.
