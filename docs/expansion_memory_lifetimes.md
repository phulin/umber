# Expansion memory lifetimes

Status: implementation map, migration guide, and retention audit for the core
command expansion engine.

The normative end state is [Runtime storage lifetimes](runtime_storage_lifetimes.md).
This document answers the narrower practical question: when expansion scans,
expands, suspends, resumes, rolls back, and crosses an editor revision, who owns
each byte and when can it be reclaimed?

[Node-region ownership](node_region_ownership.md) supplies the node-specific
coarse-owner rules: exact paragraph checkpoints retain exclusive page regions,
raw list coordinates are borrowed capabilities, and TeX copy versus consuming
move remains explicit.

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
              +-- structural definition regions + shared token-list carriers
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

The driver-completed primitive registry belongs to the first
process/session-immutable row above. An executor can retain its fixed-width,
generation-typed `PrimitiveHandle<G>` values and borrow the named immutable
rows directly. Handles contain no definition owner, never address mutable eqtb
cells, and are excluded from command roots, snapshots, formats, detached
continuations, and revision transfer. Extending a registry invalidates handles
issued against its prior extent; ordinary sessions bind only after complete
INITEX installation or format-registry reconstruction.

The implementation therefore has at most two live revision generations: the optional
prior accepted generation and the exclusively leased current candidate. There
is no third historical arena and no chunk-by-chunk history.

## Lifetime matrix

"May cross" below means that the value may intentionally survive the named
boundary. A heap allocation is permitted only at the owner boundary shown; it
does not authorize a smaller hidden arena.

| Lifetime and examples                                                                                 | State owner and allocation                                                                                                                                                            | Nested or reusable                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | Exact release                                                                                                                                                                                 | May cross suspension / revision                                                                       | Copy and heap rule                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| ----------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Process/session immutable resources: compiled command semantics, profile tables, immutable catalogues | Process binary, immutable configuration, or session capability owner; usually static data, `Arc`, or an owned validated resource                                                      | Shared, not nested; reuse is expected                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           | Process exit, session disposal, or last immutable owner                                                                                                                                       | Yes / yes when identity permits                                                                       | Startup/cold heap ownership is allowed; hot expansion borrows it                                                                                                                                                                                                                                                                                                                                                                                         |
| Interned control-sequence names and spellings                                                         | `SessionInternerEpoch` owns one append-only `Interner`                                                                                                                                | One epoch per session; entries are reused by `Symbol`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           | Whole epoch retirement                                                                                                                                                                        | Yes / yes, within that session                                                                        | Bounded heap growth is allowed; no rollback copy or per-revision reinterning                                                                                                                                                                                                                                                                                                                                                                             |
| Prior accepted generation                                                                             | `Session::prior_generation` owns one move-only lease into its `ReachabilityStore`                                                                                                     | Not nested and not reusable as candidate scratch                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | Clear its physical store slot immediately before installing the accepted candidate, or session drop                                                                                           | N/A / exactly one acceptance boundary                                                                 | A retained checkpoint may seed the other slot atomically; definition/token payload allocations are shared and the prior remains unchanged                                                                                                                                                                                                                                                                                                                |
| Current candidate generation                                                                          | `RevisionCandidate::generation` owns the other lease into that same store after execution begins                                                                                      | Exactly one session-issued candidate lease; its fixed slot is reused only after release                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | Candidate rejection/drop clears the slot; acceptance moves the lease into the prior role                                                                                                      | Yes / becomes the new prior                                                                           | One destination-local aggregate may be prepared from an accepted checkpoint; no relocation, rehome, wire round trip, or cross-generation id rewrite                                                                                                                                                                                                                                                                                                      |
| Dense meanings, registers, parameters, and rollback records                                           | `DenseState` plus split `SaveJournal` group segments, checkpoint deltas, and operation undo in the current generation                                                                 | Group segments pop and reuse buffers; first-before checkpoint deltas and operation suffixes follow their own cursors; exact byte scalars change only when those owners can change capacity                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | A cell is overwritten/restored; groups retire whole segments, rejected operations truncate their lane, and generation drop ends capacity                                                      | Yes / only through a retained checkpoint in the same generation                                       | Reads remain direct dense indexing; budget reads are constant-time scalar projections; no overlay, census, densification, compaction, forwarding, per-entry owner, or live-entry relocation                                                                                                                                                                                                                                                              |
| Macro definitions                                                                                     | One `Rc<DefinitionRegionOwner>` per nonempty format, revision-global, or nested local-group semantic region; eqtb/save lanes hold opaque eight-byte non-owning refs                   | Local group regions nest by scalar parent keys; aliases copy only refs; one admitted macro-body row clones its exact region owner once                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          | Checkpoint suffix settlement, group/stack retirement after meanings restore, final resident-row drop, or generation/format retirement                                                         | Yes / no runtime ref crosses revision                                                                 | Direct scans append to stable coarse word chunks. A local-to-global let copies one span once; definitions and chunks have no independent owner, and resident delivery performs only a stable chunk word load plus cursor increment                                                                                                                                                                                                                       |
| Token lists used by toks registers, token parameters, marks, hooks, PDF records, and stored replay    | Every live semantic carrier owns one private generation-branded `Rc<[TokenWord]>` handle                                                                                              | Builders may nest and recycle chunks; sealing transfers words to one immutable shared allocation                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | Last exact semantic-owner drop                                                                                                                                                                | Yes / no runtime handle crosses revision                                                              | Builder scratch remains arena-backed; publication may allocate the final slice; aliasing and replay allocate nothing                                                                                                                                                                                                                                                                                                                                     |
| Glue and provenance rows                                                                              | Compact direct-index `GlueArena` and `ProvenanceArena` vectors inside the current store slot                                                                                          | Not nested and not reused after publication                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | Whole-slot retirement                                                                                                                                                                         | Yes / only after explicit handle-free detachment                                                      | Small value copy and arena growth are allowed; shared heap ownership would cost more than these compact values                                                                                                                                                                                                                                                                                                                                           |
| Checkpointable page nodes and shipout-derived nodes                                                   | Exclusive `PageRegion` owners over one current-slot `ChunkPool<Node>`, exclusive durable box/form regions, and one separate `ShipoutScratchArena<G>`                                  | Paragraph checkpoints in one page share its region; raw roots are borrowed capabilities; scratch rows nest by scalar marks and retain warmed capacity                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           | A region drops after output/held-over evacuation when no checkpoint interval owns it; terminal success/rollback resets scratch wholesale                                                      | Page regions may cross exact same-generation checkpoints; scratch ids never cross suspension/revision | Ordinary transforms retain ranges and append new nodes; consuming TeX moves transfer unique regions, TeX copy operations and history-preserved moves copy exact closures, and shipout constructs only derived scratch nodes                                                                                                                                                                                                                              |
| Initialized hyphenation patterns and job-local hyphenation mutations                                  | `HyphenationTable` moves its completed language tries into one coarse immutable `Arc` owner; exception and saved-code maps remain in the one direct mutable table                     | The pattern builder is mutable only before TeX82 §919/§1335 initialization. Checkpoint capture aliases the initialized owner and records a mutable-journal cursor plus fixed scalars. Candidate creation moves the direct table, rejection undoes the candidate suffix and redoes the accepted suffix, and acceptance discards the accepted suffix                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | The pattern owner ends after the last live generation/checkpoint carrier; mutable exception/code rows and their journal end when the owning generation retires                                | Yes / yes through the ordinary prior/current fork                                                     | Initialization moves the trie map once without copying nodes. Ordinary reads borrow it directly; no checkpoint clones the mutable maps, and there is no per-pattern owner, read-time COW, registry, compaction, extra generation, or extra node indirection                                                                                                                                                                                              |
| `ExecutionScratch` macro words, argument ranges, first-scan facts, and delimiter prefixes             | One `CommandState::scratch`, with 4,096-word physical slots, one live frame-owned metadata stack, one stable segment arena, and an intrusive free head                                | A purpose-built stack-local `MacroArgumentWriter` retains the one range, delimiter coordinates, brace depth, and first-scan facts. It publishes the completed range/facts into the pending frame once; suspension retains only the typed scanner continuation. Commit changes only the frame role; retirement returns the chain to reusable high-water storage                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | Macro input retirement calls `release_argument_set`; failed match/discard returns its unpublished frame chain; retained capacity ends with generation                                         | A live frame may cross in-process suspension / never a revision                                       | Heap growth is allowed only when the generation reaches a new concurrent high water; `(frame, slot)` selects one range directly, with no generic destination dispatch, second argument table, match lane, per-macro `Vec`, range search, second cursor, relocation, compaction, segment transfer, word copy, or decision rescan                                                                                                                          |
| Transitional attempt/scanner storage                                                                  | One `CommandAttempt` owns general-list/import rows and one shared fixed-chunk scanner-token lane; ordinary definitions use final region transactions                                  | Child scopes and scanners nest; each general scanner owns only a typed lane branch coordinate, while an expanded definition continuation keeps only its arena build key and scalar progress                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | Commit/rollback or cancellation truncates the exact attempt branch or definition-region word mark; physical capacities end with `CommandState`/generation                                     | Only expanded definition scans may retain the typed continuation / no                                 | `\def` and `\gdef` append directly to final storage; `\edef` and `\xdef` do likewise unless real resource suspension parks progress. General/read/import scans retain their independently required detached lifetime                                                                                                                                                                                                                                     |
| Scanner episodes and temporary builders                                                               | `ScannerState` stores status; call-local `ScannerEpisode` and typed builder/sink coordinates name attempt or destination storage                                                      | Nested scanner calls use child state, not a new arena                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           | Return, rollback, or typed pending scanner/scalar consumption                                                                                                                                 | Only fields moved into an exact typed continuation / no                                               | Ordinary scalar copies are allowed. Token sinks are branch coordinates in shared fixed-chunk storage, never persistent per-scanner word vectors                                                                                                                                                                                                                                                                                                          |
| Ordinary Rust call-stack values                                                                       | The executing Rust function owns commands, counters, enums, small cursors, and temporary borrows                                                                                      | Naturally nested calls; stack slots may be reused by Rust                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | Function return or unwind                                                                                                                                                                     | No, unless explicitly moved into an owned continuation / no                                           | Scalar and small fixed-value copying is allowed; an incidental call-local heap object must drop on return and cannot become a hidden mailbox                                                                                                                                                                                                                                                                                                             |
| Command fuel and its profiling census                                                                 | One top-level `CommandFuelLedger`; each `CommandProcessor` stores only a mutable borrow of its singular `CommandFuel`                                                                 | Reborrowed by bounded processor and executor leaf episodes; never copied or reconstructed. One mutable remaining-budget countdown owns fuel; the immutable admitted limit derives consumed fuel only for terminal or explicitly requested publication                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           | Session or standalone ledger drop                                                                                                                                                             | The ledger crosses suspension in its session owner / no revision or snapshot payload                  | Production charging performs one exhaustion check and one decrement through the borrowed scalar ledger. The `profiling` resolution adds five exact non-fuel detail counters and their statically gated updates; the default resolution has no detail fields or calls. Neither resolution has a stored consumed count, batch-charge path, processor-owned alternative, ownership dispatch, heap indirection, or rollback refund                           |
| In-process suspension                                                                                 | `PendingCommandAttempt` owns a boxed attempt, one coarse `GenerationOwner`, fixed marks/resume coordinates, and a typed payload; `MainControl` owns singular pending-operation fields | Typed continuations can refer to nested command-side continuations                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | Successful resume consumes it and drops the extra generation owner; cancellation/drop releases the package                                                                                    | Yes / no                                                                                              | Boxing at the cold host barrier is implemented and allowed; cloning the live graph or storing untyped token mailboxes is not                                                                                                                                                                                                                                                                                                                             |
| Detached continuation                                                                                 | `OwnedCommandContinuation` owns validated handle-free recipes and DTO-local indices                                                                                                   | Can encode a nested input stack; never owns a runtime generation                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | DTO drop                                                                                                                                                                                      | N/A / yes, because it contains no runtime ids                                                         | Cold copying and heap allocation are allowed under explicit admission budgets; materialization builds fresh destination-local values atomically                                                                                                                                                                                                                                                                                                          |
| Source and input cursor storage                                                                       | `InputState::levels`, `pending_sources`, `SourceLevel`, one `PackedTokenSpanHandle` shape, and one generation-owned segmented replay lane                                             | Input levels and replay entries nest in exact LIFO order; source domains adapt once at level creation, ordinary characters project directly into the owning or compact tokenizer destination, and a tokenizer control-sequence name is call-local until its creating or probing boundary projects one packed identity; delivery writes through the admitted handle into the caller-owned `CurrentCommand` through a reference-only raw phase proof and advances the fixed frame; retirement validates and pops that exact top row, projects its semantic reason without cloning its replay trace, and advances the existing delivery loop through a scalar phase; source-only nesting ancestry is borrowed from the validated top rather than found by an enclosing-stack search; error-context traversal projects only the current, budgeted, and bottom displayed levels; file framing travels as the push's call-local name or retirement's existing result bit and renders through the live command context | The transient name ends inside the tokenizer call; EOF/input retirement pops a level and releases its exact span/replay suffix; unopened registrations end when opened or command state drops | Yes / source recipes may detach, live runtime coordinates may not                                     | Replay admission grows only at a new generation high water; warm canonical delivery and retirement advance scalars and perform no allocation, relocation, payload or replay-trace copy, owned-name-sized ordinary-character transport, generic returned-delivery construction, per-word owner clone, command-slot reconstruction, enclosing input search, name lookup, creation-policy test, omitted-level pseudoprint, or persistent framing-event poll |
| Diagnostics and observations                                                                          | Command semantic queues, operation-local `DiagnosticEffects`, one reusable typed receipt in `ObservationBuffer`, then `World`/output effect storage                                   | Operation-local batches may contain nested diagnostic programs; receipt category vectors retain only their warmed capacity after an in-place reset                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | Rollback drops the batch; commit drains/publishes it; receipt reset clears every completed category; accepted effects end with output/world or generation/session disposal                    | A typed pending operation may move the sole buffer / detached effects may cross revisions             | Owned strings/vectors are allowed at diagnostic/effect boundaries; the receipt aggregate is not copied or allocated per operation, and no semantic result may wait in an untyped hidden queue                                                                                                                                                                                                                                                            |
| Pure memo and render caches                                                                           | Session/executor cache owner; `PureMemoRuntime` and editor render-map cache enforce entry/byte budgets                                                                                | Reusable and evictable, not semantic nesting                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    | Eviction, explicit clear, or session drop                                                                                                                                                     | Yes / yes only as handle-free validated results                                                       | Bounded heap retention is allowed; cache identity cannot provide runtime liveness                                                                                                                                                                                                                                                                                                                                                                        |
| Format image and format construction                                                                  | `DetachedFormatImage` transiently owns bytes plus decoded handle-free rows; admission consumes it into a fresh destination generation                                                 | Complete validation precedes admission; construction then drains decoded rows in canonical dependency order                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | Encoded bytes drop before destination construction; each decoded payload drops or moves when its final column is installed; runtime rows end with the generation                              | N/A / yes, via handle-free schema                                                                     | Cold decode allocation and DTO-local relocation are allowed. Admission does not retain duplicate decoded/live payloads. Live ids, chunks, and generation owners are forbidden on the wire                                                                                                                                                                                                                                                                |

