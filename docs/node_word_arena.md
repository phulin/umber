# Arena-owned node lists

## Status

This document defines the node-specific implementation of
[Runtime storage lifetimes](runtime_storage_lifetimes.md). The replacement
substrate is one caller-owned `ChunkPool<T>` plus typed, coordinate-only
`ForkArena<T, Lane>` states. Runtime payload is appended once into fixed-byte
chunks owned by coarse pool pages. Lists are copy-only coordinates over those
chunks. Reads take a shared pool borrow and yield stable direct references;
allocation, sealing, promotion, settlement, and pruning require the caller's
exclusive mutable pool borrow. A TeX lifetime transition promotes whole sealed
chunks and rebrands their coordinates; it does not copy, relocate, or rewrite
surviving payload.

`ArenaListView` is a copy-only direct borrow of both the coordinate lane and
the physical pool. Its returned node references carry the pool borrow rather
than the temporary view borrow. The existing `NodeCursor` therefore retains
its slice adapter for pure tests and adds a page-material view variant without
materialization or a guard-owned lifetime.

The older complete-row `NodeArena` and its `NodePiece` composite stream remain
only as migration inputs. They are not the target representation and must not
gain another production consumer.

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
- `PageListId` for generation-owned open modes, alignments, insertions,
  page-builder material, and the physical rows later retained by boxes; and
- `DurableListId<G>` for a generation-branded coordinate view retained by box
  registers, PDF forms, or revision checkpoints.

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

Checkpoint forks share coarse immutable 64-row arena segments. Publication
continues in a uniquely owned tail segment or opens a new one after a fork;
the fork copies only compact row-location metadata and segment handles, never
a node payload. The segment handle belongs to the checkpoint-generation
operation, not to an individual list coordinate, so this adds no per-value
owner or hot lookup.

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

Ordinary setbox and PDF-form publication validate the live page coordinate,
advance the generation's conservative durable page bound, and rebrand the
coordinate. `\copy` shares that immutable row; consuming `\box` and unbox
operations transfer the coordinate while clearing the exact state carrier.
No ordinary transition walks a closure or copies a node/token payload.

The old dense relocation machinery remains only for a node graph entering from
a distinct cold format/materialization arena. That boundary starts from
explicit typed roots and copies the exact closure after full validation. It is
not callable from ordinary box, page, math, alignment, or token transitions.

## Boxes and generation ownership

The dense box-register bank stores `Option<DurableListId<G>>`. TeX assignment
and group restoration journal that copy-only value exactly like other dense
state cells. The current revision, retained checkpoint, or in-session
continuation owns the complete generation bundle which contains the durable
node, token, glue, definition, and provenance arenas.

Moving a durable box into page or mode state rebrands its coordinate while the
same coarse generation owner remains live. TeX `\copy` creates only the
logical alias required by TeX. Neither operation adds per-list or per-node
ownership.

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

Mode summaries retain their existing child coordinates directly. Capturing a
named boundary does not publish a second page-arena copy of the mutable mode
buffer merely to manufacture a root; the summary's coordinate scan alone
decides whether the conservative page bound must advance.

`\setbox` uses the same rule with a different terminal publication: its exact
operand region is transferred to the durable page prefix and assigned to the
register, including the save-journal mutation. An overwritten coordinate may
still be named by the journal, a checkpoint, or a PDF form record. The
generation keeps a conservative monotonic durable bound; operation rollback
restores the prior bound before truncating a rejected suffix.

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
