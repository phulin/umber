# Arena-owned node lists

## Status

This document defines the compact chunk/range implementation below
[Node-region ownership](node_region_ownership.md), which is authoritative for
coarse node lifetime, TeX move/copy semantics, and exact checkpoint history.
The replacement substrate is one caller-owned `ChunkPool<T>` plus typed,
coordinate-only `ForkArena<T, Lane>` states. Runtime payload is appended once
into fixed-byte chunks owned by coarse pool pages. Lists are copy-only borrowed
coordinates over those chunks; they do not own payload. Reads take a shared
region/pool borrow and yield stable direct references. Allocation, sealing,
promotion, settlement, and pruning require the caller's exclusive mutable
owner borrow.

Every nonempty list, including a one-range list, publishes its range entries
in descriptor chunks and is named by the same fixed 24-byte descriptor handle.
The page-semantic wrapper adds one niche-packed maintained identity scalar and
is capped at 32 bytes, preserving the recursive-node and dense-journal width
budgets.

When incremental convergence requests semantic identity before publication,
the existing version-1 polynomial sequence algebra is maintained during the
original node append. Each payload chunk keeps one whole-used-chunk summary in
its coarse metadata, and every canonical range entry keeps the exact summary
of that range. Slicing a long range combines whole-chunk summaries and hashes
payload only in its two partial boundary chunks; slicing a range sequence uses
stored summaries for every whole entry. Prefix and suffix subtraction preserve
the same identity when the selected boundary aligns with a range entry, so the
answer does not depend on descriptor or physical chunk layout. This adds no
per-node prefix table, alternate list topology, or root registry. When identity
is disabled, append, slice, compose, and active-range retention perform zero
node hashing and zero summary combination.

`ArenaListView` is a copy-only direct borrow of both the coordinate lane and
the physical pool. Its returned node references carry the pool borrow rather
than the temporary view borrow. The existing `NodeCursor` therefore retains
its slice adapter for pure tests and adds a page-material view variant without
materialization or a guard-owned lifetime.

`ActiveListBuilder` is the persistent append boundary. It is move-only and
stores only private generation-checked coordinates plus scalar pending-range
and descriptor state. It has explicit vacant, open, and sealed states. No
state holds a reference, pointer, `Rc`, or `RefCell`; each operation receives
the pool and arena explicitly. Open builders cannot become retained marks and
must be finalized or rolled back before a whole-chunk checkpoint boundary can
be sealed.

The `TypesetState::page_nodes` contract returns this borrowed `NodeCursor`, not
a contiguous slice. Existing row storage continues through the slice variant
during migration; the replacement page lane enters through `ArenaListView`.
Packing, protrusion, breakpoint widths, math conversion, and line
materialization therefore no longer require contiguous page ownership at
their shared state boundary.

The older complete-row `NodeArena` and its `NodePiece` composite stream are
superseded migration code and must be deleted as the remaining callers move to
the sole chunk/range topology; they must not gain another production consumer.

The logical row representation is not a format ABI. It may be compacted into
words and sidecars without changing the lifetime contract below, provided that
ordinary resolution remains direct, safe Rust and allocation-free.

## Lifetime-specific coordinates

`NodeListId<L>` is a private-construction row and row-generation coordinate
branded by its semantic lifetime. It is never an owner. Row zero is the
canonical empty list and resolves without storage. The coordinate families
are:

- `ScratchListId` for unfinished shaping, transforms, packing probes, and
  speculative operation material;
- `PageListId` for generation-owned open modes, alignments, insertions, and
  page-builder material. Durable closures use owner-relative `PageListId`
  coordinates only while borrowed through their move-only region owner.

Shipout-derived nodes use a separate `ShipoutScratchListId`, not another
`NodeListId<L>` alias. During output only, `ShipoutListId` is the tagged borrow
projection over `PageListId` and `ShipoutScratchListId`. Scratch rows contain
only scratch child coordinates. No semantic/checkpoint carrier accepts either
shipout type, so scratch escape is a Rust type error rather than a runtime
convention.

