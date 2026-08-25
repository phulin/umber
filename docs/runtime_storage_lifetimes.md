# Runtime storage lifetimes

Status: normative end-state architecture contract.

Implementation boundary: the legacy runtime-value region registry, per-value
root facades, reachability search, node-list strong/weak ownership, provenance
archive ownership, and their snapshot/private-revision/profiling adapters are
deleted. The current state core has direct dense banks, exact journals,
node/page storage, execution scratch, cold detachment, and exactly two retained
revision slots.

The structural reachability prerequisite is implemented. One
`ReachabilityStore` is created with the session interning epoch and physically
owns two inline slots for the optional accepted prior and exclusive current
candidate. `RetainedStateGeneration` and `RetainedEngineGeneration` are
move-only slot leases; the `Universe`, dense state, journals, save stacks,
checkpoints, continuations, and generation-typed sidecars reside below the
same external store. Public incremental, virtual-compile, project, fixed-point,
editor, and native sessions borrow a caller-owned store explicitly; their Rust
lifetime prevents a session or generation from outliving it. A suspended
candidate can remain beside its session across host turns because both are
statically tied to that external owner rather than either borrowing the other.
One coarse `Arc<Mutex<_>>` allocation also permits a self-contained exported
FFI session; its two slots are inline, clones allocate nothing, slot creation
and reuse allocate no control storage, and no runtime value clones the owner.

The existential admission seam is narrowed to its real shape: one executor
aggregate per state slot and one suspended runtime per executor generation.
Neither seam is a vector, registry, or searchable attachment set. Both values
are stored below the external store and recovered only through the universally
generic admitted operation; formats and cross-process continuations remain
detached and handle-free.

The slot payloads still contain the existing append-only definition,
token-list, glue, and provenance arenas. The immediate next implementation
step is to migrate those body rows/chunks into store-level reachability storage
and replace copy-only durable roots with safe non-`Copy` owners that release
rows directly. That migration must not add per-value `Arc`/`Weak`, a registry,
search, tracing collection, unsafe pointers, relocation, rehome, or another
historical generation.

The current implementation's per-operation and per-scanner scope tokens,
loans, owner rows, watermarks, and handoff machinery are transitional. They
must be deleted during migration to the end state below. They are not an
architecture authority and must not constrain that migration.

This document defines the ownership and lifetime model for Umber's live TeX
runtime. It is the authority when another architecture document discusses a
different runtime storage lifetime. It does not define a wire format or a
host-resource policy.

[Expansion memory lifetimes](expansion_memory_lifetimes.md) is the focused
plain-language map from this normative end state to the current expansion,
scanner, suspension, revision, and format implementation. Its retention audit
labels current facts and migration gaps without weakening this contract.

The central rule is that the lifetime of a value follows its semantic role.
Mutable TeX state, immutable definitions, speculative execution data, page
material, source evidence, and detached output have different owners and must
not be forced into one universal store.

Umber contains no unsafe Rust. Direct indexing, compact words, lifetime
branding, and arena storage are implemented entirely through safe Rust and
private APIs.

## Lifetime hierarchy

The runtime has the following ownership hierarchy:

```text
process
  `-- caller-owned ReachabilityStore + interning epoch
        `-- engine/editor session borrow
        +-- prior accepted slot lease (read-only, optional)
        `-- current candidate slot lease (exclusive execution lease)
              +-- dense current-value banks and TeX save journal
              +-- current partition of store-owned immutable arenas
              +-- durable node, source, mode, and page storage
              `-- one reusable ExecutionScratch<G>
```

An owner may keep a coarser owner alive, but an individual stored value never
owns itself. Runtime values use compact, copyable ids or offsets. Strong
ownership currently exists only at the session store, slot lease, checkpoint,
or detached-artifact level. The external store owns both physical slot
payloads. A macro call, scanner, TeX group, input frame, or individual value
never owns an arena. There is no per-value `Arc` ownership. The next durable-
row migration adds move-only row owners under this store, not another coarse
arena owner.

The following matrix is normative:

| Value or storage                                              | Immediate owner                | Valid until                                                     | Rollback behavior                                                               | Escape path                                      |
| ------------------------------------------------------------- | ------------------------------ | --------------------------------------------------------------- | ------------------------------------------------------------------------------- | ------------------------------------------------ |
| Interned control-sequence name and token spelling             | Session interning epoch        | Session epoch retirement                                        | Never rolled back                                                               | Detached spelling or semantic atom               |
| Current meaning, parameter, register, or code value           | Dense current-value bank       | Overwritten or bank retirement                                  | Exact undo through the TeX save journal                                         | Packed value in a checkpoint or DTO              |
| Immutable macro definition and its definition token lists     | Store-owned revision partition | Current slot retirement; row-owner release after body migration | Candidate suffix truncation before publication; published rows remain immutable | Handle-free recipe at a cold boundary            |
| Macro/scanner frames, arguments, builders, or temporary words | Current generation scratch     | Operation completion, rollback, or continuation disposal        | Reset the applicable lane lengths to saved cursors                              | None; surviving output is built in final storage |
| Pending mode material and page-builder nodes                  | Current generation storage     | Mode close, rollback, setbox publication, or shipout            | Move-only nested-region truncation after promotion/detachment or root restore   | Direct construction or shipout lowering          |
| Box-register or checkpoint-surviving node                     | Store-owned revision slot      | Slot retirement                                                 | Slot ownership restores before abandoned storage drops                          | Detached output or node recipe                   |
| Source registration and compact provenance record             | Session or revision generation | Last owning generation, live input, or output recipe retirement | Cursor restoration and suffix discard                                           | Handle-free source recipe                        |
| Structural diagnostic or rendered-source presentation         | Diagnostic or artifact DTO     | DTO disposal                                                    | Not live runtime state                                                          | Already detached and handle-free                 |
| Shipped page                                                  | `tex-out` value                | Output disposal                                                 | Outside engine rollback after publication                                       | Serialized artifact bytes or output DTO          |

## Interning epoch

The token/control-sequence interning tables are append-only within one session
epoch. Control-sequence names and any other token spellings which require
interning live there. Interning is outside TeX grouping, operation rollback,
and incremental revision rollback. Once issued within an epoch, a `Symbol`
continues to name the same immutable spelling until that epoch is retired.
Allocation order is not durable identity; formats, memos, and output use
spellings or versioned semantic atoms.

Process-wide builtins may occupy one immutable process epoch. Every mutable
engine session has a distinct session epoch layered over those builtins.
Dynamic names are never inserted into a shared process-global mutable
interner. A session-local `Symbol` cannot be admitted through another session.

Append-only does not mean unbounded daemon retention. A bounded daemon:

- gives each independent job a fresh session epoch;
- retires the complete epoch when the job and all of its continuations are
  gone;
- charges interned names, bytes, and slots to an explicit session budget; and
- ends or replaces an over-budget session at a typed boundary instead of
  recycling individual symbol slots.

An interactive incremental session may retain its epoch across revisions so
unchanged symbols remain stable. History pruning cannot reclaim individual
names from that epoch. A daemon which needs a smaller footprint creates a new
session by detaching the retained checkpoint into handle-free data and
materializing it under a fresh epoch.

## Dense mutable TeX state

The eqtb-equivalent state is not arena allocated. It consists of separate
dense current-value banks for meanings, integer and dimension parameters,
token registers, glue registers, box registers, font selectors, code tables,
and the other profile-specific families. Large sparse register domains may use
paged dense banks, but access remains a bank/index operation.

Each cell stores a packed scalar or a generation-scoped id. A read indexes the
known bank directly. It does not retain an owner, upgrade a weak reference,
search a generation table, hash content, perform a binary search, or allocate.

The exact TeX save/undo journal is a separate ordered structure. A mutation
records the old packed value and the group information required by TeX before
installing the new value. Local definitions restore at group exit; global
definitions suppress the applicable restoration exactly as TeX specifies.
Operation rollback and incremental restoration reuse this journal but do not
change its TeX grouping semantics.

A TeX group is a semantic save-journal boundary, not a memory owner. Durable
and global values allocate directly in current-generation storage. A local
binding records the prior packed coordinate in the save journal and restores
that coordinate at group exit; restoration does not copy the value. The group
owns neither the prior value nor the newly selected value.