`ExecutionScratch` now also owns the production parked-expansion suspension
lane. `ExpansionWork` stores stable `CurrentCommand` slots, variant-specific
controls, and control-sequence name bytes in fixed chunks. Command and control
coordinates carry both the issuing work owner and their lane serial; name
marks carry that owner, the live root serial, and the byte offset. Access
validates the owner before indexing, and the root serial rejects a mark after
abort and byte reuse. One 32-byte move-only root key carries the complete
logical marks plus owner and ABA serial identity; completion or abort retires
controls deepest-first and then truncates command and name lanes to those
marks. The retained chunks are current-generation capacity and never
checkpoint payload. Ordinary synchronous expansion remains in its caller-owned
command slot and enters none of these lanes. At an actual immutable-resource
suspension, the driver moves that sole command owner and its exact typed resume
state into one stable root. Main control retains only the move-only root key;
resume consumes the command once into the caller destination, and resuspension
parks that same owner again. Nested roots are strict LIFO, and owner/serial
validation rejects foreign, stale, or out-of-order keys before mutation.
Failed park returns the complete command, typed phase, and child to the direct
caller. A nested `expandafter`, expanded `scan_toks` collector, or alignment
preamble retains only its own earlier operands and the typed child edge: the
generic expansion frame is the sole owner of the command that actually
suspended. Structural controls, the `csname` name lane, and PDF string-compare
migrations remain later reviewed stages. Test/profiling counters measure
command clones, definition retains, ownership moves, lane high water,
whole-control copies, and warmed allocations independently.

`PageMaterialArena` owns one `ChunkPool<Node>` and one coordinate-only
`ForkArena`. Fixed payload chunks own each `Node` once. An `ArenaList` is the
sole list topology: it names one source range or one arena-owned nonrecursive
range record with cumulative endpoints. Detached active builders append
genuinely new nodes or retain immutable source ranges; neither operation
materializes source payload. Candidate rollback truncates payload and
descriptor chunks through the same arena mark. Indexed borrowed views resolve
the canonical ranges directly. `ParagraphTape`, alignment setting, and
`LineMaterializer` carry only list roots and compact scalar/index scratch; they
never own or materialize the source node lane.

If convergence identity is enabled before publication, original node append
also maintains one composable whole-chunk summary and descriptor publication
stores the exact summary of each canonical range. Slice and retained-range
identity combine those summaries, hashing payload only in bounded partial
boundary chunks; compose uses the already maintained list roots. Operation
rollback restores the partial-tail summary, while promotion and checkpoint
settlement move summary metadata in the same coarse envelopes. The exposed
identity node-hash and summary-combine counters prove this sublinear source
work, disabled-demand zero work, and unchanged `source_nodes_copied`.

Alignment rows and cells are transient candidate material and cannot occur at
an eligible retained boundary. Cell packaging moves the completed mode-list
root into the unset child, row packaging moves that child root into the
alignment list, and final width setting replaces unset nodes through detached
active builders while retaining unchanged `\noalign`, interline-glue, and
tabskip ranges at their original addresses. Display and ordinary handoff move
the finished `PageListId`; lifecycle diagnostics retain row/cell counts, not a
parallel root or node container.

Math lists are also transient and ineligible at retained boundaries. The pure
choice/noad pass stores borrowed page coordinates for unchanged source leaves
and owns only genuinely rewritten or generated drafts. Execution lowers those
drafts through detached active builders, retains native source ranges at their
original addresses, and hands inline, display, equation-number, and packaged
box results onward as the canonical `PageListId`. `source_nodes_copied` remains
unchanged while generated math boundaries and layout nodes advance
`new_semantic_nodes`.

### Process and session state

The command profile and compiled semantic dispatch are configuration, not
revision history. Expansion borrows them. Host capabilities are also borrowed
for an admitted episode; they are not smuggled into tokens or definitions.
The persistent expansion root contains only the active profile and
`cumulative_expansions`, whose job-level value still participates in future
expansion and suspension classification. Recoverable reports live once in the
canonical semantic-diagnostic queue and transfer to the executor as one owner;
resource resolution, dependency observation, and semantic barriers have no
parallel expansion ledger.
One stack-branded `OperationHostPreparation` remains stationary from host
preparation through delivery preflight, transaction classification, semantic
application, and save-stack settlement. The one preflight `CommandContext`
fills its mode, effective-tail, PDF-mode, and group fields in place; the
transaction boundary borrows those exact scalars instead of reconstructing
the 240-byte admitted facade. Hot and ordinary cold application use their
already-resident semantic context to write the post-apply checked save depth
back into the same preparation, and settlement drains only that scalar.
Measured definition, let, and catcode application constructs its 240-byte
semantic context directly in one callback-owned stack slot and retains that
borrow through named-token publication and §1269 `afterassignment` backup.
The callback passes only narrow mutable borrows; it neither returns nor moves
the admitted aggregate. Group application ends the callback before its
possible page-output boundary and reacquires only for command-local
publication afterward.
Diagnostic or host boundaries which can change the sampled state refresh only
their affected fields after the live dialogue. No layer returns, takes, or
stores a whole admitted context or duplicate facts aggregate. Resource
resolution, suspension packaging, rollback, and outer executor publication
still occur only after the applicable call-local context is dropped, so this
adds no owner, cache, heap indirection, or lifetime mechanism.

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

