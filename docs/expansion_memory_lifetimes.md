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
must provide a resource. **Sealing** publishes an immutable row coordinate; a
sealed builder cannot be appended to again.

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
              +-- sealed definitions, token lists, glue, and provenance
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

| Lifetime and examples                                                                                 | State owner and allocation                                                                                                                                                            | Nested or reusable                                                                                    | Exact release                                                                                                                   | May cross suspension / revision                                                           | Copy and heap rule                                                                                                                                                |
| ----------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Process/session immutable resources: compiled command semantics, profile tables, immutable catalogues | Process binary, immutable configuration, or session capability owner; usually static data, `Arc`, or an owned validated resource                                                      | Shared, not nested; reuse is expected                                                                 | Process exit, session disposal, or last immutable owner                                                                         | Yes / yes when identity permits                                                           | Startup/cold heap ownership is allowed; hot expansion borrows it                                                                                                  |
| Interned control-sequence names and spellings                                                         | `SessionInternerEpoch` owns one append-only `Interner`                                                                                                                                | One epoch per session; entries are reused by `Symbol`                                                 | Whole epoch retirement                                                                                                          | Yes / yes, within that session                                                            | Bounded heap growth is allowed; no rollback copy or per-revision reinterning                                                                                      |
| Prior accepted generation                                                                             | `Session::prior_generation` owns one move-only lease into its `ReachabilityStore`                                                                                                     | Not nested and not reusable as candidate scratch                                                      | Clear its physical store slot immediately before installing the accepted candidate, or session drop                             | N/A / exactly one acceptance boundary                                                     | Whole-slot retention is currently allowed; rows are not copied into the candidate                                                                                 |
| Current candidate generation                                                                          | `RevisionCandidate::generation` owns the other lease into that same store after execution begins                                                                                      | Exactly one session-issued candidate lease; its fixed slot is reused only after release               | Candidate rejection/drop clears the slot; acceptance moves the lease into the prior role                                        | Yes / becomes the new prior                                                               | Slot arenas are allowed; no live relocation, rehome, or cross-generation id copy                                                                                  |
| Dense meanings, registers, parameters, and TeX save records                                           | `DenseState` and `SaveJournal` in the current generation                                                                                                                              | TeX groups nest semantically; journal capacity is reusable high water                                 | A cell is overwritten/restored; journal suffix truncates on rollback; capacity and current banks end with generation            | Yes / only through a retained checkpoint in the same generation                           | Packed scalar copy is allowed; no per-group heap owner                                                                                                            |
| Macro definitions                                                                                     | The external store's current slot contains `DefinitionArena` rows plus a contiguous token-word vector                                                                                 | Rows are not nested or reused after publication                                                       | Whole-slot retirement today; direct row release is the next migration                                                           | Yes / no runtime id crosses revision                                                      | Current final allocation copies/streams words into the arena; target hot construction is destination-directed and has no temporary semantic-word vector           |
| Token lists used by toks registers, token parameters, marks, hooks, and stored replay                 | The current store slot contains sealed rows plus 64-word `TokenChunk`s                                                                                                                | Builders may nest; discarded unpublished chains and builder slots are reusable; sealed chains are not | Discard recycles an unpublished chain; sealed chains currently end with the slot                                                | Yes / no runtime id crosses revision                                                      | Direct builder append is implemented. The cold slice wrapper may read an existing slice; migrated hot scanners must append directly without an intermediate `Vec` |
| Glue and provenance rows                                                                              | Append-only `GlueArena` and `ProvenanceArena` vectors inside the current store slot                                                                                                   | Not nested and not reused after publication                                                           | Whole-slot retirement today; direct row release is the next migration                                                           | Yes / only after explicit handle-free detachment                                          | Small value copy and arena growth are allowed; no per-value heap owner                                                                                            |
| `ExecutionScratch` macro words, argument ranges, and delimiter prefixes                               | One `CommandState::scratch`, with 64-word chunks, macro slots, and free lists                                                                                                         | Macro frames nest logically and own independent indexed chains; returned slots/chunks are reused      | Macro input retirement calls `pop_macro_frame`; failed match/discard releases its chain; retained capacity ends with generation | A live frame may cross in-process suspension / never a revision                           | Heap growth is allowed only when the generation reaches a new concurrent high water; no per-macro `Vec`, arena, or payload copy                                   |
| Transitional attempt/scanner storage                                                                  | One `CommandAttempt` owns `AttemptArena` vectors, scoped rows, builders, and recycled token-buffer `Vec`s                                                                             | Child scopes and scanners nest; buffers are reused after truncation                                   | Commit/rollback truncates to the opening mark; physical capacities end with `CommandState`/generation                           | Yes, because `PendingCommandAttempt` owns the attempt / no                                | **Migration in progress:** these heaps and scanner buffers exist now. Target hot scanners write directly to final storage or fixed scratch lanes                  |
| Scanner episodes and temporary builders                                                               | `ScannerState` stores status; call-local `ScannerEpisode` and typed builder/sink coordinates name attempt or destination storage                                                      | Nested scanner calls use child state, not a new arena                                                 | Return, rollback, or typed pending scanner consumption                                                                          | Only fields moved into `PendingScanToks` / no                                             | Ordinary scalar copies are allowed. A persistent per-scanner word `Vec` is transitional, not target architecture                                                  |
| Ordinary Rust call-stack values                                                                       | The executing Rust function owns commands, counters, enums, small cursors, and temporary borrows                                                                                      | Naturally nested calls; stack slots may be reused by Rust                                             | Function return or unwind                                                                                                       | No, unless explicitly moved into an owned continuation / no                               | Scalar and small fixed-value copying is allowed; an incidental call-local heap object must drop on return and cannot become a hidden mailbox                      |
| In-process suspension                                                                                 | `PendingCommandAttempt` owns a boxed attempt, one coarse `GenerationOwner`, fixed marks/resume coordinates, and a typed payload; `MainControl` owns singular pending-operation fields | Typed continuations can refer to nested command-side continuations                                    | Successful resume consumes it and drops the extra generation owner; cancellation/drop releases the package                      | Yes / no                                                                                  | Boxing at the cold host barrier is implemented and allowed; cloning the live graph or storing untyped token mailboxes is not                                      |
| Detached continuation                                                                                 | `OwnedCommandContinuation` owns validated handle-free recipes and DTO-local indices                                                                                                   | Can encode a nested input stack; never owns a runtime generation                                      | DTO drop                                                                                                                        | N/A / yes, because it contains no runtime ids                                             | Cold copying and heap allocation are allowed under explicit admission budgets; materialization builds fresh destination-local values atomically                   |
| Source and input cursor storage                                                                       | `InputState::levels`, `pending_sources`, `SourceLevel`, and `TokenPayload`; source bytes are owned through registered source backing, token chunks, or durable/scratch coordinates    | Input levels nest as a stack; vector capacity is reusable                                             | EOF/input retirement pops a level and drops its payload; an unopened registered source ends when opened or command state drops  | Yes / source recipes may detach, live runtime coordinates may not                         | Source registration and cold packed replay may allocate. Durable-list and macro replay use cursors and do not copy their source words                             |
| Diagnostics and observations                                                                          | Command semantic queues, operation-local `DiagnosticEffects`, `ObservationBuffer`, then `World`/output effect storage                                                                 | Operation-local batches may contain nested diagnostic programs                                        | Rollback drops the batch; commit drains/publishes it; accepted effects end with output/world or generation/session disposal     | A typed pending operation may move the sole buffer / detached effects may cross revisions | Owned strings/vectors are allowed at diagnostic/effect boundaries; no semantic result may wait in an untyped hidden queue                                         |
| Pure memo and render caches                                                                           | Session/executor cache owner; `PureMemoRuntime` and editor render-map cache enforce entry/byte budgets                                                                                | Reusable and evictable, not semantic nesting                                                          | Eviction, explicit clear, or session drop                                                                                       | Yes / yes only as handle-free validated results                                           | Bounded heap retention is allowed; cache identity cannot provide runtime liveness                                                                                 |
| Format image and format construction                                                                  | `DetachedFormatImage` owns bytes plus decoded handle-free rows; a fresh destination generation owns materialized values                                                               | Cold staging may build several nested DTO tables                                                      | Staging temporaries drop after publication/error; image drops with its owner; runtime rows end with destination generation      | N/A / yes, via handle-free schema                                                         | Cold copy/heap allocation and DTO-local relocation are allowed. Live ids, chunks, and generation owners are forbidden on the wire                                 |

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

