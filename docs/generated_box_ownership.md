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

The authenticated 200-million-action book prefix attributes 698,707 of
791,159 remaining interleaved-prefix copied nodes to
`\setbox<destination>=\box<source>` with a nested box payload. Both operands
can be durable box-register owners. For an exclusively current durable source, its owner
can move directly from the source cell to the destination cell; the page
roundtrip and replacement wrapper are unnecessary. The source cell keeps its
TeX §1079 level when cleared. The destination still goes through the ordinary
local/global binding and save journal. During an operation, the source-take
inverse precedes the destination-binding inverse: rollback first swaps the
destination's current owner into its binding inverse, then consumes that one
owner from the inverse to restore the source. It never shares one owner ID
between cells or history. A checkpoint-retained source remains on the
historical copy path until its independent owner can be proven and moved.
Tracing may materialize a separate diagnostic projection only when enabled.
The page-owned output-box carrier stays on its existing path.

A second authenticated 200-million-action run after direct durable handoff
classified the actual rejected take at the scanner and carried that label to
the structural-copy event. All 108 register-take fallbacks, totaling 698,707
copied nodes, consume the live page-owned box255 output carrier. None is a
checkpoint-retained or missing durable source. The direct durable handoff
therefore preserves these book-prefix counts, while remaining useful for
ordinary uniquely current register-to-register moves. Removing the dominant
fallback requires a consumed `PageBuilderState` output-carrier receipt that
selects the packaged page prefix without taking held-over page material or
checkpoint history. The page builder owns the source; a bare `PageListId`
cannot mint that authority.

The setbox scanner completes operand expansion and classifies the register
take before opening the page construction mark. The direct-take branch only
reads the durable source and removes the pending target before it cancels
that mark, so no page node or annex payload can be published in its suffix.

The handoff uses two phases because `make_box` clears `\box<source>` before
TeX traces the destination assignment. The first phase removes the live
source binding and leaves its owner in a move-only, state-bound operation
receipt. A same-register assignment therefore traces a void old destination.
The receipt names the exact destination and operation; a different state or
target cannot finish it. If the operation rolls back before assignment, its
pending action returns the owner directly to the source, with no destination
inverse to consult. The second phase stages any checkpoint and group copies
of the old destination before changing the destination binding or its journals.
If either copy fails, the staged copy retires, the pending action disappears,
and the source binding returns unchanged. Once the destination is installed,
the ordinary binding inverse and the source-take inverse undo it in reverse
order, without duplicating the exclusive owner.

### Page-owned output carrier

The common book case is the output routine assigning `\box255` to another
register. `prepare_box255` consumes a prefix of the current page, may move
insertion records into class boxes or hold them over, packs the selected page
as a VList, and installs that wrapper in `PageBuilderState.output_box`. The
wrapper is in the page region, but its paragraph-line children and annex
payloads may occupy older page chunks shared at logical chunk boundaries
with held-over material. No construction suffix beginning at the wrapper can
claim them. `take_output_box` does remove the actual semantic owner and
records the page inverse; a copied root coordinate does neither.

For an uncheckpointed page region, the first output-carrier path should move
the _whole old region envelope_ into a durable box owner. This avoids a late
recursive census of paragraph lines, glue annexes, nested boxes, and
unreachable historical records. Before moving it, the existing page-region
successor builder prepares a fresh region containing independent copies of
every live _survivor_ root: contribution, current page/held-over insertions,
page discards, and split discards. It excludes the consumed output box. A
`prepare_box255` held-over list is installed in the builder's current-page
root before the output routine begins, so it belongs to the copied survivor
set; it is not an untracked sixth root. The builder's complete five-root
inventory is thus four copied survivors plus the consumed output box. A
complete old region can then move even if the selected box and survivor
records share physical logical chunks. Survivor copy volume is measured
separately from moved output-box volume; if survivors are large, a later
selected-range/cut transfer can improve that case without changing authority.
The old region may contain unreachable records, which are retained with the
box until that owner retires; none becomes a second semantic alias.
Profiling reports exact moved envelope records/annex words and exact
recursive survivor-copy nodes. Its semantic output and five-root counts
deduplicate whole `PageListId` values, so overlapping list slices may count
one physical record more than once. Their difference from the envelope count
is not an exact unreachable-record count.
The whole-envelope transfer still performs the existing mandatory paired
node/annex admission and validates every direct child dependency. Avoiding
new graph discovery does not weaken that check. The builder's insertion
slots are scalar class/status records; page marks refer to generation-owned
token keys, not node-region chunks. Its five payload roots enumerate the
page-owned node survivors. A page region with an incomplete external-root
inventory cannot take this route.