PDF is the concrete mutable-runtime exception to generation copying. A
checkpoint holds a fixed scalar `PdfStateSnapshot`, not cloned PDF rows. The
reachability store exclusively moves the one `PdfState` authority from the
accepted slot into the candidate; accepted admission is unavailable until
that transaction commits or rejects, and suspension retains the same
candidate owner. Dense row families keep accepted storage in place behind
logical base lengths and append candidate rows to private deltas. Exact undo
entries above the base swap in place into redo entries, so rejection restores
the accepted state and history while acceptance discards prior-only suffixes.
Image/form byte addresses remain identical throughout. No shared mutable
container, per-value owner, COW write, hash overlay, current-table clone, or
third generation participates.

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

`DefinitionArena` instead owns packed headers and words in the format,
revision-global, and nested local-group regions. Eqtb and save rows carry only
an opaque copy-only `DefinitionRef`; the arena alone decodes its region and row.
Each nonempty semantic region has one `Rc<DefinitionRegionOwner>` containing
append-only fixed word chunks and retained header/provenance records. Chunks
have neither `Rc` nor an independent pool/lifetime. Macro admission resolves
the ref/header/start and validates the initial chunk once, then clones that
exact owner into a `ResidentMacroBody`. Its flat append-only chunk directory
replaces the former linked directory pages and `OnceCell` probes. Safe Rust
cannot cache a direct chunk borrow beside its owning `Rc` without a
self-reference, raw pointer, or independent chunk owner, so each delivered word
performs one constant-time region-local directory slot access. The input row's
existing scalar cursor remains the complete rollback coordinate, and its
derived chunk changes only at explicit 4,096-word crossings.

Each TeX group owns exactly one directly keyed local region. `begin_group`
claims one direct slot from stable 64-slot chunks and replaces one current-key
scalar; it never relocates or copies an accumulated region prefix. Every region
stores its exact parent key. Reclaimed addresses are reused through an
intrusive exact free-slot chain, while the key's incarnation rejects an
ABA-stale definition coordinate. Nesting leaves the parent region intact, and
`end_group` follows the child's parent key and retires only that child. Each
child structurally pins its parent until exact child reclamation, so one scalar
current-region checkpoint lease pins the whole ancestry without walking it or
retaining a stale region collection.
An otherwise unpinned child drops immediately. A resident macro body retains
the exact retired semantic-region owner until its final input row drops;
checkpoint lineage keeps its private current-region pin only while rollback
can still require the structural slot. Release visits no peer or accumulated
history. Slot chunks are coarse generation high-water capacity;
entering a group does not allocate an individual region shell. Promotion
mappings live inside their source local region and leave with it.

Checkpoint capture stores the current key and one mutation-journal mark. The
first post-checkpoint definition or lifecycle write to a region records its
exact pre-write row, word, promotion, accounting, and retirement boundary. An
origin change to a row before that boundary records one compact inverse field
edit in the same region mutation; repeated changes to that row coalesce, and
an appended row needs no separate edit because it leaves with the suffix.
Settlement swaps those alternates while visiting only the journaled regions
and fields. In particular, if checkpoint child B ends and its parent A is then
written before B is restored, both B's lifecycle transition and A's suffix or
origin edits are settled exactly on acceptance or rejection. Nested and leased
suffixes use the same source-owned records; no active-chain, historical-region,
retired-region, or promotion sweep exists.

The non-owning definition ref has a private four-byte region-coordinate
component. Fixed coordinates 1 and 2 name format and global storage; every other
coordinate uses a 16-bit local slot address and a nonzero 16-bit incarnation.
No bit is a macro-input or locality flag, so the verified macro branch can
combine the coordinate and four-byte row into an eight-byte carrier without a
collision. A slot at incarnation 65,535 is never reused; capacity exhaustion
is explicit rather than wrapping into an ABA alias.

Integration with that macro branch must remove its bit-63 identity flag. All
32 coordinate bits belong to region address and incarnation; semantic identity
must instead be resolved from the admitted definition header.

Ordinary `\def` and `\gdef` select their local-group or revision-global arena
before scanning. The collector opens one transactional word mark there,
validates parameter structure while writing each semantic word once, and seals
the final header without a publication copy. Failure truncates that mark. Raw
definition scanning is synchronous and owns no continuation. `\edef` and
`\xdef` use the same final destination, but may retain the compact build key,
scalar scan state, and existing expansion continuation when expanded scanning
actually requests an unavailable resource. Cold memo, format, and read-import
paths may use detached `DefinitionBuilder` staging because their input already
has a separate lifetime; that is not the ordinary scanner representation.

The format, revision-global, and nested local-group regions share one compact
`DefinitionRef<G>` key and borrowed `DefinitionView<'_, G>` surface. Eqtb and
save journals copy only keys. Checkpoint rollback truncates global and local
suffix marks. Normal `endgroup` restores meanings before retiring the complete
local region; a macro input row which still consumes a local body retains one
coarse region lease until that row drains. A global `\let` of a local source is
the exceptional one-time span copy into revision-global storage and reuses the
source-region-owned promotion key.

Raw and expanded command delivery uses separate concrete destination loops.
Each active request
owns one caller-provided `Option<CurrentCommand<G>>`. Pointer-sized
`EmptyCommand` reborrows prove that resident input writes only that slot. Each
source returns only the scalar packed-resolution fact, so no command reference
crosses the input-top transition. `CommandState` reclaims its original
destination, consumes the resolver's already-decoded literal catcode for
required brace handling, applies one-delivery suppression, and returns only a
copy-small ready/outer result. The input row passes its
already-resident `TokenWord` to the dense meaning lookup, which writes the
final meaning and control-sequence fields before returning; warm resident
delivery returns only its final ready/outer scalar. A macro `Param` pushes its
admitted argument and continues inside the resident loop. Only EOF, line,
retirement, replay, diagnostic, checkpoint, or failure paths materialize a cold
status; no semantic-token value or raw-command phase crosses into a result
relay. Nested delivery has its own slot. The expanded fetch/inspect loop
keeps the initialized value in place while synchronous expansion mutates input, then raw delivery
overwrites that same value for the next token; it does not clear the `Option`,
reconstruct an empty command, or redispatch the prior meaning between
expansions. Each concrete loop returns only its final compact status and moves the command
only to its final consumer or the exact typed expansion suspension slot. The
request also owns one stack-local cold error slot. Internal delivery transitions
return a zero-sized failure marker; a real failure moves its `CommandError` into
that slot, and only the public boundary constructs the rich `Result`. Thus an
ordinary successful token neither copies nor reconstructs the error envelope.
The command slot is neither global nor a mailbox and never survives
independently of its request. `CurrentCommand` retains only its compact
immediate-delivery coordinate and the `OriginId` already packed in its
spelling. Physical ranges, active source identity, macro ancestry, and
observation payloads are resolved from immutable source/provenance owners only
at backup, observation, tracing, diagnostic, or publication demand. The
executor keeps the compact source role only because operand scanning may
retire the originating input frame before application. `CommandProcessor`
derives observation order from its next-delivery cursor and temporarily
mirrors only the current command's compact coordinate to reject a stale
move-only backup. A genuine typed retry readmits its command/cursor explicitly.
No decoded provenance copy, command projection, or delivery owner is stored in
executor preparation.

Resident source tokens and literal macro arguments return directly from input
delivery. Stored-token access projects an out-parameter slot from the packed
word already at the storage boundary and carries only that optional fact to
macro-parameter replay. The ordinary path therefore does not reconstruct a
complete token merely to ask whether it needs interception, and meaning
resolution remains the single full spelling classification.

Macro argument collection begins only after that raw-delivery boundary. Each
delivered command projects its spelling, brace kind, paragraph-token identity,
and required recovery flags once. An admitted `MacroMatchWriter` owns the one
accepted-token settlement over the reusable word lane: the lane returns its
new absolute cursor and the same borrow updates writer-local paragraph,
brace-depth, and removable-outer-group aggregates. Delimiter-prefix storage is
the existing semantic holdback lane, enriched with those first facts; overlap
recovery moves the retained record into the writer without classifying its word
or provenance again. The matcher reads the writer's depth directly, and sealing
projects its aggregate without scanning stored words. No command, input frame,
token buffer, or second argument representation crosses this transition.
Definitions carry their validated marker offsets in the immutable header, so
invocation does not reconstruct that pattern. A compulsory literal prefix with
no numbered parameters is matched directly and never creates a `MacroMatch`,
collector, argument block, or `ArgumentSet`.

An admitted control-sequence spelling probes and borrows its dense meaning row
once. The packed-token resolver accepts the actual caller-owned
`CurrentCommand`; the canonical row decodes its tag once and writes the final
static payload and command identity, or the sole macro owner, directly into
that slot. No meaning-projection carrier, intermediate resolved meaning,
whole-row copy, or second command is reconstructed. The same token
classification reports the work ledger's meaning-lookup fact, and no second
spelling decode precedes resolution. A macro row writes one compact
`DefinitionRef<G>` key into the final `CurrentCommand`; it acquires no definition
owner. Trace eligibility and expanded-loop classification borrow that resolved
meaning without a lifetime operation.
The expanded loop classifies that meaning once into return, `end_template`,
macro activation, undefined recovery, or the exact primitive dispatch. That
borrowed decision drives the expansion call, tracing, and work accounting;
policy and primitive dispatch do not repeat the meaning match.
The temporary bank borrow ends before any command-driven mutation. A macro
call borrows the definition span through synchronous matching, tracing, and
replacement-length inspection. Once matching and recovery settle, its local
macro input ownership row acquires the coarse region lease; format/global rows
need no lease. Prefix mismatch, retry, and genuine resource suspension retain
only the already-required compact command/input state. TeX assignment level
and journal state stay in the bank row and never enter the command. `\ifx`
keeps its two raw-delivery slots while comparing borrowed definition spans.

The definition arena owns published payload by region. Cold format capture
compacts reachable keys into handle-free rows, and load places them in the
immutable format region. There is no runtime compaction, forwarding pointer,
id rewrite, or move to another generation apart from the explicit one-time
local-to-global promotion.

