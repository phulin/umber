# Arena-owned node lists

## Status

This document defines the compact chunk/range implementation below
[Node-region ownership](node_region_ownership.md), which is authoritative for
coarse node lifetime, TeX move/copy semantics, and exact checkpoint history.
The 32-byte non-owning node-record design below is implementation-ready but is
not yet the production representation. Its approval and cutover are tracked by
`umber2-66p0.8.40.113.5` and its children.

The first attempted cutover exposed two prerequisites which this document now
resolves. A physical `RawChunkKey` cannot be embedded in `PageListId`, a
predecessor link, or an annex key because one logical checkpoint tail must
resolve to different accepted and candidate superblocks. A semantic region
owner cannot be embedded there either because TeX moves must transfer a sealed
closure from page to durable, history, or transient output ownership without
rewriting its records. The selected design therefore separates three facts:

- pool-stable logical coordinates identify node and annex positions;
- exactly one accepted table and at most one candidate table map those logical
  coordinates to private exact-64-KiB physical blocks; and
- move-only aggregate envelopes say which semantic owner may admit a set of
  node and annex blocks.

This is the prerequisite design tracked by `umber2-66p0.8.40.113.5.6`. It does
not authorize the production record cutover. The cutover remains blocked until
the explicit approval and implementation children named below land.

Production currently uses one caller-owned `ChunkPool<T>` plus typed,
coordinate-only `ForkArena<T, Lane>` states. The compact cutover replaces the
physical payload slots with the exact 64 KiB dense node and annex arenas from
[Dense fork-arena superblocks](fork_arena_dense_prefix_emplacement.md).
Existing list blocks become logical cursor/range boundaries inside that dense
storage; they are not physical allocations. Lists remain copy-only borrowed
coordinates and own no payload. Reads take a shared region borrow and use flat
direct indexing. Allocation, sealing, settlement, and pruning require the
caller's exclusive mutable owner borrow.

Production currently publishes every nonempty list, including a one-range
list, through the range metadata owned by `ForkArena` and names it with the
current 32-byte `ArenaListId`. The page-semantic wrapper adds one optional
maintained identity scalar, making `PageListId` exactly 40 bytes. The logical
table migration keeps equivalent range/predecessor metadata beside the flat
logical rows. The compact record keeps the complete pool-stable coordinate in
the annex rather than weakening its admission or identity proof.

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
the physical pool. Today its returned node references carry the pool borrow
rather than the temporary view borrow. The compact cutover returns the
equivalent borrowed `NodeView`, which additionally borrows the admitted annex
view. `NodeCursor` retains its slice adapter for pure tests and its
page-material variant without materialization or a guard-owned lifetime.

`ActiveListBuilder` is the persistent append boundary. It is move-only and
stores only private generation-checked coordinates plus scalar pending-range
and descriptor state. It has explicit vacant, open, and sealed states. No
state holds a reference, pointer, `Rc`, or `RefCell`; each operation receives
the pool and arena explicitly. Open builders cannot become retained marks and
must be finalized or rolled back before a whole-chunk checkpoint boundary can
be sealed.

New page material crosses that boundary through one resident-slot reservation.
Production currently lends the final `Option<Node>` place to one inlined
initializer. The compact cutover instead coordinates annex publication with
the final `VacantSlot<NodeRecord>` from the dense prefix substrate; no vacancy
tag remains. The facade derives child dependency metadata and completes the
optional identity summary from that resident value. No whole-node return value
crosses the generic arena layers. A rejected foreign builder never reserves a
destination, and a rejected child restores the annex cursor before abandoning
that exact unpublished node reservation. This removes the intermediate
whole-node carriers without adding a payload owner, alternate node
representation, allocation, or rollback state; explicit TeX/source-copy paths
remain separately named value-copy boundaries.

The `TypesetState::page_nodes` contract returns this borrowed `NodeCursor`, not
a contiguous slice. Existing row storage continues through the slice variant
during migration; the replacement page lane enters through `ArenaListView`.
Packing, protrusion, breakpoint widths, math conversion, and line
materialization therefore no longer require contiguous page ownership at
their shared state boundary.

The older complete-row `NodeArena` and its `NodePiece` composite stream are
superseded migration code and must be deleted as the remaining callers move to
the sole chunk/range topology; they must not gain another production consumer.

The logical row representation is not a format ABI. The selected replacement
is the compact record and typed annex below. It remains direct, safe Rust and
allocation-free on ordinary access.

## Unified logical tables

### Coordinate and table ownership

`NodePool` owns two typed physical stores, one for `NodeRecord` and one for
annex `u32` words. Each store allocates exact 65,536-byte superblocks through
`tex-dense-prefix`. A physical `BlockId { slot, incarnation }` is private to
those stores. It may occur only in the pool's flat tables and in move-only
storage receipts; it never occurs in `PageListId`, `PageListSpan`, a node
record, a predecessor, a checkpoint row, a format, or detached output.

The pool also owns stable logical spaces. A node cursor has this conceptual
shape:

```text
LogicalNodeCursor
  space: u32
  block_ordinal: u32
  logical_block_incarnation: u32
  offset: u32
```

`space` authenticates the one `NodePool` coordinate domain, not a page or
durable owner. A nonempty `ArenaListId`/`PageListId` stores logical head and
tail cursors plus length; the canonical empty value is all zero. The existing
optional semantic identity remains a scalar beside that coordinate. Head,
tail, maintained predecessors, child roots, and `PageListSpan` therefore use
one coordinate vocabulary. There is no physical-key compatibility form.

The node table is a flat ordinal-indexed vector. Each live row contains the
expected logical incarnation, its private physical `BlockId`, initialized
prefix, logical-list range metadata, predecessor metadata, dependency floors,
and sequence summary. Multiple small logical list ranges may occupy one dense
physical superblock; their rows name an offset range in that block. The
physical-block row separately records the logical-row interval which maps to
it. This preserves the current packed-list granularity without making each
short list consume 64 KiB. A direct node lookup is:

