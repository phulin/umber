# Mode-list rollback journal

Status: measured design; implementation gate not yet satisfied

Scope: canonical aggregate operations in `tex-exec`. This document does not
change TeX semantics or the atomic boundary in
[Stepwise Execution](stepwise_execution.md).

## Measured problem

`CanonicalStepSnapshot::capture` clones `ModeNest` before every atomic
operation. Its `Arc` roots make capture cheap, but retaining the old mode-list
root forces the first successful mutation to copy the complete `Vec<Node>`.
The `umber2-johp.298` symbolized profile attributed 5.79% self time to that
copy below `Arc::make_mut`, with 4.24% through ordinary character append and
1.39% through horizontal-run reconstitution.

The bounded `mode_list_rollback` benchmark isolates the lifetime with 16,384
pre-existing nodes and 1,024 successful appends. Its length-watermark case is
the lower-level operation an append-aware journal should reach. It deliberately
does not claim that a watermark alone is a complete rollback representation.

## Required representation

One aggregate mode savepoint is a private, generation-checked cursor into an
undo log owned by `ModeNest`. Each frame records:

- the mode-level depth and identity at entry;
- each existing level's node length and all non-node `ModeList` fields;
- inverse records for removed, replaced, or mutably borrowed nodes;
- inverse records for level push, pop, and replacement; and
- the enclosing frame cursor, so inner commit preserves inverses still needed
  by an outer rollback.

Append and tail extension record only the original length. Scalar setters are
restored from the entry projection. Before an API returns `&mut Node`,
`&mut Vec<Node>`, `&mut AlignState`, or another reference able to change
pre-existing state, its write barrier must preserve the affected old value.
Bulk ownership-taking operations record the moved value rather than cloning
it. A destructive operation may use a full owned-list inverse when a smaller
inverse would be more expensive or error-prone, but ordinary append must never
take that fallback.

Commit discards the innermost frame only after merging any inverse needed by
its parent. Rollback validates the cursor before mutation, replays inverses in
reverse order, restores scalar projections and depths, and then truncates the
log. A savepoint is neither cloneable nor externally constructible.

## Atomicity invariants

The existing aggregate ordering remains authoritative:

1. candidate observations, geometry records, boundaries, prepared pages, and
   effects remain buffered;
2. a successful operation commits the mode frame before publishing observers;
3. an ordinary error or typed resource suspension rolls back `Universe` and
   mode state before returning;
4. a fatal TeX82 §81 jump commits partial semantic state exactly as today;
5. nested alignment and box operations use nested frames, never a second
   independent rollback owner; and
6. provenance and effects remain owned by `Universe`; mode inverses store only
   already-owned identifiers and become observable only after aggregate
   rollback has restored the matching `Universe` timeline.

Semantic hashing and named checkpoints read only live mode state. Journal
cursors, inverse capacity, and spare allocation are operational state and must
not enter semantic equality, traces, formats, or durable summaries.

## Why implementation is deferred

The current mutation boundary is not journal-safe. The crate has 233
`current_list_mut`/`list_mut` and direct mutation uses, including APIs returning
`&mut Vec<Node>`, `&mut Node`, `&mut AlignState`, and `&mut ModeList`.
Installing only an append watermark would make successful character steps
fast while silently breaking rollback for math-node edits, reconstitution,
alignment state, list ownership transfer, and nested operations.

The bounded benchmark establishes the performance opportunity, but it cannot
bound the correctness risk of converting all 233 seams in the same profiling
issue. Promotion therefore requires a separate architecture change that first
makes mutation capabilities typed and non-escaping, then installs the journal,
then switches `CanonicalStepSnapshot` from a retained `ModeNest` root to its
opaque savepoint. Until all three land together, the retained root remains the
correct implementation.

## Promotion gates

- Unit tests cover nested commit/outer rollback, inner rollback/outer commit,
  every destructive mutation family, errors, suspension, and fatal commit.
- Observer, geometry, semantic-trace, page/list, effect, provenance, and named
  checkpoint projections are byte-for-byte unchanged.
- The bounded synthetic benchmark demonstrates that successful append cost is
  independent of retained prefix length.
- A symbolized optimized exhaustive profile removes the
  `ModeList`-clone/`Arc::make_mut` caller while the exact tracer report remains
  unchanged.
- The standing native and format/clippy gates pass.
