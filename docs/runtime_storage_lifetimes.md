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
same external store. At rest the session occupies only the accepted slot.
Starting an advance may occupy the other slot with one candidate; history
metadata never occupies a slot. Public incremental, virtual-compile, project,
fixed-point, editor, and native sessions borrow a caller-owned store explicitly;
their Rust lifetime prevents a session or generation from outliving it. A suspended
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

Macro definitions use compact generation-branded keys into structurally owned
regions. Eqtb and save-journal lanes copy only the key. Immutable format rows
belong to the loaded-format region, global definitions belong to the
revision-global region, and local definitions belong to nested TeX-group
regions. A local macro input row holds one coarse region lease only when it can
outlive `endgroup`; format and global rows need no per-use lifetime operation.
Stored token lists retain their existing exact non-atomic shared handles. No
definition uses a per-value owner, registry, liveness search, tracing pass,
compaction, relocation, rehome, or additional historical generation.

TeX main-memory usage is maintained by one generation-local scalar aggregate.
Publishing a definition or stored token list charges its canonical words once.
Definition charges leave with checkpoint truncation, whole local-region
retirement, or generation retirement; the last stored-token owner drop releases
that payload's charge.
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

[Node-region ownership](node_region_ownership.md) is the authoritative
specialization for runtime node closures. It preserves this document's
two-lineage and fixed-chunk rules while replacing raw-coordinate aliasing and
conservative page bounds with exclusive page/durable region owners.

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

Macro definitions and stored token lists deliberately have different lifetime
mechanisms. `DefinitionId<G>` is a `Copy`, at-most-16-byte region/row/identity
key. Region ownership is structural through the format owner, revision,
checkpoint boundary, group stack, and active local macro-input rows. Stored
token lists retain their private `Rc<[TokenWord]>`; moving transfers an owner
and an explicit clone records a true alias. No definition has a per-value
`Rc`, `Arc`, `Weak`, owner registry, or ordinary-read liveness lookup.

The following matrix is normative:

| Value or storage                                              | Immediate owner                                                                     | Valid until                                                                                     | Rollback behavior                                                                              | Escape path                                      |
| ------------------------------------------------------------- | ----------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| Interned control-sequence name and token spelling             | Session interning epoch                                                             | Session epoch retirement                                                                        | Never rolled back                                                                              | Detached spelling or semantic atom               |
| Current meaning, parameter, register, or code value           | Dense current-value bank                                                            | Overwritten or bank retirement                                                                  | TeX group saves, checkpoint deltas, or operation-local undo restore in place                   | Packed value in a checkpoint or DTO              |
| Immutable macro definition and its definition token lists     | Format, revision-global, or forked local-group definition region                    | Region/checkpoint/generation retirement; active local input rows may delay whole-region release | Rollback truncates the candidate region suffix; group restore precedes local-region retirement | Handle-free recipe at a cold boundary            |
| Stored token list                                             | Every live eqtb, journal, input/expansion, checkpoint, PDF, or continuation carrier | Last exact carrier drop                                                                         | Moves transfer; true aliases explicitly clone; truncation/pruning drops                        | Handle-free recipe at a cold boundary            |
| Macro/scanner frames, arguments, builders, or temporary words | Current generation scratch                                                          | Operation completion, rollback, or continuation disposal                                        | Reset the applicable lane lengths to saved cursors                                             | None; surviving output is built in final storage |
| Prepared cold operation or operation-local failure            | One caller-owned direct `OperationFrame`                                            | Application, rollback, typed suspension disposal, or reuse                                      | Completion consumes occupied fields; suspension moves the exact frame intact                   | Typed in-process attempt continuation only       |
| Pending mode material and page-builder nodes                  | The current exclusive `PageRegion`                                                  | Page shipout, rollback, or transfer to a durable owner                                          | Move-only region/suffix settlement after root restore                                          | Direct construction or shipout lowering          |
| Box-register or checkpoint-surviving node                     | Exclusive durable region or history-owned page region                               | Owning register/form/journal/checkpoint interval retirement                                     | Restore owners before abandoned regions drop                                                   | Detached output or node recipe                   |
| Source registration and compact provenance record             | Session or revision generation                                                      | Last owning generation, live input, or output recipe retirement                                 | Cursor restoration and suffix discard                                                          | Handle-free source recipe                        |
| Structural diagnostic or rendered-source presentation         | Diagnostic or artifact DTO                                                          | DTO disposal                                                                                    | Not live runtime state                                                                         | Already detached and handle-free                 |
| Shipped page                                                  | `tex-out` value                                                                     | Output disposal                                                                                 | Outside engine rollback after publication                                                      | Serialized artifact bytes or output DTO          |

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