```text
logical row = node_table[block_ordinal]
validate space, logical incarnation, and admitted envelope
physical block = node_store[logical_row.physical_block]
value = physical block[logical_row.physical_base + offset]
```

The quotient/remainder path for a naturally contiguous node position still
selects its physical superblock directly. A maintained predecessor is followed
only when traversal crosses one of the existing logical list-range boundaries;
nodes never acquire per-node links. Indexed reads, admitted spans, and forward
range callbacks continue to use the sole predecessor topology. There is no
linked-node traversal, descriptor search, physical-owner range scan,
forwarding lookup, or compaction.

Released logical rows enter an explicit pool-owned free-slot stack. Reuse
increments the row incarnation before publishing a new mapping, so the flat
vector remains bounded by its live/high-water demand without relocating a live
row. Fork metrics report copied live and vacant rows separately; a long
session must reach a stable table high-water when its semantic owners do. A
hole is never skipped through a search structure and compaction is not a
fallback for excessive metadata.

An annex cursor uses the existing six-word shape with `owner` redefined as the
pool-stable annex `space`:

```text
AnnexKey<K>
  space: u32
  block_ordinal: u32
  logical_block_incarnation: u32
  word_offset: u32
  word_len: u32
  publication_serial: u32
```

The annex flat table maps that logical ordinal/incarnation to a physical word
superblock and initialized prefix. Fixed records never cross a block. A
dynamic span which crosses a block publishes a compact continuation header in
each subsequent logical block. The header repeats the publication serial and
span incarnation and states that segment's word count. Resolution verifies
every crossed table row and continuation header before lending that segment;
the first-block incarnation is never treated as proof for later blocks. The
publication serial remains monotonic across rollback and rejects reuse of an
offset within a surviving logical block.

`NodePool` owns the tables because a region transfer must not make them sparse,
copy them, or rebrand coordinates. `NodeRegion` owns only disjoint move-only
envelopes over live logical rows and physical blocks. A region borrow admits a
coordinate by checking both the selected table and the region's envelope. A
coordinate from another pool, another logical incarnation, a transferred
envelope, a truncated suffix, or a recycled physical block fails before a
payload borrow is returned.

### Exactly two views

At rest the pool has one `AcceptedNodeTables` aggregate containing the node
and annex tables. An edit consumes that authority and produces one
`NodeTableFork` with exactly two borrow-scoped views:

- `AcceptedNodeView<'fork>` resolves the unchanged accepted tables and the
  parked prior suffix; and
- `CandidateNodeView<'fork>` resolves the shared complete prefix, copied
  private tails, and candidate suffix.

There is no public lineage integer and no constructor for a third view.
`NodeView`, `NodeDestination`, `PageListSpan`, cursors, predecessor resolution,
and annex codecs all require one of these borrowed views. At rest, the accepted
owner lends the same interface as `AcceptedNodeView`; callers do not branch on
storage representation.

The fork copies the flat table prefix as explicitly measured metadata. Every
complete physical block before the checkpoint maps to the same immutable
`BlockId` in both tables. If the checkpoint is inside a node block, the
candidate receives one private physical block containing exactly the
initialized checkpoint prefix and every logical table row in that physical
tail is redirected to it in the candidate table. The corresponding operation
is applied independently to the annex table. Logical ordinals, incarnations,
offsets, list predecessors, child roots, and annex keys do not change.

Checkpoint capture stores only the aggregate node cursor, annex cursor,
PageBuilder roots, and scalar/journal positions. It allocates and copies zero.
An interior fork copies at most 2,047 32-byte records, or 65,504 bytes, and at
most 16,383 annex words, or 65,532 bytes. A boundary-aligned cursor copies
zero. Candidate acceptance drops the superseded accepted private tails and
moves the candidate table vectors into the accepted role; it copies zero
payload. Rejection drops candidate-private blocks and returns the accepted
tables unchanged. Node and annex acceptance or rejection is one aggregate
operation and cannot be called component by component.

### Stale and foreign rejection

Logical ordinal reuse increments the logical incarnation before publication.
Physical slot reuse independently increments `BlockId` incarnation. Annex
offset reuse additionally receives a fresh publication serial. Published list
roots are immutable; a partial node-tail rollback is legal only while its
move-only builder has published no root, so no admitted `PageListId` can name a
reused unpublished offset. Whole logical-row reuse increments its incarnation.

Resolution fails closed on every mismatch: coordinate space, logical ordinal,
logical incarnation, selected accepted/candidate table, physical block
incarnation, initialized prefix, annex publication serial, expected annex
codec/length, region envelope, predecessor boundary, or child-root admission.
Validation completes before the physical store lends a reference. Hashes,
serialized formats, artifacts, and diagnostics never use a physical id.

## Aggregate node and annex envelopes

### Boundary and closure isolation

Every storage boundary is aggregate:

```text
AggregateNodeMark
  region_id and generation
  boundary_serial
  node logical cursor and table frontier
  annex logical cursor and table frontier
  node and annex physical-tail identities
  list-summary and dependency frontiers
```

An operation mark may name partial tails and is local rollback authority. A
checkpoint mark is a copy-only projection created only after all builders are
quiescent; creating it changes neither store. A `ClosureBuildMark` is
move-only. Beginning a new semantic region or closure build seals both current physical tails and
rotates the first subsequent node and annex append to fresh physical blocks.
The unused capacity is reported as boundary slack. Rotation is lazy, so an
empty canceled build allocates no block.

