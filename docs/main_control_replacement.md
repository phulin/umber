# Main-control replacement

Status: proposed architecture for Beads epic `umber2-awgc`.

Umber will replace the current scalar command object graph with one compact,
mutable, snapshot-native TeX core. The replacement preserves the existing
semantic tests, formats, incremental checkpoints, compact node builder, and
output authorities. It changes the representation and lifetime of work inside
the interpreter.

The design has three priorities, in order:

1. Ordinary TeX execution performs no heap allocation after bounded arena
   warmup. Values that must escape use chunk or region ownership rather than
   per-value `Arc`/`Weak` graphs.
2. State changes are direct and rollback is cheap. A snapshot is a fixed-size
   set of journal cursors, arena watermarks, and immutable-base identities;
   rollback is proportional to changes since the mark, never to live-state
   size.
3. The implementation is simple enough to audit. The hot interpreter, state
   storage, transactions, cold evidence, and barrier publication have separate
   modules with one owner for each invariant.

## Performance and correctness target

The primary performance authority is the pinned offline `2606.12566`
pdfLaTeX workload. Synthetic workloads may diagnose a mechanism but may not be
used as headline promotion evidence.

The authority pins every TeX-visible host input, including
`SOURCE_DATE_EPOCH=1787080434` and `LC_ALL=C.UTF-8`. A run that leaves the job
clock live is diagnostic only and cannot establish work-vector or artifact
identity. TeX82 §241 initializes `\time`, `\day`, `\month`, and `\year` from
the host before input begins, and the loaded LaTeX format expands those values
from its `everyjob` hook. The corrected machine-readable authority and the
negative control that exposed the missing clock pin are recorded in
[`umber2-awgc.5.3.8-authority.json`](writeback/umber2-awgc.5.3.8-authority.json).

The final target is:

- complete the pinned paper in at most 20 seconds and at most 150 MiB peak RSS
  under its unchanged wall, fuel, RSS, source, format, distribution, cache,
  and interaction contract;
- preserve exact semantic state, effects, artifacts, DVI, PDF, transcript,
  diagnostics, and incremental boundaries;
- perform no per-token heap allocation, reference-count operation, weak-index
  lookup, content hash, or generation validation in ordinary source delivery,
  stored macro replay, scanning, assignment, and character-node construction;
  and
- create and discard a rollback mark in constant time and constant retained
  space.

Each migration stage must improve the same pinned boundary by at least 15% in
CPU or RSS, establish a necessary structural invariant for a later measured
stage, or be reverted. Full correctness gates remain mandatory before a stage
merges.

## Why replacement is necessary

Before the ordinary-command cutover, `advance_episode` amortized one aggregate
savepoint across up to 256 scalar operations. It did not change the
representations inside the loop. One ordinary operation still:

1. constructs a command-processor borrow;
2. delivers a rich traced token and resolves owned meaning/provenance values;
3. scans into a large `ScannedStep` value;
4. ends the command borrow;
5. applies that value through `MainControl`;
6. updates journals, dependency state, diagnostics, and output sidecars; and
7. tests every possible barrier before repeating.

`CommandState::snapshot` clones the full rich command state. Token and macro
values use fine-grained strong owners and weak exact indexes. `CurrentCommand`
contains an owned origin, source metadata, resolved meaning, delivery identity,
and alignment state. A group-depth change ended the episode because the
aggregate retry root could not span the consumed group lineage.

This is safe and semantically explicit, but it prevents the compiler from
keeping the actual TeX machine in registers and cache. It also makes optional
evidence and incremental ownership shape the ordinary compile path.

The replacement is not a second batch engine. It becomes the sole canonical
engine and retains typed cold handlers for uncommon commands.

## Architecture

The engine is divided into a compact hot core and cold services:

```text
EngineSession
  |
  +-- HotCore
  |     +-- InputMachine
  |     +-- ExpansionMachine
  |     +-- DenseState
  |     +-- SaveJournal
  |     +-- ModeMachine
  |     +-- NodeListBuilder
  |     `-- EpisodeArenas
  |
  +-- ColdServices
  |     +-- Diagnostics
  |     +-- ProvenancePublisher
  |     +-- ObservationSink
  |     +-- DependencyRecorder
  |     +-- CheckpointPublisher
  |     `-- FormatAndOutputPublisher
  |
  `-- BarrierDriver
        +-- resource suspension
        +-- effects and readable output
        +-- diagnostics and observers
        +-- named incremental checkpoints
        +-- format and final output
        `-- cancellation and fuel
```

`HotCore` is the only owner of mutable TeX semantics. `ColdServices` may read a
barriered view or consume compact sidecars, but they do not mirror semantic
state. `BarrierDriver` is the only place an owned snapshot, detached DTO, or
structural provenance graph may be created.

## Compact representations

### Tokens and commands