Execution scratch blocks can recycle because their last semantic user is
known. A specialized macro-body input row is the call record: it holds the
8-byte non-owning definition key, optional coarse local-region lease, and an
optional `ArgumentSet` coordinate. Format and revision-global definitions need
no lease operation. Parameterless macros publish only this row; they create no
matcher, argument set, frame, block, or owner. Delivery and cold context borrow
the replacement span directly through the key.

### Macro, scanner, and operation nesting

The input stack itself is the logical macro stack; there is no parallel
activation chain. Only parameterized calls publish a sealed `ArgumentSet`.
During collection, a purpose-built stack-local `MacroArgumentWriter` holds the
admission-minted fixed-block position and provenance-run boundary, delimiter-
prefix and rollback coordinates, brace depth, and first-scan facts exactly
once. Admission validates the pending frame and fixed argument slot once.
Accepted words then advance the writer directly without repeating frame/range
validation or crossing a processor forwarding helper. Completion validates the
frame once and publishes the already-written span and facts into that slot; an
unfinished call retains only its typed scanner continuation. The frame then
contains fixed-capacity absolute argument ranges, the exact §394 facts
established while each range was first collected, and its opening segment
watermark. Those facts distinguish an ordinary forbidden `par_token`
from the same spelling committed out of a failed delimiter prefix, and record
whether the collected pre-stripping span was exactly one outer group. The
non-`\long` check and outer-pair removal consume those facts without rereading
the stored words. Nested macros append to the same `ExecutionScratch` stack;
they do not allocate child arenas. When a macro-body input retires, the
corresponding activation is removed and its strict-LIFO segment suffix is
returned for reuse.

Scanner control is mostly ordinary stack state: a `ScannerEpisode`, phase
enum, counters, and sink coordinates. `AttemptArena` owns one shared
fixed-chunk token lane. Each ordinary token scan reserves a parent-owned row
containing only a branch coordinate, appends accepted words directly to that
branch, and seals it by changing token-list metadata. Truncation clears word
lengths and returns complete chunks to the lane free list without visiting or
moving individual words. `PendingScanToks` values occupy
ABA-tagged slots in the current generation's reusable `ExecutionScratch`. A
move-only `ScannerFrameKey` is the sole root capability. Each scanner,
expansion, `\expandafter`, preflight, diagnostic, or alignment caller moves its
exact child key into a typed phase destination; resume consumes that edge
before continuing the caller. Abort follows the structural child chain
deepest-first so younger scanner episodes and attempt scopes close before
their parents. There is no global pending-scan or pending-expansion scheduler,
configuration search, or coordinate repair.

Scalar, expression, font, hyphenation, and token-list assignment scanners
deliver each raw or expanded command into their own call-local command slot.
If expansion suspends, its frame retains that exact command while the enclosing
scalar or structured frame retains the typed child destination; retry recreates
the same local slot at that phase and restores the child deepest-first. The
slot ends with the scanner call and never becomes command-state storage,
rollback state, or a searchable result channel.

The canonical `scan_toks` replacement collector keeps one such destination
for its complete synchronous loop. Raw delivery, expansion classification,
observation, and token spelling all borrow the command in that slot; a
successful iteration clears it in place for immediate reuse. Only semantic
backup or resource suspension moves the command out. Generic expansion moves
a suspended command directly into its one `ExpansionWork` frame, while the
collector continuation retains only the typed child edge; direct `\the`,
`\unexpanded`, and `\detokenize` collector phases retain their own command
because they do not enter generic expansion. The destination therefore does
not retain commands across a generation or act as a hidden result cache.
On resume, the collector consumes and decodes its parked expansion once before
entering the same resident loop. Synchronous iterations never inspect that
owner, and only a fresh immutable-resource suspension reconstructs it.

The same invocation keeps its complete `PendingScanToks` owner stationary in
one local row. Opening installs `ReplacementProgress` into that row once;
ordinary delivery, brace depth, parameter matching, direct-splice operands,
and the child edge then mutate it through a borrow. A synchronous error returns
only `CommandError` and cleanup borrows the same row. Only an immutable-resource
suspension moves the complete row into the existing recyclable scanner lane;
resumption takes that exact row back and continues through the same mutable
phase. There is no replacement-progress failure carrier or per-token phase
reconstruction.

The same scanner frame retains the one phase-tagged `TokenCollector` and the
deferred-diagnostic cursor established when its destination was reserved.
That generic owner is now exclusive to token-list and definition scanning;
macro matching has no destination variant or generic publication relay. Both
paths reuse one `ClassifiedToken`, but macro brace, delimiter, pending-
parameter, and publication state live only in `MacroArgumentWriter`. Macro
arguments remain generation scratch owned; ordinary general lists remain
attempt-lane owned; ordinary definitions write semantic words directly into
the selected transactional definition region. Standalone escaping general text instead owns one
isolated generation replay builder: nested input cannot interleave its span,
sealing moves its storage header into the final replay entry, and LIFO
retirement truncates its fixed chunks into the builder-lane high-water pool.
Case shifting selects that destination and applies `\uccode` or `\lccode` as
each accepted traced word is written, so it creates no attempt list,
publication copy, or completed-list traversal.
For a general list the collector advances its active writer from the parameter
branch to the replacement branch. For a macro definition it advances the same
compact arena build key from parameter validation to replacement writing.
Every accepted word is appended once to that active final or
rollback-truncatable storage; sealing changes metadata and returns the compact
definition key without a whole-list handoff copy. Successful
completion finds no episode-owned runaway report and returns without
allocating, copying, or rendering diagnostic context. If EOF or outer-validity
recovery did publish a report, completion borrows the existing collector views
before their scope retires and walks each word once into the report's final
selector-aware partial string, carrying the macro match character across the
synthetic `->` separator. No diagnostic token vector, second token traversal,
or success-path string exists; resource suspension retains only the collector
and cursor already owned by the scanner frame.

A nested `\unexpanded` general-text scan owns a separate child collector
because its scanner episode, rollback mark, and suspension edge must remain
independently abortable while the parent collector is parked. Completion does
not justify a second word lifetime: a token-buffer parent adopts the child's
fixed-chunk sink chain in O(1), invalidating the child list coordinate without
rereading a word. Standalone `\unexpanded` replay points its input level at the
completed attempt list directly instead of copying it into the replay lane.
An expanded macro definition is the exact boundary where child chain adoption
is not valid: traced child words shed provenance while being appended once to
the already-selected semantic definition region. Sealing does not copy them.

### Diagnostic-context publication coordinates

TeX82 §§310--318 input context has one live owner: `InputState`. Ordinary
executor scan/apply handoffs do not project that owner into a `String`.
`DiagnosticContextCoordinate` carries only the command-timeline owner and the
current exposed-row and context-scalar coordinates. A source top contributes
its packed frame, ABA-checked slot key, immutable backing identities, and
copy-small current-line/lexer state; stored and direct macro-argument tops
contribute their packed frame. The existing input-history lineage coordinate,
row-admission serial, and source-owner lifetime serial reject rollback,
push/pop ABA, and buried generated-source replacement.
Retained line, pending-source frontier, `force_eof`, and the runtime-only
terminal-string owner are compared directly. Capturing performs one top-row
read and, only for a source top, one owner-slot read; it performs no stack walk,
pseudoprint, allocation, clone, hash, or buffer move. Publication validates
that structural coordinate before traversing the live stack. A foreign owner,
input advance, push/pop, rollback, source-line or source-owner replacement, or
terminal-context replacement is stale and is rejected before rendering. The
coordinate owns no row or backing, so it is neither a cache nor a lifetime
registry and cannot cross a detached continuation boundary.

The context-consumer audit has these publication boundaries:

- command-core scanner and macro recovery render synchronously only after the
  specific error/runaway branch is selected, while their source, macro
  parameters, attempt words, and execution scratch are still live;
- `MisplacedAlignmentDelimiter`, `DeleteLast`, `SetInteractionModeValue`,
  `Unbox`, and `LastBox` carry one compact coordinate through the ordinary
  executor operation frame and render only inside the apply-side reporter;
- page building borrows live `CommandState` and renders only after selecting a
  page diagnostic; replay that has already crossed a real suspension boundary
  supplies detached text instead;
- terminal/fatal reporting renders at the terminal error seam, before command
  rollback can retire the triggering input; failure-only causal summaries are
  separate content-free bounded facts;
- `file_warning`, output-routine close, immediate output, shipout, and observer
  extraction are externally visible output seams and therefore materialize
  their final selector-aware text there; and
- portable or host-retained continuations never contain a live coordinate.
  A boundary that must outlive the admitted command generation renders once
  before detaching and retains only the final owned text.

The focused measurement around `InputState::output_open_context` records zero
renders, owned allocations, and owned bytes for coordinate capture, exactly
one render/allocation at publication, and no additional render when a stale or
foreign coordinate is rejected. This directly measures removal of the former
success-path context allocation/copy work without adding a mirror or cache.

Suspension publication is one owner transaction. The execution-scratch lane
preflights its slot, free-list capacity, and checked serial successor before it
moves `PendingScanToks`. If admission fails, the payload remains with the
caller. If admission succeeds but the processor baton already contains a key,
the displaced key is restored before the just-admitted frame is taken back.
Both paths then abort the nested continuation deepest-first, finish the scanner
episode, close its scope, and truncate through the pre-sink attempt mark. The
lane slot and definition/token-buffer capacity remain reusable; no moved child,
status, sink coordinate, or attempt row survives the failed publication.