This paired rotation is mandatory even when only one side currently has a
partial tail. It guarantees that a physical block never straddles semantic
region owners, and that every record and every region-local annex word
published by the closure occupies whole physical blocks which are not shared
with the retained source prefix. A large dynamic annex span may use several
such blocks, but none contains pre-boundary words. The design does not copy the
retained annex tail, retain the source owner, or rewrite a key to make that
isolation true.

Node publication folds two floors into physical and logical block metadata:
the earliest child-node envelope and the earliest region-local annex envelope
named by any resident record. Typed annex construction similarly propagates
the floor of nested spans into the final fixed payload. Sealing a closure
checks its declared roots, predecessor ranges, and these maintained floors
against the build boundary. It visits table/block metadata only; it does not
scan `NodeRecord`, decode annex payload, or traverse the node tree. A root or
nested dependency before the boundary selects the explicit structural-copy
fallback and cannot be detached.

### Move receipts and rollback

The facade exposes no separate node-batch and annex-batch methods. Its
transaction types are move-only and pair both components:

1. `OpenAggregateBuild` owns the source `ClosureBuildMark`. Construction
   failure consumes an `AggregateOperationReceipt` which restores the node
   cursor, annex cursor, logical-table frontiers, predecessor/summary metadata,
   and active-builder state. Publication serials and boundary serials remain
   monotonic; they are never rewound into an ABA alias.
2. `seal_aggregate_closure` first validates the declared root and all maintained
   floors without mutation. On failure it returns the original open-build
   authority. On success it returns `SealedAggregateEnvelope`, naming exact
   node and annex logical-row ranges and whole physical-block ranges.
3. `detach_aggregate_envelope` removes both ranges from the source region and
   returns `DetachedAggregateEnvelope` plus an internal
   `AggregateDetachReceipt`. The receipt records source region generation,
   boundary serial, exact insertion frontiers, roots, and both table/block
   ranges. No physical mapping changes.
4. `prepare_aggregate_transfer` validates source and destination pools, views,
   region generations, destination quiescence, table capacity, and disjoint
   envelopes, and reserves all destination metadata capacity. Failure returns
   the unchanged detached loan. `commit_aggregate_transfer` is then infallible:
   it moves the paired envelope into the destination owner and wraps the same
   `PageListId` in a destination `RegionRoot`.

If a destination is rejected, unwinds, or fails preflight after detachment,
`rollback_aggregate_detach` uses the receipt to reinsert both ranges at their
exact source frontiers. It first proves that the source has not appended or
accepted another transfer since the boundary serial. A mismatch returns the
still-owned detached loan rather than partially restoring it. No API can
attach nodes while leaving annex blocks detached, or vice versa. Whole-region
moves use the same prepared receipt with ranges beginning at the region's
first envelope.

This protocol applies to page-to-durable box publication, unique `\box` and
`\unhbox`, durable-to-page moves, PDF-form ownership, history preservation,
page succession, and transient output staging. History and output receive
owners, not raw coordinates. Successful output lowers a handle-free artifact
while the transient owner is borrowed, then retires the aggregate envelope;
the artifact retains no table or region.

### Copy semantics

Rust `Copy` on `NodeRecord` authorizes only candidate-tail duplication inside
one logical coordinate space. It is not TeX copy authority and is not a
cross-owner copy API.

`\copy`, `\unhcopy`, retained-source preservation, a foreign-pool transition,
and every explicit structural fallback traverse the exact source closure once
under its admitted view and call destination-directed constructors. Child
lists are recursively rebuilt under destination envelopes. Every fixed annex
record and dynamic annex span is decoded and republished into destination-local
annex blocks; no destination key retains a source annex space or envelope.
Font and generation-token values retain only the separately authorized
immutable aliases, and are republished when their generation/store contract
does not admit the destination. One aggregate destination operation mark
restores both arenas on any failure. Raw `NodeRecord` copying across semantic
owners is private and rejected even when source and destination share a pool.

### Lifetime wrappers

The physical substrate remains generic, but these policies are distinct:

| Lifetime                 | Wrapper and behavior                                                                                                                                                                                                                                                     |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| TeX group                | Group entry saves state/journal cursors and move-only durable closure owners. It does not fork or own page-node blocks. Restoration moves saved aggregate owners back before dropping replacements. `GroupStorage<T>` remains for genuinely group-local non-node values. |
| Page and durable closure | `NodeRegion<Role>` owns aggregate node+annex envelopes admitted through the pool tables. Page regions may participate in the one edit fork; durable regions move or copy according to TeX semantics.                                                                     |
| Paragraph checkpoint     | `PageRegionCheckpoint` stores aggregate cursors and four admitted roots under its history-owned page region. It is not an arena wrapper or node owner and capture allocates/copies zero. `CheckpointJournal<T>` stores metadata/journal values, not nodes.               |
| Operation and scratch    | `AggregateOperationMark` and paired node/annex scratch wrappers rewind partial tails together and never fork. Completed scratch is reconstructed into the destination or moves only when it already owns a self-contained aggregate envelope.                            |
| Speculative output       | A move-only transient `NodeRegion<OutputRole>` keeps candidate material private. Commit lowers detached output and retires it; rejection returns or drops the whole aggregate owner. It never acquires generation-fork semantics.                                        |

No group, journal, output, or scratch wrapper becomes forkable merely because
it uses exact superblocks. Only the generation/page aggregate owns the
accepted/candidate table transaction.

### Required substrate APIs

`tex-dense-arena` currently puts its physical store inside one
`GenerationArena`, so it cannot yet support multiple semantic regions or
move-only envelope transfer. Before node integration it must expose, in safe
Rust:

- a typed `BlockStore<T>` whose private `BlockId` can be resolved, copied into
  one fresh tail, and released only by owner receipts;
- a flat logical table with checked ordinal/incarnation rows, initialized
  prefixes, direct lookup, cursor/truncate, and explicit table-copy metrics;