A coordinate is a borrowed capability under one matching move-only
`NodeRegion`. It contains no `Arc`, `Weak`, root slot, registry key, reference
count, or drop-driven reachability action. Resolution borrows that region and
its pool, validates the compact owner identity, and indexes directly. Arena and
coordinate constructors remain private to the storage layer, so a coordinate
from an unrelated arena of the same semantic class is rejected rather than
aliasing an equal row number. A production top-level owner cannot store a raw
coordinate without its region owner.

Paragraph checkpoints within one page share the exclusive `PageRegion` that
owns their coarse immutable chunks. Publication continues in its uniquely
owned tail or opens a new chunk after sealing. A checkpoint stores only its
region id, four PageBuilder roots, sealed payload/descriptor positions, scalar
state, and journal position. It copies no payload and creates no segment,
batch, or list owner.

## Payload placement

Page rows carry final glue values directly. A node token field shares the
immutable non-atomic stored-token allocation when its spelling came from a
generation token list; constructing a mark, deferred write, special, PDF
literal, PDF navigation value, or alignment template therefore copies no token
words. Synthesized node-only token fields allocate their final owner once.
Neither representation adds an `Arc`, `Weak`, registry row, or lookup.

Diagnostic origins are copy-only coordinates and remain excluded from TeX
node equality and artifact bytes. A selected shipout provenance consumer
detaches stable source recipes while the page arena is still borrowed.

## Construction and rollback

An arena publishes a list only after validating every declared child
coordinate. Validation failure leaves the arena unchanged. The arena owns the
complete immutable row; nested node fields contain only coordinates back into
that same arena.

`OperationMark<L>` may name a partially used payload or descriptor tail and is
restricted to local failure. It cannot convert into a retained mark.
Consuming every builder and sealing both tails yields an opaque
`SealedBoundary<L>`, which alone can construct `CheckpointMark<L>`. A retained
mark therefore contains only whole-chunk counts and stable terminal keys.
The operation mark also carries the optional scalar summary of its partial
payload tail, so truncation restores payload and identity metadata atomically.
Whole-chunk detach, reattach, acceptance, pruning, and typed-lane promotion move
the summary-bearing chunk and descriptor envelopes with their payload.
Operation and page restore follows this order:

1. validate the state journal, mode/page roots, and arena cursor without
   mutation;
2. restore all canonical mode, page, alignment, insertion, and box roots;
3. truncate only the rejected current chunk suffix; and
4. release replaced coarse owners after no restored root can name them.

A foreign mark or a mark beyond the current suffix is rejected without
mutation. Reusing a released chunk slot assigns a fresh slot generation, so a
stale copied coordinate cannot alias later material even when its physical
slot number is reused.

`NodeArenaRegion<L>` is the consuming form of that cursor for a structurally
nested allocation suffix. It is neither `Clone` nor `Copy`. A top-level
`\setbox` or explicit/default shipout opens the region before its operand is
materialized, moves the token through any scanner suspension and box-body
continuation, and consumes it only at a terminal boundary. Nested box scopes
therefore close in strict suffix order. A resource suspension keeps the same
token; it never opens a replacement region or publishes a partial suffix.

## Lifetime transfer and cold materialization

Ordinary setbox and PDF-form publication consume a self-contained exclusive
region into the destination owner. Consuming `\box` and unbox operations
transfer that region while clearing the exact state carrier when no retained
checkpoint or save journal needs the old source. `\copy` and `\unhcopy`
recursively copy the exact node closure. If history must preserve the source of
an otherwise consuming move, the old region moves into history and the current
destination receives a bounded closure copy. No ordinary borrowed-range list
transition walks or copies a closure.

The old dense relocation machinery remains only for a node graph entering from
a distinct cold format/materialization arena. That boundary starts from
explicit typed roots and copies the exact closure after full validation. It is
not callable from ordinary box, page, math, alignment, or token transitions.

## Boxes and generation ownership

The dense box-register bank stores a move-only durable owner-plus-root carrier,
not a naked coordinate. TeX assignment moves the old carrier into the
save journal before installing its replacement. Group restoration moves it
back; a superseding global assignment drops it in TeX order. Retained
checkpoint history likewise owns the exact older carrier it can restore.

