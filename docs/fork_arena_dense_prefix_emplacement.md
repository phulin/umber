# Dense fork-arena prefix emplacement

Status: proposed low-level storage design; production unsafe code is not
approved.

Parent issue: `umber2-66p0.8.40.113`. Superblock revision:
`umber2-66p0.8.40.113.3`.

## Decision requested

Approve or reject a measured implementation branch for one repository-owned
low-level crate named `tex-dense-prefix`. That crate would be the only runtime
crate allowed to use unsafe code. It would own typed, generation-local
superblocks, perform one coarse combined header-and-payload allocation per
eight dense pages, and expose only safe pool, emplacement, lookup, truncation,
release, and retirement operations. `tex-state`, `tex-exec`, `tex-incr`, and
every other engine crate remain safe Rust and cannot receive a raw pointer,
`MaybeUninit<T>`, unchecked page coordinate, or deallocation capability.

This is a narrow requested exception to the current prohibition on unsafe code
in [Runtime storage lifetimes](runtime_storage_lifetimes.md). Approval
authorizes an isolated implementation and measurement branch only. It does not
approve production adoption or amend that normative contract by itself.
Production adoption still requires a second explicit approval after code
review, Miri and sanitizer gates, semantic and panic/drop gates, the routine
suite, and one authenticated whole-run allocation and public-copy census.

The precise requested response is either **approve the measured
`tex-dense-prefix` superblock branch** or **reject the unsafe superblock
substrate**. Rejection retains the current `Option<T>` page storage. It does
not authorize the per-page `Box`, ordinary `Vec<T>`, or compact-node
alternatives described below.

## Semantic ownership and physical storage are separate

`ForkArena<T, Lane>` remains the semantic owner. It defines arena and region
identity, stable chunk keys, slot generations, at most two lineages, list
predecessor links, sealed boundaries, checkpoint marks, dependency floors,
semantic summaries, and accepted-versus-candidate settlement.

`GenerationChunkStorage<T>` is the physical owner below that facade. It maps a
validated chunk key and offset to a resident `T`, grows coarse typed storage,
and returns released chunk slots to reuse. The current implementation stores
every physical slot as `Option<T>`. That duplicates vacancy state: every live
logical chunk already has exactly one dense initialized prefix `0..used`, and
no operation creates an interior hole.

The proposed change replaces only that physical representation. Fork-arena
coordinates, list topology, node-region ownership, lineage rules, rollback,
page succession, checkpoint ordering, and output identity do not change. A
sealed prefix is shared by lineage metadata in `ForkArena`, not by sharing an
allocation owner.

## Current payloads and allocation frequency

There are four production payload instantiations:

| Payload                            | Measured x86-64 size | Use                                     | Mutation shape                                                    |
| ---------------------------------- | -------------------: | --------------------------------------- | ----------------------------------------------------------------- |
| `Node<PageListId>`                 |            168 bytes | Page and durable node regions           | Append a dense suffix; truncate or retire a suffix or whole chunk |
| `PageInverse`                      |            184 bytes | Page-builder checkpoint inverse journal | Append inverse records; rewind or retire a suffix                 |
| `CheckpointDelta<GenerationBrand>` |             64 bytes | Dense state checkpoint journal          | Append first-write alternates; settle or retire a suffix          |
| `PreparedDviPage`                  |            296 bytes | Accepted/candidate output ledger        | Append per shipout; accept, reject, or retire a suffix            |

All four use `ChunkStorage<T>`, the same per-slot `Option<T>`, and the same
last-lineage assignment to `None`. None has an interior-removal API. The empty
legacy descriptor lane is `ChunkStorage<()>`; it stores no topology, never
requests a chunk, and therefore never grows physical storage.

The current source has a 512-byte logical chunk budget and sixteen chunks per
boxed page. `add_page` performs one `Box<[Option<T>]>` payload allocation each
time the free-chunk stack is empty. The page directory, `ChunkMeta` vector, and
free stack also reallocate geometrically when their capacities are exhausted.
For the measured layouts, the exact static payload-allocation frequency is:

| Payload           | Slots/chunk | Slots/page | Current payload/page | One current payload allocation per |
| ----------------- | ----------: | ---------: | -------------------: | ---------------------------------: |
| `Node`            |           3 |         48 |          8,064 bytes |                48 fresh values max |
| `PageInverse`     |           2 |         32 |          5,888 bytes |                32 fresh values max |
| `CheckpointDelta` |           8 |        128 |          8,192 bytes |               128 fresh values max |
| `PreparedDviPage` |           1 |         16 |          4,736 bytes |                16 fresh values max |

Reuse can make the interval longer: all sixteen released chunks are consumed
before `add_page` runs again. No authenticated whole-run artifact recorded the
dynamic `add_page` or allocator-call count, so this design does not invent one
from the 3,143,705 release calls. The implementation census must publish
current page allocations, high-water pages, directory reallocations, and
requested bytes before comparing the replacement.

The node instantiation dominates the known release cost. The exact named site
performed 3,143,705 168-byte `memcpy` calls, or 528,142,440 bytes. A comparable
recording attributed all `ChunkStorage::release_lineage` instantiations and
call sites together at 3,267,259 calls and 548,913,464 bytes. The saved report
does not split the remaining 123,554 calls and 20,771,024 bytes by generic
instantiation, so this design assigns them to no payload without a new symbol
census.

## Why `Node` is 168 bytes

`Node` is a 24-variant Rust enum. Its largest payloads are
`MathNoad<PageListId>` and `MathChoice<PageListId>`, each 160 bytes. The outer
enum discriminant and padding make every `Node` slot 168 bytes, even for a
small `Penalty` or `Char`.

`MathChoice` contains four 40-byte `PageListId` fields. `MathNoad` contains a
12-byte `NoadKind` plus three 48-byte `MathField<PageListId>` values. A math
field can be empty, a math character, a text math character, a sub-box handle,
or a sub-mlist handle.

`PageListId` contains a 32-byte `ArenaListId<PageMaterialLane>` and an 8-byte
optional nonzero semantic identity. The arena id contains `arena: u32`, head
and tail cursors, and `len: u32`. Each cursor contains an 8-byte
`{ slot: u32, generation: u32 }` key and an `offset: u32`.

Other measured payload sizes are 128 bytes for `LeaderPayload`, 120 for
`BoxNode`, 104 for `MathFraction`, 72 for `UnsetNode`, 56 for `Whatsit`, and
48 for both `MathListNode` and `AdjustNode`. These sizes explain why moving a
complete node is expensive; they do not justify a second node representation.

## Selected superblock geometry

One dense page retains the existing sixteen logical chunks. One typed
superblock contains exactly eight dense pages, or 128 logical chunks. Eight is
a fixed power-of-two mapping, increases capacity per payload allocator call by
exactly 8x, and puts the two widest common pools near 64 KiB without making
allocator or WebAssembly page size semantic. If the current high-water mark is
`P` dense pages, the proposed payload-call count is exactly `ceil(P / 8)`
rather than `P`; the realized reduction approaches 8x and is smaller for a
partially used final block.

For a configured `chunk_bytes`, the geometry is:

```text
slots_per_chunk      = max(1, chunk_bytes / max(1, size_of::<T>()))
slots_per_dense_page = 16 * slots_per_chunk
slots_per_superblock = 8 * slots_per_dense_page
page_payload_bytes   = slots_per_dense_page * size_of::<T>()
block_payload_bytes  = slots_per_superblock * size_of::<T>()
allocation_alignment = max(align_of::<SuperblockHeader>(), align_of::<T>())
payload_offset       = align_up(size_of::<SuperblockHeader>(), align_of::<T>())
allocation_bytes     = payload_offset + block_payload_bytes
```