`TokenWord` is a fixed-width packed value containing the TeX token class and
operand. Control-sequence tokens contain a dense symbol index. Source identity
and provenance are not embedded as owned Rust values.

The definitions-only substrate fixes `TokenWord` at 4 bytes: its high two bits
select character, control-sequence, parameter, or inaccessible frozen token,
and its low 30 bits preserve the exact operand. Character operands retain the
Unicode scalar and all sixteen category codes; control sequences retain the
complete permanent-symbol domain. The existing 8-byte `TracedTokenWord` is now
the exact composition of this token-only word and its 4-byte origin coordinate,
so introducing the packed word does not create a second token meaning.

`CommandWord` is a fixed-width decoded command containing an opcode, operand,
control-sequence index, and compact origin coordinate. It replaces the rich
`CurrentCommand` on the ordinary path. A diagnostic or observer can materialize
the existing public command value at a cold boundary.

The exact layout is versioned and asserted with size tests. It is runtime
state, not format ABI.

### Input frames

`InputFrame` is a POD record containing:

- a chunk or source identifier;
- current and limit offsets;
- frame kind and delivery flags;
- macro/argument or source-specific auxiliary data; and
- a compact trace coordinate.

The fixed layout is 40 bytes. A 16-byte `ChunkOwner` identity, start/current/
limit offsets, one auxiliary word, a 4-byte `SourceCoordinate`, and compact
kind/flag fields are all copy-only. TeX82's token-list frame kinds retain §307's
exact numeric `token_type` values; e-TeX `every_eof`, source, and Umber replay
kinds occupy explicitly separate values. A token-only `TokenSpan` is 24 bytes
and carries one chunk identity plus a half-open 32-bit range. These runtime
values derive no serialization and remain absent from format and continuation
DTOs. This stage defines and tests the values only; live source, input, and
macro delivery remain on their current representation until the ordered
migration children.

The input stack owns chunk references at frame or stack granularity. Delivering
a token increments an offset and performs no clone, allocation, weak upgrade,
or hash lookup. Source refill may allocate one source chunk; it does not
allocate per token.

Backup, `\noexpand`, templates, inserted recovery tokens, and macro arguments
use the same frame type. A small inline frame covers one- and two-token replay;
larger replay is a span in an episode arena.

### Macro storage and arguments

Macro definitions are immutable records in 64-record chunks:

```text
MacroRecord {
  definition: MacroDefinitionId,
  flags: MeaningFlags,
  parameter_roots: (u32, u32),
  parameter_program: PackedMacroPattern,
  text_allocation: (u32, u32),
  text_lengths: (u32, u32),
  observation_operand: i64,
  allocation_serial: u64,
}
```

`PackedMacroRecord` is fixed at 112 bytes and `PackedMacroPattern` at 52 bytes.
A physical arena segment stores 8-byte traced parameter and replacement words
plus compact token-list ids; it does not retain semantic children by itself.
The environment stores a copy-only definition coordinate, while its canonical
region-root set retains the containing segment once. Command admission borrows
that region owner once for all definitions reached through the command state.
Reading an admitted macro then borrows the copy-only pattern and spans directly,
without hashing, weak upgrade, or a per-definition owner.

Definition installation mutates only a private physical tail. The first
command owner or store fork seals that tail; a later definition in the same
logical 64-slot chunk appends to a fresh or recycled physical delta segment
instead of using `Arc::make_mut` to copy published records and token words. A
dense generation-bearing slot coordinate selects the current segment in
constant time, while admitted command state retains older segments for replay.
Reclaimed private segments and text allocations are reused after their last
owner retires. A compact coordinate-change journal restores only definitions
installed after a mark, so rollback never walks or copies the live arena.

Arguments are spans in reusable 4,096-word, 256-record command chunks.
`MacroArguments` is a fixed 16-byte `(chunk, start, len, record)` coordinate;
the record contains nine optional half-open ranges. `MacroActivation` is 48
bytes and stores only the definition coordinate, argument coordinate, and
invocation-origin coordinate. Replacement input carries an admitted macro
chunk index and replacement length; parameter input carries the argument
coordinate and range. Neither payload owns a per-value `Arc` buffer.

Invocation frames append one copy-only `OriginRecord::MacroInvocation` to the
packed provenance archive and remain `OriginId` coordinates in activations.
They bypass the weak rooted-value graph and its exact-candidate allocations;
their dedicated 256-key leases keep an ordinary fresh job in one affine key
run even when cold rooted origins are allocated between invocations. Recent
exact retry candidates occupy four inline entries and create no weak bucket.
Node, diagnostic, and continuation publication materialize a structural root
only at that cold boundary. Matcher scratch, argument words, and argument
records retain their warmed capacities. Retirement removes only compact
activation entries; the next quiescent top-level call clears and reuses the
argument chunks.

Runtime definitions append. Exact-content interning is not part of ordinary
execution. If format capture or incremental convergence benefits from content
identity, it computes or reuses a chunk fingerprint at that publication
boundary.

