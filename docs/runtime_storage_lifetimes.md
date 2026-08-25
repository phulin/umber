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
| Current meaning, parameter, register, or code value           | Dense current-value bank                                                            | Overwritten or bank retirement                                  | Exact undo through the TeX save journal                                       | Packed value in a checkpoint or DTO              |
| Immutable macro definition and its definition token lists     | Every live semantic carrier through one branded non-atomic shared handle            | Last exact carrier drop                                         | Rollback moves saved owners back and drops rejected/replaced owners           | Handle-free recipe at a cold boundary            |
| Stored token list                                             | Every live eqtb, journal, input/expansion, checkpoint, PDF, or continuation carrier | Last exact carrier drop                                         | Moves transfer; true aliases explicitly clone; truncation/pruning drops       | Handle-free recipe at a cold boundary            |
| Macro/scanner frames, arguments, builders, or temporary words | Current generation scratch                                                          | Operation completion, rollback, or continuation disposal        | Reset the applicable lane lengths to saved cursors                            | None; surviving output is built in final storage |
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

Nodes have four storage roles:

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
4. One generation-owned `ShipoutScratchArena<G>` contains only nodes genuinely
   derived by an active output attempt, currently math lowering. Stable rows
   retain warmed capacity; nested attempts take scalar marks and reset their
   complete suffix in O(1).

All four storage classes are owned below the external store's current slot,
not by a mode, group, box, or node. Builders select the final class before
emitting a surviving node and seal it there. A TeX box copy may share an
immutable durable segment by coordinate under the same slot lease; it never
adds a per-node or per-list reference count. No live node closure is copied
between storage classes.

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
accepted generation; it cannot retain a third generation. Its command owner
directly retains the one aggregate copy-on-write root and exact attempt mark
selected at capture; the command timeline contributes only a monotonic
identity serial and owns no root row. The retained executor store is the sole
checkpoint container. It reuses physical slots with generation-plus-serial
validation and exact live-index backreferences, so pruning drops unretained
owners in O(live checkpoints) without scanning full capacity. A checkpoint
also contains compact marks and any optional coarse packed-bank snapshot. Its
generation owns one conservative monotonic page-retention bound. A checkpoint
with an explicit page handle in page-builder or mode state may raise that bound
to the current page cursor; a checkpoint with no such carrier adds nothing.
Rootless shipout may truncate only the suffix above the bound. Pruning need not
lower it, and replacing the generation drops it wholesale. Capture does not
clone the live definition, node, provenance, input, or page object graph. An
ordinary retained-generation fork may later use that checkpoint to prepare the
sole current slot: immutable definition and stored-token payload owners are
shared directly, while mutable banks and runtime roots receive one
destination-local representation.

PDF state uses that rule directly. `PdfStateSnapshot` contains only inline
scalars, canonical append-log lengths, one absolute general-undo position, one
absolute color-undo position, and the coarse payload position. Capturing it
visits no row, clones no container or token owner, retains no new payload
owner, and allocates nothing. Pages, font operations and resources, images,
raw-object reservations, document fragments, page reservations, canonical
space-font names, annotations, links, forms, destinations, outlines, threads,
PK rows, and catalog-action rows retain insertion order in dense append logs;
lookup sidecars never define serialization order. Raw-object initialization
and reference, annotation initialization, match replacement, open-link
push/pop, form-artifact replacement, destination definition, and thread-bead
append carry exact inverse entries. Page and form color operations use a
separate inverse lane, so a form traversal can restore only its color work.

Image and form bytes move once into one generation-local payload arena. A
fork seals the selected delta boxes into one coarse immutable prefix and gives
the destination an empty private delta. The prefix retain copies no payload
bytes, and the first destination write appends to its delta instead of
copying the prefix. After the old prior retires, the coarse prefix is unique
again; there is no per-value owner, owner chain, content lookup, or third
generation. Payload ids are internal direct indices and never affect PDF
object numbering.

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
4. Reverse-replay PDF general and color undo entries, then restore its scalar
   roots and append-log selections. Remove lookup suffixes before truncating
   canonical rows; truncate image/form payload deltas last.
5. Restore input, condition, group, mode, page, source, resource, effect, and
   output cursors, transferring canonical roots before releasing replaced
   owners.
6. Restore scratch lengths and truncate unpublished provenance, input,
   durable, and mode/page storage suffixes to their validated positions.
7. Release abandoned generation and page owners only after no restored cell,
   stack entry, or cursor can name their storage.

Any validation failure leaves the runtime unchanged. Reusing a physical arena
slot requires a new generation key before another id can name it.

PDF undo positions are absolute. After retained checkpoint slots are pruned,
their existing live-index vector supplies the oldest surviving general and
color positions. `Universe::prune_pdf_history` drops only entries before those
positions from the two deque-backed lanes; it does not scan PDF rows or the
rest of the engine and does not rewrite surviving marks. With no retained
checkpoint, the low water advances to the live head. Thus the prior and
current physical slots retain at most their own contiguous rollback interval,
and moved old match/color/form values are released as soon as no live mark can
name them.

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
first, constructs a fresh command timeline and destination-local
mutable/runtime owners off-slot, and publishes them only after the complete
fork succeeds. Shared definition and stored-token carriers keep their existing
private non-atomic owners; no public id is retargeted and no accepted owner is
mutated.

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