The fixed header contains 128 `u32` initialized-prefix lengths and eight `u8`
live-chunk counts, exactly 520 bytes at four-byte alignment. `Layout::extend`
or an equivalent checked composition determines any padding before `T`; no
hand-written alignment formula is trusted by the implementation. The four
measured payloads are eight-byte aligned, giving this x86-64 geometry:

| Payload           | Page payload | Superblock payload | Header | Combined coarse allocation | Values/superblock |
| ----------------- | -----------: | -----------------: | -----: | -------------------------: | ----------------: |
| `Node`            |  8,064 bytes |       64,512 bytes |    520 |               65,032 bytes |               384 |
| `PageInverse`     |  5,888 bytes |       47,104 bytes |    520 |               47,624 bytes |               256 |
| `CheckpointDelta` |  8,192 bytes |       65,536 bytes |    520 |               66,056 bytes |             1,024 |
| `PreparedDviPage` |  4,736 bytes |       37,888 bytes |    520 |               38,408 bytes |               128 |

Thus one node superblock adds capacity for 384 values instead of the 48 added
by one current page; the corresponding capacity quanta are 256 instead of 32,
1,024 instead of 128, and 128 instead of 16 for the other three payloads.
Released chunks are still reused first.

The stable combined allocation contains prefix metadata and payload. Semantic
`ChunkMeta` rows remain in the generation directory because arena, lineage,
predecessor, summary, and dependency facts are not physical page facts. The
current measured row is 96 bytes. `used` moves exclusively into the prefix
header; no duplicate length remains. Because deleting that field may be
absorbed by layout padding, the migration requires `size_of::<ChunkMeta>() <=
96`, not an assumed saving. At that budget each dense page has at most 1,536
bytes of semantic chunk metadata; one superblock has at most 12,288 bytes.
The free stack reserves 128 `u32` entries, or 512 bytes, per published
superblock. The movable directory owner is limited by a compile-time 32-byte
budget and contains only the allocation pointer and checked layout facts.

## Growth, fragmentation, and retained RSS

Superblock growth is a transaction. It first checks all `usize` and `u32`
geometry, reserves directory, 128 chunk rows, and 128 free-stack entries, then
performs the one combined superblock allocation. Only after the header is
initialized does it publish the directory entry, chunk rows, and free keys.
Any fallible reserve or layout error leaves the pool unchanged; the unpublished
owner deallocates its uninitialized block on unwind.

The retained coarse heap operations are named explicitly:

- one combined `SuperblockHeader + [MaybeUninit<T>]` allocation for each eight
  new dense pages at a typed pool's high-water mark;
- amortized geometric reallocation of the superblock-owner directory;
- amortized geometric reallocation of semantic `ChunkMeta` rows; and
- amortized geometric reallocation of the pre-reserved free-key stack.

There is no per-node, per-chunk, or per-page allocator call. Directory growth
moves pointer-sized owners and semantic metadata but never a payload, header,
or page. Whether the global allocator obtains or returns an operating-system
mapping is outside the storage contract.

Internal fragmentation has three separately measured sources:

1. A live partial logical chunk wastes at most
   `slots_per_chunk - 1` value slots. The default node bound is two slots or
   336 bytes per partial chunk. Sealed tail slack continues to be reported by
   `unused_sealed_bytes`.
2. Eight-page growth can reserve up to seven wholly unused dense pages at the
   tail. The exact one-superblock bounds are 56,448 node bytes, 41,216
   `PageInverse` bytes, 57,344 delta bytes, and 33,152 prepared-page bytes.
3. A pool that shrinks after a peak may retain any completely free pages below
   its high-water mark. The free stack makes all of them reusable before the
   next grow; they are capacity, not live payload.

Exactly two live revision generations bound item 2 to less than two
superblock payloads across corresponding typed pools, at most one tail in each
slot. A checkpoint fork is tighter: it lends the accepted physical pool into
the candidate under the aggregate transaction, so those two semantic lineages
share one pool and one tail quantum. The bound does not cover legitimate
accepted-history payload or a workload's earlier high-water capacity. On
checkpoint rejection the lent pool returns to the accepted slot before the
candidate shell drops. An independently materialized candidate drops its own
pool on rejection. Acceptance drops the superseded prior slot after aggregate
settlement and retains the current pool under its new accepted role.
Whole-generation drop returns every still-owned superblock to the global
allocator.