Per-group arenas are incorrect for TeX. A global assignment made inside a
group survives the group, `\aftergroup` input is deliberately delivered after
the group closes, and boxes, insertions, marks, writes, and output material can
cross or outlive the group which constructed them. Putting any of those values
in group-owned memory either leaves dangling coordinates at `unsave` or
requires escape searches and copying. Current-generation durable storage plus
the exact save journal handles every case without making group structure a
lifetime graph.

```rust
struct DenseState<G> {
    meanings: DenseBank<MeaningWord<G>>,
    integers: DenseBank<i32>,
    dimensions: DenseBank<Scaled>,
    token_registers: DenseBank<Option<TokenListId<G>>>,
    boxes: DenseBank<Option<NodeId<G>>>,
    // Other typed banks have the same direct-indexed shape.
}

struct SaveJournal<G> {
    entries: Vec<UndoEntry<G>>,
}

#[derive(Clone, Copy)]
struct JournalCursor(u32);
```

Periodic dense snapshots are a coarse, tunable latency optimization. A
checkpoint may refer to an immutable packed bank image and a journal cursor so
that restoration does not replay an arbitrarily long prefix. Snapshot
frequency is not semantic: every exact rollback point is a journal cursor.
The packed image contains current cell words, not a clone of the immutable
objects those words name.

### Fresh parameter profiles

Dense state construction is profile-neutral: its fixed parameter banks begin
with zero scalars and empty generation coordinates. Fresh engine construction
then installs exactly one default batch per selected profile layer, without
creating TeX assignment-journal history. The TeX82 layer is the required base;
e-TeX and pdfTeX are optional overlays. Primitive aliases may name the same
physical parameter cell, but the catalogue reduces them to one batch entry so
that every dense cell has one initialization owner.

The TeX82 values follow tex.web §240: all integer parameters start at zero
before `\mag=1000`, `\tolerance=10000`, `\hangafter=1`,
`\maxdeadcycles=25`, `\escapechar=92`, and `\endlinechar=13` are installed.
In particular, fresh `\newlinechar` remains zero. pdfTeX's overlay follows
pdftex.web §§672 and 1064, including its nonzero compression, version,
gamma, origin, pixel-dimension, and ignored-line sentinel values. All other
catalogued scalar cells retain their specified zero value and token/glue
parameters are empty.

A repeated fresh-profile installation is a no-op and cannot overwrite later
Plain or format-source assignments. A restored format never installs a fresh
profile batch: format decoding owns every semantic parameter value and
primitive registration only reconstructs immutable lookup metadata. The sole
post-restore parameter overlay is tex.web §241's job clock. Main control copies
the host-owned `\time`, `\day`, `\month`, and `\year` into the dense bank once
before the first input line and startup banner. Process-selected diagnostic
widths remain operational state outside the format and dense banks.

## Immutable definitions

The session `ReachabilityStore` physically owns each retained slot. Each slot
currently contains one append-only `DefinitionArena` holding complete immutable
macro definitions and the token lists which constitute their parameter and
replacement text. A retained generation is only the move-only lease that
admits its slot; it does not self-own a sibling arena. Definition token lists
use definition-arena spans and are not independently owned objects yet.

A `DefinitionId` is scoped to exactly one generation and is a dense row index.
Resolution is O(1) direct indexing. Construction is private to the arena
module, occurs only after the complete row and its token spans are initialized,
and never searches or interns definitions by content. Equal definitions
allocated twice have distinct ids. Content hashes may be computed at a cold
comparison or serialization boundary, but they are neither lookup authority
nor runtime lifetime authority.

```rust
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DefinitionId<G> {
    row: NonZeroU32,
    _brand: PhantomData<fn(&G) -> &G>,
}

pub struct DefinitionArena<G> {
    rows: Vec<DefinitionRecord<G>>,
    words: Vec<TokenWord>,
}

impl<G> DefinitionArena<G> {
    pub fn get(&self, id: DefinitionId<G>) -> DefinitionView<'_, G> {
        // Private construction makes row bounds an arena invariant.
        DefinitionView::from_record(&self.rows[id.row.get() as usize - 1], self)
    }
}
```

Here `G` denotes the fresh private brand minted when a generation is admitted;
it is not one global marker shared by all generations.