- `AcceptedCandidateTables<T>` which consumes the accepted authority, lends
  exactly two view types, copies at most one initialized physical tail, and
  settles by consuming itself;
- physical-tail rotation and whole-block-range detach/reattach/attach receipts
  which reserve metadata before their infallible commit phase;
- fallible capacity/incarnation/length checks before any table or owner
  mutation; and
- counters for physical blocks, logical rows, table entries/bytes copied,
  payload tail values/bytes copied, boundary slack, transfers, rollback, stale
  rejection, and accepted/rejected payload copies.

`tex-dense-arena` does not learn about lists, children, TeX roles, node codecs,
or annex dependency floors. `ForkArena` retains those semantic responsibilities
above the new store/table APIs and replaces `RawChunkKey` in its public and
stored coordinates in one migration. `NodeRegion` alone couples the node and
annex components and issues aggregate receipts.

### Failure matrix

Every failure is detected before mutation or returns the sole move-only loan:

| Failure                                                                                                                | Required result                                                                                                                                        |
| ---------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Foreign pool space, region generation, or accepted/candidate view                                                      | Reject before table or payload access.                                                                                                                 |
| Stale logical row, physical `BlockId`, annex serial, predecessor, or root                                              | Reject before returning a borrow; increment only the matching observation counter.                                                                     |
| Cursor beyond initialized prefix, integer overflow, table/slot incarnation exhaustion, or fixed codec crossing a block | Return the typed capacity/coordinate error with all owners and cursors unchanged.                                                                      |
| Open builder, open batch, third candidate, or destination with an unsettled transfer                                   | Reject before sealing, detaching, reserving, or changing roots.                                                                                        |
| Closure root or child/annex dependency before its build boundary                                                       | Keep the source attached and select the explicit structural-copy path; never scan to rescue a move.                                                    |
| Allocation or destination metadata reservation failure                                                                 | Return the original open build or detached aggregate loan; neither component attaches.                                                                 |
| Failure after detach but before prepared commit                                                                        | Consume `AggregateDetachReceipt` to restore both exact source frontiers; if the source frontier changed, return the detached loan instead of guessing. |
| Panic during destination construction or output staging                                                                | An unwind guard restores aggregate operation marks or the detached loan before outer state rejection proceeds.                                         |
| Semantic copy decode, child, font, token, UTF-8, PDF, or annex validation failure                                      | Roll back destination node and annex cursors together and leave source ownership unchanged.                                                            |
| Acceptance/rejection preflight failure                                                                                 | Keep the two-view fork intact and usable for explicit rejection; never settle one table.                                                               |

An internal prepared commit has no recoverable validation branch. All vector
capacity, ranges, owners, incarnations, and destination quiescence were proven
by its constructor; commit only moves already-owned rows and blocks. This is
the point which prevents a partially attached annex from becoming an unwind
recovery problem.

## Lifetime-specific coordinates

`NodeListId<L>` is a private-construction row and row-generation coordinate
branded by its semantic lifetime. It is never an owner. Row zero is the
canonical empty list and resolves without storage. The coordinate families
are:

- `ScratchListId` for unfinished shaping, transforms, packing probes, and
  speculative operation material;
- `PageListId` for generation-owned open modes, alignments, insertions, and
  page-builder material. Durable closures use the same pool-stable logical
  coordinates only while admitted through their move-only region owner.

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
region id, four PageBuilder roots, sealed node and annex cursors, logical-table
frontiers, scalar state, and journal position. It copies no payload and creates
no segment, batch, or list owner.

## Payload placement

Page rows carry final glue values directly. After the compact cutover, a node
token field is a coordinate into the generation's immutable stored-token
arena; constructing a mark, deferred write, deferred special, deferred PDF
literal, PDF navigation value, or alignment template therefore copies no token
words. Synthesized node-only token fields publish their final arena span once.
The current node-side `Rc` owner is migration input, not part of the selected
layout. Neither path adds an `Arc`, `Weak`, registry row, or per-node lookup
owner.

Diagnostic origins are copy-only coordinates and remain excluded from TeX
node equality and artifact bytes. A selected shipout provenance consumer
detaches stable source recipes while the page arena is still borrowed.

## Compact non-owning node record

### Current production layout audit

The supported 64-bit compiler layout for `Node<PageListId>` is 168 bytes at
alignment 8 and `needs_drop::<Node<PageListId>>() == true`. The audit command
uses compiler layout output for the concrete page monomorphization, not an
estimate from source syntax. Its important component widths are:

| Component                          | Size / alignment | Ownership                                                               |
| ---------------------------------- | ---------------: | ----------------------------------------------------------------------- |
| `FontId`                           |           16 / 8 | Copy generation coordinate                                              |
| `PageListId`                       |           40 / 8 | Copy owner-relative list coordinate and optional semantic identity      |
| `GlueSpec`                         |           16 / 4 | Copy value                                                              |
| `NodeTokenList`                    |           24 / 4 | Transitional alias for copy-only `NodeTokenKey`; no `Drop`              |
| `BoxNode<PageListId>`              |          120 / 8 | Copy scalars, one child, one optional diagnostic child                  |
| `UnsetNode<PageListId>`            |           72 / 8 | Copy scalars and one child                                              |
| `MathField<PageListId>`            |           48 / 8 | Copy tagged field; its largest alternative is one 40-byte child         |
| `Whatsit<GlueSpec, NodeTokenList>` |           56 / 8 | Mixed; some alternatives own `String`, `Vec`, or `Box`; tokens are keys |

The number in the current-size column below is the compiler-reported variant
payload after the one-byte outer discriminant, including variant-local
padding. A variant which has no owned field still pays the 168-byte enum width.

