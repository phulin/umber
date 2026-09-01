# Dense fork-arena prefix emplacement

Status: proposed low-level storage design; production unsafe code is not
approved.

Issue: `umber2-66p0.8.40.113`.

## Decision requested

Approve or reject one isolated unsafe storage crate below the safe engine
crates. The proposed crate owns typed physical pages as dense initialized
prefixes and exposes only safe emplacement, lookup, truncation, and retirement
operations. `tex-state`, `tex-exec`, and `tex-incr` remain safe Rust.

Approval authorizes a measured implementation branch, not an automatic merge.
Production adoption still requires focused lifetime gates, Miri and sanitizer
checks, the complete routine suite, and one exact whole-run `memcpy`/`memmove`
and allocation census. Rejection leaves the current `Option<T>` storage in
place; the safe `Vec<T>` and compact-node alternatives do not meet the present
no-shift criterion.

## Semantic ownership and physical storage are separate

`ForkArena<T, Lane>` is the semantic owner. It defines arena identity, stable
chunk keys, slot generations, at most two lineages, list predecessor links,
sealed boundaries, checkpoint marks, dependency floors, optional semantic
summaries, and accepted-versus-candidate settlement. None of those rules
depends on how initialized values are represented inside a physical page.

`ChunkStorage<T>` is the physical owner. It maps a validated chunk key and
offset to a resident `T`, allocates coarse pages, and returns released chunk
slots to a free list. The current implementation stores every physical slot as
`Option<T>`. That is unnecessary occupancy metadata: every live logical chunk
already has a dense initialized prefix `0..used`, and no operation creates an
interior hole.

The proposed change replaces only this physical representation. Fork-arena
coordinates, list topology, lineage rules, node-region ownership, and
checkpoint ordering do not change. A `Vec<T>` prototype likewise replaced the
slot array inside `ChunkStorage`; it did not replace `ForkArena` or its
ownership model.

## Concrete production uses

There are four production payload instantiations:

| Payload                            | Measured x86-64 size | Use                                     | Mutation shape                                                    |
| ---------------------------------- | -------------------: | --------------------------------------- | ----------------------------------------------------------------- |
| `Node<PageListId>`                 |            168 bytes | Page and durable node regions           | Append a dense suffix; truncate or retire a suffix or whole chunk |
| `PageInverse`                      |            184 bytes | Page-builder checkpoint inverse journal | Append inverse records; rewind or retire a suffix                 |
| `CheckpointDelta<GenerationBrand>` |             64 bytes | Dense state checkpoint journal          | Append first-write alternates; settle or retire a suffix          |
| `PreparedDviPage`                  |            296 bytes | Accepted/candidate output ledger        | Append per shipout; accept, reject, or retire a suffix            |

All four use the same `ChunkStorage<T>`, the same per-slot `Option<T>`, and the
same `release_lineage` assignment to `None`. None has an interior-removal API.
The empty legacy descriptor lane is `ChunkStorage<()>`; it stores no topology,
never allocates a page, and remains a zero-work instantiation until its
always-zero checkpoint coordinates are deleted.

The node instantiation dominates the known cost. The exact named release site
performed 3,143,705 168-byte `memcpy` calls, or 528,142,440 bytes. A comparable
recording attributed all `ChunkStorage::release_lineage` instantiations and
call sites together at 3,267,259 calls and 548,913,464 bytes. The saved report
does not split the remaining 123,554 calls and 20,771,024 bytes by generic
instantiation, so this design does not assign them to a payload without a new
symbol census.

## Why `Node` is 168 bytes

`Node` is a 24-variant Rust enum. Its largest payloads are
`MathNoad<PageListId>` and `MathChoice<PageListId>`, each 160 bytes. The outer
enum discriminant and padding make every `Node` slot 168 bytes, even for a
small `Penalty` or `Char`.

`MathChoice` contains four 40-byte `PageListId` fields: `display`, `text`,
`script`, and `script_script`. `MathNoad` contains a 12-byte `NoadKind` plus
three 48-byte `MathField<PageListId>` values named `nucleus`, `subscript`, and
`superscript`. A math field can be empty, a math character, a text math
character, a sub-box handle, or a sub-mlist handle.

