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
One coarse `Rc<RefCell<_>>` allocation also permits a self-contained exported
same-thread FFI session; its two slots are inline, clones allocate nothing,
slot creation and reuse allocate no control storage, and no runtime value
clones the owner.

The existential admission seam is narrowed to its real shape: one executor
aggregate per state slot and one suspended runtime per executor generation.
Neither seam is a vector, registry, or searchable attachment set. Both values
are stored below the external store and recovered only through the universally
generic admitted operation; formats and cross-process continuations remain
detached and handle-free.

Macro definitions and stored token lists now leave their private publishers as
generation-branded, non-`Copy` handles around non-atomic shared ownership.
Every eqtb cell, save-journal word, input or expansion frame, checkpoint,
continuation, PDF record, and owning view which can use or restore the value is
an exact semantic owner. The last such drop releases the payload without a
registry, liveness search, tracing pass, compaction, relocation, rehome, or
another historical generation. Glue and provenance retain their cheaper
direct-index representation; scratch remains arena-backed and unshared.

TeX main-memory usage is maintained by one generation-local scalar aggregate.
Publishing a definition or stored token list charges its canonical words once;
the last existing semantic owner's ordinary `Rc` drop releases that charge.
Node arenas charge each logical row at publication and release it on suffix,
region, completed-page, or whole-generation retirement. The aggregate retains
both TeX82 and e-TeX node-size projections, so profile selection and ordinary
usage reads are constant-time and allocation-free. It stores no value identity,
root, liveness bit, or reference count and cannot resolve or retain a payload.

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

An owner may keep a coarser generation lease alive, but macro definitions and
stored token lists additionally follow their exact semantic carriers. A macro
definition's private generation-branded handle is one thin non-atomic `Rc`
owner whose allocation header stores its serial, parameter split, parsed
parameter program, and final-drop accounting capability immediately before
the immutable token tail. Stored token lists retain their private
`Rc<[TokenWord]>`. Both handles are deliberately non-`Copy`; cloning records a
true alias and moving transfers an existing owner. They never own an arena. Other compact runtime values remain
copyable ids or inline scalars where that is cheaper. No per-value `Arc`,
`Weak`, owner registry, or ordinary-read liveness lookup exists.

The following matrix is normative:

| Value or storage                                              | Immediate owner                                                                     | Valid until                                                     | Rollback behavior                                                             | Escape path                                      |
| ------------------------------------------------------------- | ----------------------------------------------------------------------------------- | --------------------------------------------------------------- | ----------------------------------------------------------------------------- | ------------------------------------------------ |
| Interned control-sequence name and token spelling             | Session interning epoch                                                             | Session epoch retirement                                        | Never rolled back                                                             | Detached spelling or semantic atom               |
| Current meaning, parameter, register, or code value           | Dense current-value bank                                                            | Overwritten or bank retirement                                  | TeX group saves, checkpoint deltas, or operation-local undo restore in place  | Packed value in a checkpoint or DTO              |
| Immutable macro definition and its definition token lists     | Every live semantic carrier through one branded non-atomic shared handle            | Last exact carrier drop                                         | Rollback moves saved owners back and drops rejected/replaced owners           | Handle-free recipe at a cold boundary            |
| Stored token list                                             | Every live eqtb, journal, input/expansion, checkpoint, PDF, or continuation carrier | Last exact carrier drop                                         | Moves transfer; true aliases explicitly clone; truncation/pruning drops       | Handle-free recipe at a cold boundary            |
| Macro/scanner frames, arguments, builders, or temporary words | Current generation scratch                                                          | Operation completion, rollback, or continuation disposal        | Reset the applicable lane lengths to saved cursors                            | None; surviving output is built in final storage |
| Prepared cold operation or operation-local failure            | One caller-owned direct `OperationFrame`                                            | Application, rollback, typed suspension disposal, or reuse      | Completion consumes occupied fields; suspension moves the exact frame intact  | Typed in-process attempt continuation only       |
| Pending mode material and page-builder nodes                  | Current generation storage                                                          | Mode close, rollback, setbox publication, or shipout            | Move-only nested-region truncation after promotion/detachment or root restore | Direct construction or shipout lowering          |
| Box-register or checkpoint-surviving node                     | Store-owned revision slot                                                           | Slot retirement                                                 | Slot ownership restores before abandoned storage drops                        | Detached output or node recipe                   |
| Source registration and compact provenance record             | Session or revision generation                                                      | Last owning generation, live input, or output recipe retirement | Cursor restoration and suffix discard                                         | Handle-free source recipe                        |
| Structural diagnostic or rendered-source presentation         | Diagnostic or artifact DTO                                                          | DTO disposal                                                    | Not live runtime state                                                        | Already detached and handle-free                 |
| Shipped page                                                  | `tex-out` value                                                                     | Output disposal                                                 | Outside engine rollback after publication                                     | Serialized artifact bytes or output DTO          |

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