Rust enforces part of this contract. The invariant brand prevents statically
typed ids from different admitted generations from mixing. Module privacy
prevents callers from forging ids, changing row numbers, constructing views,
or indexing the backing vectors. The borrow on `DefinitionView` prevents a
resolved reference from outliving the arena borrow.

Rust branding does not keep bytes alive, decide when a slot may retire,
or prove that a serialized integer belongs to an arena. Those are storage
invariants: the external store owns every slot payload named by dense state,
journal, stacks, and checkpoints; rows never move or mutate while that slot is
live; dynamic or type-erased admission validates the slot key and bounds once;
and retirement removes the slot only after every revision, checkpoint, and
continuation root below it has dropped. A raw copied id in an unowned local is
never lifetime authority.

Other immutable values which outlive an operation, including token-register
lists and glue specifications, currently use typed append-only arenas in the
same store-owned slot. They do not share the `DefinitionId` namespace, and
definition-text spans remain physically owned by `DefinitionArena`. Moving
these bodies into shared store-level row storage is the next migration; the
logical row/chunk format need not change for that ownership cutover.

Durable token lists currently store their semantic `TokenWord` lane in fixed-
size chunks inside the store-owned revision slot. A sealed row contains only
its head coordinate and logical
length; immutable replay retains a branded chunk cursor and hops only at fixed
chunk boundaries. The destination builder owns no word buffer: it appends a
`TokenWord`, or accepts a `TracedTokenWord` and extracts its already-packed
token lane, directly into the final chunk chain. Token-list equality, content
identity, and wire encoding deliberately exclude origin. Origin coordinates
remain in the separate generation provenance arena and are detached only at
the explicit diagnostic/source boundary; a future scanner migration must not
create a parallel per-list origin owner or materialize a semantic-word vector.

## Hot resolution and suspension

At episode admission, `tex-command` receives a borrowed view of the generation
which matches the dense state it will execute. A meaning lookup returns a
`DefinitionId`; macro entry resolves that id by direct indexing. Expansion may
hold the returned `DefinitionView` only for the lexical operation which reads
the parameter program or replacement span. Admission acquires one guard for
the episode, not one lock or owner per lookup. Cloning a coarse generation
owner pins retirement but does not freeze append-only allocation into that
generation.

No borrow crosses an executor barrier. A suspended scanner, macro expansion,
or resource request stores the current-generation execution lease plus ids and
integer cursors:

```rust
struct MacroCursor<G> {
    definition: DefinitionId<G>,
    replacement_offset: u32,
    frame: ScratchIndex<G, MacroFrameTag>,
    argument_record: ScratchIndex<G, MacroArgumentRecordTag>,
}

struct SuspendedExecution<G> {
    current: CurrentGenerationLease<G>,
    resume: ResumePoint<G>,
    request: ResourceRequest,
}
```

Resume re-borrows the generation, resolves the id again by direct indexing,
and continues at the stored cursor in the same scratch lanes. The continuation
never contains a Rust reference into an arena, no runtime type is self-
referential, and suspension never admits a second current generation.

## Execution scratch and destination-directed construction

The current candidate owns exactly one reusable `ExecutionScratch<G>`. It is
generation state, not a tree of operation, scanner, macro, or group owners.
Its packed/preallocated lanes are physically separate because their nesting
and survival patterns differ:

```rust
struct ExecutionScratch<G> {
    macro_frames: Vec<PackedMacroFrame<G>>,
    macro_argument_words: Vec<TracedTokenWord>,
    macro_argument_ranges: Vec<PackedArgumentRanges>,
    scanner_frames: Vec<PackedScannerFrame<G>>,
    scanner_words: Vec<TracedTokenWord>,
    scanner_builders: Vec<PackedScannerBuilder<G>>,
    expansion_frames: Vec<PackedExpansionFrame<G>>,
    render_bytes: Vec<u8>,
}

struct ScratchCursors {
    macro_frames: u32,
    macro_argument_words: u32,
    macro_argument_ranges: u32,
    scanner_frames: u32,
    scanner_words: u32,
    scanner_builders: u32,
    expansion_frames: u32,
    render_bytes: u32,
}
```