`Session` allocates one private `CandidateLeaseState` when the session starts.
Every `start_*_candidate` factory atomically claims that state and moves the
non-cloneable lease into `RevisionCandidate`. Preparing a completed candidate
moves the same lease into `RevisionTransaction`, so the slot cannot be reused
between preparation and atomic acceptance. Candidate or transaction rejection,
ordinary drop, failed preparation, and acceptance all release the lease
deterministically. A second factory returns `CandidateAlreadyLive` before it
can issue another candidate or construct another generation. Claiming the
existing session state performs allocation-free coarse store and candidate-
lease `Arc` retains.

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

Runtime ids are invariantly branded by a generation. Copying an id is cheap
and allowed inside an admitted generation; it never extends lifetime. The
coarse store keeps the physical slot alive, while the move-only retained lease
is statically bound to its ordinary Rust API owner and releases the slot. No
individual
token list, definition, chunk, input frame, or group owns an `Arc`.

### Durable rows and chunks

`TokenListArena` is the first implemented direct-to-final durable builder. A
builder appends semantic `TokenWord`s directly to a chain of fixed 64-word
chunks. Sealing appends one small row containing a head index and length.
Replay retains a branded chunk cursor and crosses a chunk boundary without
materializing a flat list. A discarded _unpublished_ builder returns its chain
to the arena free list.