TeX82's separate `read_toks` collector wraps scanner-status installation,
`align_state := 1000000`, builder setup, line collection, sealing, and scope
retirement in one structured cleanup transaction. Every setup, collection,
validation, or finalization error restores the saved alignment scalar and
complete prior scanner episode before truncating the exact attempt child
scope. No failure can leave a partial definition builder or eqtb write behind;
the executor installs the target meaning only after successful durable
promotion.

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
coordinate-free `CommandAttemptOperation` capability, the cold-materialized
opening coordinate, a fixed resume point, the typed requested operation frame,
and one coarse generation owner. In the ordinary path `CommandState` alone owns
that coordinate; `DirectOperationMark` moves only the opaque lifecycle edge,
so commit and rollback neither copy nor compare a duplicate mark. Preparation writes
diagnostic fields into one caller-loop `OperationFrame` and returns only a
compact readiness coordinate. A successful ordinary scan installs one compact
hot payload into that frame or one uncommon cold leaf into its adjacent
caller-owned typed slot and returns only a compact tag. The frame's singular
payload field owns the hot value or a cold-occupancy tag, so the 264-byte cold
leaf does not inflate every resident hot record. Preparation lends those exact
resident fields to the checked promotion writer, which changes the cold leaf's
small attempt-root fields to prepared-root fields in place without a builder
batch or duplicate owner receipt; application
admits one semantic `CommandContext`, consumes semantic leaves through a mutable
borrow while that context stays resident, then clears and immediately reuses
both slots. Named token-list push receipts produced during semantic apply drain
directly into the existing optional operation observer through the same
admission before detached evidence crosses settlement. They construct no
intermediate input-record or observation vector, and an unobserved publication
constructs no record and allocates nothing. Only a
genuine host boundary releases that context and admits the narrow host-specific
continuation context. Resource suspension moves that exact frame and occupied cold
slot into the attempt instead of boxing a prepared
operation or retaining completed operations in a generation-long lane. `MainControl` owns one
singular direct-retry slot: the same operation capability moves together with
exactly one in-place operation frame or alignment destination. The operation
frame owns its admitted `CurrentCommand`, parked expansion, scalar phase,
delivery cursor, scanner child, partial direct-scan phase, and mutually
exclusive operation payload directly; no nested preflight or scanned-operation
projection is constructed or extracted at preparation, suspension, or
resumption. A genuinely suspended typed scanner writes its rebuilt hot or cold
leaf directly into the same destinations and returns only the compact tag. A settled command discovered by
alignment dispatch installs its command/cursor destination before operand
scanning, so a resource failure cannot resume alignment past that command and
strand its scanner child. Resume consumes the owner; cancellation drops it.
This keeps exactly the current generation alive, not one owner per token.

Cold operand helpers use the same destination rule before any suspension:
scalar, structured, alignment, recovery, arithmetic, and leader scanners
borrow the adjacent cold slot, construct their terminal leaf there, and return
only `Result<()>` or a compact boolean. The uncommon enum therefore remains at
one address from completion through preparation and application; a by-value
`Result<ColdOperation>` carrier is absent. Only a genuine resource suspension
moves the occupied slot with its singular operation frame.

The physical executor split follows those same ownership transitions without
splitting the interpreter. `main_control/command_episode.rs` owns the resident
command episode, adjacent cold slot, and genuine suspension carriers;
`main_control/delivery.rs` mutates those destinations through delivery,
preflight, and retry; `main_control/settlement.rs` owns commit, rollback, and
publication settlement; and `main_control/executor_facts.rs` owns the one
stack-branded scalar preparation whose fields are filled and drained through
borrow-scoped live executor views. `main_control.rs` retains the one
`MainControl` loop and explicit owner-loan/return orchestration. The sibling
modules use direct static calls and introduce no transport value, heap owner,
or alternate command representation.

Nested scanner and expansion suspension uses two fixed typed scratch lanes.
`PendingScanToks` has a dedicated lane because it owns the exact collector
destination and attempt scope, including a transactional definition build key
only when an expanded definition genuinely suspends. Raw definition scanning is
synchronous. Scalar, alignment, and structured continuations share the bounded
heterogeneous lane. Its lightweight expansion wrapper carries
only an `ExpansionWork` key when a nested scanner still owns the caller route.
Their backing vectors grow only when a generation reaches a new simultaneous
high-water mark; freed slots are reused with a new serial, so a stale or
double-consumed key is rejected.
Scalar continuations such as an in-progress `\csname` move their accumulated
name into the exact enclosing expansion or conditional phase. An expandable
`\number` or `\romannumeral` moves its leading/radix/optional-space phase into
the owning expansion frame, beside that frame's exact scanner child, instead
of a command-state stack. None may become a general token, boxed-dynamic, or
caller-order mailbox.

The reusable allocation-free scalar-frame family now owns optional-equals,
keyword and matched-prefix restoration, integer, dimension, glue, filename,
internal-value, expression, and font-selector progress. Every nested scalar or
expansion is a non-`Copy` child edge tagged with its exact return destination.
The executor's singular `OperationFrame` owns one reusable `ScalarScanFrame`.
Every scalar phase writes its typed result or cold error into that same slot
and returns only a compact complete/suspended/failed status. Completion
consumes the typed value immediately; a real resource edge consumes the error
and retains the existing scanner key beside the scalar phase in the same
operation frame. Starting another phase asserts that the prior payload was
consumed, so reuse cannot retain stale values. Expression evaluation likewise
writes through a bounded caller-owned `ScalarCallFrame<T>` instead of returning
the error-sized `Result` carrier measured by the copy census.
The raw resource-capable entry points remain private. Internal structured
parents still move a child through their exact typed continuation result, so
they cannot propagate a suspension with `?` while silently abandoning it. The
executor has no retained scalar-result envelope, fallback scanner API, result
tape, mailbox, heap owner, or second representation.

One singular caller-owned preflight frame holds the sole current command,
delivery cursor, compact dispatch phase, reusable scalar result destination,
optional scalar child, completed fixed-sequence operands, and a completed hot
operation awaiting application. Preflight returns only a compact delivery tag;
it does not move that operation through a scanned-result tuple or delivery
enum. Initial raw delivery and resumed expansion write
directly into that frame's command field; settlement advances only its scalar
phase instead of transferring the whole command through a temporary slot.
Raw, settled, expanding, main-loop, prefix, leader, and direct-operation
scanning borrow and mutate that frame in place; only an actual resource
suspension moves it into the retained retry destination. Its
exact operation phases cover register and box-dimension assignments; unary
integer, dimension, and glue commands; paragraph-shape and penalty arrays;
fontdimen, font-integer, font-code, font-expansion, and font-only operations;
code tables and catcodes; openout filename scanning; marks; math families;
arithmetic target/keyword/operand sequences; and leader payload, command, and
glue delivery. Expansion frames own number, internal-value, font, mark-class,
margin-kern, PDF scalar, balanced-text, and file-enquiry children. Conditional
frames own already-pushed condition identity, inversion, and exact numeric/
font/box/stream operands. Alignment preamble frames own tabskip optional-equals
and glue phases beside their scanner episode and partially built templates.

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

An input row owns either one eight-byte ABA-checked key to an authoritative
`SourceSlot` or a classified token cursor. `InputStack` owns source slots in
fixed reusable pages; opening a source consumes the pending backing into one
slot without allocating a per-source box. The source slot owns its move-only
`SourceCursor`, registered/replacement backing, reduced-spelling arena,
`everyeof`, opening ancestry, name classification, and retirement rule until
EOF retirement. The key's runtime-only slot generation is independent of
rollback-reused semantic `InputLevelId`; every compact and cold inverse
validates it before mutation. If a partially captured source occupant is
popped and its physical row reused in the same interval, the ordered history preserves a row
replacement before the new occupant becomes eligible for direct reuse. A
24-byte copy-only lexer cursor plus the four-byte input position is the only
ordinary source execution state;
control-word and `^^` probes copy it without cloning an `Arc`, `Vec`, or `Box`.
The generation-tied `InputStack` owns stable source, stored-token, and direct
macro-argument rows. Its one compact `InputUndo` history orders copy-small
lexer/frame first touches, cold typed source-owner swaps, row replacement and
reuse, retirement, rollback/redo, candidate settlement, and prefix release.
Only the first owner-changing transition of a checkpoint-visible source row is
retained in an interval; later transitions drop their displaced intermediate
owners, because rollback needs the initial owner and redo recovers the final
owner from that same swap. An interval-local row retains no owner inverse.
Alternate owners exist only as generation-checked inverse payloads, so
candidate redo restores the exact authoritative row without a second live
input representation or the generic logical-stack stored-state machinery.
Each source slot caches its current line's contribution to TeX's `buffer`, and
the input stack owns one scalar sum across live slots. Cold line/backing owner
and row transitions update those values; byte/scalar cursor advancement does
not inspect or recount the line. Stored-token advancement journals its logical
word position plus replay run/segment/in-segment coordinate on the first warm
touch in a checkpoint interval. If the
same row later takes a cold retirement, limit, or flag transition, a separately
ordered cold-state inverse captures that state once; rollback and candidate
redo preserve both transitions without constructing a complete token frame on
every word. Each input checkpoint retains the exact total
for direct rollback and candidate redo. Buffer high-water queries never walk
the input rows and no prefix ledger or shadow stack is retained.
The source first-touch inverse is at most 48 bytes. One `CommandState`-owned
resident transition borrows the `InputStack` and looks up and discriminates the
semantic top once. Its source branch lends the row and resident slot together,
while its concrete replay, durable, attempt, and macro-argument branches borrow
the admitted span directly;
each writes the caller's final command and advances the compact position before
that top borrow ends. The same transition settles fuel, suppression, alignment,
or parameter replay and returns only its final status. No cursor/token carrier,
intermediate delivery result, second top lookup, or diagnostic revision write
crosses that boundary. The stored branches have no wrapper or status of their
own: admission chooses the exact top-row tag once, and the warm branch performs one packed load
and scalar advance, intercepts a parameter in place, or performs one final
write with at most one dense meaning lookup. Replay words advance from the
resident physical coordinate: segment metadata is inspected only at crossed
boundaries, and an e-TeX prefix changes runs only once before its body. Lazy
diagnostic invalidation reads the advanced frame
or source lexer coordinate only if a cold publication coordinate is captured
or validated; resident source, stored-token, and macro-argument delivery carry
no diagnostic field.
Stored and macro-argument cursors read only required packed-frame scalars and
never copy the complete frame. The
common packed frame on every row carries the active external-source context;
source rows install it and replay rows inherit it at admission. Main-control
root-file eligibility therefore consumes the source fact delivered with the
command rather than walking input ancestry after delivery. The stack exposes
no raw mutable top or mutable index. Its allocation gate proves
that 4,096 warmed lexer mutations perform zero allocations and one inverse at
both one and 4,096 live source rows.
When that resident selection finds an exhausted ordinary token or macro-
argument row, it projects and pops the same coordinate, settles macro scratch,
replay completion, alignment, and observation, then loops on the new top. A
direct replay retirement returns its typed episode from that transition.
TeX82 §§325/390 stack conservation instead transfers the episode's single
completion fence to the exact future backup or macro-body identity before that
descendant is admitted; further final-token macro calls repeat the same
ownership transfer. The last descendant retirement consequently publishes the
episode once. Its `loc=null` test reads the already-admitted row's scalar
position and limit; it does not index replay storage, read a definition word,
or reclassify the row's storage domain. No ordinary resident advance polls a
completion flag or scans a pending-completion mailbox. The
same TeX82 §357 restart pops an exhausted v-template after §1131's `do_endv`
has inspected and released its retained boundary. Nothing returns an
exhaustion status through the processor, and no second top lookup or owner
validation occurs. Source EOF/file warning, terminal token and a v-template
still waiting for `do_endv`, and TeX82 §§325/390 backup/macro stack conservation
remain explicit cold branches.
While that source row is resident, it is the structural lifetime proof for its
occupied physical source slot: row retirement is the only release path.
Ordinary delivery uses that resident projection without repeating the slot's
ABA-generation comparison. Cold inverse, rollback/redo, and owner-swap paths
retain complete checked keys.
Nested source opening installs ancestry as part of the same frame transition
which updates the singular session-owned TeX82 input-stack maximum; before
retirement the processor borrows it and returns only copy-small common-prefix
coordinates for `file_warning`. There is no shared usage ledger, post-open
identity search, checkpoint source clone, or retirement-time ancestry clone. A durable token-list input now owns
only the list id, chunk cursor, and length. Macro replacement input owns a
definition coordinate; macro-argument input owns a sealed absolute scratch
range and advances only the packed input frame's scalar position.
Small source-adjacent replay owns only a compact coordinate into the
generation's segmented replay lane. Its traced words and optional source
provenance are written once at admission. Popping the level releases exactly
the top entry and returns whole unused segments to reusable high-water storage;
snapshot roots share immutable active segments and never relocate live words.
The input-stack vector may keep capacity for reuse, but it must not keep the
popped source backing.