The exact lane inventory follows measured execution patterns. A value belongs
in a distinct lane whenever nesting can make one lifetime end while an older
or newer value in another class must survive. For example, putting a parent
scanner's accumulating output and a nested macro's temporary argument words in
one vector interleaves their lifetimes. The macro cannot pop its suffix without
discarding parent output, and preserving that output requires retention,
copying, or compaction. Separate physical lanes let the macro restore its own
argument-word length while the scanner destination is untouched.

Macro and scanner nesting is ordinary push/pop over lane lengths. A macro
frame records the opening lengths of its argument-word and argument-range
lanes; a scanner frame records the opening lengths of its temporary-word and
builder lanes. Return restores those lengths in O(1). No push creates an arena,
scope capability, ownership token, loan, mailbox, watermark row, or parent
graph. Fixed synchronous state stays in ordinary Rust stack locals. State that
can suspend or become too deep for the Rust stack uses explicit packed frames
and direct indices carrying the invariant generation brand `G`.

Construction is destination-directed. Ephemeral lookahead, matching, numeric
text, delimiter prefixes, and incomplete builders use scratch. Any macro
definition, token list, glue value, node list, source record, or other value
which may survive the operation is reserved, built, and sealed directly in its
final current-generation storage class. Its durable id is published only after
validation and semantic commit. Failure truncates the unpublished destination
suffix to the operation's recorded storage cursor. There is no child-to-parent
promotion, clone, compaction, relocation, slab splicing, or copying between
live runtime stores.

Scanner APIs therefore take an explicit destination kind or typed final sink.
A scanner whose result is consumed immediately may return a packed scalar or a
scratch range. A scanner whose result survives writes it directly to the final
definition, token-list, glue, node, source, effect, or output destination. A
nested macro uses only the macro lanes and direct durable reads, so its return
cannot invalidate or strand the scanner's output.

Every top-level operation begins by recording all applicable scratch lane
lengths and unpublished destination cursors. Success requires every nested
frame opened by the operation to be popped, publishes its sealed destinations,
and restores temporary lane lengths. Rejection restores semantic roots,
truncates unpublished destination suffixes, and restores the same scratch
lengths. A resource suspension performs neither transition: it retains the
exclusive current-generation lease, the same `ExecutionScratch<G>`, and only
branded frame indices and scalar resume positions. Resume continues in those
lanes. Cancellation or candidate rejection drops the complete current
generation wholesale.

Candidate acceptance is legal only when scratch is quiescent: all frame lanes
are empty, every builder is sealed or discarded, no top-level scratch cursor
is live, and no suspension owns the candidate lease. Acceptance then drops the
prior accepted generation wholesale and changes the current generation's role
to prior. It does not move or rewrite current-generation values.

Capacity growth happens only as coarse slab or lane allocation. Once the
bounded workload is warmed, macro entry/return, scanner entry/return, argument
capture, value construction, and direct indexed reads allocate zero heap.
Stack push/pop is O(1). No value carries `Arc`, `Weak`, `Box`, `Vec`, `String`,
an owner marker, or a drop-driven lifetime action. Lifetime decisions perform
no hash, search, root enumeration, content equality, or ownership-graph walk.

Migration deletes the transitional scope/loan implementation rather than
wrapping it. Acceptance tests must prove all of the following:

- no scope `Vec`, scope watermark, mailbox, owner row, loan table, or owner
  graph remains in runtime command state;
- a bounded, deeply nested single top-level operation reuses fixed warmed lane
  capacity and returns every lane to its opening length;
- warmed macro, scanner, argument, and durable-value construction attributes
  zero allocations;
- resource suspension retains the same generation and scratch indices and
  resumes without rescanning or copying;
- operation rollback restores every lane and unpublished destination cursor;
- local and global assignments across group exit preserve exact save-journal
  semantics, including `\aftergroup`, boxes, and writes;
- INITEX and complete format construction remain bounded under the documented
  fuel and memory guards;
- revision rejection drops current wholesale, and acceptance requires scratch
  quiescence before prior is dropped; and
- compile-fail fixtures prove a `G`-branded scratch or durable index cannot
  escape its generation admission or be used with another generation.

## Node lifetimes

Nodes have three storage lifetimes:

1. Execution scratch contains unfinished shaping words, packing probes, and
   temporary transformation indexes. These values are not nodes and never
   escape scratch.
2. Current-generation mode/page storage contains material for open horizontal,
   vertical, math, insertion, alignment, and page-builder lists. Save and
   operation marks name storage cursors, so rollback restores roots and
   truncates only unpublished suffixes.
3. Current-generation durable node storage contains box-register values,
   checkpoint roots, and any list known at construction time to survive its
   originating mode/page lifetime.

All three storage classes are owned below the external store's current slot,
not by a mode, group, box, or node. Builders select the final class before
emitting a surviving node and seal it there. A TeX box copy may share an
immutable durable segment by coordinate under the same slot lease; it never
adds a per-node or per-list reference count. No live node closure is copied
between storage classes.

Shipout traverses the completed page once and emits a handle-free page plan,
artifact data, and any selected detached source recipes for `tex-out`. After
successful detachment, the page root is removed and its mode/page storage is
dropped or returned to a bounded scratch pool. Published output retains no
node id, arena owner, generation owner, or engine borrow.

Rendered-source demand is the only path that invokes
`ArtifactSourceResolver::detach_artifact_source(OriginId) ->
Option<ArtifactSourceRecipe>`; the resolver validates the live coordinate and
returns an owned recipe before publication. Successful page publication may
also invoke
`ShipoutGeometrySink::committed_shipout_geometry(ShipoutGeometry)`. That value
contains only detached dimensions and count-register values. Main control may
attach live source or line attribution only while forwarding it across the
explicit observer boundary; shipout and committed artifacts never retain that
attribution.

## Source identity and provenance

Source identity has a lifetime distinct from tokens and definitions. Immutable
input bytes and their line/offset indexes are owned by a session or revision
source registration. A live input frame stores a compact source id and cursor;
the owning generation or session keeps the registration alive. Direct source
tokens carry compact registration-relative positions. Derived tokens carry
compact generation-local provenance records which name source positions,
definition sites, invocation sites, and parent expansion records by id.

Temporary rewrites and scanners keep provenance coordinates beside their words
in the appropriate scratch lane. Provenance which will survive is reserved and
built directly in current-generation provenance storage; scratch contains its
unpublished builder state, not another owned graph. Provenance never keeps a
semantic token, definition, or node alive.

Ordinary execution retains compact ids and does not materialize structural
origin graphs, path strings, excerpts, line/column presentations, or
artifact-local recipes. Cold materialization happens only when a diagnostic is
rendered, an observer explicitly requests provenance, a checkpoint must detach
portable source state, or shipout is configured to expose rendered-source
queries. The cold consumer walks the already selected typed roots, validates
their source registrations, and produces owned strings, ranges, or handle-free
recipes. If no consumer requests that evidence, it is never materialized.

Editor-stable source identity is serialized as piece/anchor or immutable
source recipes, never as a runtime `SourceId`. Deleted or foreign pieces
produce typed resolution results. Provenance is excluded from token equality,
macro identity, TeX semantic state, and output bytes except where the output
contract explicitly includes detached source metadata.

## Marks, checkpoints, and restoration

An operation mark is a fixed-size value containing journal cursors, stack
lengths, scratch-lane lengths, durable storage cursors, mode/page cursors,
source/effect ledger cursors, and the identity of the generation it addresses.
A named checkpoint refers to either the current candidate or the prior
accepted generation; it cannot retain a third generation. Its command owner
directly retains the one aggregate copy-on-write root and exact attempt mark
selected at capture; the command timeline contributes only a monotonic
identity serial and owns no root row. The retained executor store is the sole
checkpoint container. It reuses physical slots with generation-plus-serial
validation and exact live-index backreferences, so pruning drops unretained
owners in O(live checkpoints) without scanning full capacity. A checkpoint
also contains compact marks and any optional coarse packed-bank snapshot. It
does not clone the live definition, node, provenance, input, or page object
graph.

Marks can be created only at a boundary whose live builders are sealed and
whose execution scratch is quiescent. A mark is not an owning reference to
each value it can restore. The session's prior/current generation slots and
the checkpoint's direct aggregate owners provide lifetime; checkpoint cursors
provide position. Dropping or pruning the checkpoint releases those owners
immediately. Slot reuse never revalidates an old key, relocates a surviving
checkpoint, or compacts live coordinates.