The lookup index remains the authority for spelling equality. One
allocation-free, exact 64-bit recent key accelerates successful names of at
most seven UTF-8 bytes: it packs the complete bytes, length, and namespace, so
a hit performs neither hashing nor `memcmp`. The cache stores only the stable
slot already issued by the append-only epoch and is cleared at whole-epoch
retirement. It is not a second identity table, a revision owner, or durable
state.

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

Rollback storage is separate from the dense banks and split by semantic
lifetime. A local definition appends its old packed value to the active TeX
group's contiguous save segment before installing the new value. Group exit
walks that segment backward and retires it whole; warmed segment buffers may
be reused by later groups. Global definitions suppress the applicable
restoration exactly as TeX specifies.

A named checkpoint seals an interval of first-before deltas. An epoch stamp
beside the journal, not an overlay in front of eqtb reads, ensures that the
first write to a cell in an interval appends exactly one delta. The checkpoint
also stores a stable group-segment id and entry offset. A checkpoint captured
inside a group pins that group and its ancestors until restoration or
generation retirement; the ordinary level-zero policy pins no group segment.
Operation-local rollback has its own reusable ordered lane. Nested operations
store suffix positions in that lane; committing the outer operation clears it,
while rejection walks only the rejected suffix backward. Operation marks do
not start checkpoint intervals.

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
    active_groups: Vec<GroupSegment<G>>,
    checkpoint_pool: ChunkPool<CheckpointDelta<G>>,
    checkpoint_lane: ForkArena<CheckpointDelta<G>, DenseJournalLane>,
    checkpoint_epochs: HashMap<StateCell, u64>,
    operation_undo: Vec<UndoEntry<G>>,
}

struct JournalCursor {
    group_segment: u64,
    group_entry: u32,
    checkpoint_mark: CheckpointMark<DenseJournalLane>,
}
```

Dense state remains directly mutated and directly read. There is no state
overlay, threshold densification, compaction pass, forwarding coordinate,
per-entry owner, or checkpoint bank clone. Restoration walks retained deltas
backward and swaps their packed alternate words into the dense banks. Edit
start detaches the accepted chunk suffix, candidate writes append privately,
rejection swaps the candidate backward and accepted suffix forward, and
acceptance prunes the detached suffix. Whole group
segments are moved between active, checkpoint-retained, operation-pending, and
reusable-buffer owners without scanning, copying, relocating, or repacking
their live entries.

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

`DefinitionArena` is now a publisher, not the lifetime owner of published
macro bodies. A `DefinitionId<G>` privately contains one thin non-atomic owner
of a single header-plus-token-tail allocation and the invariant generation
brand. The header contains the parameter/replacement split, parsed parameter
program, monotonic cold-format serial, and exact final-drop accounting
capability. Construction occurs only after the complete immutable value is
ready. Equal definitions published twice remain distinct allocations and
distinct identities.

```rust
pub struct DefinitionId<G> {
    allocation: ThinRc<DefinitionHeader, TokenWord>,
    _brand: PhantomData<fn(&G) -> &G>,
}

pub struct DefinitionArena<G> {
    next_serial: u32,
}

