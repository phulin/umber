# Arena-owned node lists

## Status

This document defines the compact chunk/range implementation below
[Node-region ownership](node_region_ownership.md), which is authoritative for
coarse node lifetime, TeX move/copy semantics, and exact checkpoint history.
The 32-byte non-owning node-record design below is implementation-ready but is
not yet the production representation. Its approval and cutover are tracked by
`umber2-66p0.8.40.113.5` and its children.

Production currently uses one caller-owned `ChunkPool<T>` plus typed,
coordinate-only `ForkArena<T, Lane>` states. The compact cutover replaces the
physical payload slots with the exact 64 KiB dense node and annex arenas from
[Dense fork-arena superblocks](fork_arena_dense_prefix_emplacement.md).
Existing list blocks become logical cursor/range boundaries inside that dense
storage; they are not physical allocations. Lists remain copy-only borrowed
coordinates and own no payload. Reads take a shared region borrow and use flat
direct indexing. Allocation, sealing, settlement, and pruning require the
caller's exclusive mutable owner borrow.

Every nonempty list, including a one-range list, publishes its range entries
in descriptor chunks and is named by the current 32-byte `ArenaListId`. The
page-semantic wrapper adds one optional maintained identity scalar, making
`PageListId` exactly 40 bytes. The compact record keeps that complete logical
coordinate in the annex rather than weakening its owner or identity proof.

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

One `NodeAnnexArena<Lane>` owned by the same `NodeRegion` stores every rare or
large page-lifetime field. Its physical payload is `u32`, but private typed
codecs and invariant-branded handles preserve field types:

```text
AnnexKey<K>
  owner: u32
  block_ordinal: u32
  logical_block_incarnation: u32
  word_offset: u32
  word_len: u32
  publication_serial: u32
```

`AnnexKey<K>` is exactly 24 bytes, or six node words. `AnnexSpan<E>` has the
same six-word coordinate shape and interprets `word_len` in the element codec
for `E`. Constructors are private. Resolution verifies owner, flat-table
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
owner's next-publication serial is monotonic across rejection and rollback,
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

A sealed node-storage boundary contains exactly two 24-byte cursors: one for
records and one for annex words. Ordinary TeX groups, page attempts, and all
1-versus-4,096 checkpoint captures copy neither arena. The block summaries
prove at sealing time that every node at or before the boundary names only an
annex/token prefix admitted by that same aggregate boundary.

An exact edit fork uses the two-view dense wrapper independently for the node
and annex arenas. Complete blocks are shared immutable. An interior node tail
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

A TeX move transfers the one region envelope and rewrites no record. A TeX
copy or history-preservation copy traverses the exact closure and invokes
destination constructors: child roots and region-local annex spans are rebuilt
under the destination owner, while generation token and font keys keep their
authorized immutable aliases. Raw record copying into another region is not
an API. Same-region semantic copies may reuse immutable annex spans only when
the recursive child topology needs no destination rebrand. These rules keep
the record mechanically `Copy` for bounded generation-tail duplication
without turning Rust `Copy` into TeX move/copy authority.

### Quantitative contract

The representation changes the resident record from 168 to 32 bytes, a
136-byte or 80.95% reduction. An exact node superblock holds 2,048 records
with zero tail slack instead of 390 current records with 16 bytes of slack, a
5.25-times density increase. Annex capacity is 16,384 words per block. For `N`
resident records and `W` initialized annex words, physical allocation is
`ceil(N / 2,048) + ceil(W / 16,384)` exact 64 KiB blocks, plus the generation
token blocks charged to their owner. Each flat block-table entry remains one
8-byte incarnation-bearing `BlockId` per 64 KiB of payload. Fork metadata copy
is exactly eight bytes times the number of complete shared blocks in each
forked table; it is reported separately from the bounded payload tails.

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

### Migration and approval boundary

No stage introduces a second resident node representation:

1. Replace node-side `Rc` token payloads with generation-owned immutable token
   coordinates and prove group, checkpoint, copy, rollback, and retirement
   lifetimes independently.
2. Move every consumer from direct enum matching to borrowed `NodeView` and
   every producer to variant-specific destination construction while the
   existing enum is still the sole backing representation.
3. Implement the private annex codecs, handles, aggregate marks, stale-key
   controls, and native/WASM layout tests without wiring a second node store.
4. In one cutover commit, replace the enum backing with `NodeRecord`, attach
   the annex to `NodeRegion`, and delete the owned ligature, token, whatsit,
   and boxed payload fields from resident nodes.
5. Re-prove TeX move/copy, held-over evacuation, page succession, exact edit
   accept/reject, scratch rewind, semantic hashing, format materialization,
   and handle-free output before enabling dense fork-tail copy in production.
6. Run the focused construction/fork/release attribution, routine suite,
   quality gate, Miri where available, sanitizers, and Wasm runtime tests, then
   stop with the cutover unmerged.
7. In the separate measurement task, run the authenticated 50-million-command
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