The `benchmarks/tex-command` `macro_argument_matching` row warms two complete
invocations, backs up the already materialized third call token, and measures
matching plus first replacement delivery. Its unobserved configuration must
report zero allocation calls and zero requested bytes; external observation is
reported separately because constructing owned observation payloads is a cold
evidence boundary.

### Dense state

Meanings and scalar registers remain packed words in dense banks. The hot core
uses direct indexed access. Sparse e-TeX overflow banks remain paged but use
compact pages and stable page coordinates.

Cells containing token, macro, glue, box, or font values store compact arena or
immutable-base coordinates. The coordinate's owning region is retained by the
state generation, group save record, or checkpoint root. A cell read does not
acquire a new owner.

### Nodes

The existing sole mutable `NodeListBuilder`, compact node rows, and immutable
`NodeListRef` publication remain. The hot core appends ordinary characters,
kerns, glue, penalties, and rules without intermediate `Node` allocation.
Complex nodes may use arena-backed side tables. Freeze occurs only at a page,
output, diagnostic, observer, format, or named-checkpoint barrier.

## Arenas and ownership

The engine uses a small number of typed segmented arenas:

- immutable accepted token and macro chunks;
- mutable episode token and argument chunks;
- compact provenance records and runs;
- uncommon scanned operands that must survive a handler call; and
- existing mutable node-builder storage.

An arena segment is the ownership unit. A segment is retained once by an
accepted generation, live input frame, group save entry, or published
checkpoint. Individual values are copyable coordinates. Segments never move
after publication.

Each coordinate contains a slot plus a generation or namespace sufficient to
reject stale and foreign use at admission and publication boundaries. The
ordinary loop does not repeat that validation for every access after it has
borrowed the admitted segment.

Arena growth is geometric and capacity is reused after rollback. Bounded-live
work must plateau both live bytes and registry metadata. All-live controls must
grow by the exact documented payload and segment overhead.

### Runtime value-region owner and transition contract

Token lists, macro definitions, glue values, and their compact provenance do
not use independent lifetime systems. They are columns of one logical runtime
value region:

```text
RuntimeValueRegion {
  key: RegionKey,
  token_words: Vec<TracedTokenWord>,
  token_lists: Vec<TokenListRow>,
  macro_records: Vec<PackedMacroRecord>,
  macro_roots: Vec<PackedMacroRoots>,
  glue_specs: Vec<GlueSpec>,
  provenance: Vec<RuntimeOriginEntry>,
  origin_lists: Vec<RuntimeOriginListRow>,
}
```

The concrete implementation may split large columns into aligned physical
chunks, but `RegionKey` remains the sole allocation and lifetime identity.
`TokenListRef`, `MacroDefinitionRef`, `GlueSpecRef`, and `OriginListRef` are
copy-only typed id facades. Each generation-tagged id selects one dense
registry coordinate containing the region key and row or span index; the
facade itself carries no payload coordinate. These values contain no `Arc`,
`Weak`, root marker, exact-index entry, or allocation-event owner. Cold format
and detached publication may compare or hash borrowed resolved values, but
those indexes never own a runtime region.

The current `hot_core::arena` namespace, slot, generation, reservation, and
tail-watermark rules are the implementation authority. The migration factors
that lifecycle into a column-owning runtime region rather than creating a
second token/macro/glue/provenance-list allocator. One mutable candidate owns the active bump
region. Filling a physical region seals it by moving its buffers into one
`Arc<SealedValueRegion>`; payload rows and backing allocations are not copied.
The next active region reuses a retired slot only after advancing its
generation.

Ownership exists only at region granularity:

| Owner                               | What it retains                                                               | When it releases                                             |
| ----------------------------------- | ----------------------------------------------------------------------------- | ------------------------------------------------------------ |
| active candidate                    | the mutable tail and any unadmitted scanner reservation                       | reservation rollback, candidate rejection, or sealing        |
| dense Env root set                  | one sealed owner for every region used by current token, macro, or glue cells | the last current cell in that root set leaves the region     |
| first-write/save journal frame      | regions containing old coordinates recorded by that frame                     | root commit retires inverses or rollback restores them       |
| input/continuation root set         | regions used by live token frames, macro activations, and argument spans      | frame pop, continuation replacement, or detached publication |
| page, output, and node root sets    | regions reached by unpublished page/output state                              | publication, rollback, or state retirement                   |
| named checkpoint or generation fork | the sealed region-owner set admitted at the barrier                           | checkpoint pruning or generation drop                        |

Each root set deduplicates by `RegionKey` and may keep a small use count for
multiple coordinates in that _same canonical owner_. This is not per-value
reference counting: a cell mutation updates one integer in its owning root set,
and dropping a save frame or continuation drops its whole region-owner set.
There is no global graph scan. Ordinary reads use a region already admitted by
the relevant canonical root set; they do not clone an owner or repeat
generation validation per value.