impl<G> DefinitionArena<G> {
    pub fn get(&self, id: DefinitionId<G>) -> DefinitionView<G> {
        DefinitionView { id }
    }
}
```

Here `G` denotes the fresh private brand minted when a generation is admitted;
it is not one global marker shared by all generations.

Rust enforces part of this contract. The invariant brand prevents statically
typed ids from different admitted generations from mixing. Module privacy
prevents callers from forging ids, accessing the `Rc`, constructing views, or
changing serials. The owning view and iterator may cross an arena borrow
because they carry the same exact owner; dropping them decrements only the
non-atomic count and never walks or calls back into a store. The final owner
also subtracts its precomputed canonical word charge from the generation's
scalar memory total.

`TokenListArena` follows the same ownership rule. Its fixed-size chunks and
builder slots are reusable publication scratch. Sealing performs the final
durable allocation, moves the words into a private `Rc<[TokenWord]>`, and
immediately recycles the builder chain. The arena does not retain the published
payload. A `TokenListView` or cursor owns a cloned or moved handle, so active
input and expansion replay keep exactly the source they can still read.
Sequential delivery dereferences and advances that already-owned cursor in
place. It does not clone the token-list handle merely to read or validate one
word; diagnostic lookahead borrows the same cursor without advancing it.

Command input generalizes that rule across every stored source. Level creation
adapts replay, macro replacement/argument, attempt, and durable owners into one
typed `PackedTokenSpanHandle`; per-token delivery uses
`PackedTokenSources::token_at(handle, scalar_offset)` and advances only the
packed input frame. The storage boundary retains the unavoidable safe-Rust
owner-domain choice. No source-specific delivery object, second advancing
cursor, handle clone, relocation, or payload copy occurs per word.

Reads, moves, restoration, warmed reuse, and explicit alias clones allocate no
heap memory. An `Rc` count change is not construction of a new heap owner.
Scratch token lists and macro arguments do not use shared ownership: their
existing arena slots and scalar marks remain the sole scratch lifetime
authority. Glue remains inline/direct-index because shared heap ownership would
cost more than the value.

The allocation event records a definition's canonical word cost and generation
accounting capability once in the same shared header as its immutable metadata;
the header's one final destruction subtracts the cost. A token-list handle
continues to carry that information beside its shared slice and tests the real
payload's last-owner state. No table, scan, hash, tracing pass, or second
reference count participates.

The serial field is not resolution or lifetime authority. It exists only to
preserve deterministic coordinates while detaching a format. Cold capture
walks the format's semantic cells, node recipes, and PDF records, writes live
payloads at their serial positions, and leaves an empty compatibility row for
a dead serial. Such a hole owns no runtime payload and materialization does not
publish a definition for a dead row. No live handle is relocated or rehomed.

## Hot resolution and suspension

At episode admission, `tex-command` receives a borrowed view of the generation
which matches the dense state it will execute. A meaning lookup explicitly
clones the definition's non-atomic owner; macro entry moves that owner into its
active frame or owning view. Parameter and replacement reads dereference the
shared immutable slice directly. Admission acquires one guard for the episode,
not one lock or heap allocation per lookup.

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

Resume moves or clones the already-owned definition/token-list handle and
continues at the stored scalar cursor in the same scratch lanes. The
continuation never contains a Rust reference into an arena, no runtime type is
self-referential, and suspension never admits a second current generation.

## Execution scratch and destination-directed construction

The current candidate owns exactly one reusable `ExecutionScratch<G>`. It is
generation state, not a tree of operation, scanner, macro, or group owners.
Its packed/preallocated lanes are physically separate because their nesting
and survival patterns differ:

```rust
struct ExecutionScratch<G> {
    macro_frames: Vec<PackedMacroFrame<G>>,
    macro_argument_segments: Vec<MacroWordSegment>,
    free_macro_segment: u32,
    scanner_frames: Vec<PackedScannerFrame<G>>,
    scanner_words: Vec<TracedTokenWord>,
    scanner_builders: Vec<PackedScannerBuilder<G>>,
    expansion_frames: Vec<PackedExpansionFrame<G>>,
    render_bytes: Vec<u8>,
}

