# Generated paragraph and alignment box ownership

Status: inline direct-chain transfer implemented; child-bearing records and
alignment publication remain in progress, 2026-09-26.

The first production path consumes the paragraph mode-list owner and, when
hyphenation replaces its semantic tape, consumes fresh active-list segment
receipts together with the old move-only token. Plain post-line materialization partitions
that tape into monotonically disjoint windows. The final line wrapper carries
an authenticated positive descriptor of its actual child-chain chunks and
cut records. A unique `\lastbox` of an inline-only line can then move complete
logical chunks into vacant durable positions and shallow-copy just selected
records from shared cut chunks. The prepared transfer records old predecessors,
direct and paired dependency floors, and sparse positions. Rollback restores
those exact values before releasing projected cuts and the wrapper. Diagnostic
views remain page-readable until wrapper consumption.

Inline-only means every direct record has no nested child list or annex
dependency. The current prepared transfer refuses child-bearing records, which
still take the structural-copy fallback. Alignment rows do not yet publish
positive descriptors. Neither case is covered by the inline performance
claim. A direct source run can have a cut at each boundary; multiple runs may
create more than two cuts in one box. Projection cost is bounded by selected
cut records, while complete interior chunks retain their physical payload.

Before extending the transfer to child-bearing outputs, the profiling build
attributes each remaining built-root structural copy to setbox construction,
register take/copy, vsplit, lastbox fallback, or PDF form publication. It records copied node volume and
classifies the wrapper's immediate child list by leader, nested box, disc,
migrating material, math, other annex-bearing material, or inline leaves.
This census is enabled only by the profiling-stats CLI flag; normal builds
compile no shape traversal or counters. The classes identify where to add
producer-owned receipts, but do not themselves grant transfer authority.

Paragraph completion and alignment setting construct boxes from earlier page
lists. Their wrapper is new, but its child list can contain coordinates from
the consumed paragraph or unset row. A construction mark opened immediately
before publishing the wrapper does not own those children. Stamping that mark
would grant authority over too little material; widening it to the beginning
of the paragraph or alignment would claim unrelated lines, rows, diagnostic
lists, and page effects.

## Ownership at each transformation

`ArenaPostLineChannel` consumes a range of one paragraph source. The ordinary
case slices that source range and appends right skip. Other cases write an
active-list projection of source ranges, discretionary material, and generated
nodes. `extract_migrating_material_list` then separates marks, insertions, and
adjustments; hpack may build another retained projection. The emitted hbox
owns only its final retained child list and nested child closures. The original
paragraph source and diagnostic projections remain page-owned. Neighboring
lines may share a logical node block, so the original source block cannot be
loaned based only on a line's list coordinates.

Alignment setting similarly reads an unset row, constructs set cell wrappers
around earlier cell children, then constructs a set row wrapper. The unset row
and prototype are temporary sources. A set row owns its final child chain and
the child closures of its set cells. Other rows and page material stay with the
page. The alignment output list can interleave set rows with retained glue and
rules.

## Publication contract

The producer must pass a consumed-source receipt through its existing
materialization traversal. The receipt identifies each exact source range
used by the final box and whether its direct records were projected into new
chunks or uniquely consumed. Projecting direct records never transfers their
nested children by itself. A nested box contributes its own authenticated
body receipt, which is consumed once when the parent takes it. An explicit
TeX copy creates independent children and a fresh receipt; it cannot reuse the
source's move authority. The existing source chain remains readable during
line and row construction. Only removal of the last wrapper may detach its
source window, after all outputs from that source have been built.

The receipt is minted by draining the actual `ModePageListSlot` after its mutation
journal records the old root. A copied span, restored journal projection, or
live rollback frame makes that slot retained history and cannot grant move
authority. Appending a segment from a fresh active builder preserves authority;
appending a reclaimed unique list does not. The builder refuses to issue a
fresh segment after it has spliced such a list. A semantic rewrite that replaces
the removed root consumes the old receipt and mints its successor only from
fresh builder segment receipts. Fresh direct records alone do not grant ownership
of nested children named by those records. The transient direct-chunk index is built once for that final tape;
monotonically partitioned line windows binary-search the index instead of
rescanning the paragraph suffix.