`HotSnapshot` remains fixed-size. In addition to the existing dense-bank,
stack, and external-journal cursors, its value-region mark records the candidate
namespace, region-allocation sequence, sealed-suffix count, active region slot
and generation, and the six column lengths. Opening a mark clones no live value
and retains no region. The first write after a mark records the old coordinate
in the inverse journal; that journal frame's root set then retains the old
region once before the Env root set releases it.

Rollback occurs in this order:

1. validate the candidate/base identities, region generations, column
   watermarks, inverse cursor, stack cursors, and external cursors without
   mutation;
2. restore dense cells and stack records backward, transferring their region
   ownership from the inverse/save root sets to the restored canonical sets;
3. drop region-root sets owned only by discarded input, continuation, page,
   output, or save-stack suffixes;
4. truncate the active column tails and release every whole region allocated
   after the mark; and
5. advance the generation of any released slot before it can be reused.

Thus rollback work is proportional to mutated cells, popped stack entries, and
discarded _regions_, never to all allocated values. A coordinate retained only
in an arbitrary Rust local does not keep a region live and is rejected after
rollback. A coordinate retained by an old group restore, continuation, or
checkpoint does keep its sealed owner explicitly and remains resolvable through
that canonical owner.

Commit has two cases. If no coordinate in the candidate suffix survives in a
canonical root set, commit truncates that suffix and keeps its warmed buffers
for reuse. Otherwise it seals each surviving region once and transfers the
owner into the affected root sets without copying rows. A group exit applies
the same rule after restoring local saves: purely local suffix regions vanish;
a region containing a global assignment is sealed and retained by the Env root
set. Resource retry never publishes its candidate regions; suspension keeps the
fixed mark, rejection discards the suffix wholesale, and success performs the
same seal-or-truncate commit.

An active region has a bounded geometric capacity. It may span several direct
operations, so ordinary commands do not create one region owner each. When it
fills, it seals; after the last current/save/input/checkpoint coordinate moves
to a newer region, its owner drops and the allocator recycles the entire slot.
This gives bounded redefinition a plateau while allowing all-live state to grow
by the exact payload and region-header formula.

The migration must land these reduced controls before replacing the live
stores:

- accept/reject proves success moves backing buffers without copying and
  rejection makes every candidate coordinate stale;
- nested groups prove an outer saved value retains its region, local suffixes
  disappear at exit, and a global assignment transfers its region to Env;
- resource retry proves a failed suffix is discarded wholesale, warmed region
  capacity is reused, and stale retry coordinates reject;
- old snapshots prove first-write journals retain old regions until rollback
  or root commit and that snapshot capture remains fixed-size/allocation-free;
- an all-live control proves exact logical row, byte, region, and registry
  growth; and
- 10,000 bounded redefinitions, group exits, and retry failures prove stable
  payload capacity, registry capacity, and region-owner counts with zero
  per-value `Arc`, `Weak`, upgrade, index, hash, or graph-scan work.

The transitional reachable-value pools, per-value packed liveness markers, and
per-value allocation-event journals are deleted when this cutover lands. They
are not retained as a compatibility path.

### Region arena ownership contract

The implemented generic substrate in `tex-state::hot_core::arena` makes a
chunk the allocation and ownership unit and a frozen region a half-open span
within one chunk. A typed coordinate contains a 64-bit namespace, 32-bit chunk
slot, 32-bit nonzero generation, and 32-bit offset. It is a non-owning runtime
capability and is never a format or detached-checkpoint identifier. Coordinates
and spans add no `Arc`, `Weak`, hash, or owner field per stored value.

`ChunkOwner` names the namespace, slot, and generation retained by an accepted
arena layer or mutable candidate; despite the ownership-oriented name it is a
copy-only identity, not a strong owner. `TokenSpan` and `InputFrame` carry this
identity by value. Arena admission validates it once, after which frame
traversal increments only a 32-bit cursor. Sibling-candidate and retired-slot
controls prove foreign and stale spans reject, and a 10,000-cycle warmed
construction/traversal control proves retained payload and registry capacity do
not grow.

An accepted arena is an immutable chain of sealed chunk layers held by one
`Arc` per accepted layer. Creating sibling candidates clones only that layer
owner. Each candidate owns a fresh namespace and a mutable append-only overlay;
therefore it resolves inherited accepted coordinates but rejects another
candidate's overlay coordinates as foreign. Acceptance moves whole chunk
buffers into a new sealed layer without moving their payload allocations.

Reservation establishes an upper bound before append, so appends, region
freeze, and direct resolution cannot grow a payload buffer. Admission validates
the namespace, slot, generation, and span bounds once and returns a borrowed
slice for repeated indexing. A rollback mark records the candidate namespace,
live-chunk count, tail slot and generation, and tail length. Truncation clears
whole suffix chunks for later slot reuse. A partially truncated tail is sealed:
its discarded offsets are never appended into again under the same generation.
Reusing a wholly retired slot mints a fresh generation. These rules prevent
both stale-coordinate revival and cross-candidate reinterpretation.

