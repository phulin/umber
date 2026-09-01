# Dense fork-arena superblocks

Status: approved design for an isolated measured implementation; production
adoption still requires the gates and second approval below.

Parent issue: `umber2-66p0.8.40.113`. Architecture decision:
`umber2-66p0.8.40.113.2`.

## Decision

The fork arena will use dense, typed, exactly 64 KiB superblocks. A logical
chunk or sealing boundary is only a saved cursor inside that dense storage. It
is not an allocation, a physical subdivision, or an independently owned
object.

The design has two layers:

1. One repository-owned low-level crate, provisionally named
   `tex-dense-prefix`, owns `Superblock<T>`. It is the only runtime unsafe
   exception. It knows only raw allocation, checked initialized-prefix access,
   construction, truncation and drop, and deallocation.
2. Safe semantic wrappers own block tables, cursors, forks, TeX groups, page
   attempts, checkpoint journals, node regions, generations, and speculative
   output. The low-level crate has no type or operation for any of those
   concepts.

Approval authorizes a fresh implementation agent to build and measure the
isolated substrate and safe wrappers. It does not authorize a mechanical
generic cutover or production merge. In particular, fork-tail duplication is
available only to payloads which satisfy the non-owning copy contract below.
Owned payloads keep their existing move or nonforking lifetime semantics.

This revision supersedes the earlier physical-page proposals. There are no
physical tiny chunks, per-record vacancy tags, or per-chunk initialized
lengths in the selected design.

## Problem and measured constraint

The current `ChunkStorage<T>` stores every slot as `Option<T>`, although every
live allocation is already a dense prefix. On final node-lineage release, each
resident node is dropped and a preconstructed `None` representation is then
written into its vacant slot. The authenticated 50-million-command recording
attributed 3,143,705 `memcpy` calls of 168 bytes, or 528,142,440 bytes, to that
node release path.

Replacing `Option<Node>` with ordinary `Vec<Node>` removed the release row but
moved complete nodes during append. The straight prototype increased combined
public-copy volume by 556,735,405 bytes. Aggressive inlining reduced the net
increase only by shifting 441,775,783 bytes into `memmove`. The accepted design
therefore needs all of these properties together:

- dense values with no `Option<T>` vacancy representation;
- direct indexing without descriptor or chunk traversal;
- bounded rollback by truncating dense prefixes;
- no value copy when a candidate is accepted; and
- exact measurement of construction, fork-tail copy, table metadata copy, and
  release rather than moving cost between library entry points.

## The low-level substrate

`Superblock<T>` owns one allocation whose requested size is exactly 65,536
bytes. Its payload capacity is the compile-time monomorphized constant:

```text
SUPERBLOCK_BYTES = 65_536
ITEMS_PER_BLOCK   = SUPERBLOCK_BYTES / size_of::<T>()
PAYLOAD_BYTES     = ITEMS_PER_BLOCK * size_of::<T>()
TAIL_SLACK_BYTES  = SUPERBLOCK_BYTES - PAYLOAD_BYTES
```

The allocation begins at `align_of::<T>()`; the unused remainder is tail slack.
The owner stores the initialized length outside the payload allocation. No
header consumes payload bytes, and no slot contains `Option<T>` or another
vacancy tag.

The first implementation supports non-zero-sized `T` with
`size_of::<T>() <= 65_536`. It rejects a zero-sized or too-large layout before
allocation. All size, alignment, capacity, byte-offset, and pointer arithmetic
is checked in target `usize`; published lengths and block ids are additionally
checked in their explicit integer domains. A later zero-sized specialization
is possible but is not part of this performance fix. The existing empty
descriptor lane requests no storage and therefore never instantiates a
`Superblock<()>`.

Conceptually the safe surface is:

```rust
pub struct Superblock<T> { /* private allocation and initialized length */ }
pub struct VacantSlot<'a, T> { /* private final slot */ }
pub struct InitializedSlot<'a, T> { /* private committed value */ }

impl<T> Superblock<T> {
    pub fn try_new() -> Result<Self, LayoutError>;
    pub const fn capacity() -> usize;
    pub fn len(&self) -> usize;
    pub fn get(&self, offset: usize) -> Option<&T>;
    pub fn get_mut(&mut self, offset: usize) -> Option<&mut T>;
    pub fn push_with(
        &mut self,
        build: impl for<'slot> FnOnce(VacantSlot<'slot, T>)
            -> InitializedSlot<'slot, T>,
    ) -> Result<&mut T, CapacityError>;
    pub fn truncate(&mut self, new_len: usize);
}
```