Restore is atomic and follows this order:

1. Validate the checkpoint/session identity, generation ancestry, all journal
   and stack cursors, arena positions, and external ledger cursors without
   mutation.
2. Validate that the checkpoint names the live prior or current slot and hold
   the applicable coarse generation lease through restoration.
3. Restore dense banks by loading the selected coarse packed image when one is
   present and replaying the exact journal suffix to its cursor; otherwise
   undo the live journal directly to the cursor.
4. Restore input, condition, group, mode, page, source, resource, effect, and
   output cursors, transferring canonical roots before releasing replaced
   owners.
5. Restore scratch lengths and truncate unpublished provenance, input,
   durable, and mode/page storage suffixes to their validated positions.
6. Release abandoned generation and page owners only after no restored cell,
   stack entry, or cursor can name their storage.

Any validation failure leaves the runtime unchanged. Reusing a physical arena
slot requires a new generation key before another id can name it.

## Incremental revisions and two-generation ownership

An incremental session has exactly these possible live runtime generations:
the prior accepted generation and one current candidate. There is never a
second candidate or a history-owned generation. They are admitted under
distinct invariant generative brands. Prior admission is read-only; every
revision-local allocation and mutable root belongs to current. Candidate
creation consumes an exclusive current-candidate lease, so another factory or
caller cannot issue a concurrent candidate. Candidate execution may compare
detached evidence from prior, but an accepted current root cannot contain a
prior-generation id or owner.

Rejection consumes the exclusive lease, clears the current store slot, and
leaves prior unchanged. Acceptance first requires quiescent scratch and
validates current-generation locality without mutation. It then consumes the
lease, clears the complete former prior slot, and changes current's role to
prior. No row, slab, or value moves. History retains only detached semantic
evidence, hashes, schedules, and output prefixes. It never retains a live
checkpoint or generation owner.

There is no runtime compactor, relocation map, generation graph, forwarding
pointer, slab splice, tracing collector, or content-equality merge. Routine
edits do not clone the prior runtime graph. The current generation rebuilds
only the state required by execution in its own append-only arenas; explicit
format, artifact, and detached-continuation boundaries are the only cold copy
paths. Reclamation is currently the O(1) removal of a whole store slot.
Obsolete rows inside the accepted slot therefore remain until it is replaced
or the session resets. The immediate next migration replaces that over-
retention with direct non-`Copy` root ownership and row release; it is not a
collector or registry.

## Detached boundaries

Formats, resource continuations which leave the engine session, pure memo
entries, committed output, and every value crossing a serialization, process,
or thread boundary are handle-free DTOs. They contain validated scalars,
strings, bytes, canonical content identities, source recipes, and DTO-local
indices. They contain no `Symbol`, `DefinitionId`, node id, source id, arena
offset, Rust reference, `Arc`, journal cursor, or generation key.

Detachment walks an explicit typed root set and assigns dense DTO-local indices.
Materialization validates the complete DTO, reserves destination storage,
interns names into the destination session epoch, constructs destination-local
definition and durable ids, rewrites DTO-local indices with dense relocation
vectors, and publishes only after the full object is valid. Failure drops
staging and leaves the destination unchanged. Staging is stamped for exactly
one destination; publication rejects a foreign destination before moving any
staged graph, and successful publication moves the validated graph once rather
than cloning live values.

An in-process resource-pending continuation retains the exclusive current-
generation lease, its `ExecutionScratch<G>`, and branded resume indices as
described above. Before that continuation crosses a process/thread boundary or
enters serialized session storage, it must detach to the handle-free
continuation schema. Runtime ids never become wire identity.

`EffectJournal` is an in-session revision reconciliation package, not a cold
DTO. It owns detached-value `EffectRecord` rows together with runtime-local
publication identities, semantic ordinals, and placement sidecars whose only
meaning is within the current retained revision graph. Cold artifact and
effect consumers materialize record order and detach record payloads; they do
not serialize, memoize, or emit the journal's publication sidecars.

## Crate and module responsibilities