Moving a durable box into page state transfers its exclusive self-contained
region when unique. Page construction still publishes a compatibility page
closure, so assignment to a durable register currently performs one explicit,
counted `page_to_durable` copy; whole-envelope transfer is the remaining seam
once page construction produces an independently consumable closure. TeX
`\copy` creates an independent node
closure but continues to share the selected immutable token-list and glue
values, matching TeX82. Neither operation adds per-list or per-node ownership.

## Shipout boundary

A completed explicit shipout operand is a page `Node` plus its page closure.
Default output consumes box 255 into page storage, using one bounded historical
copy only when a retained checkpoint prevents the exact move. PDF form
traversal likewise copies its immutable durable closure into page material.
Shipout traversal then borrows only page or self-contained scratch rows.

Output-only math nodes are appended directly to final stable rows in the one
reusable `ShipoutScratchArena<G>`. No temporary node vector is drained into
those rows. Each aggregate output transaction records a scalar scratch mark;
success and rollback reset the nested suffix wholesale while preserving row
and node-vector capacity at warmed high water. Typed token/node source handles
keep deferred writes, PDF identifiers, thread attributes, and color-stack
actions borrowed through replay. Only genuinely surviving semantic escapes
stream directly into durable builders; artifact/effect bytes are materialized
once into their detached final owner.

Shipout walks the source once and builds a handle-free page plan, artifact
data, effects, and requested stable source recipes. `tex-out` receives no node
id, arena cursor, arena owner, generation owner, runtime handle, or engine
borrow. Aggregate failure first restores state, page-builder, PDF, World, and
engine-usage roots, then resets scratch and releases any explicit page suffix.
A normal huge-page rejection, memo replay hit, successful commit, or void
operand uses the same terminal whole-region/reset rules. No graph scan,
compaction, relocation, free list, or per-row owner participates.

Named checkpoints own scalar state and owner-relative marks through checkpoint
history's page-region owners. Rootless mode state adds no node owner. After
shipout, handle-free output keeps no runtime node and the held-over closure is
evacuated into a new page region. An old region remains only while its
contiguous checkpoint interval is retained, then drops wholesale. No monotonic
page bound substitutes for ownership.

A legal retained boundary has one quiescent empty outer mode and therefore no
mode child coordinate. Capturing it does not publish a second page-arena copy
or manufacture a node root.

`\setbox` uses the same rule with a different terminal publication: its exact
operand region is transferred to an exclusive durable carrier and assigned to
the register, including the save-journal mutation. An overwritten region moves
into the journal or checkpoint history when it remains restorable; otherwise
it drops. Operation rollback restores the prior owner before truncating a
rejected suffix.

## Semantic identity and detached boundaries

Node semantic equality hashes logical node kind, semantic scalar and payload
values, and recursively resolved child content. Arena row number, allocation
order, cursor, capacity, diagnostic origin, and coarse owner identity are not
semantic.

`identity_nodes_hashed` and `identity_summaries_combined` distinguish boundary
payload work from stored-summary work. The long-middle-range gate bounds the
former by two chunk capacities, requires the latter to cover the interior, and
keeps `source_nodes_copied` at zero. Disabled-demand coverage requires both
identity counters to remain zero across publication, slicing, composition, and
active-range retention.

Formats, memos, output DTOs, and process or thread messages use their own dense
local indices. Cold detachment assigns those indices from explicit roots;
materialization validates the complete schema and allocates destination-local
generation rows before publication. A runtime `NodeListId` is never serialized.

## Validation

The focused node-arena tests prove exact escaping-closure relocation, shared
child relocation once, exclusion of unrelated rows, owner-checked suffix
rollback, strict nested-region release, failed-region transfer to enclosing
rollback, warmed capacity reuse, stale-alias rejection, invalid-child atomic
rejection, and page-prefix preservation. Mode, box, paragraph, alignment,
page-builder, shipout, DVI, PDF, and aggregate rollback suites remain the
semantic acceptance authority.