On native targets, allocator retention can keep freed virtual or resident
pages in its cache, so semantic deallocation does not promise an immediate RSS
decrease. On `wasm32`, linear memory normally cannot shrink; dropped blocks
return to the Rust allocator for reuse but the host-visible memory high-water
may remain. The gate therefore records logical live bytes, reusable capacity,
requested allocation bytes, allocator calls, and process/linear-memory
high-water separately.

The design uses `std::alloc::Layout` and the target global allocator. It uses
no `mmap`, native page-size query, virtual-memory reservation, pointer-width
cast, or assumption that a 64 KiB payload causes exactly one WebAssembly
`memory.grow`. Every size and offset is checked in target `usize` and every
published page/chunk coordinate is checked against the `u32` handle domain.
Zero-sized and over-aligned test payloads have explicit gates. The unused
`ChunkStorage<()>` lane still performs no allocation because it never requests
a chunk.

## Stable pages and safe engine handles

The superblock is the combined allocation, not its small directory owner.
Once allocated, its address and layout never change. A directory `Vec` may
move its 32-byte owners, but each owner still points to the same allocation.
Dense page `p` is the immutable slot interval
`p * slots_per_dense_page..(p + 1) * slots_per_dense_page` inside that
allocation. A superblock never reallocates to grow; growth always appends a
new allocation.

Existing `RawChunkKey { slot, generation }` remains the engine coordinate. Its
checked physical mapping is:

```text
dense_page       = slot / 16
chunk_in_page    = slot % 16
superblock       = dense_page / 8
page_in_block    = dense_page % 8
slot_in_block    = ((page_in_block * 16 + chunk_in_page) * slots_per_chunk)
                   + offset
```

No raw address is stored in a handle. Directory relocation therefore cannot
invalidate a coordinate, and superblock growth cannot move a resident value.
Every lookup first validates pool owner, arena owner, chunk-slot generation,
lineage admission, live state, and offset. Slot-generation increment on final
release prevents ABA reuse. Generation exhaustion retires that physical chunk
permanently rather than wrapping and aliasing a stale key.

All returned references are ordinary borrows tied to the safe pool facade.
The low-level pool requires an offset below the private initialized prefix for
`&T` or `&mut T`; its exclusive mutable borrow proves Rust aliasing. The safe
`ChunkStorage` facade additionally validates the sole live lineage and an
exclusive unsealed tail before requesting mutation or emplacement. It exposes
no mutation operation for a shared sealed chunk. A mutable pool borrow needed
for growth, truncation, or release excludes every outstanding resident
reference, so directory movement and destruction cannot race a borrow.

## Exactly two generations and bounded lineage sharing

The runtime permits one accepted/prior revision generation and one
candidate/current revision generation. It never creates a second candidate or
a history-owned third generation. At rest the accepted aggregate exclusively
owns its typed pools. Beginning a checkpoint fork consumes the accepted
region/history authority and lends the same move-only pool into the candidate
slot under one aggregate transaction; the prior slot keeps no independently
usable pool owner. While the two semantic lineage views exist, the candidate
holds the sole physical owner and the transaction controls its disposition.
Rejection returns that owner before the candidate shell retires; acceptance
keeps it in the candidate that becomes accepted. Consequently no superblock,
page, or chunk needs `Rc`, `Arc`, a weak reference, or an independent drop
callback.

Inside a forked arena, one sealed physical chunk may have at most two lineage
entries. The accepted and current lineages name the same `RawChunkKey` and
initialized prefix. Neither may append to that shared chunk. Candidate values
go only to private current tail chunks. Dropping one lineage changes only the
two-entry `ChunkMeta::lineages`; it neither changes a prefix length nor drops a
value. Dropping the last lineage releases the chunk.