| Node kind      | Current size | Exact fields and current ownership                                               | Selected compact placement                                                 |
| -------------- | -----------: | -------------------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| `Char`         |           31 | `FontId` 16, `char` 4, `OriginId` 4; all Copy                                    | Inline                                                                     |
| `Lig`          |           71 | `FontId` 16, `char` 4, two flags, owned `Vec<char>` 24, owned `Vec<OriginId>` 24 | Typed `LigaturePayload` plus packed `LigatureSource` span in the annex     |
| `Kern`         |            7 | `Scaled` 4 and `KernKind` 1; Copy                                                | Inline                                                                     |
| `MarginKern`   |           23 | `Scaled` 4, side 1, `FontId` 16, character byte 1; Copy                          | Inline                                                                     |
| `Glue`         |          151 | `GlueSpec` 16, kind 1, optional `LeaderPayload` 128; Copy but large              | Ordinary and rule leaders inline; h/v leader uses typed `LeaderBoxPayload` |
| `Penalty`      |            7 | `i32`; Copy                                                                      | Inline                                                                     |
| `Rule`         |           27 | Three `Option<Scaled>` values, 8 each; Copy                                      | Three inline values plus presence flags                                    |
| `HList`        |          127 | `BoxNode<PageListId>` 120; Copy                                                  | Typed `BoxPayload`                                                         |
| `VList`        |          127 | `BoxNode<PageListId>` 120; Copy                                                  | Typed `BoxPayload`                                                         |
| `Unset`        |           79 | `UnsetNode<PageListId>` 72; Copy                                                 | Typed `UnsetPayload`                                                       |
| `Disc`         |          127 | Three 40-byte child coordinates, kind, and physical replace count; Copy          | Typed `DiscPayload`                                                        |
| `Mark`         |           31 | class 2 plus copy-only 24-byte `NodeTokenList` coordinate                        | Class inline and a generation-owned `NodeTokenKey` inline                  |
| `Ins`          |           71 | class, size, `GlueSpec`, depth, penalty, and one 40-byte child; Copy             | Typed `InsertionPayload`                                                   |
| `Whatsit`      |           63 | One 56-byte whatsit; ownership varies by subtype and is audited below            | Scalar subtypes inline; typed annex record or span for dynamic subtypes    |
| `MathOn`       |            7 | `Scaled`; Copy                                                                   | Inline                                                                     |
| `MathOff`      |            7 | `Scaled`; Copy                                                                   | Inline                                                                     |
| `Direction`    |            1 | `MathBoundary`; Copy                                                             | Header subtype                                                             |
| `MathNoad`     |          167 | `NoadKind` 12 and three 48-byte `MathField` values; Copy in substance            | Typed `MathNoadPayload`                                                    |
| `FractionNoad` |          111 | two 40-byte children, thickness, and two optional delimiters; Copy               | Typed `FractionPayload`                                                    |
| `MathStyle`    |            1 | style byte; Copy                                                                 | Header subtype                                                             |
| `MathChoice`   |          167 | four 40-byte children; Copy                                                      | Typed `MathChoicePayload`                                                  |
| `MathList`     |           55 | display flag and one 40-byte child; Copy                                         | Typed `ListPayload`                                                        |
| `Nonscript`    |            0 | no fields                                                                        | Header only                                                                |
| `Adjust`       |           55 | pre flag and one 40-byte child; Copy                                             | Typed `ListPayload`                                                        |

The whatsit audit is exhaustive because it contains most of the hidden
ownership:

| Whatsit subtype                                                          | Current size | Owned field                                    | Selected placement                                               |
| ------------------------------------------------------------------------ | -----------: | ---------------------------------------------- | ---------------------------------------------------------------- |
| `OpenOut`                                                                |           31 | `String` path                                  | Inline typed UTF-8 annex span                                    |
| `CloseOut`                                                               |            2 | none                                           | Inline                                                           |
| `DeferredWrite`                                                          |           31 | Copy-only `NodeTokenList` coordinate           | Inline `NodeTokenKey`                                            |
| `Special`                                                                |           55 | `String` class and `Vec<u8>` payload           | Typed `SpecialPayload` containing two annex spans                |
| `DeferredSpecial`                                                        |           55 | `String` class and copy-only token coordinate  | Typed `DeferredSpecialPayload` containing one span and token key |
| `PdfReferenceObject`, `PdfAnnotation`, `PdfLinkStart`, `PdfLinkEnd`      |            7 | none                                           | Inline                                                           |
| `PdfAccessibility`, `PdfRunningLink`                                     |            1 | none                                           | Header subtype/flag                                              |
| `PdfLiteral`                                                             |           31 | `Vec<u8>`                                      | Mode in header and inline annex span                             |
| `DeferredPdfLiteral`                                                     |           31 | Copy-only `NodeTokenList` coordinate           | Mode in header and inline token key                              |
| `PdfSetMatrix`                                                           |           31 | `Vec<u8>`                                      | Inline annex span                                                |
| `PdfSave`, `PdfRestore`, `PdfSavePos`, `PdfSnapRefPoint`, `PdfEndThread` |            0 | none                                           | Header subtype                                                   |
| `PdfColorStack`                                                          |           39 | `Vec<u8>` in `Set` or `Push`                   | Id/action in header/word and inline annex span                   |
| `PdfSnapY`                                                               |           19 | none; `GlueSpec` is Copy                       | Inline                                                           |
| `PdfSnapYComp`, `Language`                                               |            3 | none                                           | Inline                                                           |
| `PdfRefXForm`, `PdfRefXImage`                                            |           19 | none                                           | Inline                                                           |
| `PdfDestination`                                                         |           15 | boxed 60-byte record; identifier stores a key  | Typed `PdfDestinationPayload`; identifier stores a token key     |
| `PdfThread`                                                              |           15 | boxed 80-byte record and two token coordinates | Typed `PdfThreadPayload`; both token values are coordinates      |