Canonical command input now consumes this frame layout through a narrow
`tex-state::packed_input` seam. A live source or token level has one 40-byte
frame; its copy-only owner identity is the sole live level identity, and its
32-bit current offset is the sole token-level cursor. Source levels retain the
existing exact 64-bit byte/scalar/line cursor as their cold physical sidecar;
the frame's current offset is only a delivered-token count and is normalized
when unchanged future state converges across comment edits. Token-level
factories construct chunk-owned packed traced words directly for stored replay,
backup/noexpand, alignment templates, inserted recovery, and every-hook
payloads. Production `TokenPayload` has only packed, macro-replacement, and
argument-range representations; no parallel rich token owner survives level
construction or admission. Backup keeps physical source coordinates in a
sparse cold sidecar, so ordinary word delivery does not construct diagnostic
ranges. Resource suspension detaches handle-free words and coordinates, and
resume admits fresh input, macro, argument, and invocation chunks at the exact
portable cursors. Runtime chunk identities never enter the continuation or
schema-12 format DTOs. Live macro meaning and diagnostic reads admit one
copy-only definition coordinate through the aggregate runtime-value registry
and borrow its traced row; only cold detached or stale APIs repeat identity
validation. Transaction marks remain for `umber2-awgc.4.2`.

The packed cutover itself has an assertion-bearing warmed gate for ordinary
source delivery, packed backup/replay, stored replay, and macro matching,
argument replay, and expansion. All four rows require exactly zero allocation,
requested bytes, `Arc` and weak retains, weak upgrades, weak-index work, exact
comparisons, and content hashes. The structural gate passes. The immutable
12,000,000-fuel arXiv prefix retains exact fuel and frame-step coordinates;
under the versioned direct-prefix contract in
[`umber2-awgc.12`](writeback/umber2-awgc.12.md), its four secondary counters
carry attributed deltas for aggregate replay that no longer occurs after the
accepted transaction cutover. The final audit, current guarded receipt, and
exact vectors are published in
[`umber2-awgc.3.4`](writeback/umber2-awgc.3.4.md). Promotion requires that
evidence together with the clean exhaustive semantic tracer; neither channel
substitutes for the other.

Arena accounting uses two deliberately separate measures. _Logical values_
and _logical value bytes_ count only live initialized elements and
`len * size_of::<T>()`. _Retained payload_ counts the capacities of accepted,
live-overlay, and reusable-overlay value buffers. _Retained registry bytes_
counts accepted slot-table entries plus the capacities of the candidate slot
table and live-order vector. It excludes allocator bookkeeping, the outer arena
value, and the `Arc` header. Thus all-live controls can state exact logical
growth, while bounded-live controls must show unchanged payload and registry
capacities. Once a suitable reusable chunk and metadata capacity have warmed,
the arena itself performs no heap growth during reserve, append, freeze, or
truncate cycles.

### Compact stack, dense-bank, and journal contract

The storage-only substrate in `tex-state::hot_core::{stack,state,journal}`
provides the mutable companions to the region arena without moving any TeX
command semantics. `PodStack<T>` accepts only copyable entries. Its first eight
entries are inline, its mark is a 32-bit length, and truncation retains any
warmed spill capacity. Input, save, condition, group, and mode owners will use
separate typed stack instances; the substrate does not combine those semantic
families or expose them through a detached DTO.

`DenseBank<T>` is a fixed-length direct-indexed store. Its first 32 cells are
inline and larger banks use one retained contiguous spill allocation. A typed
runtime coordinate contains a 64-bit bank namespace, a nonzero 32-bit bank
generation, and a 32-bit dense index. Reads validate foreign namespaces, stale
generations, and bounds. Resetting a quiescent bank generation retains its
allocation, clears write epochs, and makes every prior coordinate stale. The
coordinate is a non-owning runtime capability: it derives no serialization
and is not a format or detached-checkpoint identifier.

`FirstWriteJournal<Target>` owns only inline-small copyable inverse records and
nested mark frames. The target continues to own values and one 32-bit write
epoch per cell. The first write to a cell under the active mark records its old
packed value and prior epoch; later writes under that mark coalesce. A nested
rollback restores inverse records backward and truncates the suffix. A nested
commit retains its inverses for the parent and transfers their write epochs to
the parent, while a root commit retains values but retires inverse history.
Marks are strictly LIFO and target-generation checked. After spill warmup,
bounded mark/write/rollback cycles reuse both inverse and mark capacity.

Accounting keeps logical payload separate from retained storage. Stack and
journal reports expose live entries plus retained spill capacity; dense-bank
logical bytes count packed values, while retained heap bytes include each
cell's write epoch. None of these per-entry representations contains `Arc`,
`Weak`, a hash, or a serialized handle. Checkpoint sealing and command-state
adoption remain later migration work.