struct BankCell<T> {
    value: T,
    level: u32,
    save_serial: u64,
}

struct SaveJournal<G> {
    active_groups: Vec<GroupSegment<G>>,
    checkpoint_pool: ChunkPool<CheckpointDelta<G>>,
    checkpoint_lane: ForkArena<CheckpointDelta<G>, DenseJournalLane>,
    save_serial: u64,
    operation_undo: Vec<UndoEntry<G>>,
    group_capacity_bytes: usize,
    checkpoint_capacity_bytes: usize,
    operation_capacity_bytes: usize,
}

struct JournalCursor {
    group_segment: u64,
    group_entry: u32,
    checkpoint_mark: CheckpointMark<DenseJournalLane>,
}
```

Dense state remains directly mutated and directly read. Every authoritative
bank cell carries one runtime-only save serial. A mutation compares that serial
directly with the journal's monotonic interval serial; the first write appends
the exact prior value, level, and serial, while later writes in the interval
append no checkpoint delta. Restoration walks retained deltas backward and
swaps their packed alternate words and prior serials into the dense banks. The
serial is absent from format images and is never copied as a checkpoint-wide
payload. There is no hash lookup, state overlay, threshold densification,
compaction pass, forwarding coordinate, per-entry owner, or checkpoint bank
clone. Edit
start detaches the accepted chunk suffix, candidate writes append privately,
rejection swaps the candidate backward and accepted suffix forward, and
acceptance prunes the detached suffix. Whole group
segments are moved between active, checkpoint-retained, operation-pending, and
reusable-buffer owners without scanning, copying, relocating, or repacking
their live entries. The journal updates three exact byte scalars only when a
group buffer, checkpoint pool, or operation lane can change capacity.
Execution-budget checks read those scalars in constant work; they do not walk
groups or checkpoint chunks on each command. These scalars describe physical
capacity only and are neither semantic state, a liveness registry, nor a
second journal representation.

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

`DefinitionArena` owns three storage classes behind one compact key and borrowed
view interface:

- the immutable format region, populated only by atomic format materialization;
- the revision-global region, whose row/word marks participate in candidate
  checkpoint detachment, acceptance, and rejection; and
- nested forked local-group regions. Entering a TeX group pushes exactly one
  child region. The arena stores only one current-region scalar; each stable
  region slot stores its exact parent key. A nested child does not split its
  parent: leaving restores meanings first, follows that parent key, and retires
  only the exact child. There is no relocatable active-prefix vector.
  The child is dropped immediately when it has no lease. Otherwise the child
  is marked retired, and the final active-input or checkpoint lease release
  directly drops that region's payload. Local regions occupy stable 64-slot
  chunks behind one coarse generation-owned slot store. Entry pops one exact
  address from an intrusive free-slot chain or grows by one chunk; it never
  allocates an individual region shell or relocates an accumulated prefix.
  Reuse increments the address's
  incarnation inside the existing region coordinate, so stale keys cannot
  alias the new occupant. Neither group transition searches local-region
  history. A child structurally pins its parent until the child itself is
  reclaimed. Consequently one lease of the current region pins the complete
  checkpoint ancestry in constant work; the checkpoint owner stores that one
  scalar lease rather than a region container. Final child reclamation releases
  that one exact parent pin. A final release drains a reclaimable retired
  ancestry iteratively, so maximum supported group depth does not become Rust
  call-stack depth.

`DefinitionId<G>` is only an eight-byte stable storage key. Its region and row
locate one immutable header plus the contiguous `[parameter][replacement]`
word span. The header stores span boundaries, origin, content identity, and
the validated at-most-nine-marker parameter pattern. `DefinitionView<'a, G>`
borrows that metadata and span for one synchronous use, so invocation never
rescans the parameter text. Cold checkpoint identity resolves the key through
arena metadata and records the result in a separate identity-only table; the
hot key and `CurrentCommand` carry no content hash. Equal definitions retain
distinct storage keys but have equal content identities regardless of
allocation order. A global `let` repeats the exceptional promotion of the same
local key by reusing its cached global key. That mapping remains owned by the
source local region, so exact region retirement and checkpoint settlement
discard it without a generation-wide promotion sweep.

