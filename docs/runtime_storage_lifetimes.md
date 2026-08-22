# Runtime storage lifetimes

Status: normative end-state architecture contract.

Implementation boundary: the legacy runtime-value region registry, per-value
root facades, reachability search, node-list strong/weak ownership, provenance
archive ownership, and their snapshot/private-revision/profiling adapters were
deleted together by `umber2-66p0.2`. The branch is intentionally compiler-red
until children `.3` through `.7` install the lifetime owners defined here; no
compatibility storage path is available during that interval.

The `.3` state core now owns one bounded append-only interning epoch, one
coarse generation bundle, direct contiguous and page/index dense current-value
banks, generation-typed definition/token/glue coordinates, and one exact
ordered TeX save/operation-undo journal. Code-table INITEX defaults are virtual
values of page/index dense banks rather than persistent roots. Journal cursors
are generation-branded and dynamically owner-checked; interning is outside
their rollback domain. Node/page arenas, attempt storage, cold detachment, and
incremental generation retention remain the explicit `.4`--`.7` consumers of
that core, so the rewrite branch stays compiler-red without a compatibility
owner between those stages.

This document defines the ownership and lifetime model for Umber's live TeX
runtime. It is the authority when another architecture document discusses a
different runtime storage lifetime. It does not define a wire format or a
host-resource policy.

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
  `-- engine session and its interning epoch
        `-- incremental revision generations
              +-- dense current-value banks and TeX save journal
              +-- immutable DefinitionArena
              +-- durable node and source storage
              `-- execution episode
                    +-- AttemptArena and scanner scratch
                    `-- mode/page arenas
```

An owner may keep a coarser owner alive, but an individual stored value never
owns itself. Runtime values use compact, copyable ids or offsets. Strong
ownership exists only at the session, generation, arena, page, checkpoint, or
detached-artifact level. There is no per-value `Arc` ownership.

The following matrix is normative:

| Value or storage                                                        | Immediate owner                | Valid until                                                     | Rollback behavior                                                               | Escape path                                           |
| ----------------------------------------------------------------------- | ------------------------------ | --------------------------------------------------------------- | ------------------------------------------------------------------------------- | ----------------------------------------------------- |
| Interned control-sequence name and token spelling                       | Session interning epoch        | Session epoch retirement                                        | Never rolled back                                                               | Detached spelling or semantic atom                    |
| Current meaning, parameter, register, or code value                     | Dense current-value bank       | Overwritten or bank retirement                                  | Exact undo through the TeX save journal                                         | Packed value in a checkpoint or DTO                   |
| Immutable macro definition and its definition token lists               | Revision `DefinitionArena`     | Owning generation retirement                                    | Candidate suffix truncation before publication; published rows remain immutable | Explicit copy into a destination generation           |
| Scanner buffer, macro argument, expansion scratch, or speculative value | `AttemptArena`                 | Operation completion, rollback, or continuation disposal        | Whole arena or suffix discard                                                   | Typed commit promotion                                |
| Pending mode material and page-builder nodes                            | Mode/page arena                | Mode close, rollback, or shipout                                | Arena suffix truncation after roots are restored                                | Promotion to durable node storage or shipout lowering |
| Box-register or checkpoint-surviving node                               | Durable generation arena       | Generation retirement                                           | Generation ownership restores before abandoned storage drops                    | Cold whole-generation copy or detached output         |
| Source registration and compact provenance record                       | Session or revision generation | Last owning generation, live input, or output recipe retirement | Cursor restoration and suffix discard                                           | Handle-free source recipe                             |
| Structural diagnostic or rendered-source presentation                   | Diagnostic or artifact DTO     | DTO disposal                                                    | Not live runtime state                                                          | Already detached and handle-free                      |
| Shipped page                                                            | `tex-out` value                | Output disposal                                                 | Outside engine rollback after publication                                       | Serialized artifact bytes or output DTO               |

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

Every incremental revision generation owns an append-only
`DefinitionArena`. The arena stores complete immutable macro definitions and
the token lists which constitute their parameter and replacement text.
Definition token lists use definition-arena spans; they are not independently
owned objects.

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

Rust branding does not keep bytes alive, decide when a generation may retire,
or prove that a serialized integer belongs to an arena. Those are storage
invariants: the generation owner retains every arena named by its dense state,
journal, stacks, and checkpoints; rows never move or mutate while that
generation is live; dynamic or type-erased admission validates the generation
key and bounds once; and retirement occurs only after every owning revision,
checkpoint, or continuation has released the generation. A raw copied id in an
unowned local is never lifetime authority.

Other immutable values which outlive an attempt, including token-register
lists and glue specifications, use their own typed append-only generation
arenas with the same private-id, admission, and coarse-ownership rules. They do
not share the `DefinitionId` namespace, and definition-text spans remain
physically owned by `DefinitionArena`.

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
or resource request stores the generation owner plus ids and integer cursors:

```rust
struct MacroCursor<G> {
    definition: DefinitionId<G>,
    replacement_offset: u32,
    argument_record: AttemptOffset,
}