## Snapshot and rollback model

### Fixed-size mark

A `HotSnapshot` contains only:

- input, parameter, condition, group, save, and mode stack lengths;
- mutation-journal cursor;
- token, argument, provenance, and node arena watermarks;
- page/PDF/effect/output journal cursors;
- source and resource transaction cursors; and
- immutable-base and engine-owner identities.

Taking or discarding the mark is O(1) in live-state size and performs no heap
allocation after mark-stack warmup.

### Mutation journal

Every mutable state write records the cell coordinate and previous packed
value on first write after the active rollback mark. Repeated writes coalesce
within the mark. Group semantics use the same underlying save records but have
their own TeX group boundary and restoration behavior.

Rollback walks journal records backward, restores packed values, truncates
stacks and arena suffixes, and restores external-ledger cursors. Its cost is
O(changes since the mark). Commit drops the mark and retires unreachable
journal history when no group or checkpoint owns it.

### Narrow resource transactions

Ordinary commands do not run beneath an aggregate retry snapshot. A command
that may suspend must either:

1. determine and acquire its resource before semantic mutation; or
2. open a narrow `HotSnapshot` around the mutation sequence that depends on
   the resource.

This lets episodes cross group entry and exit. A group change is semantic work,
not an external commit barrier.

The executor-side migration contract is
`tex_exec::transaction_protocol`. Its exhaustive classifier assigns every
canonical `Meaning` and `UnexpandablePrimitive` a fixed mutation, resource,
effect, output, and recovery capability set before execution. Preflight is a
read-only classification step with three typed results:

- an ordinary command carries its capability record directly and has no
  transaction value;
- a resource command names the retry projection needed while its operands are
  scanned and its immutable resource is acquired; and
- an operation that can fail after mutation names a narrow transaction before
  it runs.

The transaction vocabulary mirrors the fixed fields of `HotSnapshot` without
exposing snapshot internals across crates. State owners select their marks;
callers cannot independently assemble the two sets. Input, parameter,
condition, group, save, and mode owners select their respective stack marks;
dense state selects the first-write journal; source, resource, page, PDF,
effect, and output owners select their external journal cursors; and the input,
parameter, mode/page/PDF, and provenance owners select the token, argument,
node, and provenance arena watermarks they can change. Admission rejects any
missing, extra, or foreign owner/mark projection. The protocol is copy-only and
allocates nothing; command-family migration onto those marks belongs to
`umber2-awgc.4.2` and `umber2-awgc.4.3`.

The ordinary/group cutover is now active in `MainControl`. Raw delivery first
settles a command and applies the exhaustive capability record before operand
scanning. Successful ordinary assignments, material commands, and group entry
or exit mutate canonical state directly: they create neither `StepSnapshot`
nor `CommandStateSnapshot`. Their admission advances the compact environment
write epoch and commit advances the node-operation watermark; TeX's save stack remains the sole
owner of local/global group restoration. Group depth is no longer an episode
stop, so one bounded episode may enter, mutate inside, and leave nested groups.

The resource/effect/PDF/checkpoint cutover is also active. Expandable delivery
settles in the same command-processor borrow as raw preflight, then operand
scanning produces one typed prepared operation. Missing fonts, input streams,
PDF images, and `\input` files retain that completed request across host
acquisition; `\immediate` additionally retains the already-consumed nested PDF
command. Observed retry moves its unpublished evidence buffer and opaque
delivery-order cursor with the typed continuation, so it neither clones the
observer root nor changes raw/expanded provenance. Retry therefore neither
rewinds input nor rescans operands. Nested expanded token collectors retain
their accumulator and special-splice route, while `\expandafter` and `\csname`
retain their consumed operands and partial name respectively. Expandable
`\number` and `\romannumeral` scans likewise retain their sign and provenance
before the first expanded number token and their accumulated value, radix,
vacuous flag, and overflow state while probing for the next digit. TeX82
§442's alphabetic constant also keeps its completed character code while its
following expanded optional-space probe is suspended. A resumed conversion therefore continues
at the exact expanded-token boundary instead of restarting after an
already-consumed sign, digit, or character constant. Semantic apply
begins only after resource resolution and uses direct owner journals;
output-capable box closing, ErrorStop recovery, observed and tracked commands,
and private revisions use the same path. A private revision opens only a
fixed-size allocation-suffix mark, never an aggregate state root.

Active-alignment delivery and the explicit `diagnostic_expand_step` host API
now use the direct operation path too. Alignment retry retains only the typed
delivery entry point while `CommandState` owns the exact blocked expansion and
input continuation. Diagnostic assignment retry retains either its settled
command plus delivery cursor or its fully scanned operation. Both paths use
the private-revision allocation suffix and the existing mode journal where
semantic assignment apply can fail; neither constructs `StepSnapshot`,
`CommandStateSnapshot`, or `LocalRetrySnapshot`. The compatibility aggregate
executor and its forced negative controls have been deleted. The final
profiling, exhaustive command-stream, and pinned-prefix consolidation passed
under `umber2-awgc.4.4`; the journaled transaction cutover is promoted.