Exact names may change. The important boundary is that safe callers never
receive a raw pointer, `MaybeUninit<T>`, unchecked length mutation, or
deallocation capability.

The crate's unsafe work is limited to:

1. allocate and deallocate one checked 65,536-byte `Layout` for `T`;
2. derive a slot pointer after checked index and byte-offset arithmetic;
3. create a reference only below the initialized prefix;
4. write exactly one vacant slot and publish the longer prefix only after the
   value is initialized;
5. shorten the prefix before dropping a truncated suffix; and
6. drain the initialized prefix before deallocation.

The substrate does not mint `BlockId`, interpret a cursor, share a block,
create a fork, advance a generation, settle output, or know a TeX lifetime.
Those are safe-layer responsibilities.

## Dense indexing and block tables

A safe dense arena owns a flat `Vec<BlockId>` and a logical length. `BlockId`
is a checked numeric handle into a typed block owner table. It includes enough
incarnation information to reject a stale reused entry. Superblock allocation
never moves; growth appends a new owner and one id to the table.

For a logical index, resolution is:

```text
block_index = logical_index / ITEMS_PER_BLOCK
offset      = logical_index % ITEMS_PER_BLOCK
block_id    = block_table[block_index]
value       = block_owner(block_id)[offset]
```

This is one quotient/remainder operation and one direct block-table lookup,
then a checked initialized-prefix access. `ITEMS_PER_BLOCK` is a constant for
each monomorphized `T`, so the compiler may replace hardware division with its
usual constant-divisor sequence. The contract is O(1) direct indexing; it has
no descriptor traversal, chunk walk, owner-range search, or per-record prefix
bookkeeping.

The table deliberately remains an ordinary flat safe vector. A candidate may
copy a prefix of small `BlockId` rows so that its read path remains the same
single lookup. Candidate acceptance moves that table owner; it does not rebuild
it. The implementation must count table entries and bytes copied separately
from payload copies. A recursive persistent vector, per-block `Rc`, forwarding
table, and tree lookup are out of scope unless a measured metadata census first
shows that the flat table matters.

## Cursors, sealing, and truncation

A cursor is the logical boundary:

```text
ArenaCursor {
    complete_blocks,
    tail_len,
}
```

The exact representation may be one checked logical length, because
`ITEMS_PER_BLOCK` recovers both fields. It must still validate that the named
block exists and that `tail_len` is within its initialized prefix.

Sealing a list, operation, TeX group, or checkpoint saves a cursor. It changes
no payload and allocates no tiny chunk. Multiple logical chunks may end in one
superblock, and their boundaries need no rows in the low-level allocation.
Semantic wrappers may retain their own necessary list links, summaries, roots,
and identity metadata; none of that changes physical indexing.

Truncation to a saved cursor performs exactly two physical actions:

1. remove and drop every block-table entry after the cursor's tail block; and
2. truncate the remaining tail block to `tail_len`.

If the cursor is on a block boundary, the old tail is removed whole. The safe
wrapper validates all semantic roots before it mutates the table. Removed
superblocks are either returned to an explicitly owned reuse pool or
deallocated according to that wrapper's lifetime policy. The substrate merely
truncates and drops the values it is told to remove.

Normal checkpoint capture and every ordinary TeX group boundary only save a
cursor. They copy no values and create no block owner.

## Generation fork and bounded tail copy

A revision fork is the one operation that may need a physical payload copy.
Given a checkpoint inside one superblock:

```text
accepted: [complete shared blocks] [checkpoint tail | detached prior suffix]
candidate:[complete shared blocks] [copied checkpoint tail | private suffix]
```

Complete blocks strictly before the checkpoint are shared read-only by the two
semantic views. The candidate gets a private tail by copying only the
initialized checkpoint prefix of the one tail block. The copy may happen when
the fork is created or lazily on the candidate's first write. A checkpoint on
an exact block boundary needs no tail copy.

The fork transaction is the sole physical owner while both views exist. It
contains the accepted table, the candidate table or shared table prefix plus
private ids, the detached prior suffix, and the candidate-private blocks. No
individual block needs `Rc`, `Arc`, or a callback into a semantic owner.