`tex-state` owns the session interning epoch, external `ReachabilityStore`,
move-only retained slot leases, generation brands,
dense current-value banks, exact TeX save journal, definition arenas, durable
node arenas, source/provenance arenas, opaque ids, admission, marks, and atomic
restore. It exposes borrowed views and typed mutation APIs, not backing
vectors or unchecked constructors.

`tex-command` owns raw token delivery, expansion, scanners, input stacks,
macro activations, scanner builders, command-side `ExecutionScratch<G>` lane
layouts, and typed suspended command state. It borrows the exclusively
admitted current generation for hot direct indexing and stores branded indices
plus scalar cursors whenever that borrow ends.

`tex-exec` owns operation boundaries, semantic dispatch, mode/page storage
selection, node-building lifetimes, resource/effect barriers, shipout
detachment, and the ordering of aggregate restore. It cannot construct state
ids or publish partially sealed values.

`tex-incr` owns the prior/current generation state machine, detached named
checkpoint evidence, history pruning, candidate acceptance/rejection, and
convergence comparison. It never relocates runtime roots or inspects private
arena storage.

`tex-out` accepts only validated handle-free page plans, artifacts, source
recipes, and output DTOs. It owns serialization and output-specific lowering.
It never receives a live engine, runtime id, generation owner, or arena view.

The format boundary is a cold `tex-state` codec over handle-free schemas.
Format loading stages a fresh destination generation and session-local ids;
format dumping emits canonical logical content and DTO-local references.
Runtime layout, arena capacity, generation keys, and journal history are not
format ABI. PDF format state follows the same rule: live token and durable-node
coordinates detach through caller-provided recipes into an owned wire payload,
and decoding returns an unpublished destination-local `PdfState` for the
aggregate format staging transaction.

The continuation boundary is jointly typed by `tex-command` and `tex-exec`.
An in-session continuation retains the current-generation lease and its same
scratch lanes; a detached continuation contains only portable recipes and
logical cursors. Materializing a detached continuation is an atomic
destination-local rebuild through `tex-state` admission APIs.

## Required hot-path properties

After bounded capacity warmup, ordinary source delivery, meaning lookup, macro
expansion, scanning, assignment, argument capture, and value/node construction
perform zero heap allocation. An ordinary read requires:

- no `Weak` upgrade or lookup;
- no `Arc` retain or release;
- no generation/root registry search;
- no binary search;
- no content hash or content comparison; and
- no per-value heap-owner construction.

Generation validation occurs once when the exclusive current lease, a
continuation, checkpoint, or detached value is admitted. Within that admitted
borrow, ids resolve by typed direct indexing. The coarse store makes one
session-boundary `Arc<Mutex<_>>` allocation, justified by the self-contained
long-lived FFI API; its slots are inline, and candidate creation, admission,
slot reuse, and value operations allocate no reachability-control storage.

## Non-goals and forbidden designs

This architecture does not reproduce TeX's monolithic memory array, require a
JIT, make runtime allocation order durable identity, or make snapshot cadence
semantic. It does not require compact ids to be globally unique after their
generation is retired.

The following designs are forbidden:

- unsafe code anywhere in Umber, including arena access and id construction;
- stored Rust references in definitions, state cells, stacks, checkpoints,
  continuations, memos, or output;
- self-referential arena owners or continuations;
- arena allocation of eqtb-equivalent current-value banks;
- content interning, hash lookup, binary lookup, or liveness search for
  `DefinitionId` resolution;
- rollback by cloning a live object graph or scanning all live values;
- per-value `Arc`, `Weak`, reference counts, or implicit drop-driven store
  callbacks; move-only durable owners release through admitted store APIs;
- a global generation/root registry consulted by ordinary reads;
- per-macro, per-scanner, per-operation, or per-TeX-group arenas;
- scope tokens, scope owner rows, loan registries, watermarks, mailboxes, or
  ownership graphs for scratch lifetime;
- more than the prior accepted and exclusively leased current candidate
  generations;
- publishing runtime ids through formats, serialized continuations, memos,
  output DTOs, process messages, or thread messages;
- promotion, copying, relocation, compaction, or slab splicing between scratch
  and live current-generation stores;
- in-place generation or node-arena compaction; and
- provenance ownership which keeps otherwise-dead semantic values alive.