Retained checkpoint marks name sealed whole-chunk boundaries. Operation marks
may additionally name the initialized length of the one exclusive partial
tail. Rollback validates all roots and marks before mutation, restores semantic
owners, truncates the private tail, releases private whole chunks, and
reattaches the detached accepted suffix. Acceptance removes roots for the
superseded suffix, releases its last lineages, and promotes current metadata.
The page pool performs only the requested physical truncations and releases.

Page succession follows the same ownership rule. A self-contained sealed
successor suffix takes the second lineage slot without relocating values. An
interleaved prefix still uses the existing explicit structural-copy fallback.
Superblocks do not carry semantic lineage counts: the pool owner keeps the
allocation alive, and the per-chunk bounded lineage rows determine whether a
prefix is initialized.

## Released-page reuse

Final lineage release is one guarded transition:

1. Preflight validates the arena, lineage, roots, chunk generation, and complete
   prefix without mutation.
2. The semantic owner removes the last root, marks the chunk non-live, clears
   both lineage slots, and advances or permanently exhausts its generation.
3. The page header sets the chunk's initialized length to zero before running
   a destructor and decrements that page's live-chunk count.
4. A range guard drops each former prefix value exactly once. It advances
   before each destructor, so unwind never retries the panicking value and
   continues through the rest.
5. After complete destruction, or from the guard during unwind, the chunk key
   is pushed onto the already-reserved free stack if its generation remains
   reusable.

Because growth reserves one free entry for every published chunk, the final
push does not allocate. A page whose live-chunk count reaches zero is a
released dense page. Its sixteen keys are already on the free stack and are
admitted before growth; the allocation and page index remain unchanged. Reuse
checks zero prefixes, mints only the already-advanced chunk generation, and
never exposes bytes from the former incarnation.

A superblock is never individually compacted, moved, or partially deallocated.
It remains at the typed generation's high-water mark until whole-generation
drop. This makes release and reuse allocator-free and prevents a dangling page
index even when every page in the block is temporarily free.

## Isolated safe API and unsafe proof surface

`DensePrefixPage<T>` changes from an owning boxed page into a borrowed checked
view of one page inside `DensePrefixPool<T>`. Only the pool owns
`DensePrefixSuperblock<T>` and its allocation:

```rust
pub struct DensePrefixPool<T> { /* private typed superblocks */ }
pub struct DensePageKey { /* checked block and page indices */ }
pub struct DensePrefixPageRef<'pool, T> { /* shared checked view */ }
pub struct DensePrefixPageMut<'pool, T> { /* exclusive checked view */ }

impl<T> DensePrefixPool<T> {
    pub fn grow_superblock(&mut self) -> Result<(), DensePoolError>;
    pub fn page(&self, key: DensePageKey) -> Option<DensePrefixPageRef<'_, T>>;
    pub fn page_mut(
        &mut self,
        key: DensePageKey,
    ) -> Option<DensePrefixPageMut<'_, T>>;
}

impl<T> DensePrefixPageRef<'_, T> {
    pub fn capacity(&self) -> u32;
    pub fn initialized(&self, chunk: PageChunk) -> u32;
    pub fn get(&self, chunk: PageChunk, offset: u32) -> Option<&T>;
}

impl<T> DensePrefixPageMut<'_, T> {
    pub fn capacity(&self) -> u32;
    pub fn initialized(&self, chunk: PageChunk) -> u32;
    pub fn get(&self, chunk: PageChunk, offset: u32) -> Option<&T>;
    pub fn get_mut(&mut self, chunk: PageChunk, offset: u32) -> Option<&mut T>;
    pub fn emplace_with(
        &mut self,
        chunk: PageChunk,
        initialize: impl for<'slot> FnOnce(VacantEntry<'slot, T>)
            -> InitializedEntry<'slot, T>,
    ) -> &mut T;
    pub fn truncate(&mut self, chunk: PageChunk, new_len: u32);
    pub fn clear(&mut self, chunk: PageChunk);
}
```