Rejection drops the candidate's private tail and suffix, then restores the
accepted table and detached suffix. Acceptance drops the superseded prior tail
and suffix, then moves the candidate table and private block ownership into the
accepted role. Acceptance performs no value copy. The fork's total payload
copy is bounded by the initialized prefix of one 64 KiB superblock for each
forked eligible typed arena, independent of the accepted prefix length and
candidate suffix length.

The safe fork wrapper must prove:

- complete shared blocks are immutable until settlement;
- the candidate never appends into the accepted checkpoint-tail block;
- each private block has exactly one owner;
- every table entry resolves to the expected typed block incarnation;
- rejection restores the exact accepted cursor, roots, and table order;
- acceptance makes superseded private/prior ids stale where reuse is allowed;
  and
- a second candidate or third live revision generation cannot be created.

## Payload eligibility and semantic wrappers

The superblock substrate is generic; forking is not automatically generic.
Bytewise tail duplication is sound only for values with no independent
ownership or destructor obligation. The default eligibility bound is `T:
Copy`. If a repository type cannot implement `Copy` for incidental reasons, a
sealed, explicitly reviewed equivalent may certify the same facts: bitwise
duplication creates two independently usable values, requires no owner-count
update, and neither copy has `Drop`. That contract must not be a convenient
alias for `Clone`.

A type containing `Vec`, `Box`, reference-counted ownership, a unique handle,
or another destructor-bearing value is not eligible for raw fork-tail copy.
It uses a nonforking wrapper, explicit semantic cloning where TeX requires a
copy, or move-only ownership settlement. The implementation must audit the
actual production `Node` representation before enabling the fork-copy path;
the current `Node` enum contains owned payloads and must not be bit-copied as
if it were non-owning.

The safe wrappers remain distinct:

| Wrapper                            | Boundary behavior                                                                                   | Fork-tail copy                                                          |
| ---------------------------------- | --------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- |
| Generation and node-region storage | Shares complete immutable blocks; private candidate suffix; preserves region and stale-handle rules | Only after the concrete payload passes the non-owning eligibility audit |
| Group-local storage                | Saves a cursor, restores meanings, truncates the local suffix, then drops the region                | Never                                                                   |
| Page-attempt scratch               | Rewinds for retry or moves a completed owner into the next semantic stage                           | Never                                                                   |
| Checkpoint/save journal            | Saves cursors and detaches or truncates suffixes under journal ordering                             | Never merely to capture a checkpoint                                    |
| Speculative output                 | Keeps candidate pages/effects private; moves them on acceptance and drops them on rejection         | Never                                                                   |

This separation prevents `PageInverse`, `CheckpointDelta`,
`PreparedDviPage`, or another owned record from acquiring fork semantics merely
because it uses the same physical allocator. It also preserves the move versus
copy rules in [Node-region ownership](node_region_ownership.md) and the exact
lifetime owners in [Expansion memory lifetimes](expansion_memory_lifetimes.md).

### Current payload eligibility audit

The first isolated implementation audited every current production
`ForkArena<T, _>` payload on the supported 64-bit host layout. The sizes come
from compiler layout output for the concrete monomorphizations; `needs_drop`
and owned fields come from the current definitions. `Clone` is deliberately
not evidence of fork-copy eligibility.

| Current payload                    | Size / alignment | `needs_drop` | Ownership evidence                                                         | Actual lifetime requirement                                    | Classification                                                        |
| ---------------------------------- | ---------------: | :----------: | -------------------------------------------------------------------------- | -------------------------------------------------------------- | --------------------------------------------------------------------- |
| `Node<PageListId>`                 |          168 / 8 |     yes      | `Vec` fields in ligatures and other owned node variants                    | Node-region generation fork and TeX semantic move/copy rules   | Not eligible; representation or explicit semantic-copy decision first |
| `PageInverse`                      |          184 / 8 |     yes      | `Vec<PageInsertion>`, optional owned insertion state, and owned mark state | Reversible page journal; cursor capture and suffix retirement  | Nonforking/truncating journal storage                                 |
| `CheckpointDelta<GenerationBrand>` |           64 / 8 |      no      | Scalar cell coordinate plus generation coordinates; not currently `Copy`   | Ordered alternate journal; no copy merely to save a checkpoint | Nonforking/truncating journal storage                                 |
| `PreparedDviPage`                  |          296 / 8 |     yes      | Owned `DviPagePlan`, boxed effect slice, and publication records           | Candidate-private output moved on commit or dropped on reject  | Move-only speculative-output storage                                  |

