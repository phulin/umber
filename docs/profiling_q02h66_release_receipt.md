# q02h.66 paragraph replay release receipt

Issue: `umber2-q02h.66`\
Implementation commit: `c1983a68` (`Make paragraph replay a measured slow-path win`)\
Measured: 2026-07-17 on the `codex/umber2-vfqs` worktree

## Release decision

Merge the finished-line-only paragraph slow path and keep it default-disabled.
The representative pagination-changing edits are faster even after charging the
complete one-time history priming cost. The aggregate five-edit session is
effectively neutral, so this result does not justify unconditional enablement.

The implementation deliberately keeps the existing height/page-preserving fast
path separate. A line-breaking dependency change takes a one-shot cold fallback
for that revision while preserving the prior accepted paragraph history for a
later inverse edit.

## Optimized timing gate

Command:

```sh
CARGO_NET_OFFLINE=true cargo run --profile profiling -q -p umber \
  --bin gentle-profile -- \
  --repo-root /Users/phulin/Documents/Projects/umber2/.worktrees/umber2-vfqs \
  --incremental-edit --iterations 20 --warmups 1
```

The balanced AB/BA run used five accepted edits per session. Deltas are
paragraph memo enabled minus disabled:

| Path                                          |       Mean |     Median |
| --------------------------------------------- | ---------: | ---------: |
| Slow pagination-changing edits                | -15.299 ms | -15.004 ms |
| Slow edits plus complete priming              |  -2.005 ms |  -3.492 ms |
| Interaction follow-up                         |  -0.735 ms |  -0.217 ms |
| Independent height/page-preserving fast path  |  +0.365 ms |  +0.898 ms |
| Forced line-breaking cold fallback            |  +2.739 ms |  +2.680 ms |
| Complete five-edit baseline-inclusive session |  +0.365 ms |  +1.310 ms |

Memo-disabled and memo-enabled priming means were 241.260 ms and 254.554 ms.
The disabled runtime never activates its dependency stamp map, so this control
does not include memoization bookkeeping.

Each representative slow edit mounted 450 finished-line results, skipped
24,896 commands, reduced executed tokens from 129,370 to 104,472, and reduced
executed commands from 41,334 to 33,874. Validation had no typed misses and took
approximately 2.7 ms. Accepted paragraph history retained 3,029,160 bytes.
Every edit preserved the disabled named-boundary schedule and emitted
cold-identical 100-page DVI; the baseline output was 279,176 bytes.

## Architecture and soundness

- Paragraph history retains finished lines only. The prepared-hlist tier and
  its parallel cache, provenance, ownership, and accounting paths were removed.
- Typed read sets validate by changed-at stamp and exact semantic fallback.
  Observations are read-only; after tracking activates, actual mutations alone
  populate a mutation-only `AHashMap`. Broad invalidation is a scalar stamp.
- Scalar code-table observations share one per-table mutation clock and retain
  exact values. This reduced the Unicode detached-write retention gate from the
  initially detected 203,280 bytes to 2,488 bytes.
- Root paragraphs replay a compact final count/integer delta. Paragraphs entered
  in a live group retain their exact ordered local/global setter script. The
  entry class is validated before semantic dependency work.
- Balanced child groups are reusable. Unsupported surviving writes, input and
  group transitions, box consumption, inline math, output effects, and nested
  unsupported construction are explicit barriers.
- Retained line graphs are closure-validated once and then mounted. Final
  `prev_graf` and `last_badness` are restored in cold-equivalent order.

## Verification

All commands passed after the final timing-affecting changes:

```sh
CARGO_NET_OFFLINE=true scripts/check-and-test.sh
CARGO_NET_OFFLINE=true scripts/check-snapshot-budgets.sh
CARGO_NET_OFFLINE=true scripts/test-incremental-fuzz.sh
CARGO_NET_OFFLINE=true cargo test -q -p umber --test it e2e_conformance
```

The scripted fuzz gate compared 1,000 accepted edits against cold execution.
The e2e filter ran four conformance cases. The snapshot gate met every latency
and retained-allocation budget.

The final adversarial review found no remaining code or algorithmic blocker. It
verified root/live mutation ownership, `last_badness` ordering, retained history
after break fallback, nested shifted-box barriers, dependency rollback and
backdating, dormant memo-disabled tracking, and the post-activation timing
control. Its remaining observations are follow-up optimization opportunities:
mutation-map clone/restore scaling, repeated read-only semantic validation,
whole-revision rebreak fallback, and approximately 3.03 MB retained history.