Immediately before wrapper publication, the builder closes the selected
node-and-annex body intervals, excludes obsolete source projections and page
migrations, and writes one authenticated body descriptor into the wrapper's
private fixed annex. The descriptor is non-owning while the box remains in its
mode or page list. Removing that exact wrapper grants the transfer authority.
The transfer preflight validates all direct references and paired annex floors
over the selected intervals. It may project direct records into a private
chunk at a shared boundary, but it cannot take a sibling's chunk. The loan
retains enough coordinates for operation rollback to restore the source owner.

This arrangement adds no recursive child-graph walk after output. It keeps
source-derived provenance local to the traversal that already selects each
line or cell. A dynamic sidecar is bounded by the selected intervals and
merged exclusions; repeated wrapper rebinding must replace the old sidecar
instead of appending another copy. Empty bodies need a wrapper-only receipt
without sealing an empty node or annex tail.

### Required lower arena primitive

The current `PageBoxSegment` denotes one interval ending at the wrapper and
can subtract exclusions from that interval. It cannot name an earlier source
interval without also spanning all intervening line or row constructions.
`preflight_page_interior_intervals` and `transfer_page_interior_intervals`
already accept disjoint positive ranges, but the wrapper codec cannot publish
those ranges as authenticated provenance. A producer API must therefore
accept a move-only consumed source list and selected direct-record runs,
then return a private projection root and their positive paired chunk ranges.
It must split at most two boundary chunks per contiguous selected run;
fully selected interior chunks transfer without rewriting their payload.
Discretionary and left/right skip materialization can create several runs in
one box. The returned
receipt must specify nested child receipts separately and invalidate its
source window, so two wrappers cannot claim one chunk. The wrapper publisher
combines that receipt with ranges built during materialization and writes a
positive-range descriptor. Preflight verifies direct node and annex edges and
the wrapper's binding to those ranges before granting the loan.

The physical predecessor link of the first moved interior chunk can still
point to a preceding source chunk. Detaching it is reversible: the loan stores
the original predecessor and paired dependency floor, recalculates the moved
head's intrinsic annex floor from its direct records, and restores both only
after rollback has returned the chunk to its original owner. The rest of the
source remains untouched. Failed destination preflight restores the original
edge before the operation exposes a new root. A historical checkpoint can
still require a preservation copy; its roots are not consumed by this loan.

The current `append_validated_active_list_range` calls `copy_shared_then_splice`
after `slice_direct_root`; it explicitly copies a shared source root because
its head predecessor is write-once. It is an output constructor, not a
consumption proof. `PageListId` and `PageListSpan` are copyable coordinates,
and both paragraph and alignment materializers currently carry them. Marking
the output list as unique would leave those older roots able to refer to the
same chunks. The new primitive must first establish semantic consumption at
`take_nodes` and partition the source into disjoint windows. It can then
project boundary records and rebind the selected root while keeping the old
source topology available for operation rollback.

For paragraphs, semantic break windows are disjoint in their source indices,
but a chosen discretionary pre-list and its post-list are separate child
closures that cross the break. A diagnostic line can reuse semantic source
records, and hpack may preserve a separate diagnostic child root. Those
diagnostic references cannot share transferred chunks with the semantic
output; they need an explicit owned copy or must stay in a page-owned range
that is excluded from the move. For alignments, each unset cell is visited
once by the setting pass and a set cell points to that cell's old children.
The prototype and old unset wrappers are separate, obsolete sources. Repeated
alignment templates create new cell lists on each row; they do not grant
reuse of a row's old child receipt. A retained tabskip or rule is a direct
record and can share a boundary chunk with another row, so it too needs the
bounded boundary projection.

The direct-record receipt must include all retained child-bearing node kinds:
H/V and unset boxes, box leaders inside glue, discretionary pre/post/replace,
insertions, adjustments, and math node child lists. The ordinary paragraph
materializer migrates insertions and adjustments out of the line; their
page-owned projections are excluded. A retained leader remains in the line,
so its child closure belongs to the selected box even though the direct node
is glue. A direct dependency check at transfer remains a validation gate; it
cannot discover missing provenance by recursively copying the graph.

## Validation

Semantic tests must consume the last generated paragraph line and a set
alignment row through `\setbox=\lastbox`, retain their siblings, and compare
the output and list effects. Counter tests must show no additional recursive
closure copy on those unique transfers. An explicit `\copy` must retain an
independent closure. Operation rollback and retained checkpoint controls must
respect their distinct ownership rules. PDF form construction must use the
same receipt when it consumes a generated box, rather than inferring ownership
from an enclosing suffix.