`PageListId` contains a 32-byte `ArenaListId<PageMaterialLane>` and an 8-byte
optional nonzero semantic identity. The arena id contains `arena: u32`, head
and tail cursors, and `len: u32`. Each cursor contains an 8-byte
`{ slot: u32, generation: u32 }` key and an `offset: u32`.

Other measured payload sizes are 128 bytes for `LeaderPayload`, 120 for
`BoxNode`, 104 for `MathFraction`, 72 for `UnsetNode`, 56 for `Whatsit`, and
48 for both `MathListNode` and `AdjustNode`. These sizes explain why moving a
complete node is expensive; they do not justify a second node representation.

## Generation-owned typed superblocks

The existing physical page is the proposed typed superblock. It contains
sixteen fixed logical chunks. A generation owns a directory of these
superblocks plus its chunk metadata and free-chunk stack:

```rust
struct GenerationChunkStorage<T> {
    pages: Vec<DensePrefixPage<T>>,
    chunks: Vec<ChunkMeta>,
    free: Vec<u32>,
}
```

The directory may move its small page headers when it grows; resident `T`
values remain in the page's stable boxed allocation. No node or other payload
moves when the directory grows.

For a configured logical `chunk_bytes`, the geometry is:

```text
slots_per_chunk = max(1, chunk_bytes / max(1, size_of::<T>()))
slots_per_page  = 16 * slots_per_chunk
payload_bytes   = slots_per_page * size_of::<T>()
alignment       = align_of::<T>()
```

The default node pool uses a 512-byte logical chunk. A 168-byte node therefore
gives three nodes per logical chunk, 48 nodes per physical page, and 8,064
payload bytes per page at 8-byte alignment. The current `ChunkMeta` is 96
bytes, so its sixteen rows occupy 1,536 bytes per page. Prefix lengths move out
of those rows into one 64-byte page array; `used` is deleted from `ChunkMeta`
rather than duplicated. The exact post-layout metadata size is a compile-time
budget gate, not an assumed saving.

At the same 512-byte setting, the expected dense payload geometry is two
184-byte page inverses per logical chunk and 5,888 bytes per page; eight
64-byte checkpoint deltas and 8,192 bytes per page; and one 296-byte prepared
page and 4,736 bytes per page. The zero-sized descriptor lane allocates no
page. These are payload bytes; allocator-private headers are not observable.

Each new physical page performs exactly one boxed payload allocation. The
page directory and chunk/free metadata vectors retain their existing amortized
growth allocations. The engine performs no direct operating-system allocation
or `mmap`; whether the global allocator obtains a fresh OS mapping is outside
the storage contract. Within one page, allocating or reusing any of its sixteen
logical chunks performs no heap or OS allocation.

Released logical chunks return to the generation's free stack. Their physical
page remains at the generation's high-water mark and is reused before another
page is allocated. A page allocation is returned only when the complete typed
generation owner drops. This preserves current allocation behavior and avoids
turning lineage retirement into allocator traffic.

A larger multi-page slab is not part of this proposal. It could reduce page
allocation calls but would increase minimum retention, complicate exact page
drop, and change the allocation census independently of the vacancy problem.

## `DensePrefixPage<T>`

The low-level crate owns this representation:

```rust
pub struct DensePrefixPage<T> {
    slots: Box<[MaybeUninit<T>]>,
    initialized: [u32; 16],
    slots_per_chunk: NonZeroU32,
}
```

The invariant is exact: for chunk `c`, only
`c * slots_per_chunk .. c * slots_per_chunk + initialized[c]` contains valid
`T` values. Every later slot in that chunk is uninitialized. Each length is at
most `slots_per_chunk`. The sixteen ranges are disjoint. There is no per-value
tag, bitmap, `Option<T>`, spare value, or interior vacancy.

The safe facade is deliberately small:

```rust
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
```