No retained field in the replacement record owns a `Vec`, `String`, `Box`,
`Rc`, allocator callback, reference count, or destructor obligation.

### Exact record layout

The canonical runtime representation replaces the enum; it does not sit
beside it:

```rust
#[repr(C)]
#[derive(Clone, Copy)]
struct NodeRecord<Lane> {
    header: u32,
    words: [u32; 7],
    lane: PhantomData<fn(&Lane) -> &Lane>,
}
```

The zero-sized brand does not affect layout. Compile-time assertions require
size 32, alignment 4, `Copy`, and `needs_drop == false` on every supported
target. The header assigns bits 0--4 to the 24-value node kind, bits 5--9 to a
kind-local subtype, and bits 10--31 to presence bits, the 16-bit insertion or
mark class, physical replace count, and other kind-local flags. Invalid kind,
subtype, flag, Unicode scalar, enum, option, and coordinate encodings fail
closed.

Seven payload words are the exact common-inline budget. They hold a full
16-byte `FontId`, Unicode scalar, and origin for `Char`; a full font, amount,
and character for `MarginKern`; all four glue words plus all three rule-leader
dimensions; or one six-word typed handle plus one scalar. Every unused word is
zeroed. Code never transmutes the record, hashes padding, or serializes native
bytes: constructors and views explicitly encode and decode integers, scaled
values, handles, and enums.

`NodeRecord` fields and raw append are private to the node-storage module.
Consumers match a borrowed `NodeView<'region, Lane>` projection whose large
alternatives borrow typed annex views. That projection is neither stored nor
an owner and therefore is not a second node representation. Construction uses
variant-specific destination methods such as `push_char`, `push_box`, and
`push_math_noad`; there is no owned `NodeDraft` enum crossing the arena. Cold
format DTOs stream through the same methods and never become a second runtime
topology.

### Typed word annex

The pool's one logical `NodeAnnexArena<Lane>` stores every rare or large
page-lifetime field. Each `NodeRegion` owns the disjoint annex envelopes which
its nodes may admit. The physical payload is `u32`, but private typed codecs
and invariant-branded handles preserve field types:

```text
AnnexKey<K>
  space: u32
  block_ordinal: u32
  logical_block_incarnation: u32
  word_offset: u32
  word_len: u32
  publication_serial: u32
```

`AnnexKey<K>` is exactly 24 bytes, or six node words. `AnnexSpan<E>` has the
same six-word coordinate shape and interprets `word_len` in the element codec
for `E`. Constructors are private. Resolution verifies space, region envelope, flat-table
ordinal, logical block incarnation, initialized bounds, publication serial,
expected fixed word count or span divisibility, and the type selected by the
node header before returning a borrowed view. The flat view table maps that
logical coordinate to the current accepted or candidate physical `BlockId`.
Fork-tail duplication gives the private physical block the same logical
incarnation, so an unchanged copied key resolves through the candidate table
without rewriting the node. A stale, foreign, truncated, or kind-mismatched
key cannot alias replacement storage.

The annex uses the same exact 64 KiB dense superblocks as the node arena. One
block contains 16,384 words. A fixed record is padded to the next block rather
than crossing a boundary, so it resolves through one direct flat-table lookup.
Dynamic spans may cross blocks and cost one direct lookup per block actually
consumed. The largest fixed record is the 41-word math choice, so boundary
padding is at most 160 bytes. There is one annex table and one annex cursor,
not one allocation or cursor per payload type.

Every fixed record starts with its publication serial; the sizes below include
that word. Reusing a truncated offset assigns a fresh serial without changing
the logical incarnation of valid earlier records in the same block. The
pool's next-publication serial is monotonic across rejection and rollback,
never rewinds, and fails before u32 exhaustion. Dynamic spans begin with the
same serial word. The fixed codecs are:

| Typed codec              | Words | Exact content                                                                                               |
| ------------------------ | ----: | ----------------------------------------------------------------------------------------------------------- |
| `LigaturePayload`        |    12 | serial, font 4, displayed character 1, and one six-word `LigatureSource` span; flags are in the node header |
| `BoxPayload`             |    29 | serial, four dimensions, glue ratio, packed box/glue enums, child, optional diagnostic child, overlap       |
| `LeaderBoxPayload`       |    33 | serial, four glue words, and the 28 content words of `BoxPayload`                                           |
| `UnsetPayload`           |    16 | serial, child, and five scaled values; span count, orders, and box kind are in the node header              |
| `DiscPayload`            |    31 | serial and three children; kind and physical count are in the node header                                   |
| `InsertionPayload`       |    18 | serial, child, glue, size, split depth, and penalty; class is in the node header                            |
| `MathNoadPayload`        |    37 | serial, noad kind, and three fixed 11-word math fields                                                      |
| `FractionPayload`        |    24 | serial, two children, thickness, and delimiter values; presence flags are in the node header                |
| `MathChoicePayload`      |    41 | serial and four children                                                                                    |
| `ListPayload`            |    11 | serial and one child; `MathList` display or `Adjust` pre is in the node header                              |
| `SpecialPayload`         |    13 | serial, UTF-8 class span, and byte payload span                                                             |
| `DeferredSpecialPayload` |    13 | serial, UTF-8 class span, and generation token key                                                          |
| `PdfDestinationPayload`  |    14 | serial, identifier, structure value, and destination values; tags/presence are in the node header           |
| `PdfThreadPayload`       |    19 | serial, typed identifier, dimensions, attributes token key, and running flag                                |

A `LigatureSource` element is two words: one validated Unicode scalar and one
`OriginId`. A flag distinguishes the current empty-origins diagnostic case
from a full origin vector, so format and diagnostic round trips do not invent
origins. Byte strings pack four bytes per annex word and retain their exact
byte length; UTF-8 spans validate once at publication. The side record never
stores another node kind and cannot resolve without its originating node's
kind-specific codec, so it is overflow storage rather than a parallel node
enum.