Raw delivery writes directly into the active request's caller-owned
`CurrentCommand`. Admission consumes the `PackedTokenSpanHandle` into a
concrete replay, attempt, or durable input-row variant; stored delivery
dispatches from that top-row tag and projects the resident packed word into final
meaning and spelling fields, and advance their packed frame in place. Source
levels write the same destination after tokenization. Parameter interception
remains a local status before resolution and may push a literal argument level
before the resident transition returns. The reference-only empty-slot proof
retains no backing handle or cursor, needs no rollback record, and is never
moved into a typed suspension; cold input transitions return only copy-small
facts after its reborrow has ended.

Resolution consumes the empty-slot reborrow and returns only packed scalar
facts. The resident transition then borrows the same caller-owned command slot
directly. One settlement applies noexpand, outer validity,
alignment classification, and optional observation in canonical order;
the singular command-state transition first-touches the `align_state` rollback
scalar only for a literal brace, immediately before its adjustment, and stores
that exact adjustment on the final command for one later backup. Ordinary and
delimiter classifications append no delivery-owned scalar undo. Internal
recovery, ErrorStop deletion, math-shift lookahead, and
output-list draining supply their local final or discard slots directly and
create no returned command envelope.

The physical processor split follows those same transitions without splitting
ownership: `processor/expand.rs` owns one const-specialized raw/expanded
fetch-and-inspect state machine, while `processor/next.rs` owns the public raw
policy entries and resident-command observation; the `CommandState` resident
transition owns ordinary token/macro retirement and immediate retry;
`end_input.rs` owns cold source acquisition/EOF, terminal/v-template
retirement, and explicit stack conservation;
`outer_recovery.rs` owns scanner-status interception; `backup.rs` and
`recovery.rs` own exact replay insertion and executor-facing recovery; and
`alignment_interception.rs` owns the delimiter/v-template handoff. Every call
still borrows the same `CommandProcessor` and writes through the same caller
destination; the modules add no queue, facade, or second command value.

ErrorStop interaction is also an explicit ownership transition rather than a
property polled by raw delivery. A command-side report applies its typed
`Insert` or `Delete` outcome immediately through the live processor. An
executor-side report places the same single outcome in the operation-local
diagnostic handoff; the canonical executor/processor seam consumes it once
before the operation commits. The world error channel owns prompting and
rendering only, while the command machine remains the sole input-stack owner.

Main control likewise owns one call-local `Option<CurrentCommand>` final slot
for preflight, ordinary expansion, main-loop lookahead, alignment bodies,
prefixes, leader handoff, and `goto reswitch`. Preflight raw-fetches into that
slot, classifies it once, publishes an unexpandable result directly, and
continues through expansion in place only when classification requires it.
The general ordinary scanner then borrows the same slot in the same admitted
command context. A resource, transaction, diagnostic, alignment, or tracked
observation boundary stops before that scan and retains only the exact frame
state required by its established path. Delivery returns only a compact status;
an already-scanned hot or cold operation resides in the caller-owned operation
frame and bypasses the command-context front of operation preparation. Cold
resource rooting changes only resident root fields, and hot/cold application
consumes only semantic leaves without another command processor or whole-frame
move. Filler loops overwrite and reuse the command slot, and only the final
consumer, an explicit backup, or an exact typed suspension moves the settled
owner out. No command is cloned merely to cross the delivery API, and no
settled command is backed up or redelivered across preflight.

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

World checkpoints now own only scalar effect/input/artifact positions, fixed
stream cursor/offset and clock state, and undo-journal marks. Read streams name
immutable input records plus byte cursors, write streams name immutable
session path ids, and terminal/log state is the scalar column offset TeX
actually consults. A checkpoint owns no shared mutable stream buffer and
causes no first-mutation COW. Effect rows, aligned
publication sidecars, input records/content, and reduced dependency facts live
in coarse immutable accepted blocks; a candidate appends into private suffixes
and private counter/dependency journals. Within one World owner, selecting a
candidate drains the exact accepted suffix into reusable detached-prior
buffers whose capacity was warmed by the original semantic appends. Rejection
moves those payloads and journal writes back; acceptance clears the buffers
without releasing capacity. Thus candidate settlement performs no prefix scan,
`split_off`, payload clone, or per-checkpoint tail allocation, and heap payload
addresses remain stable across both paths. Source registrations use that same
accepted-block/private-suffix split. Loaded and generated immutable font
contexts instead live in fixed-capacity coarse chunks owned by the physical
generation. A logical font row holds only its chunk coordinate; rollback
truncates that coordinate and its mutable dense runtime row while leaving the
context address stable until whole-generation retirement. Font identifier and
expansion mutations are candidate overlays with exact reverse journals. Every
font-bearing meaning, node, and PDF record validates at publication, so
checkpoint capture and same-generation restore copy fixed font cursors without
scanning those roots. Dense source/font/input identities share run-compressed
accepted metadata and mint a fresh candidate run, so neither payload count nor
retained-boundary count is a fork-time copy. Provisional page-output receipts
cannot cross a quiescent
checkpoint; committed artifact bytes have already crossed exactly once to the
durable artifact ledger. Numbered write streams do not retain a redundant
partial-line mirror because TeX never consults it for wrapping or any later
semantic decision; terminal and log offsets remain exact direct live state.

A TeX group is both a save-and-restoration boundary and a local-definition
region boundary. The save journal records old compact values and exact group
entry/exit order. On exit it restores local meanings and preserves later global
assignments before the definition store retires every segment belonging to
that group. A checkpoint or active local macro input row may delay release of
the whole retired region, but no definition inside it has an individual owner.
Stored token-list lifetime remains carrier-based. `aftergroup` and
`afterassignment` payloads are command roots until their specified replay
point.

Command checkpoint envelopes do not describe those private roots. One retained
generation owner carries the attempt mark and exact ABA-checked timeline row,
and its copied cursor is only that row's serial. The timeline row stores each
logical stack's real `{top, undo}` rollback mark once. In particular, it stores
the aftergroup lane mark directly instead of counting or traversing group or
aftergroup payloads during capture or restore; replay, diagnostic, and
afterassignment occupancy are not duplicated as cursor fields.

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

Session admission retains the validated encoded container as the exact frozen
`JobStart` image. The first execution materializes destination-local
definitions, token lists, glue, fonts, nodes, dense values, PDF state, and
hyphenation state from those bytes, then publishes complete component identity.
Fresh jobs capture the same image after pre-job setup. The session binds the
image to explicit command profile, compatibility, and job-clock metadata and
charges image and metadata bytes separately from live checkpoint history.
`JobStart` itself remains evidence-only in that history: fallback cold-loads a
new independent generation, while ordinary candidates keep using the exclusive
prior/current fork path.

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
   `ExecutionScratch`. Its argument words occupy scratch segments A and B.
2. `\inner` seals another scratch frame and segment C. Its absolute ranges are
   nested semantically under `\outer`, but C belongs to the same bump stack,
   not a child arena.
3. The toks scanner owns phase/counter state and two typed branch coordinates
   in its parent's shared attempt token lane. Its unfinished state moves into an
   ABA-tagged `ExecutionScratch` slot; each enclosing expansion moves that
   non-`Copy` key into its own typed caller frame.
4. `PendingCommandAttempt` takes the attempt arena, cold-materialized fixed opening mark, resume
   coordinates, typed resource operation, and one coarse current-generation
   owner. Ordinary Rust locals and borrows end before control returns to the
   host. Scratch segments A--C remain where they are; nothing is copied.
5. On resume, the package is consumed and validated against the same
   generation. The scanner continues at its exact phase and appends each
   accepted semantic word directly to its existing parent destination
   `TokenListBuilder`; today the attempt sinks are promoted afterward.
6. If scanning fails, its unpublished destination chunks are discarded and
   may be reused. If it succeeds, sealing publishes a durable row and every
   chunk in that row stays until generation retirement.
7. `\inner` finishes first, so the live stack rewinds before C and returns its
   physical segment to the spare pool. `\outer` may reuse C later while A and
   B remain live. When `\outer`'s replacement input retires, the stack rewinds
   before A and B. The durable toks list is unaffected.

No step creates a scanner arena, promotes a scratch chunk, copies an arena
owner into each frame, or treats A, B, or C as revision history.