### Durable checkpoints

A named incremental checkpoint is created only when scanners and transient
builders are quiescent. Mutable arena suffixes are sealed into immutable
segments; the checkpoint retains segment roots plus compact stack and journal
coordinates. Format and continuation DTOs remain handle-free.

## Persistent interpreter

The ordinary interpreter borrows `HotCore` once and retains its input cursor,
meaning banks, stack cursors, and arena writers across commands. Its loop is:

```text
while budget remains {
  token = input.next_raw()
  command = expansion.next_command(token)
  dispatch(command, hot_core)
  if a real barrier is pending {
    break
  }
}
```

The first ownership cutover is active: `MainControl` owns one
`PersistentInterpreter` for the complete engine session. `CommandProcessor`
is now only a borrow-scoped facade over that owner and the matching `Universe`
command context. Processor facades may end at semantic-apply or host barriers,
but they do not reconstruct, clone, or replace command state. Assertion-bearing
lifecycle accounting rejects overlapping facades and proves that group and
resource transitions retire every borrow before semantic mutation, rollback,
or host fulfillment. Later dispatch fusion can lengthen those borrows without
introducing another executor or state owner.

Expansion and scanners are methods over the same input and state views.
Common primitives scan operands and apply their mutation directly. They do not
construct a universal `ColdOperation` value.

Rare or structurally large primitives may call a typed cold handler. The
handler receives scalar values or arena spans and mutates the same `HotCore`.
It is not a second semantic executor. A cold handler that may publish an
effect returns a typed barrier request.

Fuel remains charged at the same canonical logical points. An attached
observer selects an observed interpreter instantiation with an event sink. The
ordinary production instantiation has a zero-sized sink, allowing evidence
branches and builders to compile away.

## Cold evidence

### Provenance

Ordinary tokens carry compact source coordinates or provenance-run indexes.
Macro expansion appends one compact frame record and refers to it by index.
Direct source runs require no structural allocation.

At a diagnostic, rendered-source, observer, checkpoint, or output boundary,
`ProvenancePublisher` traverses the compact records and materializes the
existing structural roots required by that consumer. Publication is budgeted,
atomic, and cached per sealed segment. With no consumer, it performs no work.

### Observations and dependencies

The unobserved interpreter does not allocate observation names, mutation keys,
records, or dependency projections. The observed interpreter writes compact
typed records to an append-only sidecar and renders them after the command's
semantic mutation commits.

Dependency recording is selected once at episode admission. Direct state
getters are used when no tracked region is active; there is no atomic check on
every getter in that instantiation.

### Diagnostics

Input frames always retain enough compact coordinates to reconstruct TeX's
error context. Strings and structural input descriptions are created only when
an error or tracing parameter demands them. Ordinary execution does not create
empty diagnostic vectors or format trace values.

## Module boundaries

The replacement should converge on modules of roughly these responsibilities:

- `hot_core/mod.rs`: aggregate semantic owner and admitted borrowed views;
- `hot_core/layout.rs`: packed words, coordinates, and size/layout assertions;
- `hot_core/arena.rs`: typed segmented arenas and sealing;
- `hot_core/input.rs`: source and token frames plus raw delivery;
- `hot_core/expand.rs`: macro expansion and condition delivery;
- `hot_core/scan/`: scalar and structured scanners over the persistent input;
- `hot_core/dispatch.rs`: opcode dispatch and common direct handlers;
- `hot_core/state.rs`: dense banks and compact value coordinates;
- `hot_core/journal.rs`: group save records, transaction marks, and rollback;
- `hot_core/mode.rs`: mode stack and the sole node builder;
- `cold/diagnostics.rs`: context and trace rendering;
- `cold/provenance.rs`: structural provenance publication;
- `cold/observation.rs`: canonical observation publication;
- `cold/checkpoint.rs`: named snapshot sealing and DTO construction; and
- `barrier.rs`: typed resource, effect, output, checkpoint, and cancellation
  exits.

`MainControl` becomes a small lifecycle facade over the persistent interpreter
and barrier driver. Scanner and executor modules remain testable separately,
but their production path is not separated by a heap-owning universal DTO.

## Migration plan

The work is tracked by dependency-ordered Beads children:

| Issue           | Deliverable                                                       |
| --------------- | ----------------------------------------------------------------- |
| `umber2-awgc.1` | Pinned arXiv baseline and structural allocation/episode census    |
| `umber2-awgc.2` | Typed arenas and fixed-size snapshot marks                        |
| `umber2-awgc.3` | Packed token, input-frame, macro, and argument representation     |
| `umber2-awgc.4` | Journaled narrow transactions that span group changes             |
| `umber2-awgc.5` | Persistent fused expansion, scanning, and dispatch                |
| `umber2-awgc.6` | Cold provenance, observation, dependency, and diagnostic sidecars |
| `umber2-awgc.7` | Full cutover, legacy deletion, and final pinned validation        |