Ordinary `def` and `gdef` know their destination before scanning. The scanner
opens a transactional word mark in that final local or global region, validates
parameter structure while appending each word once, and seals by appending only
the compact header. Failure truncates the exact word mark. There is no
attempt-local publication body, final-body copy, per-definition allocation, or
raw-definition continuation. `edef` and `xdef` use the same destination
transaction and retain continuation state only when expanded scanning actually
encounters a resource suspension. Detached `DefinitionBuilder` staging remains
only for cold format/memo/import batches whose source is already outside the
live scanner.

```rust
pub struct DefinitionId<G> {
    region: DefinitionRegionCoordinate,
    row: NonZeroU32,
    _brand: PhantomData<fn(&G) -> &G>,
}

pub struct DefinitionArena<G> {
    format: DefinitionRegion,
    global: DefinitionRegion,
    local_slots: Rc<LocalDefinitionSlots>,
    active_local: u32,
    mutations: Vec<DefinitionRegionMutation>,
}

impl<G> DefinitionArena<G> {
    pub fn get(&self, id: DefinitionId<G>) -> DefinitionView<'_, G> {
        // O(1) checked region/row admission into immutable words.
    }
}
```

Here `G` denotes the fresh private brand minted when a generation is admitted;
it is not one global marker shared by all generations.

Rust enforces part of this contract. The invariant brand prevents statically
typed ids from different admitted generations from mixing. Module privacy
prevents callers from forging keys, constructing views, or changing region
coordinates. A view cannot cross the store borrow that admits it. Eqtb, save,
and operation lanes copy the key without changing ownership; local-region
liveness resides only in the structural group/checkpoint/input owners.
Checkpoint capture retains the one current-region key and a scalar mutation
journal mark. On the first definition or lifecycle write to a region in the
new checkpoint epoch, the generation journal records that exact region's
pre-write header, word, promotion, and retirement marks. Candidate selection,
acceptance, rejection, and direct cursor restore traverse only this journal
suffix. An existing header's first origin change adds one compact inverse field
edit to its region record; repeated changes coalesce, while a newly appended
header is already covered by the row suffix. Detachment and replay swap those
field alternates without copying whole headers or regions. This includes an
ancestor first written after its checkpoint child was ended; no active-chain or
historical-region scan is needed. Detached accepted suffixes and origin
alternates remain source-region-owned until rejection reattaches them or
acceptance releases them.

The four-byte region coordinate reserves all of its bits for addressing:
values 1 and 2 name the fixed format and global regions, while local values
encode a 16-bit slot address plus a nonzero 16-bit incarnation. It contains no
macro-carrier or locality flag. A later eight-byte non-owning `DefinitionId`
can therefore pair this explicit coordinate with its four-byte row without
reinterpreting a flag bit. Reuse stops permanently when one slot reaches
incarnation 65,535; allocation then consumes another address, and exhaustion
returns `CapacityOverflow`. Incarnations never wrap, so an ABA-stale key cannot
silently become valid.

The macro integration branch must remove its bit-63 identity flag: all 32
coordinate bits are address/incarnation state, and semantic identity is
resolved from the admitted definition header rather than encoded in the
carrier.

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

That same common packed frame stores the active external-source identity.
Source admission writes its own identity; replay and macro-argument admission
inherit the enclosing value. A delivered command carries that execution fact
beside its distinct spelling provenance, so checkpoint-origin classification
requires one semantic-top read and no later ancestry or source-owner lookup.

Ordinary key reads, `let` aliases, moves, restoration, and warmed arena reuse
allocate no heap memory and copy no token payload. A global `let` whose source
is local is the sole runtime escape conversion: it copies that immutable span
once into revision-global storage and records the source/global key pair for
reuse. No root scan, compactor, relocation, or per-definition reference count
participates. Cold format capture compacts only reachable definition keys into
handle-free rows; materialization publishes those rows into the immutable
format region and aliases reuse the mapped row.

## Hot resolution and suspension