The old envelope can also contain an obsolete chunk whose predecessor was
already moved into, then retired with, another owner. Whole-envelope
admission rejects that chunk even when the live output and all four survivor
roots remain readable. Root/region admission and envelope admission therefore
have distinct errors: stale or foreign live-root admission is an error, while
an `InvalidChunk` or `InvalidRegion` from the historical envelope alone
declines the move before source consumption. The ordinary recursive copy then
reads and validates the live output root independently. It still fails on a
genuinely invalid reachable root; no stale coordinate becomes move authority.

Admission uses the actual page-builder output-box slot and the existing
successor's complete root inventory. It rejects a retained page-region
checkpoint, a prepared successor/candidate, an unfinished durable-to-page
loan, and any live or rollback-restorable mode, alignment, scanner, or
detached page-node root outside the builder inventory. The mode-list
succession preflight already checks mode levels and their journal; other
external owners need equivalent checked receipts or must make this fast path
decline. The admission and successor-copy preflight finish before clearing
the output slot, detaching a chunk, or publishing a durable register binding.
The ordinary output routine has already armed `output_successor_build` for
the next-page suffix. That construction mark is expected and stays distinct
from `PageRegionHistory.pending_successor`, which holds a prepared owner
transition. The former cannot by itself veto the common output-carrier take.
No caller may request the move by supplying a `PageListId` alone.
`PageRegionHistory` mints this admission from its actual checkpoint and
pending-successor state when lending its current arena and builder to the
command. The command cannot create a retained page checkpoint while that
borrow is live. The current arena can then replace its resident node region
with the prepared successor while keeping the command's borrow valid; the
returned move-only loan holds the old region. The builder's copied survivor
roots are rebased in place without resetting its active operation journal.

The page operation owns an inverse for the output slot; the durable operation
owns the register binding and the whole-region transfer loan. Forward order
is: prepare successor and paired node/annex transfer; consume output slot;
install successor; loan old region envelopes to the durable owner; install
destination binding. Rollback reverses the binding first, returns the exact
paired chunks to the old page region, restores the old current-region slot,
retires the unexposed successor, then applies the page output-slot inverse.
This ordering leaves no page journal pointing at a region whose chunks are
durable-owned. Commit retires the old empty page-region shell and consumes
the loan. A failure after successor preparation but before publication drops
only the prepared successor; a failure after detachment uses the same exact
inverse before any page root becomes visible again. The transfer cannot move
an accepted checkpoint's chunks because their page roots remain meaningful
after the operation commits.
The old region's unreachable node and annex words must be counted against
the selected box's reachable nodes, and survivor-copy words counted
separately. The route is worthwhile only if it replaces a larger recursive
box copy without causing excessive retained backing.

TeX's same-register case still clears the page carrier before destination
assignment and tracing. For `\setbox255=\box255`, the old destination is
void at that assignment point; a local group save restores void, while a
global assignment retains the moved owner. When the destination is another
register, an overwritten owner keeps its usual group/checkpoint history;
dimension edits to the moved wrapper remain exclusive and reversible. An
explicit `\copy255` continues to allocate an independent copy.
Tracing must format the new box directly from the live page carrier before
consumption. Calling `copy_box_to_page(255)` would promote that carrier into
durable storage and destroy the admission proof. After source clear, tracing
reads the old destination and reports the preformatted new value; any
diagnostic projection is explicitly independent and paid only when tracing
is enabled.

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