`PageChunk` is a checked value in `0..16`. `VacantEntry` exposes no pointer or
`MaybeUninit`; its sole consuming method is
`write(value: T) -> InitializedEntry<T>`. The initialized entry is an unwind
guard. `emplace_with` increments the prefix only after the closure returns the
guard, then disarms it and returns the resident reference. Engine code writes
`entry.write(Node::Penalty(value))` or clones an admitted source directly at
that boundary. No complete temporary is stored in the arena facade, and no
whole-`T` vacancy representation is written.

Stable Rust 1.93 has no fully safe equivalent. `Vec::spare_capacity_mut`
permits a safe `MaybeUninit::write`, but committing the new length requires
unsafe `Vec::set_len`. Both `Vec::push_mut` and `push_within_capacity` remain
unstable on this toolchain. `Vec::push`, `extend`, and `resize_with` all moved
whole nodes in the measured prototypes.

## Smallest unsafe boundary

The recommended boundary is a new workspace crate used below `tex-state`, not
unsafe code inside an engine crate and not a general allocator framework. It
contains no TeX, list, lineage, checkpoint, or generation semantics. Its crate
root denies unsafe operations inside unsafe functions unless each operation is
inside an explicit reviewed block.

The required unsafe operations are:

1. Convert a slot below the checked initialized prefix to `&T` or `&mut T`.
2. Drop one initialized slot in place during truncation.
3. Continue dropping the remainder of a shortened range from an unwind guard.
4. Drop all initialized ranges when the page owner drops.

Allocation itself remains safe through `Box::new_uninit_slice`; no raw
allocator call, custom layout, pointer arithmetic allocation, or manual
deallocation is required. `MaybeUninit::write` is safe. The unsafe code only
asserts initialization already represented by the private prefix lengths and
performs in-place access or destruction.

The proof obligations are:

- the boxed slice has `T`'s size and alignment because `Box` created it;
- all index arithmetic is checked before borrowing the slice;
- only `emplace_with` can increase a prefix, and it does so by exactly one
  after successful initialization;
- only `truncate`, `clear`, and page drop decrease a prefix;
- a mutable page borrow excludes every shared value borrow and every other
  mutable operation;
- no raw pointer, slot reference, vacant entry, or initialized guard escapes
  its page borrow;
- a value is dropped exactly once after its prefix length no longer includes
  it; and
- zero-sized and over-aligned `T` values obey the same index and lifetime
  rules, with the descriptor lane retaining its no-allocation fast path.

Generation and stale-handle validation stay outside this crate. `ForkArena`
must validate pool owner, arena owner, chunk slot generation, lineage, and
offset before asking the page for a reference. The low-level crate cannot mint
or accept an arena coordinate, so it cannot weaken stale-key rejection.

### Panic and drop behavior

If the initializer panics before `write`, nothing was initialized and the
prefix is unchanged. If it panics after `write` but before returning, the
`InitializedEntry` guard drops the value and the prefix remains unchanged.

`truncate` records the shorter prefix before running any destructor. A range
guard advances its cursor before each `drop_in_place`; if a destructor panics,
the guard continues dropping every later value during unwind and never retries
the panicking value.

Page destruction first snapshots all sixteen lengths and sets every stored
length to zero. One page-wide guard then drops every snapshotted range. This is
important: a loop of independent chunk truncations would leak later chunks if
one destructor panicked. As with `Vec`, a second destructor panic during an
existing unwind aborts the process; no Rust container can recover from that
language-wide terminal condition.

### Validation of the unsafe crate

The crate requires ordinary unit and property tests for empty/full prefixes,
all sixteen chunks, repeated truncate/reuse, zero-sized values, 64-byte-aligned
values, checked arithmetic overflow, and randomized legal operation sequences.
Drop-tracked tests cover normal clear, page drop, initializer panic before and
after write, a destructor panic in the first/middle/last position, and exact
continuation across later chunks.

An explicit Miri gate exercises the same matrix and checks stacked-borrow and
initialization rules. AddressSanitizer and leak-sanitizer runs cover randomized
operation sequences and panic paths. The engine integration gate separately
proves stale key rejection after chunk-slot reuse because that property belongs
to `ForkArena`, not the page substrate.