The current physical allocator gives these payloads 512-byte logical chunks
and groups sixteen chunks in an allocation page. That is baseline evidence,
not a geometry to preserve: the dense design replaces it with direct flat
64 KiB blocks and cursor-only logical boundaries. The only authenticated
payload-copy row available before this handoff remains the node-release row:
3,143,705 calls of 168 bytes, or 528,142,440 bytes. No per-type public-copy row
was published for the other three payloads, so their baseline is _unattributed_,
not zero.

This audit authorizes no production fork-copy payload. The isolated generation
proof therefore uses a synthetic `Copy` record only. In particular, the
current `Node` must not enter that path until a later representation or
node-specific semantic-copy proposal is measured and approved.

## `push_with` and placement limits

`VacantSlot` makes the initialization transaction safe:

- it names exactly the next slot and exposes no pointer;
- `insert(value)` initializes that slot once and returns an
  `InitializedSlot` guard;
- the block prefix grows only after the builder returns the initialized guard;
- a panic before insertion leaves the prefix unchanged; and
- a panic after insertion but before commit drops the unpublished value once.

This API can be generic and monomorphized, so LLVM may construct a producer's
return value directly in the final slot. Stable Rust does not, however,
language-guarantee placement construction of an arbitrary `T`. The terminal
safe operation is still conceptually `slot.insert(value)`, and an optimizer is
allowed to materialize and move that value.

The implementation therefore adds an exact `memcpy`/`memmove` construction
gate before claiming success. If the safe generic builder still moves complete
nodes, the next decision is a narrow node-specific construction API with its
own reviewed proof, or a representation change that makes node construction
cheap. The generic substrate must not expose `unsafe fn construct_at`, make
every producer unsafe, or claim a no-copy language guarantee which Rust does
not provide.

## Drop, unwind, and aliasing invariants

The initialized prefix is the only physical liveness fact. At every observable
point, slots `0..len` contain valid `T` values and slots `len..capacity` are
uninitialized.

Truncation records the shorter prefix before calling any destructor. A range
guard advances before each `drop_in_place`; if one destructor panics, the guard
continues draining the rest during unwind and never retries the panicking
value. Whole-block drop uses the same rule and deallocates only after its prefix
is logically empty. As with `Vec`, a second destructor panic during an existing
unwind aborts the process.

Borrowing a resident value is tied to the arena or block-table borrow. A
mutable borrow excludes lookup, table growth, fork settlement, truncation, and
drop. Shared complete blocks expose only shared references while a fork is
live. No raw pointer, vacant guard, or resident reference can escape the
corresponding safe borrow.

The complete invariant set is:

1. each published superblock is exactly 65,536 allocated bytes and never
   reallocates;
2. `ITEMS_PER_BLOCK` is nonzero and all published coordinates fit their
   integer domains;
3. each superblock has one dense initialized prefix and no interior hole;
4. only successful construction increases the prefix by one;
5. truncation shortens the prefix before any removed value is dropped;
6. every initialized value is dropped exactly once;
7. every `BlockId` resolves to its expected type and live incarnation;
8. a fork shares only immutable complete blocks and owns every private block
   exactly once;
9. only an eligible non-owning payload may use bytewise fork-tail copy; and
10. semantic roots, journals, output, and continuations retire before the
    physical blocks they can name.

## Native, WebAssembly, and 32-bit targets

The 64 KiB size is an arena allocation quantum, not an operating-system or
WebAssembly ownership primitive. The implementation uses the Rust global
allocator and `Layout`; it uses no `mmap`, native page-size query, linear-memory
address assumption, or promise that one block maps to one WebAssembly
`memory.grow`.

On native targets, freeing a block does not promise that the allocator returns
resident memory to the operating system. On `wasm32`, linear memory normally
does not shrink. Tests and measurements therefore distinguish logical live
bytes, reusable block capacity, requested allocation bytes, allocator calls,
peak RSS, and WebAssembly linear-memory high-water.

All layout calculations run in target `usize`, including on 32-bit targets.
Conversion to `u32` block ids or logical coordinates is fallible and occurs
before publication. Tests cover the last valid block/index, overflow before
allocation, alignment, a one-item block, and a type which does not divide
65,536 evenly. No pointer is serialized, hashed into semantic identity, sent
between threads, or stored in a format or continuation.