Migration is bottom-up. A new representation becomes canonical storage before
the old representation is deleted. Temporary adapters may exist only at a
barrier and must have counters proving they are absent from the pinned hot
path. No stage may introduce a runtime engine selector.

During migration, differential tests compare the new core with the current
canonical engine at bounded quiescent boundaries. Promotion requires exact
state, effects, artifacts, DVI/PDF, diagnostics, fuel, and rollback outcomes.
Once a family is promoted, its old storage and dispatch path are deleted in the
same or immediately following commit.

## Measurement gates

The child-1 receipt fixes the comparison authority. Every later stage records:

- full-process and engine-only wall and CPU time;
- peak and milestone RSS;
- command fuel and logical work vectors;
- heap calls and requested bytes by subsystem;
- arena committed, live, and high-water bytes;
- delivered tokens, macro calls, arguments, scanner entries, and state writes;
- episode length distribution and stop reasons;
- snapshot count, rollback count, changed cells, and rollback work;
- `Arc` clone/drop, weak lookup, content hash, and provenance publication
  counts; and
- exact semantic and output identities.

Every fixed-prefix row must also record the pinned job-clock and locale
environment above. Matching source, format, distribution, cache, command-line
arguments, and fuel is insufficient when the TeX-visible clock differs.

The current-core structural fields come from the profiling-only
`HOT_CORE_CENSUS` report documented in
[`profiling.md`](profiling.md#main-control-hot-core-structural-census). Its
owner, stop-reason, command-family, and phase vocabularies are fixed-width and
exhaustive, and its episode histogram preserves every length from zero through
the canonical 256-operation bound. The production feature resolution contains
none of its counters, scopes, allocator wrapper, fields, or calls.

The immutable integrated comparison authority is
[`umber2-awgc.1.3`](writeback/umber2-awgc.1.3.md), with the complete schema-1
census in
[`umber2-awgc.1.3-census.json`](writeback/umber2-awgc.1.3-census.json). It
fixes the exact 12,000,000-fuel structural/profile boundary, the separate
100,000,000-fuel production wall/RSS authority, the disjoint zero-loss CPU
attribution, and the owner-specific promotion budget for every later child.
The two boundaries must be reported together but never treated as equal work.

The fixed-prefix promotion contract is versioned at transaction cutovers.
Before direct-prefix commit, all six command-work counters were comparable
only while both rows replayed the same aggregate prefixes. After
`umber2-awgc.4.2`, successful ordinary commands commit before a later resource
transaction. A resource miss therefore no longer rolls back or re-executes
that prefix. The post-cutover contract preserves exactly:

- the 12,000,000 fuel boundary and 11,999,815 raw token-frame position;
- all exhaustive semantic, diagnostic, state, effect, artifact, DVI, and PDF
  identities; and
- the historical six-counter vector as evidence, never as work to recreate.

Expanded deliveries, meaning lookups, scanner-status tokens, and deferred-write
expansions are replay-sensitive classifications at that fixed raw-work
boundary. A promotion reports each delta against the historical vector and
attributes every increase as well as every decrease to the changed endpoint
mix. A focused resource-retry control must prove that the new transaction adds
no work unrelated to the retained prefix. Restoring aggregate replay or
synthetically charging eliminated work is forbidden. The decision and first
exact receipt are [`umber2-awgc.12`](writeback/umber2-awgc.12.md).

The same optimized profile, CPU affinity policy, source, format, distribution,
offline cache, environment, and guards are used for before/after measurements.
Synthetic benchmarks are retained for focused asymptotic controls only.

## Non-goals

- No JIT is required for this replacement.
- No unsafe code is required merely to obtain compact layouts or direct
  indexing.
- The format ABI does not serialize runtime coordinates, arena capacities, or
  journals.
- Snapshot equality is not weakened.
- Diagnostics, provenance, incremental execution, and observers are not
  deleted; they are moved to explicit cold publication paths.
- The replacement does not preserve compatibility wrappers after their final
  consumer migrates.

## Completion criteria

The epic closes only when:

- every supported TeX82, e-TeX, and pdfTeX profile uses the same `HotCore`;
- the rich scalar input, `CurrentCommand`, universal `ScannedStep`, blanket
  retry snapshot, and per-value weak-ownership hot paths are absent;
- group changes do not force episode publication;
- bounded-live arenas and journal metadata plateau while all-live controls grow
  exactly;
- source/loaded formats, TRIP/e-TRIP, exhaustive command tracing, incremental
  convergence, the full routine suite, and `scripts/check.sh` pass; and
- the pinned paper meets the 20-second and 150-MiB gates under unchanged
  guards.