## Exactly two generations and two lineages

The aggregate runtime permits an accepted/prior revision generation and one
candidate/current revision generation. It does not create a third candidate
generation. Each generation owns its typed chunk pools and their page
high-water storage.

Inside a forked arena, one sealed physical chunk may have at most two lineage
entries. A retained accepted prefix can be shared by the prior and current
lineages without copying payload. Both lineages name the same initialized
prefix, and neither may append to that sealed chunk. New candidate values go
only to private current tail chunks.

Retained checkpoint marks name sealed whole-chunk boundaries. Operation marks
may additionally name the initialized length of the one exclusive partial tail.
Rollback validates roots and marks first, restores semantic owners, truncates
only the private tail prefix, releases private whole chunks, and reattaches the
detached accepted suffix. Acceptance releases the superseded prior chunks and
promotes the current suffix. These are the existing fork-arena operations; the
page crate merely performs requested prefix destruction.

Page succession follows the same rule. A self-contained sealed successor
suffix can move or take the second lineage slot without relocating values. An
interleaved prefix still uses the existing explicit structural-copy fallback.
Dropping one of two lineages changes only `ChunkMeta`. Dropping the last lineage
clears the chunk's dense prefix, increments the chunk-slot generation, and
pushes the slot onto the free stack.

Superblocks do not carry independent lineage or reference counts. They remain
owned by their one typed generation until that generation drops. Thus lineage
retirement reuses memory, generation retirement returns it, and no page can
outlive or be shared across generation owners.

## Quantitative acceptance model

The authenticated baseline recorded:

| API       |      Calls |         Bytes |
| --------- | ---------: | ------------: |
| `memcpy`  | 13,581,465 | 2,026,475,309 |
| `memmove` |    191,437 |    32,922,922 |
| Combined  | 13,772,902 | 2,059,398,231 |

The release target is exactly 3,143,705 calls and 528,142,440 bytes at 168
bytes. Production acceptance removes that caller and the whole 168-byte
release size family attributable to it. The new combined total must fall by a
material fraction of those 528,142,440 bytes. No comparable volume may appear
under append, clone, `Vec` growth, allocation, `memmove`, `copy`, or another
library copy API.

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
cleanup. The storage prototype emitted no public copy or move call. Raw
artifacts are under the gitignored
`target/umber2-66p0.8.40.113/dense-prefix.*` namespace.

The design retains exactly the same payload allocation count and high-water
policy as current physical pages: one payload allocation per page, then zero
allocation for its sixteen chunk admissions and reuses. The full-run allocator
gate must confirm that page count, allocation calls, and requested bytes do not
increase.

Fragmentation is explicit. Each live partial logical chunk wastes at most
`slots_per_chunk - 1` value slots; for default nodes that is at most two slots
or 336 bytes. Sealing a small list can retain that tail slack, already reported
by `unused_sealed_bytes`. Completely free pages remain bounded by the
generation's observed high-water page count and disappear with the generation.
This proposal adds no per-value fragmentation or separate payload arena.

## Alternatives

### One boxed initialized page per physical page: selected

`Box<[MaybeUninit<T>]>` preserves the current single stable payload allocation
for sixteen logical chunks, exact alignment, page reuse, and coarse drop. The
private length array replaces existing `used` fields and is not a second
occupancy representation. This is the proposed substrate.

### Ordinary `Vec<T>` per chunk or page: rejected

`Vec` provides safe truncation and drop, which made it useful as a correctness
prototype. Stable safe Rust does not provide destination construction plus
length commit. The exact whole-run measurements show that `push` and `extend`
move the 168-byte node and erase or relocate the release saving.

### Compact node plus arena-owned rare payloads: deferred

The largest inline values are math payloads containing multiple 40-byte list
handles. A compact tag and handles into generation-owned side tables could
reduce resident width, but it would change clone, semantic identity,
serialization, child dependency floors, traversal, destruction, and every
variant projection. It would add a side lookup on affected traversal paths.