`DensePageKey` and `PageChunk` have private constructors and checked ranges.
The page-view mutation methods are internal to the low-level crate's safe
adapter; engine crates see only the higher `ChunkStorage` facade that performs
arena, generation, and lineage validation.
`VacantEntry` exposes no pointer or `MaybeUninit`. Its only consuming operation
is `write(value: T) -> InitializedEntry<T>`. `emplace_with` increments the
prefix only after the closure returns that initialized guard, then disarms it
and returns the resident reference. Engine code remains safe and writes a
producer result directly into its final slot.

The low-level crate's reviewed unsafe operations are exactly:

1. allocate and deallocate the checked combined header/payload `Layout`;
2. initialize and access the header at the allocation's aligned base;
3. derive one slot pointer from checked block, page, chunk, and offset
   arithmetic;
4. create `&T` or `&mut T` only for a slot below the checked initialized
   prefix;
5. write one vacant `T` and commit its prefix through the type-state guard; and
6. drop initialized values in place during truncate, final release, and
   whole-superblock retirement.

The crate root denies unsafe operations inside unsafe functions unless each
operation appears in an explicit documented block. It contains no TeX,
ForkArena, region, lineage, checkpoint, generation-settlement, or output
semantics.

The proof obligations are:

- layout composition proves header and every `T` slot size and alignment;
- all multiplication, addition, pointer-offset, and coordinate conversions are
  checked before allocation or access;
- only `emplace_with` increases a prefix, by one and only after successful
  initialization;
- only `truncate`, final release, and owner drop decrease a prefix;
- all page ranges, chunk ranges, and initialized prefixes are disjoint and
  bounded;
- a mutable page borrow excludes every shared resident borrow and other
  mutation;
- no raw pointer, slot reference, vacant entry, initialized guard, or page view
  escapes its pool borrow;
- a value is dropped exactly once after its prefix no longer includes it;
- a superblock deallocates only after every nonzero prefix has been drained;
  and
- zero-sized and over-aligned `T` values preserve logical write/drop counts and
  target alignment.

Stable Rust 1.93 has no fully safe equivalent. `Vec::spare_capacity_mut`
permits a safe `MaybeUninit::write`, but committing the new length requires
unsafe `Vec::set_len`. `Vec::push_mut` and `push_within_capacity` remain
unstable on this toolchain. `Vec::push`, `extend`, and `resize_with` moved whole
nodes in the measured prototypes.

## Panic and whole-generation drop

If an initializer panics before `write`, no value exists and the prefix is
unchanged. If it panics after `write` but before returning, the
`InitializedEntry` guard drops that value and the prefix remains unchanged.
No partially initialized slot becomes visible.

`truncate` records the shorter prefix before running any destructor. A range
guard advances its cursor before each `drop_in_place`; if a destructor panics,
the guard continues dropping every later value during unwind and never retries
the panicking value. Last-lineage release uses the same guard and publishes the
free key only after the complete range is logically absent.

Whole-generation drop first removes suspended execution, builders, roots,
journals, durable regions, and page history under the aggregate ordering in
[Node-region ownership](node_region_ownership.md). One pool-wide guard walks
every superblock. Before visiting a block it snapshots all 128 lengths and sets
them to zero; its cursor advances before every destructor and every block
deallocation. If one destructor panics, the guard continues across later
chunks, pages, and superblocks during unwind. A loop of independent page or
block drops is insufficient because a panic could otherwise leak later
allocations.

As with `Vec`, a second destructor panic during an existing unwind aborts the
process. That language-wide terminal condition is not presented as recovery.
No semantic owner, initialized prefix, or free key remains published twice in
any catchable panic path.

## Quantitative acceptance model

The authenticated baseline recorded:

| API       |      Calls |         Bytes |
| --------- | ---------: | ------------: |
| `memcpy`  | 13,581,465 | 2,026,475,309 |
| `memmove` |    191,437 |    32,922,922 |
| Combined  | 13,772,902 | 2,059,398,231 |