Checkpoint history retains only frame versions that an observable mark can
name. `LogicalStack` admits a frame into one reusable physical row and tags
that row with the current checkpoint interval. A push after a pop overwrites
the row directly when it was admitted or already replaced in that same
interval and its current occupant has no compact or stored inverse: no
intervening checkpoint or operation mark can observe the old payload. A
partially captured occupant is replacement-visible even in that interval, so
reuse moves it aside before its inverse can address the new row. The first replacement of a row visible at a mark moves its old
payload into one generation-checked slab slot and journals only that handle;
later replacements and mutable cursor, phase, or status changes coalesce in
the same interval. Acceptance, rejection, or rollback releases the required
old version without scanning roots, cloning `InputLevel`, or retaining a third
lineage. Memory therefore follows physical stack high water plus one required
version per marked depth and interval, never unobserved push/pop count.

## Retention audit

This is a source audit, not an RSS measurement. **Fact** means the ownership
and release path are directly visible in the cited source. **Impact hypothesis**
means the topology is verified but workload size or frequency still needs a
benchmark, heap profile, or counter. The category numbers are:

1. intentional coarse-owner retention until wholesale retirement;
2. bounded reusable high-water capacity;
3. stale or unreachable payload still retained by a live owner; and
4. true unbounded/per-operation growth or an API which permits it.

| Category and confidence                        | Owner/type and source                                                                                                                                                                                                           | Retention trigger and release today                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | Model verdict and likely impact                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | Principled correction                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| ---------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Resolved, fact                                 | `DefinitionRef<G>` keys in format/revision-global/forked-local regions, and exact-owner `TokenListId<G>` values                                                                                                                 | Definition keys are copy-small and non-owning; format/global ownership is structural, while active local macro-input rows pin a whole region. Token lists retain their private exact non-atomic owners                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | Definition rollback truncates arena marks and `endgroup` retires whole local regions after meaning restore; token-list last-owner drop remains exact                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | Preserve compact definition keys, borrowed views, coarse local leases, exact token-list carrier ownership, and two generations. Do not add per-definition `Rc`, `Arc`, `Weak`, atomics, owner registries, tracing, compaction, relocation, or rehome                                                                                                                                                                                                                            |
| Resolved, fact                                 | Initialized trie owner and bounded runtime mutation value in `crates/tex-state/src/hyphenation.rs`                                                                                                                              | TeX82 §919/§1335 moves the completed pattern map into one coarse immutable `Arc`; runtime exceptions are capped by `hyph_size`, and saved codes contain at most 256 byte-character mappings for each of 256 languages                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | Aggregate capture/clone/restore and prior-to-current fork clone no initialized node, edge, or value vector. The fixed mutable exception/code workload is copied independently for exact rollback and fork isolation. The explicit `hyphen_checkpoint_gate` reports identical allocation calls and requested bytes at 64 and 7,000 initialized pattern nodes for all four operations                                                                                                                                                                                                                                                                                        | Charge initialized pattern payload bytes once to their coarse owner in aggregate retained-byte accounting, and charge each checkpoint only for its bounded mutable exception/code value plus the shared-owner handle. Preserve the two-generation lifecycle and do not add per-value owners, a registry, compaction, read-time COW, or another indirection inside trie rows                                                                                                     |
| 1, fact                                        | `SessionInternerEpoch`/`Interner` in `crates/tex-state/src/{session_epoch,interner}.rs`                                                                                                                                         | First intern retains spelling/hash entry through all revisions; epoch retirement releases it                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          | Intended session stability, bounded by explicit interner budgets; stale spellings can remain but are not a revision-generation leak                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | Preserve one bounded epoch; detach spellings at cold boundaries rather than cloning interners                                                                                                                                                                                                                                                                                                                                                                                   |
| 1, fact                                        | Accepted `EffectJournal`, `World` effects, source fragments protected by retained checkpoints                                                                                                                                   | Semantic publication or checkpoint reachability keeps output/source evidence until output disposal, checkpoint pruning, generation retirement, or session drop                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | Intended externally observable/document-linear retention. Source fragment bytes are pruned when no accepted layout or retained checkpoint needs them; retired metadata has a budget                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | Keep exact roots and budgets; measure output retention separately from expansion scratch                                                                                                                                                                                                                                                                                                                                                                                        |
| 2, fact                                        | `ExecutionScratch` macro slots, absolute-offset segment stack, sealing lane, and spare-segment pool in `crates/tex-command/src/execution_scratch.rs`                                                                            | A deeper/larger set of simultaneously live macro arguments grows storage; strict-LIFO frame return rewinds the logical suffix while physical segment and descriptor capacities end with generation                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    | Correct reusable generation high water, bounded by maximum simultaneously live macro-argument demand. Sealing and warmed indexed delivery allocate and copy zero bytes                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | Preserve the single scratch owner, direct segment/slot access, and runtime stale-frame rejection; expose counters/budgets if adversarial nesting needs a hard ceiling                                                                                                                                                                                                                                                                                                           |
| 2, fact; resolved                              | `AttemptArena::token_lane` in `crates/tex-command/src/attempt.rs` and `crates/tex-command/src/attempt/token_lane.rs`                                                                                                            | Scanner rows hold only typed branch coordinates into one attempt-owned fixed-chunk lane. Commit publishes the existing branch; rollback or cancellation truncates rows and returns complete chunks                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    | Retained payload is reusable generation high water bounded by maximum simultaneous scanner output. There is no stale per-scanner payload or word-vector owner, and warmed one/4,096-scan evidence allocates and transfers zero words                                                                                                                                                                                                                                                                                                                                                                                                                                       | Preserve destination-directed append, metadata-only seal, chunk-granular truncation, and typed suspension. Do not add promotion, rehome, compaction, a root registry, duplicate lists, or per-scan heap owners                                                                                                                                                                                                                                                                  |
| 2, fact                                        | Popped input/pending/diagnostic vectors in `crates/tex-command/src/{input/mod,state}.rs` and `DiagnosticEffects` in `crates/tex-state/src/diagnostic.rs:67`                                                                     | Pop/drain removes payload but retains vector allocation; state/generation or operation owner drop releases capacity                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | Reusable stack/queue high water. Live pending entries are future-relevant; no stale source payload after an ordinary pop was found                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | Keep typed stacks where needed, shrink only at a cold quiescent boundary; replace hidden pending scanner/expansion mailboxes with explicit continuation fields                                                                                                                                                                                                                                                                                                                  |
| Resolved, fact                                 | Split `SaveJournal` storage in `crates/tex-state/src/journal.rs`                                                                                                                                                                | TeX saves occupy reusable per-group segments and retire whole at exit; checkpoint intervals retain one first-before delta per written cell; nested operation attempts retain only their ordered suffix until commit or rollback. Exact group, checkpoint-pool, and operation capacity scalars change only with their physical capacities                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | The former 112-byte mixed generation-long row and 56 MiB vector high water are removed. Dense reads remain unchanged, stable checkpoint cursors pin only legitimate open-group segments, and the ordinary level-zero policy leaves no closed group history retained. Per-command budget enforcement reads three scalars instead of walking every live and spare segment                                                                                                                                                                                                                                                                                                    | Preserve direct reads, reverse rollback, packed entries, O(1) append and budget projection, whole-segment retirement, and profiling of live/spare capacity; do not add overlays, censuses on ordinary commands or checkpoint capture, densification, compaction, forwarding, or per-entry ownership                                                                                                                                                                             |
| Target corrected by `node_region_ownership.md` | `PageRegion`, exclusive durable node regions, `ShipoutScratchArena<G>`, and typed shipout sources in `tex-state`/`tex-exec`                                                                                                     | Each page-building period owns one move-only region; its paragraph checkpoints occupy one contiguous history interval and share payload once. Shipout-only rows reuse stable scratch buffers and are unreachable from semantic carriers                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               | Handle-free output plus held-over evacuation releases an uncheckpointed page immediately; pruning the last row drops a historical page region wholesale; generation replacement drops all remaining page/scratch storage                                                                                                                                                                                                                                                                                                                                                                                                                                                   | Preserve typed borrowed coordinates, direct scratch construction, source-token streaming, atomic payload/descriptor settlement, and the raw-root compile-fail boundary. Do not add a page-batch graph, reference count, root registry, or per-row reclamation                                                                                                                                                                                                                   |
| Target corrected by `node_region_ownership.md` | Ordinary box, unbox, setbox, page, math, alignment, PDF-form, and node-token lifetime transitions                                                                                                                               | Runtime list coordinates are borrowed under exclusive page/durable regions; node token fields share immutable stored-token allocations                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | Unique consuming moves transfer regions without copies. `\copy`/`\unhcopy`, a move whose source is retained by history, and a retained nested child copy exactly the bounded recursive closure                                                                                                                                                                                                                                                                                                                                                                                                                                                                             | Preserve exact TeX deep copy versus consuming transfer, group/save-journal ownership, rollback, region-local child closures, and cold-only format materialization; never restore live relocation or token-word rebuilding                                                                                                                                                                                                                                                       |
| Target corrected by `node_region_ownership.md` | Superseded or rollback-abandoned durable immutable values and core/node checkpoint state                                                                                                                                        | Definition regions own payload structurally and token-list carriers release private shared payloads on last drop. Dense state remains one directly journaled owner. Checkpoint history directly owns exclusive page regions and durable save-journal closures; a checkpoint row retains only fixed marks and owner-relative roots. Glue/provenance retain direct-index rows with bounded publisher cursors. The completed primitive registry is one immutable shared root                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             | Candidate fork, restore, reject, accept, and first mutation allocate independently of accumulated dense writes, save history, glue/provenance rows, unchanged page-node prefixes, and primitive rows. Rejection/retry restores exact region ids and chunk generations. There is no ordinary-read overlay, first-write COW, per-value owner, prefix replay, coordinate relocation, or page-batch count                                                                                                                                                                                                                                                                      | Preserve direct dense reads, exclusive page/durable regions, exact private-suffix journals/cursors, direct history ownership, and the two-lineage bound. Publication and retained-region accounting belong to checkpoint history; do not move them back into candidate construction or raw root coordinates                                                                                                                                                                     |
| 2, fact; resolved                              | `CommandGenerationOwner`, `CommandTimeline`, `PackedJournal`, and `LogicalStack` in `crates/tex-command/src/{snapshot,scalar_journal,timeline}.rs` plus the parked command slot in `crates/tex-exec/src/retained_generation.rs` | One `CommandState` owns checkpoint frames in generation-checked reusable 128-row pages and descriptor-free scalar undo/redo in reused fixed chunks. Named-boundary publication appends one frame with fixed logical-stack and journal marks. Dense first-touch bits coalesce safe same-cell writes to first-old/final-new; the large pending filename uses its own chunk lane, while ordered diagnostic pushes remain non-coalescible. Aggregate release returns the obsolete frame row and advances each journal/logical-stack base to the earliest surviving live root. Whole prefix chunks return to their pools and their old marks fail closed; frozen `JobStart` needs no live cursor. The retained generation parks the sole physical command owner while a checkpoint is idle; candidate creation moves it into the current borrower, detaches the prior suffix, and rejection or acceptance settles that suffix in place. Input, parameter, condition, group, aftergroup, and alignment payloads stay in append storage after a logical pop until their containing prefix becomes releasable | Scalar mutations publish zero arena list descriptors or per-event heap owners. Warmed capture, same-generation restore, command fork, first mutation, fork-plus-first-mutation, and warmed prefix release allocate zero accumulated payload. Reverse rollback and forward redo preserve exact state and ordered-diagnostic order; stable surviving-floor marks remain valid across release and reuse, released row/mark generations fail closed, and suspension keeps the typed command owner reachable through the retained-generation sidecar. No checkpoint carries an aliasing owner, and no payload prefix is cloned or replayed outside the explicit edit transition | Preserve direct mutation, reusable fixed frame rows, dense interval-local first touch, explicit non-coalescible ordering, thread confinement, move-only owner handoff, reverse undo/forward redo, and the prior/current lineage bound. Keep `ForkArena` for stable-range data, not scalar history. Do not add an aggregate-root clone, shared mutable tail, first-write COW, compactor, root registry, per-frame heap owner, ordinary-path atomic admission, or a third lineage |
| Target corrected by `node_region_ownership.md` | Coarse `ModeNestStorage`, `PageBuilderState`, and exclusive page-region history in `tex-exec` and `tex-state`                                                                                                                   | Eligible named boundaries retain one rootless outer-mode mark plus owner-relative PageBuilder roots in their current page region. Candidate fork moves the accepted region/history authority into one transaction, detaches the selected suffix and later page regions, and rejection returns it after rollback                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | Capture copies fixed runtime/page marks but no node. Multiple paragraph boundaries in a page share its region. Acceptance/rejection settle PageBuilder roots before payload/descriptors; publication charges every page region once, fixed marks/counters per restart root, and detached evidence separately                                                                                                                                                                                                                                                                                                                                                               | Preserve short typed mode read/write guards, direct checkpoint-history region ownership, rollback before returning a rejected owner, one-way handle-free shipout detachment, and flat accumulated-state allocation gates. Optional convergence identity must use complete journal-maintained component roots and fail closed while any root is absent. Do not expose `RefCell`, add page-batch counts, COW, compaction, root registration, or a third lineage                   |
| Resolved for payload retention                 | Format capture in `crates/tex-state/src/format.rs` and `StateCore::capture_format_values`                                                                                                                                       | Cold capture visits semantic state, node recipes, and PDF-only token/node roots; live payloads serialize at stable serial coordinates and dead serials become empty compatibility rows                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | Detached images no longer keep stale definition/token bytes, and materialization does not publish dead definitions. Empty coordinate holes preserve schema references without relocating live handles                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | Preserve the cold handle-free traversal and PDF root coverage; do not turn it into runtime liveness authority or live-handle relocation                                                                                                                                                                                                                                                                                                                                         |
| Resolved, fact                                 | Frozen `DetachedFormatImage` bytes in `tex-incr::FrozenJobStartAnchor` plus destination staging                                                                                                                                 | Session admission retains exactly one immutable encoded image and explicit profile/compatibility/job-clock metadata. JobStart fallback decodes it into an independent current generation; decoded staging disappears after atomic publication                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | The separately charged image replaces a live bootstrap generation/checkpoint root. Runtime journals can release and physically reuse obsolete prefixes while format-loaded, fresh, dense, hyphenation, font, PDF, and page state retain byte-exact fallback identity                                                                                                                                                                                                                                                                                                                                                                                                       | Preserve the exact immutable image, explicit metadata validation, cold atomic materialization, complete component identity, and ordinary prior/current bound. Do not add a decoded session overlay, format-owned runtime generation, root registry, or streaming decode unless a later measured cold-input peak independently requires it                                                                                                                                       |
| 1, fact                                        | `PendingCommandAttempt` in `crates/tex-command/src/attempt.rs:1515` and executor pending slots                                                                                                                                  | Suspension retains one boxed attempt, typed payload, and coarse generation owner; resume consumes it, cancellation/drop releases it                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | Intentional exact continuation retention. It can delay retirement of precisely one candidate generation but is not a leak                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | Keep move-only typed continuations; guarantee cancellation drops them before candidate retirement                                                                                                                                                                                                                                                                                                                                                                               |
| Resolved, fact                                 | `CandidateLeaseState`/`CandidateLease` and public candidate creation in `crates/tex-incr/src/{candidate_lease,lib}.rs`                                                                                                          | The session owns one atomic slot; the move-only lease transfers from candidate to transaction and releases on acceptance, explicit rejection, failure, or drop                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | Exactly one current slot can coexist with the optional prior. A second factory fails before issuing a candidate or generation; repeated claims reuse the one session allocation                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | Implemented. Keep lifecycle, 8,192-cycle reuse, prior-plus-current high-water, and second-factory rejection controls active                                                                                                                                                                                                                                                                                                                                                     |
| 1/2, fact                                      | `PureMemoRuntime` and `RenderMapCache` in `crates/tex-state/src/pure_memo.rs` and `crates/tex-incr/src/lib.rs`                                                                                                                  | Validated results remain until byte/entry eviction, explicit clear, or session drop; container capacity may remain after clear                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | Intentional bounded cache retention, not semantic ownership and not a generation leak                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | Preserve hard budgets and handle-free entries; make retained-byte metrics include container overhead where useful                                                                                                                                                                                                                                                                                                                                                               |
| No defect found, fact                          | Input/source stack and registrations in `crates/tex-command/src/input`                                                                                                                                                          | Source EOF pops and drops the level; opening a pending source removes it from the map; never-opened registrations end with command state. V-template and `every_eof` retention follow TeX semantics                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | No stale popped source owner or per-line leak was found. Input vector capacity is category 2; pending registrations are still callable                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | Keep exact retirement classifications and add a counter/profile before changing source retention                                                                                                                                                                                                                                                                                                                                                                                |
| Resolved, fact                                 | Semantic diagnostics, operation observations, and pending resource continuation                                                                                                                                                 | A completed processor episode moves the semantic-diagnostic `Vec` allocation wholesale to the executor and leaves command state with a fresh empty queue; rollback/drop discards it, while suspension retains exact typed state                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | The former drain/collect element-copy seam is absent, with no duplicated diagnostic representation or unbounded per-operation diagnostic leak. Published effects can grow with real output                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | Keep transactional whole-owner delivery and type each pending field; measure effect-ledger growth as output, not scratch                                                                                                                                                                                                                                                                                                                                                        |

