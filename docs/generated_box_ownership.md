# Generated paragraph and alignment box ownership

Status: implementation design, 2026-09-26.

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
source's move authority.

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
accept a move-only consumed source list and a selected direct-record window,
then return a private projection root and its positive paired chunk ranges.
It must split at most the two boundary chunks of that window; fully selected
interior chunks transfer without rewriting their payload. The returned
receipt must specify nested child receipts separately and invalidate its
source window, so two wrappers cannot claim one chunk. The wrapper publisher
combines that receipt with ranges built during materialization and writes a
positive-range descriptor. Preflight verifies direct node and annex edges and
the wrapper's binding to those ranges before granting the loan.

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

## Validation

Semantic tests must consume the last generated paragraph line and a set
alignment row through `\setbox=\lastbox`, retain their siblings, and compare
the output and list effects. Counter tests must show no additional recursive
closure copy on those unique transfers. An explicit `\copy` must retain an
independent closure. Operation rollback and retained checkpoint controls must
respect their distinct ownership rules. PDF form construction must use the
same receipt when it consumes a generated box, rather than inferring ownership
from an enclosing suffix.