No trustworthy integrated variant-frequency or traversal-cost census has been
published, so this document makes no frequency or speed claim. More
importantly, a smaller node still moves through safe `Vec` append and therefore
reduces rather than eliminates shifted copy traffic. Compacting may be a later
independent layout project; it is not the vacancy fix and must not introduce a
second runtime node representation.

### Per-node `Box`, duplicate node storage, cache, or threshold: rejected

These add allocation, pointer chasing, dual ownership, or input-dependent
behavior. They violate the issue constraints and the node-region ownership
contract.

### External arena or allocator crate: rejected for this boundary

A general-purpose third-party slab, bump allocator, or arena has a much larger
unsafe and dependency surface and does not expose this exact sixteen-chunk,
two-lineage, truncatable-prefix contract. Depending on its safe facade would
move the audit outside the repository without removing the need to prove
in-place destruction and generation reuse. A repository-owned small crate is
more reviewable.

### Raw combined metadata-and-payload allocation: deferred

One custom allocation could place prefix lengths and payload in the same block
and keep page-directory headers smaller. It would require manual `Layout`
composition, allocation failure handling, pointer arithmetic, and deallocation.
The expected header-copy saving is small and unmeasured. The proposed boxed
slice keeps allocation safe; the whole-run census will reveal whether moving
the 64-byte page headers is material before expanding the unsafe surface.

## Staged implementation and approval points

1. **Unsafe-boundary approval.** The user approves a named low-level workspace
   crate, its unsafe exception, and the four-operation proof surface above.
   Without this approval, no production source changes.
2. **Isolated crate.** Implement `DensePrefixPage<T>`, unit/property tests,
   compile-time geometry budgets, Miri, and sanitizer gates. Engine crates do
   not depend on it yet. Review every unsafe block and generated assembly for
   168-byte emplacement and destruction.
3. **Node-first integration branch.** Replace `ChunkStorage<Node>` behavior in
   a branch through the generic storage boundary and run the exact 1/4,096
   node release/reuse/lineage gate. This is a measurement stage, not a retained
   Node-specific implementation.
4. **Atomic generic cutover.** Once node evidence passes, switch generic
   `ChunkStorage<T>` so `Node`, `PageInverse`, `CheckpointDelta`, and
   `PreparedDviPage` use one representation. Delete the `Option<T>` path in the
   same change; do not keep a feature flag, threshold, or second production
   implementation. Keep the descriptor lane allocation-free.
5. **Semantic gates.** Run fork-arena, node-region, page checkpoint, state
   journal, output-ledger, succession, stale-key, candidate accept/reject,
   cancellation, and panic/drop tests. The one/4,096 gate must prove exact
   retained values, shared-lineage behavior, drop order/count, reclamation,
   generation increment, and stale-key rejection.
6. **Workspace gates.** Run `cargo test -q --tests`, then `scripts/check.sh`.
   Run the explicit Miri/sanitizer storage gates and relevant optional tooling
   checks. Any DVI/PDF, checkpoint identity, effect, or drop-order difference
   rejects the implementation.
7. **One whole-run decision census.** On the pinned authenticated 50-million
   command workload, record exact `memcpy`, `memmove`, allocator calls/requested
   bytes, page counts, retained bytes, and the canonical work vector. Reject a
   release-only win, any new whole-node append/clone/allocator owner, any
   significant page-header move family, allocation growth, or semantic-vector
   drift.
8. **Merge approval.** Present the code review, unsafe proof, Miri/sanitizer
   receipts, semantic gates, and whole-run before/after table. Merging the new
   crate and its unsafe exception requires a second explicit user approval.

The principal rollback risks are double-drop or leak on unwind, publishing a
length before initialization, resolving an offset after truncation, and
changing last-lineage release order. The isolated type-state guard, page-wide
drop guard, unchanged fork metadata, and stale-key gates address those risks.
The parity risk is not considered low merely because the representation is
internal; exact DVI/PDF and checkpoint identities remain mandatory.