The release target is exactly 3,143,705 calls and 528,142,440 bytes at 168
bytes. Production acceptance removes that caller and the whole 168-byte
release size family attributable to it. No comparable volume may appear under
append, clone, directory growth, allocation, `memmove`, `copy`, or another
library API.

The straight dense `Vec<Node>` prototype removed the release bin but produced
17,324,345 `memcpy` calls and 2,609,483,201 bytes plus 35,652 `memmove` calls
and 6,650,435 bytes. Combined bytes increased by 556,735,405. Full append and
clone inlining reduced the net increase to 67,235,438 bytes only by increasing
`memmove` bytes by 441,775,783. Both fail.

The benchmark-only initialized-prefix prototype used an exact 168-byte
drop-tracked value. Its one/4,096 rows retained the first and last values,
performed zero drops for a simulated first shared-lineage release, dropped
exactly 1/4,096 values on final release, reused the same prefix, and dropped one
post-write initializer panic exactly once. A three-value middle-destructor
panic dropped all three and left a zero prefix.

Across construction, reuse, release, and panic gates, the exact process-wide
public-copy probe recorded only 21 `memcpy` calls and 328 bytes plus two
zero-byte `memmove` calls. Every copy symbol resolved to standard-output
writing, and both moves resolved to the standard library's environment-map
cleanup. The storage prototype emitted no public copy or move call. That
prototype proved initialized-prefix emplacement, not the selected superblock
allocation or its retention bounds.

The implementation gate must add exact counters for current pages, proposed
superblocks, pages published, completely free pages, live prefixes, header
bytes, payload capacity, semantic metadata capacity, free-stack capacity,
allocation calls, requested bytes, and generation-retirement deallocations.
For an old high-water count of `P` pages, the selected pool must use exactly
`ceil(P / 8)` superblock payload allocations, with no per-page call hidden in
metadata construction.

## Alternatives

| Alternative                      | Payload allocations | Stable resident addresses | Safe destination commit | Main cost or reason not selected                                   |
| -------------------------------- | ------------------- | ------------------------- | ----------------------- | ------------------------------------------------------------------ |
| Per-page `Box<[MaybeUninit<T>]>` | One per 16 chunks   | Yes                       | Needs isolated unsafe   | Retains current per-page allocator frequency                       |
| Eight-page typed superblock      | One per 128 chunks  | Yes                       | Needs isolated unsafe   | Selected; bounded tail reserve and generation high-water retention |
| Ordinary `Vec<T>` per chunk/page | Geometric           | No across growth          | Yes                     | Measured whole-node copies/moves exceed baseline                   |
| Compact `Node` plus side arenas  | Layout-dependent    | Possible                  | Separate project        | Changes semantic layout and still does not itself remove moves     |

### Per-page initialized `Box`: rejected

One `Box<[MaybeUninit<T>]>` per dense page would preserve stable payloads,
exact alignment, and the same prefix proof. It would still perform a payload
allocator call for every 16 fresh chunks: every 48 nodes in the default pool.
That is the draft this revision replaces. It is neither the selected
superblock model nor an acceptable implementation shortcut.

### Eight-page typed superblock: selected for measurement

The selected substrate amortizes one combined allocation across 128 chunks,
keeps fixed page offsets, makes header and payload immovable, and performs
release and reuse without allocator traffic. Its costs are the explicit
eight-page growth quantum, one raw-layout allocation/deallocation proof, and
high-water capacity retained until the typed generation drops.

### Ordinary `Vec<T>`: rejected

`Vec` provides safe truncation and drop, which made it useful as a correctness
prototype. Stable safe Rust does not provide destination construction plus
length commit. The exact whole-run measurements show that `push` and `extend`
move the 168-byte node and erase or relocate the release saving. A page-sized
`Vec` can also relocate every resident value on growth, violating stable
coordinates unless it is itself capped and boxed, which returns to the
per-page alternative.