A sealed list is different. Toks registers, token parameters, TeX marks,
stored hooks, input cursors, journal entries, page state, or checkpoints may
still hold its compact id. The arena does not maintain a reachability count.
Consequently every chunk in that sealed list's chain currently stays present
until its store slot is cleared, even if the current register has since been overwritten.
Recycling one such chunk would silently change any cursor or older restored
value that still addresses it.

This is also the current rule for any definition representation composed from
fixed backing chunks: all chunks remain in the same store slot. Chunks do not
have independent history or retirement yet. The current `DefinitionArena`
contains definition rows and one contiguous parameter/replacement word vector,
separate from `TokenListArena`. The document therefore does not claim that
current macro definition text is chunked. Direct non-`Copy` row owners and
row-level release are the immediate next implementation step.

The arena's `Vec<TokenChunk>` may reallocate its Rust backing allocation while
no admitted view is live; stored cursors contain indices, not pointers. There
is no runtime compaction, forwarding pointer, id rewrite, or move to another
generation. "Fixed chunk" means fixed capacity and stable logical index, not a
promise of a permanently pinned machine address.

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
in-progress `\csname` remain in their dedicated typed state. None may become a
general token, boxed-dynamic, or caller-order mailbox.

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
Small source-adjacent replay can own a packed chunk. Popping the level drops
the payload. The input-stack vector may keep capacity for reuse, but it must
not keep the popped source backing.

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

