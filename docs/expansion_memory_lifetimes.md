# Expansion memory lifetimes

Status: implementation map, migration guide, and retention audit for the core
command expansion engine.

The normative end state is [Runtime storage lifetimes](runtime_storage_lifetimes.md).
This document answers the narrower practical question: when expansion scans,
expands, suspends, resumes, rolls back, and crosses an editor revision, who owns
each byte and when can it be reclaimed?

Three labels keep implemented behavior separate from design intent:

- **Implemented now** is verified by the current source and routine gates.
- **Migration in progress** is a safe current representation which the
  normative storage rewrite still requires us to replace.
- **Target invariant** is required by `runtime_storage_lifetimes.md`; it must not
  be cited as an implemented guarantee until the named gap is closed.

The older `reachability_owned_values.md` described the deleted region/root
registry. It is available only in Git history and is not an alternative live
ownership model.

## Terms

A **session** is one long-lived editor or batch-engine instance. A **revision
generation** is the move-only lease and complete slot payload for one accepted
or candidate revision. A **durable value** is immutable data which may be named by live TeX
state, an input cursor, or a checkpoint for the rest of that generation.
**Scratch** is reusable unpublished storage whose last user can be identified
exactly. A **continuation** is the owned state needed to resume after the host
must provide a resource. **Sealing** publishes a generation-branded immutable
owner; a sealed builder cannot be appended to again.

"Chunk" always means a fixed-capacity physical backing unit inside an arena.
A chunk is not a revision, a history entry, or an independent lifetime owner.

## Lifecycle at a glance

```text
process-immutable tables and compiled semantics
  `-- caller-owned ReachabilityStore + append-only interning epoch
        `-- engine/editor session borrow
        +-- prior accepted slot lease (optional, read-only)
        `-- current candidate slot lease (exclusive)
              +-- dense TeX state + exact save journal
              +-- shared definition/token-list carriers + inline arenas
              +-- command/input/scanner state and checkpoint roots
              +-- reusable ExecutionScratch
              `-- one active operation
                    +-- ordinary Rust stack locals
                    +-- nested macro frames and scanner state
                    `-- typed suspension package, when the host is needed

accept candidate:
  validate and quiesce -> clear old prior slot -> candidate becomes prior

reject or cancel candidate:
  drop its continuations -> drop command/executor roots -> clear current slot