### Compact node plus arena-owned rare payloads: deferred

A compact tag and generation-owned side tables could reduce resident width,
but it would change variant projection, clone, semantic identity,
serialization, child dependency floors, traversal, and destruction. No
integrated variant-frequency or traversal-cost census supports that change.
A smaller node also reduces rather than eliminates moves through ordinary
`Vec` append. Compacting remains a possible independent layout project; it is
not the vacancy fix and must not create a second production node format.

Per-node boxes, duplicate node stores, caches, thresholds, third-party arenas,
and per-page reference counts remain rejected because they add allocator
traffic, pointer chasing, dual ownership, input-dependent behavior, or a wider
audit surface.

## Staged migration and gates

1. **Superblock-boundary approval.** The user approves the named
   `tex-dense-prefix` crate, its six-operation unsafe exception, the single
   combined allocation per eight pages, and the first implementation branch.
   Without this approval, no production source changes.
2. **Isolated pool.** Implement the checked layout owner, borrowed page API,
   emplacement guards, release guard, whole-block drop guard, counters, and
   compile-time geometry budgets. Engine crates do not depend on it yet.
3. **Low-level validation.** Unit and property tests cover empty/full prefixes,
   every block/page/chunk boundary, repeated truncate/reuse, checked overflow,
   zero-sized and 64-byte-aligned values, initializer panic before/after write,
   destructor panic at first/middle/last positions, continuation across later
   chunks/pages, and generation-exhausted non-reuse. Run the same matrix under
   Miri, AddressSanitizer, and leak sanitizer.
4. **Node-first integration branch.** Replace `ChunkStorage<Node>` through the
   generic safe facade and run the exact one/4,096 node
   release/reuse/two-lineage gate. This is measurement, not a retained
   Node-specific representation.
5. **Atomic generic cutover.** If node evidence passes, switch generic
   `ChunkStorage<T>` so `Node`, `PageInverse`, `CheckpointDelta`, and
   `PreparedDviPage` use one representation. Delete the `Option<T>` and
   per-page-box paths in the same change. Keep the descriptor lane
   allocation-free.
6. **Semantic lifecycle gates.** Run fork-arena, node-region, page checkpoint,
   state journal, output ledger, succession, stale-key, generation exhaustion,
   candidate accept/reject, cancellation, and panic/drop tests. Prove exact
   roots, lineage count, retained values, addresses, drop order/count,
   rollback, free-page reuse, generation increment, and stale-key rejection.
7. **Portability and workspace gates.** Run `cargo test -q --tests`, then
   `scripts/check.sh`, plus the explicit native Miri/sanitizer gates and the
   `wasm32-unknown-unknown` build/tests. Any DVI/PDF, checkpoint identity,
   effect, output, or drop-order difference rejects the implementation.
8. **One whole-run decision census.** On the pinned authenticated 50-million
   command workload, record exact `memcpy`, `memmove`, allocator calls and
   requested bytes, old pages versus new blocks, live/reusable/retained bytes,
   peak RSS or WASM linear-memory pages, and the canonical work vector. Reject
   a release-only win, allocation growth outside the documented eight-page
   quantum/vector amortization, a new whole-node owner, material directory
   copies, or semantic-vector drift.
9. **Merge approval.** Present the code review, every unsafe proof, Miri and
   sanitizer receipts, semantic gates, native/WASM results, and whole-run
   before/after table. Merging the crate and changing the normative unsafe
   prohibition require a second explicit user approval.

The principal rollback risks are double-drop or leak on unwind, publishing a
prefix before initialization, resolving an offset after truncation, aliasing a
shared sealed chunk mutably, reusing a stale generation, moving a payload while
its directory grows, and retaining excessive tail blocks on native or WASM.
The type-state emplacement guard, range and block drop guards, immutable
superblock allocation, bounded lineage metadata, checked generation reuse,
exact counters, and two-stage approval address those risks. Exact semantic and
output parity remains mandatory because physical representation is not a
license to change behavior.