struct PendingResource<G> {
    generation: GenerationOwner<G>,
    attempt: AttemptArena<G>,
    resume: ResumePoint<G>,
    request: ResourceRequest,
}
```

Resume re-borrows the generation, resolves the id again by direct indexing,
and continues at the stored cursor. The continuation never contains a Rust
reference into an arena and no runtime type is self-referential.

## Attempts, scratch, and commit promotion

An execution operation owns an `AttemptArena`. Scanner buffers, macro
arguments, expansion scratch, temporary token lists, builders, speculative
values, and cold-handler operands allocate there. Small scalar scratch may
remain inline, but it has the same operation lifetime.

An attempt mark is a tuple of arena lengths and subordinate builder cursors.
Failure truncates to the mark; rejecting the operation drops the whole arena.
Capacity may be retained by an operation-local pool after every value has been
logically discarded. Rollback performs no object-by-object liveness discovery.

A resource-pending continuation may take ownership of an entire
`AttemptArena`. All links within it are typed offsets or ranges. The
continuation also owns the relevant generation at coarse granularity, so every
generation-scoped id in the arena remains admissible. Resumption moves the
arena back into the operation. Cancellation drops it wholesale.

Only explicit commit promotion lets an attempt value escape. The destination
is selected by the semantic role:

- a macro and its definition text are copied into the current
  `DefinitionArena`;
- a token-register list or glue value is copied into its typed durable value
  arena;
- page material is copied into the applicable mode/page arena;
- a box-register or checkpoint survivor is copied into the durable generation
  arena;
- source evidence is copied into generation source/provenance storage; and
- an effect, memo, or output value is lowered to a handle-free DTO.

Promotion begins from the operation's explicit typed escape roots. It copies
only those values and follows only schema-declared child fields. A temporary
dense relocation vector, indexed by attempt-local id, records each destination
id and rewrites child handles as rows are copied:

```rust
fn promote_nodes<G>(
    roots: &[ScratchNodeId],
    from: &AttemptArena<G>,
    into: &mut DurableNodeArena<G>,
    relocation: &mut Vec<Option<NodeId<G>>>,
) -> Vec<NodeId<G>>;
```

There is no liveness graph search, content hash, exact-candidate lookup, binary
lookup, or attempt-wide scan. The relocation vector is temporary scratch and
is cleared or dropped when publication completes. Destination rows become
visible only after every copied child has validated and every root has been
rewritten; failure leaves the destination unpublished.

Direct-operation completion chooses an explicit disposition for its attempt
suffix. After a fully applied operation has promoted and installed every
declared escape root, command state recomputes one private per-table high-water
cursor from its live input, macro, alignment, builder, and scanner-continuation
coordinates. It then discards only the suffix beyond that cursor; a macro
argument created during delivery therefore survives until its activation and
parameter levels retire. A resource-unavailable operation instead moves its
opening mark together with the scanned operation into the typed retry
continuation and performs no reclamation. Rollback restores semantic roots
before recomputing the same live cursor. Named command timeline rows contain
only the canonical empty attempt mark; a named checkpoint performs one final
reclamation before enforcing that durable-only invariant.

## Node lifetimes

Nodes have three storage lifetimes:

1. Operation scratch contains unfinished ligature/shaping work, temporary
   transformed lists, packing probes, and speculative nodes. These values die
   with the attempt unless explicitly promoted.
2. A mode/page arena owns the material of open horizontal, vertical, math,
   insertion, alignment, and page-builder lists. Save and operation marks name
   arena cursors, so rollback restores roots and truncates only the changed
   suffix.
3. A durable generation arena owns node lists which survive their originating
   mode/page lifetime, including box-register values and checkpoint roots.
   Promotion copies the exact escaping closure and rewrites handles through a
   dense relocation vector.

A box moved from a register into current mode storage changes its coarse owner
or is copied when the source lifetime would otherwise end. A box copied by TeX
may share a whole immutable durable segment through its generation owner; it
never adds a per-node or per-list reference count.

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

Attempt-local rewrites and scanners keep provenance beside their words as
offsets into attempt storage. Promotion copies only provenance required by an
escaping token, node, diagnostic, or checkpoint and rewrites its ids with the
same dense relocation method as other arena values. Provenance never keeps a
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
lengths, arena watermarks, mode/page cursors, source/effect ledger cursors, and
the identity of the generation it addresses. A named checkpoint owns the
generation set needed by those coordinates and contains the same kind of
compact marks, plus any optional coarse packed-bank snapshot. It does not clone
the live definition, node, provenance, input, or page object graph.

Marks can be created only at a boundary whose live builders either belong to a
named arena with a cursor or have been committed. A mark is not an owning
reference to each value it can restore. The checkpoint's coarse generation
owners provide lifetime; its cursors provide position.

Restore is atomic and follows this order:

1. Validate the checkpoint/session identity, generation ancestry, all journal
   and stack cursors, arena watermarks, and external ledger cursors without
   mutation.
2. Acquire or install the checkpoint's coarse generation owners so both the
   restored and abandoned ids remain resolvable during restoration.
3. Restore dense banks by loading the selected coarse packed image when one is
   present and replaying the exact journal suffix to its cursor; otherwise
   undo the live journal directly to the cursor.
4. Restore input, condition, group, mode, page, source, resource, effect, and
   output cursors, transferring canonical roots before releasing replaced
   owners.
5. Truncate attempt, provenance, input, and mode/page arena suffixes to their
   validated watermarks.
6. Release abandoned generation and page owners only after no restored cell,
   stack entry, or cursor can name their storage.

Any validation failure leaves the runtime unchanged. Reusing a physical arena
slot requires a new generation key before another id can name it.

## Incremental revisions and two-generation ownership

An incremental session has at most two live runtime generations: the prior
accepted generation and one current candidate. They are admitted under
distinct invariant generative brands. Prior admission is read-only; every
revision-local allocation and mutable root belongs to current. Candidate
execution may compare detached evidence from prior, but an accepted current
root cannot contain a prior-generation id or owner.

Rejection drops current wholesale and leaves prior unchanged. Acceptance first
validates current-generation locality without mutation, then consumes current,
promotes its coarse owner to the accepted slot, and drops the complete former
prior generation. History retains only detached semantic evidence, hashes,
schedules, and output prefixes. It never retains a live checkpoint or
generation owner.

There is no runtime compactor, relocation map, generation graph, forwarding
pointer, slab splice, row-level collector, or content-equality merge. Routine
edits do not clone the prior runtime graph. The current generation rebuilds
only the state required by execution in its own append-only arenas; explicit
format, artifact, and detached-continuation boundaries are the only cold copy
paths. Reclamation is the O(1) drop of a whole untracked generation. Obsolete
rows inside the accepted current generation therefore remain until that
generation is replaced or the session resets.

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

An in-process resource-pending continuation may retain an owned
`AttemptArena`, generation owner, and cursor as described above. Before that
continuation crosses a process/thread boundary or enters serialized session
storage, it must detach to the handle-free continuation schema. Runtime ids
never become wire identity.

`EffectJournal` is an in-session revision reconciliation package, not a cold
DTO. It owns detached-value `EffectRecord` rows together with runtime-local
publication identities, semantic ordinals, and placement sidecars whose only
meaning is within the current retained revision graph. Cold artifact and
effect consumers materialize record order and detach record payloads; they do
not serialize, memoize, or emit the journal's publication sidecars.

## Crate and module responsibilities

`tex-state` owns the session interning epoch, generation brands and owners,
dense current-value banks, exact TeX save journal, definition arenas, durable
node arenas, source/provenance arenas, opaque ids, admission, marks, and atomic
restore. It exposes borrowed views and typed mutation APIs, not backing
vectors or unchecked constructors.

`tex-command` owns raw token delivery, expansion, scanners, input stacks,
macro activations, scanner builders, command-side `AttemptArena` layouts, and
typed suspended command state. It borrows an admitted `tex-state` generation
for hot direct indexing and stores ids plus cursors whenever that borrow ends.

`tex-exec` owns operation boundaries, semantic dispatch, mode/page arenas,
node-building lifetimes, commit promotion, resource/effect barriers, shipout
detachment, and the ordering of aggregate restore. It cannot construct state
ids or publish partial promoted values.

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
An in-session continuation owns its attempt/generation package; a detached
continuation contains only portable recipes and logical cursors. Materializing
a detached continuation is an atomic destination-local rebuild through
`tex-state` admission APIs.

## Required hot-path properties

After bounded capacity warmup, ordinary source delivery, meaning lookup, macro
expansion, scanning, assignment, and node construction perform no heap
allocation attributable to lifetime management. An ordinary read requires:

- no `Weak` upgrade or lookup;
- no `Arc` retain or release;
- no generation/root registry search;
- no binary search;
- no content hash or content comparison; and
- no per-value owner construction.

Generation validation occurs once when an episode, continuation, checkpoint,
or detached value is admitted. Within that admitted borrow, ids resolve by
typed direct indexing. Coarse owners may use `Arc` at generation or immutable
segment boundaries when sharing across revisions is required, but no operation
retains such an owner per value or per read.

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
- per-value `Arc`, `Weak`, reference counts, owner markers, or drop-driven
  reachability;
- a global generation/root registry consulted by ordinary reads;
- publishing runtime ids through formats, serialized continuations, memos,
  output DTOs, process messages, or thread messages;
- promotion by retaining attempt storage, scanning it for liveness, or
  rewriting it in place;
- in-place generation or node-arena compaction; and
- provenance ownership which keeps otherwise-dead semantic values alive.