Node token payloads are the one deliberate non-region lifetime. A compact
six-word `NodeTokenKey<Generation>` addresses immutable words in the coarse
generation token store. It uses the same owner, logical ordinal/incarnation,
offset, length, and publication-serial proof, resolved through the admitted
accepted or candidate token table. The token store, not an `Rc` cloned into
each node, owns those words until generation retirement. This is required
because TeX copy shares stored token values and a durable closure can outlive
the page region which first referenced it. Synthesized node-only token lists
publish once into that same generation store. Generation settlement keeps
accepted prefix words under its one accepted/candidate transaction and drops
rejected or superseded suffixes only after every node region which can name
them has settled.

The dense `PageListId` head and tail coordinates follow the same view-relative
rule: they name logical node positions and logical block incarnations, while
the admitted accepted or candidate node table supplies the physical block.
Forking therefore changes neither embedded child coordinates nor retained
`PageListSpan` roots.

### Construction, access, and mutation

Publication is one aggregate transaction:

1. validate source fonts, token keys, and every child `PageListId` under the
   admitted region;
2. append dynamic annex spans and then the fixed typed payload, if any;
3. reserve the final node slot and write its complete 32-byte record there;
4. update the node block's maximum referenced annex/token frontier and child
   dependency floor; and
5. publish the longer node prefix only after every field and summary succeeds.

Failure restores the annex cursor before abandoning the unpublished node
slot. There is no per-node heap allocation and no complete temporary node
value. Common inline construction touches only the node arena. A rare fixed
payload adds one append to the already-warmed annex; dynamic data streams once
into that same annex.

Published nodes and annex records are immutable. Operation-local math,
alignment, shaping, and shipout scratch may mutate their existing typed
scratch builders, then publish through the same variant-specific methods.
Scratch arenas use the same record and annex codecs but remain nonforking and
rewind both cursors together. No mutable annex reference survives node
publication, append, rollback, settlement, or region transfer.

An inline read performs one node quotient/remainder and one flat node-table
lookup. A fixed overflow read adds one annex quotient/remainder and one flat
annex-table lookup. Consuming a dynamic span adds one lookup per crossed annex
block, proportional to bytes or ligature elements actually consumed. List
children remain the existing `PageListId`/`PageListSpan` direct roots; no list
lookup gains a descriptor walk, owner-range search, linked list, forwarding
coordinate, or allocation.

### Forks, handle validity, and reclamation

A sealed node-storage boundary contains paired logical cursors and their
constant-size table and summary frontiers: one component for records and one
for annex words. Ordinary TeX groups, page attempts, and all
1-versus-4,096 checkpoint captures copy neither arena. The block summaries
prove at sealing time that every node at or before the boundary names only an
annex/token prefix admitted by that same aggregate boundary.

An exact edit fork uses one aggregate two-view wrapper whose node and annex
components settle together. Complete blocks are shared immutable. An interior node tail
copies `32 * tail_nodes`, with an exact maximum of 65,504 bytes for 2,047
records. An interior annex tail copies `4 * tail_words`, with an exact maximum
of 65,532 bytes for 16,383 words. Candidate keys retain the same flat
block ordinals and logical incarnations because each copied tail occupies the
corresponding private physical block. A boundary-aligned tail copies zero. The
exact node-region payload-copy maximum is therefore 131,036 bytes per fork,
independent of accepted prefix, candidate suffix, list count, or checkpoint
count. The generation token store has its own exact 65,532-byte tail maximum,
charged once to aggregate generation settlement rather than once per node
region. If all three tails are interior and maximal, the aggregate bound is
196,568 bytes.

Rejection drops candidate-private node and annex blocks and restores the exact
accepted tables and cursors. Acceptance first removes roots and journals that
name the superseded accepted suffix, then drops that node suffix, then drops
its annex suffix, and finally moves the candidate flat tables into the
accepted owner. Acceptance copies zero payload. Reusable physical block slots
and newly occupied logical ordinals increment their respective incarnations
before publication. Reusing an offset in a surviving partial block changes its
publication serial. Stale keys therefore fail even if ordinal, slot, and
offset recur. The at-most-two-generation rule applies to both tables as one
aggregate transaction; no third node or annex view can be created.

Page and durable `NodeRegion` owners reclaim node and annex blocks wholesale
after roots, children, journals, active builders, and output borrows retire in
the order defined by [Node-region ownership](node_region_ownership.md).
Region-local paths, PDF bytes, ligature source rows, and every fixed annex
payload therefore retire with the page, box, form, or history region that owns
them. Immutable token words retire with their coarse generation after its node
regions. Handle-free DVI/PDF artifacts retain neither key.

A TeX move transfers the paired node and annex envelopes and rewrites no record. A TeX
copy or history-preservation copy traverses the exact closure and invokes
destination constructors: child roots and region-local annex spans are rebuilt
under the destination owner, while generation token and font keys keep their
authorized immutable aliases. Raw record copying into another region is not
an API. Semantic copies always reconstruct destination-local annex data, even
when source and destination share a pool. These rules keep
the record mechanically `Copy` for bounded generation-tail duplication
without turning Rust `Copy` into TeX move/copy authority.

### Quantitative contract

The representation changes the resident record from 168 to 32 bytes, a
136-byte or 80.95% reduction. An exact node superblock holds 2,048 records
with zero tail slack instead of 390 current records with 16 bytes of slack, a
5.25-times density increase. Annex capacity is 16,384 words per block. For `N`
resident records and `W` initialized annex words, physical allocation is
`ceil(N / 2,048) + ceil(W / 16,384)` exact 64 KiB blocks, plus the generation
token blocks charged to their owner. The physical table retains one 8-byte
incarnation-bearing `BlockId` per 64 KiB of payload. Logical list-range rows
and their copied bytes are accounted separately. Fork metadata copy reports
both physical entries and logical rows; it is never folded into the bounded
payload tails.

