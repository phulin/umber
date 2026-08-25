# Arena-owned node lists

## Status

This document defines the node-specific implementation of
[Runtime storage lifetimes](runtime_storage_lifetimes.md). The former
structural per-list payload-owner model is deleted. Runtime lists are now
copy-only coordinates whose storage is owned by operation, page, or revision
generation arenas.

The logical row representation is not a format ABI. It may be compacted into
words and sidecars without changing the lifetime contract below, provided that
ordinary resolution remains direct, safe Rust and allocation-free.

## Lifetime-specific coordinates

`NodeListId<L>` is a private-construction owner, row, and row-generation
coordinate branded by
its semantic lifetime. Row zero is the canonical empty list and resolves
without storage. The public lifetime families are:

- `ScratchListId` for unfinished shaping, transforms, packing probes, and
  speculative operation material;
- `PageListId` for open modes, alignments, insertions, and page-builder
  material; and
- `DurableListId<G>` for lists retained by box registers or revision
  checkpoints.

Shipout-derived nodes use a separate `ShipoutScratchListId`, not another
`NodeListId<L>` alias. During output only, `ShipoutListId<G>` is the tagged
borrow projection over `PageListId`, `DurableListId<G>`, and
`ShipoutScratchListId`. No semantic/checkpoint carrier accepts either shipout
type, so scratch escape is a Rust type error rather than a runtime convention.

A coordinate is not an owner. It contains no `Arc`, `Weak`, root slot,
registry key, reference count, or drop-driven reachability action. Resolution
borrows the one matching `NodeArena`, validates its compact owner identity, and
indexes its row directly. Arena and coordinate constructors remain private to
the storage layer, so a coordinate from an unrelated arena of the same
semantic class is rejected rather than aliasing an equal row number.

## Payload placement

Scratch and page rows carry glue and token payloads owned by that same
semantic lifetime. A durable row carries `GlueId<G>` and `TokenListId<G>`
instead. Those ids resolve only through the matching coarse generation owner;
the node or list never retains an individual glue or token owner.

Diagnostic origins are copy-only coordinates and remain excluded from TeX
node equality and artifact bytes. A selected shipout provenance consumer
detaches stable source recipes while the page arena is still borrowed.

## Construction and rollback

An arena publishes a list only after validating every declared child
coordinate. Validation failure leaves the arena unchanged. The arena owns the
complete immutable row; nested node fields contain only coordinates back into
that same arena.

`NodeArenaCursor<L>` records the arena identity and row count. Operation and
page restore follows this order:

1. validate the state journal, mode/page roots, and arena cursor without
   mutation;
2. restore all canonical mode, page, alignment, insertion, and box roots;
3. truncate only the rejected arena suffix; and
4. release replaced coarse owners after no restored root can name them.

A foreign cursor or a cursor beyond the current suffix is rejected without
mutation. Reusing released trailing storage assigns a fresh row generation, so
a stale copied coordinate cannot alias a later page even when its physical row
number is reused.

`NodeArenaRegion<L>` is the consuming form of that cursor for a structurally
nested allocation suffix. It is neither `Clone` nor `Copy`. A top-level
`\setbox` or explicit/default shipout opens the region before its operand is
materialized, moves the token through any scanner suspension and box-body
continuation, and consumes it only at a terminal boundary. Nested box scopes
therefore close in strict suffix order. A resource suspension keeps the same
token; it never opens a replacement region or publishes a partial suffix.

## Exact promotion

Promotion starts from explicit typed escape roots. The source arena performs a
postorder walk over only schema-declared child fields and records one dense
source-row-to-destination-row relocation vector. Shared children are copied
once. Rows unrelated to the roots are not visited.

Page glue and token payloads found during that same ordered traversal are
batched through the generation's atomic `promote_values` seam. Durable node
rows are then staged with the returned `GlueId<G>` and `TokenListId<G>` values
and rewritten child coordinates. Destination list roots are returned only
after the complete closure is initialized. A failure publishes no box
register, checkpoint, mode, or page root.

There is no content lookup, weak-candidate search, liveness scan, attempt-wide
scan, in-place rewrite, forwarding pointer, or partially relocated visible
graph.

## Boxes and generation ownership

The dense box-register bank stores `Option<DurableListId<G>>`. TeX assignment
and group restoration journal that copy-only value exactly like other dense
state cells. The current revision, retained checkpoint, or in-session
continuation owns the complete generation bundle which contains the durable
node, token, glue, definition, and provenance arenas.

Moving a durable box into page storage copies its exact closure when the page
lifetime cannot borrow the durable arena through the operation. TeX `\copy`
may reuse the same durable coordinate while the same coarse generation owner
remains live. Neither operation adds per-list or per-node ownership.

## Shipout boundary

A completed explicit shipout operand is a page `Node` plus its move-only page
region; default output is the immutable durable box-255 root after the register
is cleared. PDF forms are durable roots as well. All three are wrapped in a
typed `ShipoutRoot<G>` and traversed in place. Child coordinates remain in the
same source arena. Shipout never promotes or rehomes their graphs.

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

Named checkpoints own scalar state, explicit roots, and marks into their one
coarse generation. The generation maintains a monotonic conservative page
bound: rootless captures do not advance it; any capture with a page handle may
advance it to the current page cursor. Rootless shipout truncates only above
that bound, preserving partial-page checkpoints in either restore order.
Pruning may leave the bound conservative; generation replacement releases the
complete page arena.

`\setbox` uses the same rule with a different terminal publication: its exact
operand closure is first promoted into durable generation storage and assigned
to the register, including the save-journal mutation, and only then is the
page region consumed. An overwritten durable coordinate may still be named by
the journal, a checkpoint, or a PDF form record. Durable rows therefore cannot
use this page-suffix rule; their eventual correction requires coarse
reachability ownership across those exact restore carriers.

## Semantic identity and detached boundaries

Node semantic equality hashes logical node kind, semantic scalar and payload
values, and recursively resolved child content. Arena row number, allocation
order, cursor, capacity, diagnostic origin, and coarse owner identity are not
semantic.

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