## Measurement and acceptance

The historical baseline is:

| API       |      Calls |         Bytes |
| --------- | ---------: | ------------: |
| `memcpy`  | 13,581,465 | 2,026,475,309 |
| `memmove` |    191,437 |    32,922,922 |
| Combined  | 13,772,902 | 2,059,398,231 |

The first production target remains removal of the exact 3,143,705 by
168-byte release row, or 528,142,440 bytes, without moving comparable volume
to append, fork, acceptance, table growth, allocator internals, `memmove`, or a
different copy API.

Counters and focused gates report at least:

- superblocks allocated, reused, truncated, dropped, and deallocated;
- values constructed, fork-tail copied, truncated, and dropped;
- forked arenas and exact copied tail values/bytes;
- flat block-table entries and bytes copied;
- logical live bytes, payload capacity, and per-block tail slack;
- allocator calls and requested bytes;
- accepted/candidate/private/shared block counts; and
- native RSS or WebAssembly linear-memory high-water separately.

The direct-index benchmark checks first, last, and random values across many
block boundaries and records zero descriptor or predecessor visits. The
one-versus-4,096 checkpoint gate proves checkpoint capture performs no payload
copy or allocation. Fork gates cover empty, exact-boundary, one-item-tail, and
full-tail checkpoints and prove the copied payload never exceeds one block.
Acceptance must report zero value copy; rejection must restore the exact prior
state and drop every candidate-private value once.

## Implementation phases

1. **Eligibility and baseline audit.** Inventory every proposed `T`, its
   `size_of`, alignment, `Drop` and owned fields, actual fork requirement, and
   current allocation/copy rows. Classify it as eligible fork-copy,
   nonforking/truncating, or move-only. Do not grant eligibility from `Clone`.
2. **Isolated substrate.** Add the low-level crate with the exact 64 KiB owner,
   checked layout, initialized-prefix API, `VacantSlot`, guarded truncate/drop,
   and counters. No engine crate depends on it yet.
3. **Safe dense arena.** Add the flat checked `BlockId` table, cursor,
   quotient/remainder indexing, growth, truncation, and reuse policy in safe
   Rust. Prove no descriptor traversal and no per-record vacancy metadata.
4. **Semantic wrappers.** Implement the distinct group, scratch, journal,
   speculative-output, and generation-fork policies. First prove fork-tail
   sharing with synthetic `Copy` payloads. Do not route owned production
   payloads through byte duplication.
5. **Measured node integration.** Integrate only the node path authorized by
   the eligibility audit. Preserve node-region roots, exact edit rollback,
   page succession, stale-coordinate rejection, TeX move/copy semantics, and
   the at-most-two-generation rule. Run the exact construction and tail-copy
   census before expanding to another payload.
6. **Lifecycle and portability validation.** Run focused arena, region,
   checkpoint, journal, output, panic/drop, cancellation, and generation tests;
   Miri; AddressSanitizer and leak sanitizer; `wasm32-unknown-unknown` build and
   tests; `cargo test -q --tests`; and `scripts/check.sh`.
7. **One authenticated decision census.** Run the pinned 50-million-command
   allocation plus `memcpy`/`memmove` census. Publish construction, fork-tail,
   acceptance, rejection, table-copy, allocator, retained-capacity, RSS, and
   semantic-vector results against the baseline.
8. **Second approval.** Present the implementation, unsafe proof, gates, and
   census before changing the repository-wide unsafe prohibition or merging a
   production cutover.

The first implementation handoff ends after phases 1 through 4 and focused
proofs. It must not silently proceed to an owned `Node` bit-copy, generic
production migration, authenticated 50-million-command run, or production
merge without reporting the eligibility result and obtaining the authority
named by the implementation Bead.

## Rollback plan

The work lands in separable commits: isolated crate, safe dense arena,
semantic wrappers, then each production integration. Until the final cutover,
the existing `Option<T>` storage remains the production fallback. A failed
construction-copy gate, owned-payload audit, Miri/sanitizer result,
native/WASM result, semantic gate, or whole-run census removes the integration
commit rather than adding a second representation, compatibility branch,
threshold, or hidden copy path.

Rollback restores the previous safe storage owner and leaves the isolated
crate unused or removes it entirely. No format, output, checkpoint, or runtime
handle ABI may depend on the new physical block id before production approval,
so rollback requires no persisted-data migration.