struct ScratchCursors {
    macro_frames: u32,
    macro_argument_segments: u32,
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

Macro and scanner nesting is ordinary push/pop over lane lengths. Match
admission initializes the next macro frame in place. That frame records its
segment-chain endpoints and up to nine direct word ranges; one direct
current-argument slot owns collection facts and its cursor until completion.
The pending frame appends directly to its chain in the stable segment arena.
An older active frame may retire beneath it and return its disjoint chain
immediately; commit changes only the pending frame's role, while discard or
later frame retirement returns that frame's chain to the intrusive free head.
A scanner frame records the opening lengths of its
temporary-word and builder lanes. No push creates an arena,
scope capability, ownership token, loan, mailbox, watermark row, or parent
graph. Fixed synchronous state stays in ordinary Rust stack locals. State that
can suspend or become too deep for the Rust stack uses explicit packed frames
and direct indices carrying the invariant generation brand `G`.

The executor's cold preparation boundary is one such fixed synchronous owner,
not another scratch lane. One caller-loop `OperationFrame` holds the prepared,
applied, or diagnostic payload while preparation returns only a compact status.
Application consumes its fields directly and completion reuses the empty slot.
A resource suspension moves that same frame into the singular typed attempt;
there is no per-operation box and no append lane retaining completed commands.

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

Nodes have four storage roles:

1. Execution scratch contains unfinished shaping words, packing probes, and
   temporary transformation indexes. These values are not nodes and never
   escape scratch.
2. One caller-owned stable coarse `ChunkPool<Node>` is the physical authority
   for current-generation active-material and page-material lanes. Each typed
   `ForkArena` owns only lane coordinates, lifecycle metadata, and the sole
   direct-range/nonrecursive-range-sequence list topology. Active
   paragraph, math, alignment, and box regions promote only as sealed whole
   batches into page ownership. Operation marks may restore a partial tail;
   retained checkpoints can name only sealed whole-chunk boundaries. A
   shared `&ChunkPool` yields stable direct payload borrows; every append,
   seal, transfer, rollback, or prune requires the caller's exclusive
   `&mut ChunkPool`. Persistent active lists retain only a move-only checked
   builder coordinate and scalar tail state; they never retain that borrow.
   A conservative durable bound protects page chunks
   rebranded into state carriers.
3. Cold format decode stages validated node rows directly into the same
   generation-local page-material pool as its immutable initial accepted
   prefix, then seals the initial checkpoint. Loaded durable roots are typed
   rebrands of those canonical coordinates; ordinary resolution has no cold
   fallback or later materialization copy.
4. One generation-owned `ShipoutScratchArena<G>` contains only nodes genuinely
   derived by an active output attempt, currently math lowering. Stable rows
   retain warmed capacity; nested attempts take scalar marks and reset their
   complete suffix in O(1).

All four storage classes are owned below the external store's current slot,
not by a mode, group, box, or node. Builders consume runtime nodes once into
active chunks; sealed regions transfer whole chunk envelopes into page
material without payload copying. Setbox, PDF-form, box, unbox, page, math, and
alignment transitions promote or rebrand typed coordinates. A TeX box copy
shares the immutable range-list root under the same slot lease; it never adds a
per-node or per-list reference count. Retained runtime checkpoints name sealed
chunk and descriptor boundaries. Post-fork publication opens only the current
suffix, and forking copies no node or token payload.

Paragraph post-line materialization is the range-preserving case where its
input is already immutable page material. The production tape consumes one
`PageListId` plus scalar break actions and reborrows it through `NodeCursor`.
Each completed line is one canonical page-material list assembled by a
detached active builder: unchanged paragraph and discretionary-branch spans
remain source ranges, while skips, direction repair, changed discretionary
records, shaping/PDF replacements, and overfull rules are appended exactly
once. A distinct TeX-physical source is retained only as an optional detached
diagnostic projection; an ordinary paragraph creates no second list. Lineage
and boundary evidence are scalar scratch, never `Vec<Node>` payload. The
page-material counters distinguish actual appends from published-source copies,
and address-stability tests exercise both the zero-copy route and a nonzero
negative control.

The page builder publishes four independent page-material roots for its
contribution list, current page, page discards, and split discards. The
aggregate checkpoint records those roots directly; it never composes them into
a synthetic list. Restart publication is admitted only with one quiescent,
empty outer vertical mode, so the mode checkpoint retains scalar continuation
state but no active builder or transient mode-material root.

Node token fields share the existing non-atomic stored-token payload for a
true semantic alias. This applies to marks, deferred writes and specials, PDF
literals and identifiers, and alignment templates. The node adds no word copy,
content hash, owner search, `Arc`, or `Weak`; final drop of either semantic
carrier releases the shared allocation exactly once.

Shipout receives a typed `ShipoutRoot<G>` and traverses page or durable rows in
place. Explicit operands are borrowed from page storage; box 255 and PDF forms
are borrowed from immutable durable storage. `ShipoutListId<G>` can additionally
name the scratch lane during traversal, but `ShipoutScratchListId` is a distinct
private-construction coordinate which no page state, mode, journal, format,
memo, checkpoint, or artifact field accepts. Deferred writes and PDF navigation
payloads likewise retain typed source coordinates through suspension and are
streamed into expansion or their final detached/durable destination. No source
node closure or token payload is copied into another live arena for shipout.

Successful publication lowers directly to a handle-free page plan and artifact.
Failure restores scalar roots and journal cursors, then resets the complete
shipout-scratch suffix. Published output retains no node id, arena owner,
generation owner, or engine borrow.

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
accepted generation; it cannot retain a third generation. Named-boundary
publication appends a move-only frame to the physical command owner's typed
fork arena and retains only its sealed mark, one-cell coordinate, attempt
mark, and coarse generation capability. Checkpoint aliases copy those fixed
coordinates and never alias mutable command storage. The retained executor
store parks the sole `CommandState` owner. Edit selection rewinds the accepted
suffix in place and detaches its whole chunks; rejection rewinds current cells
and redoes the detached prior cells before reattachment, while acceptance
prunes the detached chunks. A checkpoint also contains compact marks and any
optional coarse packed-bank snapshot. Its
generation owns one conservative monotonic page-retention bound. A checkpoint
with an explicit page handle in page-builder or mode state may raise that bound
to the current page cursor; a checkpoint with no such carrier adds nothing.
Rootless shipout may truncate only the suffix above the bound. Pruning need not
lower it, and replacing the generation drops it wholesale. The explicit
command-root fork copies the aggregate's vectors and scalar coordinates but
shares immutable definitions and stored-token payloads through their existing
private owners; it does not traverse or copy definition, node, provenance, or
page payload graphs. An ordinary retained-generation fork later clones that
root into the sole current slot, while mutable banks and other runtime roots
receive one destination-local representation.

PDF state uses that rule directly. `PdfStateSnapshot` contains only inline
scalars, canonical append-log lengths, fixed general/color version roots, two
logical history coordinates, and the coarse payload position. Capturing it
visits no row, clones no container or token owner, retains no new payload
owner, and allocates nothing. Pages, font operations and resources, images,
raw-object reservations, document fragments, page reservations, canonical
space-font names, annotations, links, forms, destinations, outlines, threads,
PK rows, and catalog-action rows retain insertion order in dense append logs;
lookup sidecars never define serialization order. Raw-object initialization
and reference, annotation initialization, match replacement, open-link
push/pop, form-artifact replacement, destination definition, and thread-bead
append publish result versions into one coarse packed general lane. Page and
form color operations publish into the aligned color lane, so a form traversal
can restore only its color work.

PDF candidate creation is an exclusive transaction, not a state fork. The
reachability store moves the unique `PdfState` authority from the accepted
slot into the candidate and leaves the accepted `PdfStateSlot::Loaned`.
Accepted admission returns `CandidateTransactionActive` until the candidate
ends; a suspended candidate continues to own the same transaction. No shared
mutable PDF container, `RefCell`, COW root, or destination clone exists on the
ordinary PDF path.

Every canonical row family uses `PdfRows<T>`. Outside a transaction its
accepted rows are one ordinary contiguous `Vec<T>`. Candidate creation records
one logical base length per family without visiting rows. Accepted-only suffix
rows remain physically in that vector, while candidate rows append to a
private delta. Reads and mutations use one direct base/delta branch. Rejection
drops the deltas and reveals the accepted suffixes; acceptance truncates those
prior-only suffixes and moves candidate deltas into the retained vectors.
Space-font lookup uses the same object-id cutoff plus a candidate-local lookup
table. Form artifacts use direct object-id dense tombstones, so deletion and
restoration never allocate and lookup iteration cannot define serialization
order.

General and color mutations append result versions to candidate-private coarse
lanes and advance fixed persistent-trie roots. A candidate pins the accepted
roots and selects its early checkpoint roots without replaying the intervening
accepted history. Current reads resolve through the selected root; historical
keyed lookup is a bounded 64-probe trie walk and is not candidate lifecycle
work. Rejection restores the accepted roots and drops the private suffix.
Acceptance promotes that suffix at the named boundary without walking
accepted events. Image and form payload boxes remain in the same dense row
allocation throughout; neither capture, candidate creation, rejection, nor
acceptance copies payload bytes.
Payload ids remain internal direct indices and never affect PDF object
numbering.

Marks can be created only at a boundary whose live builders are sealed and
whose execution scratch is quiescent. A mark is not an owning reference to
each value it can restore. The session's prior/current generation slots and
the checkpoint's direct aggregate owners provide lifetime; checkpoint cursors
provide position. Dropping or pruning the checkpoint releases those owners
immediately. Slot reuse never revalidates an old key, relocates a surviving
checkpoint, or compacts live coordinates.

Page candidate settlement orders semantic roots ahead of physical chunk
ownership. Selection prevalidates both owners, rewinds the four PageBuilder
roots while all accepted chunks remain attached, and only then detaches the
accepted suffix. Rejection first undoes and drops every current root, releases
current chunks and reattaches accepted chunks, then redoes the accepted roots.
Acceptance drops accepted root inverses before pruning the detached accepted
chunks. No fallible step may begin after this prevalidated root transition.

Named execution evidence is not itself checkpoint authority. A fresh command
processor owns one move-only job-start eligibility receipt, consumed before
execution begins. Live execution can produce another receipt only after a
root-main-file outer paragraph reaches the quiescent publication barrier.
Shipout completion remains detached evidence for hosts and telemetry but has no
receipt constructor, so it cannot enter retained restart history.

Restore is atomic and follows this order:

1. Validate the checkpoint/session identity, generation ancestry, all journal
   and stack cursors, arena positions, and external ledger cursors without
   mutation.
2. Validate that the checkpoint names the live prior or current slot and hold
   the applicable coarse generation lease through restoration.
3. Restore dense banks by loading the selected coarse packed image when one is
   present and replaying the exact journal suffix to its cursor; otherwise
   undo the live journal directly to the cursor.
4. Select the PDF general and color version roots, then restore its scalar
   cursors and transactional row selections. Remove candidate lookup suffixes
   before resetting canonical row deltas; reset image/form payload deltas last.
5. Restore input, condition, group, mode, page, source, resource, effect, and
   output cursors, transferring canonical roots before releasing replaced
   owners.
6. Restore scratch lengths and truncate unpublished provenance, input,
   durable, and mode/page storage suffixes to their validated positions.
7. Release abandoned generation and page owners only after no restored cell,
   stack entry, or cursor can name their storage.

Any validation failure leaves the runtime unchanged. Reusing a physical arena
slot requires a new generation key before another id can name it.

PDF history positions are absolute logical coordinates. The live-index vector
still supplies the oldest retained general and color positions, while pruning
advances scalar low-water coordinates without relocating packed event indices
or rewriting surviving marks. The single accepted event owner retains the
named version ancestry; an exclusive candidate adds only private event, trie,
stack-node, and color-value suffixes. Rejection drops those suffixes and
acceptance promotes them. There is no replay lane, copied current table, root
registry, compaction pass, or additional PDF authority.

The focused release gate in
`benchmarks/tex-state/src/bin/pdf_checkpoint_gate.rs` measures the PDF mark in
isolation. Across warm 2026-08-25 implementation runs, one million captures
took 45--115 ns each and one million no-mutation restores took 149--263 ns
each; the best paired 64 MiB result was 50 ns capture and 149 ns restore. Both
paths performed zero allocation calls and requested zero bytes with either a
1-byte or 64 MiB accumulated image payload. Retained payload bytes equaled the
single authoritative payload exactly (1 byte and 67,108,864 bytes); checkpoint
capture added none. The replaced representation necessarily allocated and
copied the complete payload `Vec` at each capture, so the corresponding 64 MiB
case had a direct lower bound of 67,108,864 requested bytes per mark before
counting cloned row vectors, maps, tokens, colors, or forms. The exact arXiv
baseline attribution remains 134.27 MiB of live bytes under PDF/checkpoint
clone stacks and 33.477 billion profiled cycles for the complete row.

The authenticated arXiv 20M row used the pinned 2026-03-01 distribution,
schema-12 `pdflatex.fmt`, 123-key offline closure, fixed clock, 45-second and
1,536 MiB guards, and the same input digests as the 854,600 KiB baseline.
Three final transactional runs produced wall/user/system seconds of
13.90/15.02/1.66, 13.43/14.81/1.52, and 14.10/15.50/1.63, each with peak RSS
of 654,224 KiB. The median wall and user times were 13.90 and 15.02 seconds,
respectively, versus 19.78 and 20.46 seconds at baseline; median RSS was
654,224 KiB, 200,376 KiB lower. Every run stopped at the exact terminal vector
`(20000000,19913119,2218327,6020965,16785710,4011)`.

Publishing the loaded format as the ordinary initial accepted checkpoint was
measured with the same authenticated format, distribution, 123-key closure,
clock, offline mode, and guards. Three fuel-1 runs reached 249,300, 249,304,
and 249,496 KiB RSS, preserving the former 249,108 KiB cold full-buffer peak
within run noise. Three 20M runs produced wall/user/system seconds of
14.07/15.43/1.61, 13.54/14.94/1.51, and 14.00/15.35/1.59, with RSS of 536,064,
536,644, and 536,068 KiB. Median RSS was 536,068 KiB, 118,156 KiB (115.39 MiB)
below the preceding 654,224 KiB result. Every fuel-1 run stopped at
`(1,1,0,1,0,0)`, and every 20M run preserved the exact terminal vector
`(20000000,19913119,2218327,6020965,16785710,4011)`.

Consuming format admission was measured with the same authenticated schema-12
format, distribution, 123-key closure, fixed clock, offline mode, and guards.
The encoded image is released before destination construction, and decoded
font, definition, token-list, glue, node-list, cell, hyphenation, and PDF rows
are moved or drained into their final owners. Three fuel-1 runs reached 51,848,
51,852, and 51,664 KiB RSS and preserved `(1,1,0,1,0,0)`, putting cold startup
below the 150 MiB engine target. The profiling build reached 49,488 KiB RSS,
reported zero physical or external node-graph copies, and retained the named
cold-materialization, interpreter, and generation-boundary attribution. Its
13,630,601,192 cumulative cold-materialization requested bytes describe arena
capacity and repeated allocation requests during admission rather than live
or retained bytes; peak process RSS is the resident-footprint guard.

`benchmarks/tex-state/src/bin/pdf_fork_metadata.rs` measures candidate
begin-plus-reject by field family. At 10,000 rows every measured family is
independent of accumulated size and performs zero allocation calls requesting
zero bytes: page reservations 0.58 microseconds, font resources 0.61, external
image metadata 1.06, raw objects 0.74, annotations 0.52, destinations 0.49,
threads 0.49, form-artifact index 0.50, space-font names 0.46, color stacks
0.51, and match bytes 0.48. Before the exclusive transaction, the remaining
10,000-row mutable families cost up to 2.31 milliseconds and 1,680,398 bytes
per fork. `pdf_undo_distance.rs` now starts from an early retained mark and
compares 1,024 with 16,384 later accepted general/color versions. Open,
rejection, and acceptance each perform zero allocations and request zero
bytes. First mutation is identical at 18 allocations and 12,868 requested
bytes. Every lifecycle phase reports zero replay work; the separately reported
historical resolution bound is 128 probes for the two family lookups. The
exact rollback test covers overwritten/deleted keyed values, PDF object order,
and stable form-payload addresses across both rejection and acceptance.

## Incremental revisions and two-generation ownership

An incremental session has exactly these possible live runtime generations:
the prior accepted generation and one current candidate. There is never a
second candidate or a history-owned generation. They are admitted under
distinct invariant generative brands. Prior admission is read-only; every
revision-local allocation and mutable root belongs to current. Candidate
creation consumes an exclusive current-candidate lease, so another factory or
caller cannot issue a concurrent candidate. Candidate execution may compare
detached evidence from prior. It may also be seeded by the one validated
aggregate checkpoint-fork operation: the operation checks every accepted root
first, moves the same physical command owner into a destination-local current
suffix, and publishes the destination only after the complete fork succeeds.
Logical revision acceptance transfers that owner lease without changing its
physical timeline identity or rebranding retained prefix marks. Shared
definition and stored-token carriers keep their existing private non-atomic
owners; no public id is retargeted and no third lineage is created.

A loaded format enters this same model as the initial accepted generation.
Admission consumes the validated detached image, drops its encoded bytes
before construction, drains or moves every decoded row into one generation,
and retains its pre-job `JobStart` checkpoint in the prior slot. The first
document candidate therefore uses the same exclusive checkpoint fork as every
later candidate. Rejection leaves that generation unchanged; the first
acceptance retires it through the ordinary prior/current swap. There is no
format-owned third generation, permanent image owner, complete decoded/live
overlap, or format-specific runtime lookup layer.

Rejection consumes the exclusive lease, clears the current store slot, and
leaves prior unchanged. Acceptance first requires quiescent scratch and
validates current-generation locality without mutation. It then consumes the
lease, clears the complete former prior slot, and changes current's role to
prior. No row, slab, or value moves. History retains only detached semantic
evidence, hashes, schedules, and output prefixes. It never retains a live
checkpoint or generation owner.

There is no runtime compactor, relocation map, generation graph, forwarding
pointer, slab splice, tracing collector, or content-equality merge. Routine
edits perform at most one aggregate checkpoint fork, never a per-checkpoint
copy or a serialization round trip. Definitions and stored token lists release
on their last semantic-owner drop; compact glue/provenance and durable node
arenas retain their separate policies. Explicit format, artifact, and
detached-continuation boundaries remain the cold detached-copy paths.
Whole-slot retirement remains the final release for generation-owned
publishers, inline arenas, and reusable capacities. Node graph lifetime changes
belong to the separate node transfer/copy work, not this shared-ownership
policy.

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
- no per-value heap-owner construction (an allocation-free `Rc::clone` for a
  true semantic alias is allowed).

TeX main-memory usage reads and capacity observations additionally perform one
scalar projection. They never visit eqtb banks, register banks, definitions,
token payloads, page roots, node closures, or a deduplication structure.

Generation validation occurs once when the exclusive current lease, a
continuation, checkpoint, or detached value is admitted. Within that admitted
borrow, shared handles dereference their immutable slice directly and compact
inline ids resolve by typed direct indexing. The coarse store makes one
session-boundary `Rc<RefCell<_>>` allocation, justified by the self-contained
same-thread FFI API; its slots are inline, and candidate creation, admission,
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
- per-value `Arc`, `Weak`, atomic counts, owner registries, or implicit
  drop-driven store callbacks; private non-atomic shared ownership is required
  for immutable aliasable definitions and stored token lists;
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