| Category and confidence                  | Owner/type and source                                                                                                                                       | Retention trigger and release today                                                                                                                                                                                                                                                                                                              | Model verdict and likely impact                                                                                                                                                                                                                                       | Principled correction                                                                                                                                                                                     |
| ---------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1, fact; migration substrate implemented | `ReachabilityStore` and its slot-local `DefinitionArena`, sealed `TokenListArena` rows/chunks, `GlueArena`, and `ProvenanceArena`                           | The external store physically owns both prior/current payloads; publishing a row still retains it until its slot is cleared                                                                                                                                                                                                                      | The sibling-owner defect is removed and every semantic root is below one session store. Slot-local append-only rows still over-retain unreachable values                                                                                                              | Next migrate body rows/chunks into store-level storage and install direct non-`Copy` row owners; retain fixed slot admission and direct final builders                                                    |
| 1, fact                                  | `SessionInternerEpoch`/`Interner` in `crates/tex-state/src/{session_epoch,interner}.rs`                                                                     | First intern retains spelling/hash entry through all revisions; epoch retirement releases it                                                                                                                                                                                                                                                     | Intended session stability, bounded by explicit interner budgets; stale spellings can remain but are not a revision-generation leak                                                                                                                                   | Preserve one bounded epoch; detach spellings at cold boundaries rather than cloning interners                                                                                                             |
| 1, fact                                  | Accepted `EffectJournal`, `World` effects, source fragments protected by retained checkpoints                                                               | Semantic publication or checkpoint reachability keeps output/source evidence until output disposal, checkpoint pruning, generation retirement, or session drop                                                                                                                                                                                   | Intended externally observable/document-linear retention. Source fragment bytes are pruned when no accepted layout or retained checkpoint needs them; retired metadata has a budget                                                                                   | Keep exact roots and budgets; measure output retention separately from expansion scratch                                                                                                                  |
| 2, fact                                  | `ExecutionScratch` macro slots, chunk vector, `macro_free_slots`, and chunk free list in `crates/tex-command/src/execution_scratch.rs:238`                  | A deeper/larger set of simultaneously live macro arguments grows storage; frame return recycles logical chunks but vector capacity ends with generation                                                                                                                                                                                          | Correct reusable generation high water, bounded by maximum simultaneous live scratch demand. No per-call leak was found                                                                                                                                               | Preserve the single scratch owner; expose counters/budgets if adversarial nesting needs a hard ceiling                                                                                                    |
| 2, fact; target representation violation | `AttemptArena::recycled_token_buffers` and its other vectors in `crates/tex-command/src/attempt.rs:458`                                                     | Scanner/operation rows truncate on commit or rollback; emptied word vectors and arena capacities remain until command/generation drop                                                                                                                                                                                                            | Payload is reusable high water, usually bounded by maximum nested scanner count and largest warmed buffers. The per-scanner `Vec` pool violates the target even though it is not a leak                                                                               | Migrate scanners to fixed scratch lanes and destination-provided sealed builders, then delete attempt scopes/buffer recycling                                                                             |
| 2, fact                                  | Popped input/pending/diagnostic vectors in `crates/tex-command/src/{input/mod,state}.rs` and `DiagnosticEffects` in `crates/tex-state/src/diagnostic.rs:67` | Pop/drain removes payload but retains vector allocation; state/generation or operation owner drop releases capacity                                                                                                                                                                                                                              | Reusable stack/queue high water. Live pending entries are future-relevant; no stale source payload after an ordinary pop was found                                                                                                                                    | Keep typed stacks where needed, shrink only at a cold quiescent boundary; replace hidden pending scanner/expansion mailboxes with explicit continuation fields                                            |
| 2, fact                                  | `SaveJournal.entries` in `crates/tex-state/src/journal.rs:130`                                                                                              | Rollback truncates the suffix, retaining vector capacity until generation drop                                                                                                                                                                                                                                                                   | Correct high-water reuse; logical journal length follows exact TeX history, physical capacity follows deepest mutation history                                                                                                                                        | Preserve cursor rollback; budget or report capacity instead of shrinking hot storage                                                                                                                      |
| 3, fact                                  | Superseded or rollback-abandoned durable definition/token/glue/provenance rows                                                                              | A durable row can be published and then lose every live state/input/checkpoint root through overwrite, group restoration, or later operation failure; the external store has no row release API yet                                                                                                                                              | The unreachable row/chunks remain until slot retirement. The owner topology is ready for row-level reclamation, but body migration is not implemented in this commit                                                                                                  | Add safe move-only durable owners and direct store row release, then destination-directed publication. Do not add tracing, registry search, per-value `Arc`/`Weak`, compaction, relocation, or rehome     |
| 2, fact; resolved                        | `CommandGenerationOwner` in `crates/tex-command/src/snapshot.rs` and `RetainedCheckpointSlots` in `crates/tex-exec/src/retained_generation.rs`              | Each snapshot owner directly retains its aggregate `CommandStateRoots` and exact attempt mark. Pruning drops the complete checkpoint and therefore its root owner immediately; the command timeline retains only a monotonic identity serial. Executor checkpoint slots and their exact live-index backreferences are reused with a fresh serial | The former append-only timeline leak is closed. Logical retention is proportional to live checkpoints, while physical slot/live/free vectors remain bounded reusable high water proportional to maximum simultaneous retention; stale keys cannot alias a reused slot | Preserve direct owner release, generation-plus-serial validation, and O(live) pruning. Do not scan full slot capacity, relocate a live row, compact coordinates, or make the timeline a second root owner |
| 3, fact; impact hypothesis               | Format capture in `crates/tex-state/src/format.rs:247` and `StateCore::capture_format_values`                                                               | Capture serializes all interner entries and all definition/token/glue rows, including unreachable append-only rows; image drop releases DTOs, but loading republishes every encoded row into the new generation                                                                                                                                  | Verified over-retention topology and potentially larger format files/load RSS. Actual INITEX dead-row ratio requires measurement                                                                                                                                      | Cold root-walk live format cells and recipes, assign DTO-local indices, and encode only the transitive reachable closure; cold relocation is allowed                                                      |
| 2, fact; impact hypothesis               | `DetachedFormatImage { bytes, decoded }` in `crates/tex-state/src/format.rs:215` plus destination staging                                                   | A validated image keeps both compressed/encoded bytes and decoded rows; materialization also builds destination rows until caller drops the image                                                                                                                                                                                                | Expected cold double/triple residence, bounded by format admission limits, not a live-memory violation. Peak RSS needs measurement                                                                                                                                    | Offer consuming materialization or release decoded sections incrementally after validation only if profiling justifies the complexity                                                                     |
| 1, fact                                  | `PendingCommandAttempt` in `crates/tex-command/src/attempt.rs:1515` and executor pending slots                                                              | Suspension retains one boxed attempt, typed payload, and coarse generation owner; resume consumes it, cancellation/drop releases it                                                                                                                                                                                                              | Intentional exact continuation retention. It can delay retirement of precisely one candidate generation but is not a leak                                                                                                                                             | Keep move-only typed continuations; guarantee cancellation drops them before candidate retirement                                                                                                         |
| Resolved, fact                           | `CandidateLeaseState`/`CandidateLease` and public candidate creation in `crates/tex-incr/src/{candidate_lease,lib}.rs`                                      | The session owns one atomic slot; the move-only lease transfers from candidate to transaction and releases on acceptance, explicit rejection, failure, or drop                                                                                                                                                                                   | Exactly one current slot can coexist with the optional prior. A second factory fails before issuing a candidate or generation; repeated claims reuse the one session allocation                                                                                       | Implemented. Keep lifecycle, 8,192-cycle reuse, prior-plus-current high-water, and second-factory rejection controls active                                                                               |
| 1/2, fact                                | `PureMemoRuntime` and `RenderMapCache` in `crates/tex-state/src/pure_memo.rs` and `crates/tex-incr/src/lib.rs`                                              | Validated results remain until byte/entry eviction, explicit clear, or session drop; container capacity may remain after clear                                                                                                                                                                                                                   | Intentional bounded cache retention, not semantic ownership and not a generation leak                                                                                                                                                                                 | Preserve hard budgets and handle-free entries; make retained-byte metrics include container overhead where useful                                                                                         |
| No defect found, fact                    | Input/source stack and registrations in `crates/tex-command/src/input`                                                                                      | Source EOF pops and drops the level; opening a pending source removes it from the map; never-opened registrations end with command state. V-template and `every_eof` retention follow TeX semantics                                                                                                                                              | No stale popped source owner or per-line leak was found. Input vector capacity is category 2; pending registrations are still callable                                                                                                                                | Keep exact retirement classifications and add a counter/profile before changing source retention                                                                                                          |
| No defect found, fact                    | Semantic diagnostics, operation observations, and pending resource continuation                                                                             | Commit drains or moves the sole owner; rollback/drop discards it; suspension retains exact typed state                                                                                                                                                                                                                                           | No duplicated observation owner or unbounded per-operation diagnostic leak was found. Published effects can grow with real output                                                                                                                                     | Keep transactional delivery and type each pending field; measure effect-ledger growth as output, not scratch                                                                                              |

The external reachability-store prerequisite and append-only command-timeline
inconsistency are resolved: prior/current now share one physical owner domain,
and checkpoint pruning
now drops the checkpoint's direct command-root owner, and reusable serial-
validated executor slots retain bounded high-water metadata without scanning
unused capacity. Exclusive candidate issuance is also implemented, resolving
the former unbounded multi-candidate finding. The format closure and abandoned
durable rows are verified over-retention. Store-level body migration and direct
row release are therefore the immediate next implementation step. No
compactor, forwarding scheme, tracing collector, or reachability registry is
an acceptable correction.

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
- No stored Rust reference, per-value `Arc`/`Weak`, or liveness lookup on the
  ordinary expansion path.
- No heap allocation after bounded hot-path warmup except at an explicitly
  cold host, diagnostic, format, or output boundary.
- No runtime id in a format, detached continuation, memo wire value, output,
  process message, or thread message.

These restrictions deliberately leave one simple reclamation policy: exact
scratch reuse while the current generation runs, followed by wholesale
generation retirement when its one coarse lifetime ends.