```

The implementation therefore has at most two live revision generations: the optional
prior accepted generation and the exclusively leased current candidate. There
is no third historical arena and no chunk-by-chunk history.

## Lifetime matrix

"May cross" below means that the value may intentionally survive the named
boundary. A heap allocation is permitted only at the owner boundary shown; it
does not authorize a smaller hidden arena.

| Lifetime and examples                                                                                 | State owner and allocation                                                                                                                                                            | Nested or reusable                                                                                                       | Exact release                                                                                                                        | May cross suspension / revision                                                               | Copy and heap rule                                                                                                                                               |
| ----------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Process/session immutable resources: compiled command semantics, profile tables, immutable catalogues | Process binary, immutable configuration, or session capability owner; usually static data, `Arc`, or an owned validated resource                                                      | Shared, not nested; reuse is expected                                                                                    | Process exit, session disposal, or last immutable owner                                                                              | Yes / yes when identity permits                                                               | Startup/cold heap ownership is allowed; hot expansion borrows it                                                                                                 |
| Interned control-sequence names and spellings                                                         | `SessionInternerEpoch` owns one append-only `Interner`                                                                                                                                | One epoch per session; entries are reused by `Symbol`                                                                    | Whole epoch retirement                                                                                                               | Yes / yes, within that session                                                                | Bounded heap growth is allowed; no rollback copy or per-revision reinterning                                                                                     |
| Prior accepted generation                                                                             | `Session::prior_generation` owns one move-only lease into its `ReachabilityStore`                                                                                                     | Not nested and not reusable as candidate scratch                                                                         | Clear its physical store slot immediately before installing the accepted candidate, or session drop                                  | N/A / exactly one acceptance boundary                                                         | A retained checkpoint may seed the other slot atomically; definition/token payload allocations are shared and the prior remains unchanged                        |
| Current candidate generation                                                                          | `RevisionCandidate::generation` owns the other lease into that same store after execution begins                                                                                      | Exactly one session-issued candidate lease; its fixed slot is reused only after release                                  | Candidate rejection/drop clears the slot; acceptance moves the lease into the prior role                                             | Yes / becomes the new prior                                                                   | One destination-local aggregate may be prepared from an accepted checkpoint; no relocation, rehome, wire round trip, or cross-generation id rewrite              |
| Dense meanings, registers, parameters, and TeX save records                                           | `DenseState` and `SaveJournal` in the current generation                                                                                                                              | TeX groups nest semantically; journal capacity is reusable high water                                                    | A cell is overwritten/restored; journal suffix truncates on rollback; capacity and current banks end with generation                 | Yes / only through a retained checkpoint in the same generation                               | Packed scalar copy is allowed; no per-group heap owner                                                                                                           |
| Macro definitions                                                                                     | Every live semantic carrier owns one private generation-branded thin non-atomic owner of a header-plus-token-tail allocation                                                          | Aliases clone explicitly; moves transfer owners                                                                          | Last eqtb, journal, input/expansion, checkpoint, PDF, continuation, or view owner drop                                               | Yes / no runtime handle crosses revision                                                      | Serial, parameter program, split, accounting, and tokens occur once in the shared allocation; aliasing, reads, moves, restore, and warmed reuse allocate nothing |
| Token lists used by toks registers, token parameters, marks, hooks, PDF records, and stored replay    | Every live semantic carrier owns one private generation-branded `Rc<[TokenWord]>` handle                                                                                              | Builders may nest and recycle chunks; sealing transfers words to one immutable shared allocation                         | Last exact semantic-owner drop                                                                                                       | Yes / no runtime handle crosses revision                                                      | Builder scratch remains arena-backed; publication may allocate the final slice; aliasing and replay allocate nothing                                             |
| Glue and provenance rows                                                                              | Compact direct-index `GlueArena` and `ProvenanceArena` vectors inside the current store slot                                                                                          | Not nested and not reused after publication                                                                              | Whole-slot retirement                                                                                                                | Yes / only after explicit handle-free detachment                                              | Small value copy and arena growth are allowed; shared heap ownership would cost more than these compact values                                                   |
| Checkpointable page nodes and shipout-derived nodes                                                   | The current generation owns `PageNodeArena` plus one physically separate `ShipoutScratchArena<G>`                                                                                     | Page roots survive only through explicit semantic carriers; scratch rows nest by scalar marks and retain warmed capacity | Rootless shipout truncates page suffix above the monotonic retained bound; terminal success/rollback resets scratch wholesale        | Page roots may cross same-generation checkpoints; scratch ids never cross suspension/revision | Shipout borrows Page/Durable graphs in place, streams token sources, and allocates only genuinely derived nodes directly in scratch; no graph copy or rehome     |
| `ExecutionScratch` macro words, argument ranges, and delimiter prefixes                               | One `CommandState::scratch`, with 64-word chunks, macro slots, and free lists                                                                                                         | Macro frames nest logically and own independent indexed chains; returned slots/chunks are reused                         | Macro input retirement calls `pop_macro_frame`; failed match/discard releases its chain; retained capacity ends with generation      | A live frame may cross in-process suspension / never a revision                               | Heap growth is allowed only when the generation reaches a new concurrent high water; no per-macro `Vec`, arena, or payload copy                                  |
| Transitional attempt/scanner storage                                                                  | One `CommandAttempt` owns `AttemptArena` vectors, scoped rows, builders, and recycled token-buffer `Vec`s                                                                             | Child scopes and scanners nest; buffers are reused after truncation                                                      | Commit/rollback truncates to the opening mark; physical capacities end with `CommandState`/generation                                | Yes, because `PendingCommandAttempt` owns the attempt / no                                    | **Migration in progress:** these heaps and scanner buffers exist now. Target hot scanners write directly to final storage or fixed scratch lanes                 |
| Scanner episodes and temporary builders                                                               | `ScannerState` stores status; call-local `ScannerEpisode` and typed builder/sink coordinates name attempt or destination storage                                                      | Nested scanner calls use child state, not a new arena                                                                    | Return, rollback, or typed pending scanner/scalar consumption                                                                        | Only fields moved into an exact typed continuation / no                                       | Ordinary scalar copies are allowed. A persistent per-scanner word `Vec` is transitional, not target architecture                                                 |
| Ordinary Rust call-stack values                                                                       | The executing Rust function owns commands, counters, enums, small cursors, and temporary borrows                                                                                      | Naturally nested calls; stack slots may be reused by Rust                                                                | Function return or unwind                                                                                                            | No, unless explicitly moved into an owned continuation / no                                   | Scalar and small fixed-value copying is allowed; an incidental call-local heap object must drop on return and cannot become a hidden mailbox                     |
| In-process suspension                                                                                 | `PendingCommandAttempt` owns a boxed attempt, one coarse `GenerationOwner`, fixed marks/resume coordinates, and a typed payload; `MainControl` owns singular pending-operation fields | Typed continuations can refer to nested command-side continuations                                                       | Successful resume consumes it and drops the extra generation owner; cancellation/drop releases the package                           | Yes / no                                                                                      | Boxing at the cold host barrier is implemented and allowed; cloning the live graph or storing untyped token mailboxes is not                                     |
| Detached continuation                                                                                 | `OwnedCommandContinuation` owns validated handle-free recipes and DTO-local indices                                                                                                   | Can encode a nested input stack; never owns a runtime generation                                                         | DTO drop                                                                                                                             | N/A / yes, because it contains no runtime ids                                                 | Cold copying and heap allocation are allowed under explicit admission budgets; materialization builds fresh destination-local values atomically                  |
| Source and input cursor storage                                                                       | `InputState::levels`, `pending_sources`, `SourceLevel`, compact `TokenPayload` coordinates, and one generation-owned segmented replay lane                                            | Input levels and replay entries nest in exact LIFO order; retired coarse segments are reusable high water                | EOF/input retirement pops a level and releases its replay suffix; unopened registered sources end when opened or command state drops | Yes / source recipes may detach, live runtime coordinates may not                             | Replay admission grows only at a new generation high water; ordinary push/pop does not allocate, relocate, compact, or copy a whole payload                      |
| Diagnostics and observations                                                                          | Command semantic queues, operation-local `DiagnosticEffects`, `ObservationBuffer`, then `World`/output effect storage                                                                 | Operation-local batches may contain nested diagnostic programs                                                           | Rollback drops the batch; commit drains/publishes it; accepted effects end with output/world or generation/session disposal          | A typed pending operation may move the sole buffer / detached effects may cross revisions     | Owned strings/vectors are allowed at diagnostic/effect boundaries; no semantic result may wait in an untyped hidden queue                                        |
| Pure memo and render caches                                                                           | Session/executor cache owner; `PureMemoRuntime` and editor render-map cache enforce entry/byte budgets                                                                                | Reusable and evictable, not semantic nesting                                                                             | Eviction, explicit clear, or session drop                                                                                            | Yes / yes only as handle-free validated results                                               | Bounded heap retention is allowed; cache identity cannot provide runtime liveness                                                                                |
| Format image and format construction                                                                  | `DetachedFormatImage` owns bytes plus decoded handle-free rows; a fresh destination generation owns materialized values                                                               | Cold staging may build several nested DTO tables                                                                         | Staging temporaries drop after publication/error; image drops with its owner; runtime rows end with destination generation           | N/A / yes, via handle-free schema                                                             | Cold copy/heap allocation and DTO-local relocation are allowed. Live ids, chunks, and generation owners are forbidden on the wire                                |

### Process and session state

The command profile and compiled semantic dispatch are configuration, not
revision history. Expansion borrows them. Host capabilities are also borrowed
for an admitted episode; they are not smuggled into tokens or definitions.

`ReachabilityStore::new` creates the session's interning epoch and one coarse
allocation containing its inline two-slot store. Its `Interner` holds an
append-only string arena,
entry vector, and hash buckets. Symbols remain stable through edits, so group
exit, operation rollback, and candidate rejection do not remove names.
Explicit budgets cap names, slots, and bytes. A symbol is an id, not an owner.

### Prior and current generations

The implemented invariant is exactly two inline slots in one external store,
not "two plus any number of candidates callers happen to retain." The prior
slot is optional and read-only. The current candidate has the only execution
lease. Acceptance first proves that scratch and typed operations are
quiescent, prunes optional checkpoint roots, clears the old prior slot, and
moves the candidate lease into the prior role. Rejection clears only the
current slot.

`RetainedEngineGeneration::fork_checkpoint` is the generic transition between
those roles. It validates an accepted `EngineCheckpoint`, prepares the complete
state/command/mode runtime and its fresh command timeline before insertion, and
then occupies the sole free store slot. A failed build or a rejected candidate
cannot alter the accepted slot. The checkpoint's definition and stored-token
carriers clone only their existing private shared handles, so their payload
bytes are not recopied; publisher scratch and mutable execution roots are
destination-local.

`Session` allocates one private `CandidateLeaseState` when the session starts.
Every `start_*_candidate` factory atomically claims that state and moves the
non-cloneable lease into `RevisionCandidate`. Preparing a completed candidate
moves the same lease into `RevisionTransaction`, so the slot cannot be reused
between preparation and atomic acceptance. Candidate or transaction rejection,
ordinary drop, failed preparation, and acceptance all release the lease
deterministically. A second factory returns `CandidateAlreadyLive` before it
can issue another candidate or construct another generation. Claiming the
existing session state performs allocation-free coarse store and candidate-
lease retains: the store uses same-thread `Rc`, while the host cancellation
lease remains independently thread-safe.

A direct `&mut Session` borrow tied to `RevisionCandidate` would prevent the
public persistent compile coordinator from storing a session and its
resource-suspended candidate side by side across host turns. Both instead
share one caller-owned `ReachabilityStore`; their lifetime marker statically
prevents escape beyond that owner without making either value
self-referential. Exported FFI sessions may instead own the one coarse store
allocation directly. The
coarse session-boundary lease and fixed store slots remain interior owners;
neither is a
per-value row, runtime registry, or ordinary-read lookup.

Runtime handles are invariantly branded by a generation. Definition and stored
token-list handles are deliberately non-`Copy`: a move transfers ownership and
an explicit clone records a true alias by changing a non-atomic `Rc` count.
The coarse store and move-only retained lease still govern generation
admission, but they no longer retain unreachable definition or token-list
payloads. No individual value owns an `Arc` or `Weak`.

### Durable rows and chunks

`TokenListArena` keeps its fixed 64-word chunks solely as reusable builder
scratch. A builder appends semantic `TokenWord`s directly to a chain. Sealing
performs the final immutable shared-slice allocation, publishes a branded
owner, and immediately returns the entire builder chain to the free list. A
discarded _unpublished_ builder follows the same recycling path without ever
publishing an owner.

`DefinitionArena` similarly publishes one immutable header-plus-token-tail
allocation and retains no body row. Its thin non-atomic owner does not repeat
the serial, parameter split, parsed parameter program, or accounting
capability: those values occur once in the shared header and final header drop
performs exact accounting release. A stored token-list view, cursor, macro
meaning, input level, continuation, journal word, checkpoint, or PDF record
owns the same allocation directly. Its retirement is therefore the ordinary
last-owner drop; no arena callback, owner registry, root search, or tracing
collector participates.

Raw and expanded command delivery is destination-directed. Each active request
owns one caller-provided `Option<CurrentCommand<G>>`; resolution constructs in
that slot and nested delivery has its own slot. The driver returns only a
compact status, and moves the command only to its final consumer or the exact
typed expansion suspension slot. The slot is neither global nor a mailbox and
never survives independently of its request.

The publisher retains only a monotonic serial for cold format coordinates, and
the token-list publisher retains warmed builder chunks and slots. Neither
retains a published payload. A dead serial becomes an empty compatibility row
in a detached format rather than a live runtime value. There is no runtime
compaction, forwarding pointer, id rewrite, or move to another generation.

Execution scratch chunks can recycle because their last semantic user is
known. The macro activation owns its frame id; its replacement input retires
only after all argument replay above it has ended; `pop_macro_frame` then
invalidates the slot serial and links the whole chain onto the free list. No
durable state or checkpoint is allowed to hold that frame afterward. The
difference is exact lifetime knowledge, not the physical chunk size.

### Macro, scanner, and operation nesting

`ParameterState::activations` is the logical macro stack. Each
`MacroActivation` names its definition and one sealed scratch `MacroFrameId`.
The frame contains fixed-capacity argument ranges into its private scratch
chain. Nested macros allocate other slots/chains from the same
`ExecutionScratch`; they do not allocate child arenas. When a macro-body input
retires, the corresponding activation is removed and its scratch chain is
returned for reuse.

Scanner control is mostly ordinary stack state: a `ScannerEpisode`, phase
enum, counters, and sink coordinates. The current `AttemptArena`, however,
still allocates `AttemptTokenBuffer { words: Vec<_> }` rows and keeps emptied
buffers in `recycled_token_buffers`. `PendingScanToks` values occupy
ABA-tagged slots in the current generation's reusable `ExecutionScratch`. A
move-only `ScannerFrameKey` is the sole root capability. Each scanner,
expansion, `\expandafter`, preflight, diagnostic, or alignment caller moves its
exact child key into a typed phase destination; resume consumes that edge
before continuing the caller. Abort follows the structural child chain
deepest-first so younger scanner episodes and attempt scopes close before
their parents. There is no global pending-scan or pending-expansion scheduler,
configuration search, or coordinate repair.

Multi-child primitives require a caller frame of their own. For example,
`\pdfstrcmp` stores whether its left or right expanded scan owns the child; the
right phase also retains the completed left attempt-list coordinate. A retry
therefore cannot accidentally deliver the right child to the syntactically
identical left scan call.

pdfTeX's file enquiries retain their requested intent in the owning expansion
frame rather than a shared command-state mailbox. The same typed destination
rule covers `\pdfmatch`, object/form/image/graphics scanners, glyph mappings,
document fragments, and navigation actions: each optional or repeated
balanced-text site names its exact phase and moves all already-scanned operands
into that phase. Success consumes the phase, another resource suspension moves
it back into the reusable scratch lane, and abort follows its child edge before
discarding the caller.

Alignment preamble collection is likewise a typed caller. Its reusable scratch
frame moves the live scanner episode, builder, attempt-buffer coordinates, and
partially collected columns across suspension. The expansion immediately after
`\span` is its `SpanExpansion` child destination. When that root reaches main
control without a separate command retry, the exact non-`Copy`
`ScannerFrameKey` moves into `PendingAlignmentDelivery`; an absent command retry
does not imply an absent alignment-scanner owner. Success consumes the child
before the preamble finishes, repeated resource suspension reinstalls the same
edge, and abort closes the child before the parent episode and attempt scope.

Direct-to-final construction means the eventual owner supplies the builder.
An unknown-length toks value, mark, hook, or other durable list appends into
its final generation chunks and seals the row only after validation. A failed
builder discards its unpublished chunks. Scratch is not "promoted," copied,
spliced, or rehomed into durable storage. The token-list destination API now
exists, but scanners still collect traced words in attempt buffers before
promotion, so the complete hot-path invariant is not yet implemented.

### Suspension, input, and delivery

An ordinary function local cannot survive suspension. Every value needed on
resume is moved into a typed pending state. For the resource path,
`PendingCommandAttempt` owns the complete attempt, a non-`Copy`
`CommandAttemptOperation` capability, a fixed resume point, the typed requested
operation, and one coarse generation owner. `MainControl` owns one singular
direct-retry slot: the same operation capability moves together with exactly
one preflight or alignment destination. A settled command discovered by
alignment dispatch installs its command/cursor destination before operand
scanning, so a resource failure cannot resume alignment past that command and
strand its scanner child. Resume consumes the owner; cancellation drops it.
This keeps exactly the current generation alive, not one owner per token.

Nested scanner and expansion suspension uses the same fixed typed scratch lane
described above. Its backing vectors grow only when a generation reaches a new
simultaneous high-water mark; freed slots are reused with a new serial, so a
stale or double-consumed key is rejected. Scalar continuations such as an
in-progress `\csname` move their accumulated name into the exact enclosing
expansion or conditional phase. An expandable
`\number` or `\romannumeral` moves its leading/radix/optional-space phase into
the owning expansion frame, beside that frame's exact scanner child, instead
of a command-state stack. None may become a general token, boxed-dynamic, or
caller-order mailbox.

The reusable allocation-free scalar-frame family now owns optional-equals,
keyword and matched-prefix restoration, integer, dimension, glue, filename,
internal-value, expression, and font-selector progress. Every nested scalar or
expansion is a non-`Copy` child edge tagged with its exact return destination.
The raw resource-capable entry points are private; the executor-facing retained
surface returns a move-only result that carries either the completed value, the
exact child capability, or a non-resource failure. An external caller therefore
cannot use a raw scalar API, and an internal structured caller cannot propagate
a suspension with `?` while silently abandoning its child.

One singular direct-operation parent owns the settled command, delivery cursor,
completed fixed-sequence operands, current phase, and scalar child. Its exact
phases cover register and box-dimension assignments; unary integer, dimension,
and glue commands; paragraph-shape and penalty arrays; fontdimen, font-integer,
font-code, font-expansion, and font-only operations; code tables and catcodes;
openout filename scanning; marks; math families; arithmetic target/keyword/
operand sequences; and leader payload, command, and glue delivery. Expansion
frames own number, internal-value, font, mark-class, margin-kern, PDF scalar,
balanced-text, and file-enquiry children. Conditional frames own already-pushed
condition identity, inversion, and exact numeric/font/box/stream operands.
Alignment preamble frames own tabskip optional-equals and glue phases beside
their scanner episode and partially built templates.

Structured scanner parents retain character/register definitions, token-list
register and parameter assignments, glue parameters, rules, packing, insert,
box, input-stream, filename, font-definition, generated-font, math, accent,
hyphenation, PDF object/form/image/graphics/navigation/action/document
fragments, immediate extension, write-stream, and expanded-write phases. A
token-list right-hand side retains its completed selector and owner before a
nested scalar or collector begins; an expanded write retains the artificial
write episode, stopper level, already-collected list, and write-word count.
Consequently a completed selector or earlier operand is never reconstructed
from the command opcode after input has advanced.

Success consumes the deepest child before its parent and moves the result into
the named destination. Resource resuspension reinstalls the same edge. Abort
walks the exact chain deepest-first, including a failure while storing a newly
allocated parent frame: the live child is closed before the unpublished parent
is discarded. Reusable frame slots carry a fresh serial, so stale ABA keys and
double consumption are rejected; warmed nesting reuses the lane's high-water
capacity without allocation. No known resource-capable scalar site uses an
inferred owner, destination search, root mailbox, per-command retry queue,
caller-order result tape, input rollback replay, or fallback redispatch.

The handle-free `OwnedCommandContinuation` schema, validation, and atomic
destination materializer are implemented, but the module still carries a
migration allowance because runtime detachment adapters are not installed.
It is therefore a real cold ownership boundary under construction, not a
claim that every live suspension can already detach across a revision.

An input level owns either a source cursor or a classified token cursor. A
source level owns its `SourceCursor`, registered backing, and optional boxed
open-depth snapshot until EOF retirement. A durable token-list input now owns
only the list id, chunk cursor, and length. Macro replacement input owns a
definition coordinate; macro-argument input owns a scratch replay cursor.
Small source-adjacent replay owns only a compact coordinate into the
generation's segmented replay lane. Its traced words and optional source
provenance are written once at admission. Popping the level releases exactly
the top entry and returns whole unused segments to reusable high-water storage;
snapshot roots share immutable active segments and never relocate live words.
The input-stack vector may keep capacity for reuse, but it must not keep the
popped source backing.

An exhausted alignment V-template is intentionally retained until TeX's
semantic `endv` transition; it is not stale merely because it has delivered
its last ordinary token. Likewise, a source registered in `pending_sources`
remains callable by its id until it is opened or the command state ends.

### Diagnostics, effects, groups, caches, and ids

Diagnostics are transactional output, not generation ownership. A
`Diagnostic` builds owned print operations in an operation-local
`DiagnosticEffects`. Rollback drops them; commit moves them into `World` and
its effect/output journal. Observation buffers follow the same single-owner
rule and may move into a typed suspension. Published effects may grow with the
document because they are externally observable output, then end with the
world/output owner. Queue capacities may remain as bounded high water.

A TeX group is a save-and-restoration boundary. The save journal records old
packed values and exact group entry/exit order. On group exit it restores local
assignments and preserves later global assignments. It does not own an arena,
and ending a group cannot reclaim a sealed token list or definition; an older
journal/checkpoint value may still name it. `aftergroup` and
`afterassignment` payloads are command roots until their specified replay
point, not group-local allocation pools.

The pure memo and render-map caches are operational, handle-free, and
explicitly byte/entry bounded. They may reuse results across revisions only
after validating their semantic stamps. Eviction drops payloads and cannot
retire runtime rows. Monotonic generation, frame, source, and input ids are
small identity scalars; an id consumes no backing memory by itself and cannot
keep an owner alive.

### Format build and load

Format capture is a cold boundary. `DetachedFormatImage` owns encoded bytes and
a decoded logical image containing names, definitions, token lists, glue,
fonts, sparse cells, and other handle-free recipes. Runtime ids and generation
owners are absent. Loading validates the complete graph, allocates fresh
destination-local rows, stamps all destination ids, and publishes the staged
runtime atomically. Failure drops staging without partially replacing the
live universe.

Cold format work may allocate and copy. It is the one place where DTO-local
index relocation is appropriate. It does not authorize live compaction or
forwarding. Current capture enumerates every interner entry and every durable
definition/token/glue row, including rows no live cell reaches; the retention
and format-size consequence is audited below.

## Worked nested example

Suppose macro `\outer` calls `\inner`. While matching `\inner`'s second
argument, a toks scanner expands an `\input`-dependent command and the host has
not supplied the file.

1. `\outer` has one live macro activation and one sealed frame in the shared
   `ExecutionScratch`. Its argument words occupy scratch chunks A and B.
2. `\inner` allocates another scratch slot and chunk C. Its ranges are nested
   semantically under `\outer`, but C belongs to the same scratch owner, not a
   child arena.
3. The toks scanner owns phase/counter state and, in the current implementation,
   two attempt token-buffer sinks. Its unfinished state moves into an
   ABA-tagged `ExecutionScratch` slot; each enclosing expansion moves that
   non-`Copy` key into its own typed caller frame.
4. `PendingCommandAttempt` takes the attempt arena, fixed opening mark, resume
   coordinates, typed resource operation, and one coarse current-generation
   owner. Ordinary Rust locals and borrows end before control returns to the
   host. Scratch chunks A--C remain where they are; nothing is copied.
5. On resume, the package is consumed and validated against the same
   generation. The scanner continues at its exact phase. In the target path it
   appends each accepted semantic word directly to a destination
   `TokenListBuilder`; today the attempt sinks are promoted afterward.
6. If scanning fails, its unpublished destination chunks are discarded and
   may be reused. If it succeeds, sealing publishes a durable row and every
   chunk in that row stays until generation retirement.
7. `\inner` finishes first, so C is linked to the scratch free list. `\outer`
   may reuse C later while A and B remain live. When `\outer`'s replacement
   input retires, A and B are released. The durable toks list is unaffected.

No step creates a scanner arena, promotes a scratch chunk, copies an arena
owner into each frame, or treats A, B, or C as revision history.

## Retention audit

This is a source audit, not an RSS measurement. **Fact** means the ownership
and release path are directly visible in the cited source. **Impact hypothesis**
means the topology is verified but workload size or frequency still needs a
benchmark, heap profile, or counter. The category numbers are:

1. intentional coarse-owner retention until wholesale retirement;
2. bounded reusable high-water capacity;
3. stale or unreachable payload still retained by a live owner; and
4. true unbounded/per-operation growth or an API which permits it.

| Category and confidence                                         | Owner/type and source                                                                                                                                       | Retention trigger and release today                                                                                                                                                                                                                                                                                                              | Model verdict and likely impact                                                                                                                                                                                                                                       | Principled correction                                                                                                                                                                                                      |
| --------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Resolved, fact                                                  | `DefinitionId<G>` and `TokenListId<G>` in the retained state/command/executor carrier graph                                                                 | Private generation-branded non-atomic shared owners are moved between carriers or explicitly cloned for aliases; last-owner drop releases the immutable payload                                                                                                                                                                                  | Overwritten, rollback-abandoned, pruned, rejected, retired-input/expansion, PDF, continuation, and generation values no longer remain merely because their publisher is live                                                                                          | Preserve exact carrier cloning/moves and private `Rc`; do not add `Arc`, `Weak`, atomics, owner registries, tracing, compaction, relocation, rehome, or a third generation                                                 |
| 1, fact                                                         | `SessionInternerEpoch`/`Interner` in `crates/tex-state/src/{session_epoch,interner}.rs`                                                                     | First intern retains spelling/hash entry through all revisions; epoch retirement releases it                                                                                                                                                                                                                                                     | Intended session stability, bounded by explicit interner budgets; stale spellings can remain but are not a revision-generation leak                                                                                                                                   | Preserve one bounded epoch; detach spellings at cold boundaries rather than cloning interners                                                                                                                              |
| 1, fact                                                         | Accepted `EffectJournal`, `World` effects, source fragments protected by retained checkpoints                                                               | Semantic publication or checkpoint reachability keeps output/source evidence until output disposal, checkpoint pruning, generation retirement, or session drop                                                                                                                                                                                   | Intended externally observable/document-linear retention. Source fragment bytes are pruned when no accepted layout or retained checkpoint needs them; retired metadata has a budget                                                                                   | Keep exact roots and budgets; measure output retention separately from expansion scratch                                                                                                                                   |
| 2, fact                                                         | `ExecutionScratch` macro slots, chunk vector, `macro_free_slots`, and chunk free list in `crates/tex-command/src/execution_scratch.rs:238`                  | A deeper/larger set of simultaneously live macro arguments grows storage; frame return recycles logical chunks but vector capacity ends with generation                                                                                                                                                                                          | Correct reusable generation high water, bounded by maximum simultaneous live scratch demand. No per-call leak was found                                                                                                                                               | Preserve the single scratch owner; expose counters/budgets if adversarial nesting needs a hard ceiling                                                                                                                     |
| 2, fact; target representation violation                        | `AttemptArena::recycled_token_buffers` and its other vectors in `crates/tex-command/src/attempt.rs:458`                                                     | Scanner/operation rows truncate on commit or rollback; emptied word vectors and arena capacities remain until command/generation drop                                                                                                                                                                                                            | Payload is reusable high water, usually bounded by maximum nested scanner count and largest warmed buffers. The per-scanner `Vec` pool violates the target even though it is not a leak                                                                               | Migrate scanners to fixed scratch lanes and destination-provided sealed builders, then delete attempt scopes/buffer recycling                                                                                              |
| 2, fact                                                         | Popped input/pending/diagnostic vectors in `crates/tex-command/src/{input/mod,state}.rs` and `DiagnosticEffects` in `crates/tex-state/src/diagnostic.rs:67` | Pop/drain removes payload but retains vector allocation; state/generation or operation owner drop releases capacity                                                                                                                                                                                                                              | Reusable stack/queue high water. Live pending entries are future-relevant; no stale source payload after an ordinary pop was found                                                                                                                                    | Keep typed stacks where needed, shrink only at a cold quiescent boundary; replace hidden pending scanner/expansion mailboxes with explicit continuation fields                                                             |
| 2, fact                                                         | `SaveJournal.entries` in `crates/tex-state/src/journal.rs:130`                                                                                              | Rollback truncates the suffix, retaining vector capacity until generation drop                                                                                                                                                                                                                                                                   | Correct high-water reuse; logical journal length follows exact TeX history, physical capacity follows deepest mutation history                                                                                                                                        | Preserve cursor rollback; budget or report capacity instead of shrinking hot storage                                                                                                                                       |
| Resolved, fact                                                  | `PageNodeArena`, `ShipoutScratchArena<G>`, and typed shipout sources in `tex-state`/`tex-exec`                                                              | Checkpoints with explicit page carriers monotonically raise one generation bound; rootless checkpoints add nothing. Shipout-only rows reuse stable scratch buffers and are unreachable from semantic carriers                                                                                                                                    | Rootless `ShipoutComplete` releases incidental page suffixes above the retained bound; repeated shipouts retain bounded scratch high water, and generation replacement drops all page/scratch storage                                                                 | Preserve typed Page/Durable borrowing, direct scratch construction, source-token streaming, O(1) whole-suffix reset, and the compile-fail boundary; do not add copying, promotion, a root registry, or per-row reclamation |
| Resolved for definitions/token lists; glue/provenance unchanged | Superseded or rollback-abandoned durable immutable values                                                                                                   | Definition/token-list carriers own private shared payloads and release them on last drop; tiny glue and provenance retain their cheaper existing direct-index generation storage                                                                                                                                                                 | The append-only payload leak is closed for aliasable macro bodies and stored token lists. Node policy remains in the separate node-lifetime issue                                                                                                                     | Preserve shared ownership only for aliasable immutable payloads; keep scratch arena-backed and keep node graph-copy/transfer policy out of this change                                                                     |
| 2, fact; resolved                                               | `CommandGenerationOwner` in `crates/tex-command/src/snapshot.rs` and `RetainedCheckpointSlots` in `crates/tex-exec/src/retained_generation.rs`              | Each snapshot owner directly retains its aggregate `CommandStateRoots` and exact attempt mark. Pruning drops the complete checkpoint and therefore its root owner immediately; the command timeline retains only a monotonic identity serial. Executor checkpoint slots and their exact live-index backreferences are reused with a fresh serial | The former append-only timeline leak is closed. Logical retention is proportional to live checkpoints, while physical slot/live/free vectors remain bounded reusable high water proportional to maximum simultaneous retention; stale keys cannot alias a reused slot | Preserve direct owner release, generation-plus-serial validation, and O(live) pruning. Do not scan full slot capacity, relocate a live row, compact coordinates, or make the timeline a second root owner                  |
| Resolved for payload retention                                  | Format capture in `crates/tex-state/src/format.rs` and `StateCore::capture_format_values`                                                                   | Cold capture visits semantic state, node recipes, and PDF-only token/node roots; live payloads serialize at stable serial coordinates and dead serials become empty compatibility rows                                                                                                                                                           | Detached images no longer keep stale definition/token bytes, and materialization does not publish dead definitions. Empty coordinate holes preserve schema references without relocating live handles                                                                 | Preserve the cold handle-free traversal and PDF root coverage; do not turn it into runtime liveness authority or live-handle relocation                                                                                    |
| 2, fact; impact hypothesis                                      | `DetachedFormatImage { bytes, decoded }` in `crates/tex-state/src/format.rs:215` plus destination staging                                                   | A validated image keeps both compressed/encoded bytes and decoded rows; materialization also builds destination rows until caller drops the image                                                                                                                                                                                                | Expected cold double/triple residence, bounded by format admission limits, not a live-memory violation. Peak RSS needs measurement                                                                                                                                    | Offer consuming materialization or release decoded sections incrementally after validation only if profiling justifies the complexity                                                                                      |
| 1, fact                                                         | `PendingCommandAttempt` in `crates/tex-command/src/attempt.rs:1515` and executor pending slots                                                              | Suspension retains one boxed attempt, typed payload, and coarse generation owner; resume consumes it, cancellation/drop releases it                                                                                                                                                                                                              | Intentional exact continuation retention. It can delay retirement of precisely one candidate generation but is not a leak                                                                                                                                             | Keep move-only typed continuations; guarantee cancellation drops them before candidate retirement                                                                                                                          |
| Resolved, fact                                                  | `CandidateLeaseState`/`CandidateLease` and public candidate creation in `crates/tex-incr/src/{candidate_lease,lib}.rs`                                      | The session owns one atomic slot; the move-only lease transfers from candidate to transaction and releases on acceptance, explicit rejection, failure, or drop                                                                                                                                                                                   | Exactly one current slot can coexist with the optional prior. A second factory fails before issuing a candidate or generation; repeated claims reuse the one session allocation                                                                                       | Implemented. Keep lifecycle, 8,192-cycle reuse, prior-plus-current high-water, and second-factory rejection controls active                                                                                                |
| 1/2, fact                                                       | `PureMemoRuntime` and `RenderMapCache` in `crates/tex-state/src/pure_memo.rs` and `crates/tex-incr/src/lib.rs`                                              | Validated results remain until byte/entry eviction, explicit clear, or session drop; container capacity may remain after clear                                                                                                                                                                                                                   | Intentional bounded cache retention, not semantic ownership and not a generation leak                                                                                                                                                                                 | Preserve hard budgets and handle-free entries; make retained-byte metrics include container overhead where useful                                                                                                          |
| No defect found, fact                                           | Input/source stack and registrations in `crates/tex-command/src/input`                                                                                      | Source EOF pops and drops the level; opening a pending source removes it from the map; never-opened registrations end with command state. V-template and `every_eof` retention follow TeX semantics                                                                                                                                              | No stale popped source owner or per-line leak was found. Input vector capacity is category 2; pending registrations are still callable                                                                                                                                | Keep exact retirement classifications and add a counter/profile before changing source retention                                                                                                                           |
| No defect found, fact                                           | Semantic diagnostics, operation observations, and pending resource continuation                                                                             | Commit drains or moves the sole owner; rollback/drop discards it; suspension retains exact typed state                                                                                                                                                                                                                                           | No duplicated observation owner or unbounded per-operation diagnostic leak was found. Published effects can grow with real output                                                                                                                                     | Keep transactional delivery and type each pending field; measure effect-ledger growth as output, not scratch                                                                                                               |

The external reachability-store prerequisite, append-only command-timeline
inconsistency, and aliasable immutable payload retention are resolved.
Prior/current share one physical owner domain; checkpoint pruning drops its
direct command-root owner; definition and token-list payloads follow their
exact semantic carriers; and cold format capture includes state, node, and PDF
roots without retaining dead payload bytes. No compactor, forwarding scheme,
tracing collector, ownership registry, relocation, rehome, or third generation
is part of the correction.

Exact TeX main-memory totals follow those same lifecycle facts. Definition and
stored-token publication adds one precomputed canonical charge, their real last
shared owner subtracts it, and node arenas add or subtract row charges at
publication and release. The single generation-local aggregate stores no ids
or roots. Ordinary box and page work reads it directly; it no longer scans
meanings, the 65,536 token and box registers, payload identities, or node
closures merely to discover the current total.

## Non-negotiable prohibitions

- No compactor, in-place relocation, forwarding pointer, live-id rewrite, or
  cross-generation rehome.
- No promotion or copying from scratch into durable hot storage; the final
  owner provides the builder.
- No per-macro, per-scanner, per-operation, or per-TeX-group arena.
- No untyped or hidden `Vec` mailbox for suspended semantic payload.
- No historical runtime generation beyond the optional prior and exclusive
  current candidate.
- No treating a backing chunk as a revision, checkpoint, independently
  reclaimable history segment, or lifetime owner.
- No stored Rust reference, per-value `Arc`/`Weak`, atomic ownership, or
  liveness lookup on the ordinary expansion path. Private non-atomic shared
  handles are required for aliasable immutable definitions and token lists.
- No heap allocation after bounded hot-path warmup except at an explicitly
  cold host, diagnostic, format, or output boundary.
- No runtime id in a format, detached continuation, memo wire value, output,
  process message, or thread message.

These restrictions leave two deliberate reclamation policies: exact arena
reuse for scratch, and last-semantic-owner drop for published immutable macro
bodies and stored token lists. Coarse generation retirement remains the final
boundary for compact inline arenas and publisher scratch capacity.
