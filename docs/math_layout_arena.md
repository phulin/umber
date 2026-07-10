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
horizontal extents. A `MathBox` stores dimensions, shift, axis, glue-setting
metadata, and a list handle rather than owning another vector. Existing h/v
boxes that enter a math field are imported with those complete box properties,
so `clean_box` can reuse a sole unshifted box exactly as TeX82 does without
hiding it behind an opaque or newly packed wrapper.

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

The ordinary `tex-typeset` API remains pure and returns an owned `MathLayout`.
Execution uses the optional `MathLayoutSink` entry point instead: conversion
invokes an exec-owned destination with the completed root before returning.
The destination expands transparent sequence spans directly into one reusable
node buffer, freezes each completed box child through `Universe`, and leaves no
standalone `lower_math_hlist` task-stack phase. A formula-local cache also
avoids repeatedly interning the same small set of spacing glues.

The sink sees only read-only structural spans and cannot construct or mutate
their handles. State nodes still cross the live boundary only through
`Universe`; no raw node store or opaque handle constructor is exposed to
`tex-typeset`.

Fresh box assignments preserve this bottom-up epoch graph through the
`\setbox` commit boundary. The assignment promotes the graph once and then
releases the construction suffix; it does not first clone every just-lowered
child list into that same epoch. Values sourced from an existing box register
remain explicitly classified as shared and take the cloning path required to
detach their survivor-owned children. This ownership distinction stays in the
execution layer and does not expose arena identity outside `Universe`.

## Compatibility and validation

Public inspection helpers expose list slices/iterators without exposing span
constructors. Tests should assert the same logical node and box structure
through those helpers. The migration is complete when production conversion no
longer constructs recursive `Vec<MathNode>` trees.

The required gate is exact committed math DVI parity plus `tex-typeset`,
`tex-exec`, replay, shadow, live-boundary, and full workspace tests. Performance
validation uses the Plain TeX math workload, a deeply nested focused benchmark,
allocation counts, retired instructions, and a fresh CPU sample.