The deterministic gates report:

- zero `needs_drop` and exact 32/4 size/alignment for every node lane;
- one node allocation per 2,048 cold records and no allocation after warmed
  capacity unless a node or annex block boundary is crossed;
- zero per-node `Vec`, `String`, `Box`, `Rc`, or individually scoped heap
  owner; allocator calls equal exact superblocks crossed, including blocks
  required by a genuinely large dynamic span;
- one payload lookup for inline nodes and exactly one additional lookup for a
  fixed annex payload;
- zero node/annex copy at checkpoint capture, exact maxima of 65,504 node and
  65,532 annex bytes at fork, and zero payload copy at acceptance;
- exact table entries/bytes copied, node/annex tail values/bytes copied,
  boundary padding, live bytes, reusable capacity, and stale-key rejections;
  and
- removal of the authenticated 3,143,705 by 168-byte release row, or
  528,142,440 bytes, without comparable construction, append, token, annex,
  allocator, `memmove`, or other copy-API displacement.

Native and WebAssembly use the same explicit u32 record and codec. Neither
stores pointers, assumes an operating-system page, hashes allocation identity,
or serializes runtime words. The 64 KiB allocation and all word/byte/coordinate
arithmetic remain checked in target `usize`; external ids remain explicitly
bounded to their u32 domains.

### Source decomposition before integration

`crates/tex-state/src/node_record.rs` is already 2,392 lines after the isolated
codec work. Table and envelope integration must not extend that monolith. The
first implementation child performs a behavior-preserving module split with
these ownership boundaries and approximate ceilings:

- `node_record/mod.rs`, under 250 lines: private exports, module wiring, shared
  record/view entry points, and compile-time layout assertions;
- `node_record/layout.rs`, under 350 lines: `NodeRecord`, header/kind/subtype
  encoding, scalar word helpers, and typed key extraction;
- `node_record/annex.rs`, under 550 lines: `AnnexKey`, typed markers, fixed and
  dynamic codecs, publication serial validation, and the logical annex facade;
- `node_record/node_codec.rs`, under 600 lines: outer node, box, glue, ligature,
  insertion, and math encode/decode projections;
- `node_record/whatsit_codec.rs`, under 550 lines: every whatsit, PDF payload,
  byte/UTF-8 span, and identifier codec; and
- `node_record/tests.rs`, split further by codec family if it exceeds roughly
  600 lines.

Logical table and ownership code belongs beside, not inside, that codec tree:
`logical_node_table.rs` owns logical coordinates and borrowed views;
`node_envelope.rs` owns aggregate marks and transfer receipts; existing
`fork_arena.rs` retains list topology; and `node_region.rs` retains semantic
roles and TeX transitions. The split lands before new behavior so review can
distinguish mechanical movement from the coordinate and ownership changes.

### Migration and approval boundary

No stage introduces a second resident node representation:

1. Replace node-side `Rc` token payloads with generation-owned immutable token
   coordinates and prove group, checkpoint, copy, rollback, and retirement
   lifetimes independently.
2. Move every consumer from direct enum matching to borrowed `NodeView` and
   every producer to variant-specific destination construction while the
   existing enum is still the sole backing representation.
3. Implement the private annex codecs and native/WASM layout tests without
   wiring a second node store. Split the codec source before adding storage
   integration.
4. Add the pool-owned logical node/annex tables and exactly-two-view fork API
   below `ForkArena`; replace every physical `RawChunkKey` in list coordinates
   and predecessor metadata while the enum remains the sole resident node.
5. Add aggregate marks, paired envelope rotation, detach/transfer/rollback
   receipts, dependency floors, and stale-key controls. Prove page/durable,
   history, succession, and transient-output moves before changing backing.
6. In one cutover commit, replace the enum backing with `NodeRecord`, connect
   the typed annex, and delete the owned ligature, token, whatsit, and boxed
   payload fields from resident nodes.
7. Re-prove TeX move/copy, held-over evacuation, page succession, exact edit
   accept/reject, scratch rewind, semantic hashing, format materialization,
   and handle-free output before enabling dense fork-tail copy in production.
8. Run the focused construction/fork/release attribution, routine suite,
   quality gate, Miri where available, sanitizers, and Wasm runtime tests, then
   stop with the cutover unmerged.
9. In the separate measurement task, run the authenticated 50-million-command
   census and seek the dense-superblock design's second approval before any
   production merge.

Any failed ownership, stale-key, semantic, allocation, Wasm, or copy census
removes the cutover commit. It must not retain the old enum as fallback, add a
compatibility representation, add per-node allocation or reference counts, or
weaken the direct-list and two-generation contracts.

## Construction and rollback

An arena publishes a list only after validating every declared child
coordinate. Validation failure leaves the arena unchanged. The arena owns the
complete immutable row; nested node fields contain only coordinates back into
that same arena.

`AggregateOperationMark<L>` may name partially used node and annex tails and
is restricted to local failure. It cannot convert into a retained mark.
Consuming every builder and sealing both components yields an opaque aggregate
boundary, which alone can construct a checkpoint mark. A retained mark
therefore contains logical cursors plus stable table/summary frontiers. The
operation mark carries the optional scalar summaries of its partial tails, so
truncation restores payload, topology, and identity metadata atomically.
Whole-block detach, reattach, acceptance, pruning, and typed-role transfer move
the summary-bearing aggregate envelope with its node and annex payload.
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

`SealedAggregateEnvelope<L>` is the consuming form of the paired cursors for a
structurally nested allocation suffix. It is neither `Clone` nor `Copy`. A top-level
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
