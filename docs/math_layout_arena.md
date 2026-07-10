# Math layout arena

## Problem

Appendix G conversion currently builds a recursive owned `FrozenHList` tree.
Every nested math field allocates one or more `Vec<MathNode>` values, list
measurements rescan those vectors, and `tex-exec` then walks the tree again to
lower it into epoch `NodeListId` spans. A completed box is subsequently walked
once more for survivor promotion. Profiling the repeated Plain TeX math
workload attributes about 26% of CPU samples to recursive
`mlist_to_hlist`/`clean_box`, with allocator operations prominent among the
self samples.

Reusing empty vectors would reduce allocator overhead but would leave all of
those representations and traversals in place. The conversion result should
instead be a compact graph in one owned arena.

## Representation

`tex-typeset` will own a pure `MathLayout` result containing one contiguous
node vector and one root list handle. A list handle identifies an immutable
span allocated earlier in that vector and carries its already-computed
horizontal extents. A `MathBox` stores dimensions, shift, axis, and a list
handle rather than owning another vector.

Parent lists are built after their children. When a parent needs to concatenate
an existing list without copying its nodes, it emits an internal sequence
reference. Sequence references are transparent during measurement, inspection,
and lowering. This gives the arena a directed acyclic, bottom-up shape:

```text
child spans -> measured boxes -> parent spans -> root span
```

Span constructors are private. They validate that every referenced span ends
before the new span begins, so safe code cannot create cycles or forward
references.

## Conversion

`clean_box` returns a measured, span-backed `MathBox`. Recursive sub-mlists
append into the same arena. The first Appendix G pass retains compact work
records containing classes, penalties, and list handles rather than owned
hlists. The second pass creates the root span from sequence references and new
spacing, delimiter, and penalty nodes.

Packing computes dimensions once when a span-backed box is created. Reboxing
creates a small wrapper span around the original list instead of inserting into
an owned vector. Script attachment likewise composes the base and script spans
without copying either list.

The input math list remains immutable. The `make_ord` ligature/kern rewrite is
copy-on-write: recursive lists are borrowed directly unless an adjacent math
character pair actually requires TeX's mutating ligature/kern rule.

## Lowering boundary

`tex-typeset` remains pure: it reads through `MathTypesetState` and returns an
owned `MathLayout`. `tex-exec` lowers the arena bottom-up through `Universe`.
Each box child span is frozen into an epoch `NodeListId`; sequence references
are flattened into their containing list and do not create boxes or observable
nodes. Lowering uses one node scratch vector plus box-frame start indices, so a
completed child slice can be frozen and replaced by its box node without a
per-box output vector. A formula-local cache also avoids repeatedly interning
the same small set of spacing glues.

Lowering is iterative, so pathologically deep math input cannot overflow the
Rust stack. No raw node store or handle constructor crosses the aggregate
`Universe` boundary.

## Compatibility and validation

Public inspection helpers expose list slices/iterators without exposing span
constructors. Tests should assert the same logical node and box structure
through those helpers. The migration is complete when production conversion no
longer constructs recursive `Vec<MathNode>` trees.

The required gate is exact committed math DVI parity plus `tex-typeset`,
`tex-exec`, replay, shadow, live-boundary, and full workspace tests. Performance
validation uses the Plain TeX math workload, a deeply nested focused benchmark,
allocation counts, retired instructions, and a fresh CPU sample.