The command checkpoint timeline keeps one stable reusable frame-row arena with
direct accepted-order links. Root selection detaches its later accepted chain
without searching or copying a key index, candidate publication links only a
private current chain, rejection cancels any retained `ExpansionWork` before
undoing that chain and redoing the detached prior journals, and acceptance
releases obsolete journal chunks while retaining candidate marks. Either
settlement moves the discarded frame chain's existing head, tail, and length
to one reusable-chain owner in constant work. It visits no discarded row and
manufactures no row key; the next publication takes one row lazily and assigns
its fresh incarnation then. Production
non-JobStart coverage crosses input, macro-definition, scanner, condition,
alignment, mode/page, suspension, and cancellation boundaries; the standalone
gate exposes exact delta work, identical one-versus-4,096 settlement counters,
zero settlement allocations, and the single lazy reuse visit/incarnation.

The external reachability-store prerequisite, append-only command-timeline
inconsistency, and aliasable immutable payload retention are resolved.
Prior/current share one physical owner domain; checkpoint pruning drops its
direct command-root owner; definition and token-list payloads follow their
exact semantic carriers; and cold format capture includes state, node, and PDF
roots without retaining dead payload bytes. No compactor, forwarding scheme,
tracing collector, ownership registry, relocation, rehome, or third generation
is part of the correction.

Exact TeX main-memory totals follow those same lifecycle facts. Definition
regions add each sealed span's precomputed canonical charge and subtract it on
checkpoint truncation or whole-region release. Stored-token publication adds
its charge once and its real last shared owner subtracts it; node arenas add or
subtract row charges at publication and release. The single generation-local
aggregate stores no ids or roots. Ordinary box and page work reads it directly;
it no longer scans meanings, the 65,536 token and box registers, payload
identities, or node closures merely to discover the current total.

Borrow-scoped host preparation similarly owns no copied page-insertion map.
Expansion-time insertion enquiries read the authoritative row through the live
`CommandContext`; the borrow ends with that processor episode and no projection
survives list mutation, suspension, or error re-entry.

## Non-negotiable prohibitions

- No compactor, in-place relocation, forwarding pointer, live-id rewrite, or
  cross-generation rehome.
- No attempt-local definition body or publication copy beside the selected
  transactional definition region. Sealing appends only its compact header and
  returns the final key without traversing or copying its words.
- No per-macro, per-scanner, or per-operation arena. Forked local definition
  regions are owned only by their TeX groups and retire as whole regions.
- No untyped or hidden `Vec` mailbox for suspended semantic payload.
- No historical runtime generation beyond the optional prior and exclusive
  current candidate.
- No treating a backing chunk as a revision, checkpoint, independently
  reclaimable history segment, or lifetime owner.
- No stored Rust reference, per-value `Arc`/`Weak`, atomic ownership, or
  liveness lookup on the ordinary expansion path. Private non-atomic shared
  handles remain only where stored token lists require exact alias ownership;
  definitions use compact structural-region keys.
- No heap allocation after bounded hot-path warmup except at an explicitly
  cold host, diagnostic, format, or output boundary.
- No runtime id in a format, detached continuation, memo wire value, output,
  process message, or thread message.

These restrictions leave three deliberate reclamation policies: exact arena
reuse for scratch, checkpoint or whole-region retirement for definition spans,
and last-semantic-owner drop for stored token lists. Coarse generation
retirement remains the final boundary for compact inline arenas and publisher
scratch capacity.