At episode admission, `tex-command` receives a borrowed view of the generation
which matches the dense state it will execute. The packed-token resolver takes
the actual caller-owned `CurrentCommand` target, performs one dense-row access,
and lets that canonical row decode its compact definition key once into the
final slot. `CurrentCommand` acquires no definition owner. A committed local
macro body obtains one coarse region lease in its specialized input row;
format and revision-global rows store only the key. Parameter and replacement
reads borrow the arena span directly. Admission acquires one guard
for the episode, not one lock or heap allocation per lookup.

Each active next-command request also owns one reusable `CurrentCommand` slot.
Reference-only `EmptyCommand` reborrows prove that input writes its final
meaning into that slot without adding storage or moving the command. Input
returns only packed scalar resolution facts, and resident settlement reclaims
the original caller-owned destination directly.
The input stack ends its raw borrow before a cold line, EOF, parameter push, or
suspension transition; meaning resolution ends its dense-state borrow before
outer recovery, alignment settlement, observation, or delivery. Raw delivery
records token-frame, scanner, and optional meaning-lookup work together in the
singular fuel ledger after resolution only when the existing `profiling`
resolution is selected. The default resolution compiles those updates out;
its admission charge remains the sole accounting at the canonical episode
boundary.

No borrow crosses an executor barrier. A suspended scanner, macro expansion,
or resource request stores the current-generation execution lease plus ids and
integer cursors:

```rust
struct MacroBodyCursor<G> {
    definition: DefinitionId<G>,
    definition_region: DefinitionRegionLease<G>,
    arguments: Option<ArgumentSet<G>>,
    frame: ResidentSpanCursor,
}

struct SuspendedExecution<G> {
    current: CurrentGenerationLease<G>,
    resume: ResumePoint<G>,
    request: ResourceRequest,
}
```

Resume retains the admitted input row and continues at its stored scalar
cursor. A local-definition row retains the one direct region lease admitted at
the push boundary; a format or revision-global row has no lifetime operation.
The continuation never contains a Rust reference into an arena, no runtime
type is self-referential, and suspension never admits a second current
generation.

## Execution scratch and destination-directed construction

The current candidate owns exactly one reusable `ExecutionScratch<G>`. It is
generation state, not a tree of operation, scanner, macro, or group owners.
Its packed/preallocated lanes are physically separate because their nesting
and survival patterns differ:

```rust
struct ExecutionScratch<G> {
    argument_sets: Vec<PackedArgumentSet<G>>,
    macro_words: FixedChunkLifoLane<TokenWord, 4096>,
    macro_origins: ProvenanceChangeRuns,
    scanner_frames: Vec<PackedScannerFrame<G>>,
    scanner_words: FixedChunkForkLane<TracedTokenWord, 64>,
    scanner_builders: Vec<PackedScannerBuilder<G>>,
    expansion_frames: Vec<PackedExpansionFrame<G>>,
    render_bytes: Vec<u8>,
}

struct ScratchCursors {
    argument_sets: u32,
    macro_words: u32,
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
absolute lane and reclaim marks plus up to nine direct word ranges; one direct
current-argument slot owns collection facts and its cursor until completion.
The pending frame appends directly to the logically contiguous fixed-chunk
lane. Commit changes only the pending frame's role, while discard or later
frame retirement truncates to its mark and returns suffix chunks to reuse. If
an older active frame retires beneath a pending child, it transfers the earlier
reclaim mark. Only the still-unpublished child suffix may rebase, and only once
the last active ancestor has retired; sealed ranges and admitted cursors never
move. Rebase is an explicit forward copy of every word in that unpublished
suffix, and exact test accounting distinguishes it from the no-copy ordinary
seal/replay/retire route.
A scanner frame records the opening lengths of its temporary-word and builder
lanes. A token-list destination is one branch coordinate in the shared
fixed-chunk scanner lane: nested destinations fork without moving the parent,
sealing publishes the same branch, and rollback returns its whole chunks. No push creates an arena,
scope capability, ownership token, loan, mailbox, watermark row, or parent
graph. Fixed synchronous state stays in ordinary Rust stack locals. State that
can suspend or become too deep for the Rust stack uses explicit packed frames
and direct indices carrying the invariant generation brand `G`.

The executor's cold preparation boundary is one such fixed synchronous owner,
not another scratch lane. One caller-loop `OperationFrame` holds the prepared,
applied, or diagnostic payload while preparation returns only a compact status.
Application consumes its fields directly and completion reuses the empty slot.
One admitted semantic command context remains resident across ordinary cold
inspection, application, and command-owned named-hook receipt drainage; only an
actual host boundary releases it. A resource suspension moves that same frame
into the singular typed attempt; there is no per-operation box and no append
lane retaining completed commands.

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

Executor mode settlement follows the same explicit barrier. A checkpoint fork
creates one candidate-labelled `ModeNest`; accepting or rejecting consumes that
label exactly once. Ordinary `Drop` performs no mode rollback and an unresolved
normal drop is an invariant failure. Terminal MainControl completion moves the
mode capability into a prepared settlement receipt instead of disposing it;
Session consumes the receipt before the prior/current state slots settle. The
retained mode mark remains the fixed rootless outer level, while all nested
topology and mode roots belong solely to the live candidate.
Candidate execution borrows temporarily detached runtime and MainControl owners
through a prepared guard whose unwind path parks both sidecars infallibly. An
outer owned generation guard then performs complete aggregate rejection before
the current slot retires. MainControl preparation consumes its existing mode
storage directly, and acceptance changes the candidate label in place, so
neither production disposition constructs a default mode stack.

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
   below exclusive `NodeRegion` owners. Each typed `ForkArena` owns only lane
   coordinates, lifecycle metadata, and the sole direct-range/nonrecursive-
   range-sequence list topology. One `PageRegion` owns each page-building
   period; durable boxes/forms own separate self-contained regions. Active
   paragraph, math, alignment, and box regions promote only as sealed whole
   envelopes into an exclusive destination. Operation marks may restore a
   partial tail; retained checkpoints can name only sealed whole-chunk
   boundaries. A shared region/pool borrow yields stable direct payload
   borrows; every append, seal, transfer, rollback, or prune requires the
   caller's exclusive mutable owner borrow. Persistent active lists retain only
   a move-only checked builder coordinate and scalar tail state; they never
   retain that borrow.
3. Cold format decode stages validated node rows directly into the same
   generation-local page-material pool as its immutable initial accepted
   prefix, then seals the initial checkpoint. Loaded durable roots are typed
   rebrands of those canonical coordinates; ordinary resolution has no cold
   fallback or later materialization copy.
4. One generation-owned `ShipoutScratchArena<G>` contains only nodes genuinely
   derived by an active output attempt, currently math lowering. Stable rows
   retain warmed capacity; nested attempts take scalar marks and reset their
   complete suffix in O(1).

The physical pool remains below the external store's current slot, but semantic
node lifetime belongs to move-only regions. Builders consume runtime nodes once
into active chunks; sealed regions transfer whole chunk envelopes into page
material without payload copying. Setbox, PDF-form, consuming box/unbox, page,
math, and alignment transitions move exclusive owners or ranges. TeX `\copy`
and `\unhcopy` deep-copy the exact recursive node closure. If a retained
checkpoint or save journal must preserve the source of a consuming move, the
old region moves into history and the current destination receives that exact
closure copy. Glue and stored token values retain their selected explicit
shared owners. No per-node or per-list reference count is added.

`PageRegionHistory` owns the one page-lifetime `NodePool`; an individual
`PageMaterialRegion` contains a `NodeRegion<PageRole>` plus region-local scratch
and semantic-identity state, never another pool. Mutable and immutable
`PageMaterialArena`/`PageMaterialView` facades borrow both authorities
explicitly. Current and retained prior page regions can therefore own distinct
chunk envelopes across succession while payload addresses remain stable.
History acceptance, restore, canceled successor preparation, last-checkpoint
pruning, and uncheckpointed succession explicitly retire discarded region
envelopes and generation-invalidate their ids.

A `ClosureBuildMark` seals payload and descriptor tails before structural box
construction. Sealing first proves that the owner-relative root, every range,
every recursively stored child coordinate, and every referenced chunk lie in
the suffix; the caller supplies a consumed-roots receipt after auditing the
checked spans held by PageBuilder, ModeList, and their same-region journal and
checkpoint projections. Rejection mutates neither source authority nor
lifecycle counters and returns the move-only build mark.
Success detaches whole envelopes into a `SealedNodeClosure`; a transient loan
can reattach them without copying, while transfer rebrands only coordinates in
the bounded suffix scan and preserves payload addresses. Interleaved prefix
children and foreign or independently retained roots use the explicit
reason-counted structural-copy fallback.

This is a transfer foundation, not the durable-carrier cutover. Production
setbox completion and failed shipout currently consume an explicit
compatibility receipt and retain their constructed suffix in the page region.
Page succession still recursively copies exactly the held-over closure into the
new region. TeX `\copy`/`\unhcopy` and explicit `publish_source_copy` remain
semantic structural copies. No production path yet transfers a sealed suffix
into a `DurableRole` carrier, and no second representation, reference count,
compaction, or forwarding coordinate is introduced.

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

Demand-enabled page identity uses the existing composable polynomial sequence
root. Original appends maintain one whole-used-chunk summary in coarse payload
metadata, while canonical range publication stores one exact range summary in
the descriptor entry. A long direct subrange hashes only its two partial
boundary chunks and combines summaries for its interior; a range-sequence
subrange combines whole entry summaries and handles only its two partial entry
boundaries. Prefix/suffix subtraction keeps identity independent of range and
chunk layout. Partial operation marks retain the payload-tail summary, and
whole-chunk promotion or accepted/candidate settlement moves summaries with the
same envelopes. No per-node prefix table, root registry, or source payload copy
participates. Without explicit identity demand, these paths do no hash or
summary work.

The page builder stores four independent checked `PageListSpan` roots inside
the current `PageRegion`: contribution list, current page, page discards, and
split discards. The aggregate checkpoint row records those same-region spans,
sealed payload/descriptor positions, scalar state, and journal position; it
never composes them into a synthetic list. Root and arena marks settle
atomically, so rollback does not carry a proof across owner or generation
boundaries. Multiple paragraph checkpoints in one page share the same region
and copy no nodes. Restart publication is admitted only with one quiescent,
empty outer vertical mode, so the mode checkpoint retains scalar continuation
state but no active builder or transient mode-material root.

Shipout starts a new page region. The existing page-break traversal moves
self-contained whole held-over envelopes when unique or copies only the exact
held-over closure when an old checkpoint must retain its region. Handle-free
output owns no runtime node. Checkpoint history retains an old page region only
while a boundary in its contiguous interval remains; pruning the last such row
drops the whole region.

Node token fields share the existing non-atomic stored-token payload for a
true semantic alias. This applies to marks, deferred writes and specials, PDF
literals and identifiers, and alignment templates. The node adds no word copy,
content hash, owner search, `Arc`, or `Weak`; final drop of either semantic
carrier releases the shared allocation exactly once.

Shipout receives a typed page root. Box 255 is consumed or history-preserved
into page storage before traversal, and PDF forms are explicitly copied from
their immutable durable owner into page material. `ShipoutListId` can name page
or scratch rows during traversal, but every stored `ShipoutScratchNode` child is
a `ShipoutScratchListId`; page coordinates are borrowed only while recursively
materializing that self-contained scratch closure. No page state, mode,
journal, format, memo, checkpoint, or artifact field accepts the scratch
coordinate. Deferred writes and PDF navigation payloads retain branded source
coordinates through suspension and are streamed into expansion or their final
detached/durable destination.

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
capacity. A checkpoint also contains compact marks and any optional coarse
packed-bank snapshot. Node history separately owns exclusive page regions in
document order. A legal paragraph checkpoint stores an owner-relative row in
its current region; a rootless mode checkpoint adds no node owner. After
shipout, pruning the final row in an old page interval drops that whole region.
The explicit command-root fork copies the aggregate's vectors and scalar coordinates but
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
direct checkpoint-history page-region owners provide lifetime; checkpoint
cursors provide position. Dropping or pruning the last boundary row for one
page drops that region immediately. Slot reuse never revalidates an old key,
relocates a surviving checkpoint, or compacts live coordinates.

Page candidate settlement orders semantic roots ahead of physical chunk
ownership. Selection prevalidates the region/history owner, rewinds the four
PageBuilder roots while all accepted chunks remain attached, and only then
detaches the selected region suffix plus later accepted regions. Rejection
first undoes and drops every current root and later candidate region, releases
current chunks, reattaches accepted chunks/regions, then redoes the accepted
roots. Acceptance removes checkpoint rows and accepted root inverses before
pruning the detached suffix and dropping later accepted regions. No fallible
step may begin after this prevalidated root transition.

PageBuilder insertions, insertion-class lookup positions, the five class-zero
marks, sparse mark classes, and mark-class lookup positions are canonical
current values beside that same reversible fixed-chunk journal. They have no
parallel append history. Checkpoint selection swaps journal alternates over the
explicit selected suffix; acceptance releases detached prior chunks without a
record visit, and rejection visits only current undo plus the already selected
prior redo. The removed insertion/mark lane rebuild cannot turn accumulated
page history into a publication or settlement scan.

Named execution evidence is not itself checkpoint authority. A fresh command
processor owns one move-only job-start eligibility receipt, consumed before
execution begins. Live execution can produce another receipt only after an
outer paragraph or outermost shipout reaches the quiescent publication barrier
with a frozen `RootDocument` or `UserDocumentInclude` source role. Package,
class, generated-input, and format-initialization boundaries retain their
ordinary semantic effects but cannot produce a receipt. The mechanical barrier
is proved independently before that role policy is applied.

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

A loaded format enters this model as one exact frozen `JobStart` image, not an
initial accepted runtime generation. The session retains the immutable encoded
bytes plus explicit profile, compatibility, and job-clock metadata. First-job
and later JobStart fallback decode into an independent current generation;
decoded staging disappears after atomic publication. Ordinary accepted
boundary restarts still use the exclusive checkpoint fork. There is no
format-owned third runtime generation, permanent decoded overlay, complete
decoded/live overlap, or format-specific runtime lookup layer. The separately
charged image replaces the live bootstrap cursor, allowing command and state
journal prefix chunks to be released and physically reused.

Rejection consumes the exclusive lease, clears the current store slot, and
leaves prior unchanged. Acceptance first requires quiescent scratch and
validates current-generation locality without mutation. It then consumes the
lease, clears the complete former prior slot, and changes current's role to
prior. No row, slab, or value moves. History retains detached semantic
evidence, hashes, schedules, output prefixes, and the exclusive page regions
required by exact paragraph restart. It never retains a third runtime
generation or infers node liveness from raw roots.

Convergence is the other terminal transition. The first mapped schedule row
whose complete reachable-state identity matches stops candidate execution at
that checkpoint. The candidate detaches only effects, artifacts, and DVI plans
through the match; the session joins that prefix to its detached accepted
suffix. After validation, the candidate is rejected wholesale. One aggregate
accepted-generation operation then substitutes the current immutable editor
backing, maps every retained root-source cursor and physical-line coordinate,
releases boundary rows between restart and convergence, and rebases the
surviving boundary effect/artifact prefixes. This is bounded mark/journal and
boundary-row work: it does not traverse or copy definition, token, node, PDF,
or provenance graphs. The accepted generation is now the new revision, and the
candidate slot is empty.

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
plus scalar cursors whenever that borrow ends. Its logical stacks admit each
immutable frame payload once, journal only compact first-touch execution state
or stable displaced-payload handles in fixed chunks, and settle exactly one
current plus one detached accepted suffix. Stable checkpoint rows carry direct
predecessor/successor coordinates, so selection detaches that suffix without a
parallel order vector, position search, or copied key range. Large source-line
state is isolated in a reusable stored-state slab so hot token cursor records
stay bounded.

`tex-exec` owns operation boundaries, semantic dispatch, mode/page storage
selection, node-building lifetimes, resource/effect barriers, shipout
detachment, and the ordering of aggregate restore. It cannot construct state
ids or publish partially sealed values.

`tex-incr` owns the accepted/candidate generation state machine, detached named
checkpoint evidence, history pruning, candidate acceptance/rejection, and
convergence comparison. It requests aggregate root-source rehome through a
typed `tex-exec` operation; it never inspects or rewrites private arena
storage.

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
- no per-value heap-owner construction (an allocation-free `Rc::clone` remains
  allowed only for a true stored-token-list alias).

The next-command pipeline additionally requires one caller-owned command value,
pointer-sized phase proofs only, and no raw-delivery envelope, duplicate command
representation, or borrow retained across a cold input transition.

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
  drop-driven store callbacks; private non-atomic shared ownership remains only
  for stored token lists, while definitions use compact structural-region keys;
- a global generation/root registry consulted by ordinary reads;
- per-macro, per-scanner, or per-operation arenas; forked local definition
  regions are the sole TeX-group-owned arena;
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
