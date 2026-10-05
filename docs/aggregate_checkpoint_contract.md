# Full checkpoints and TeX group ownership

Status: implemented ownership contract, 2026-10-02.
The source mapping below names the owning implementations and their proof
responsibilities; Beads records revision-scoped performance evidence separately.

## One authority and one mutable generation

One job/checkpoint family owns one working mutable engine generation, its
immutable loaded format, and retained named checkpoints. Independent jobs or
format admissions may have their own generations. An edit temporarily retains the accepted
alternate values and cold captures while the same engine executes a candidate.
Accepted and candidate are two histories, not two mutable eqtb banks or two
interpreters. Only one candidate transaction can be active.

This contract supersedes earlier two-slot _target_ architectures, fixed-size
command-capture claims, group-retained checkpoint proposals, token-reference
rollback epochs/quarantine, and prohibitions on direct definition root counts.
Current `Universe::fork_runtime_checkpoint` already moves the sole
`StateCore`, page, World and other mutable owners into candidate execution;
`StateCore::begin_checkpoint_candidate` reverse-swaps the same dense banks.
The second retained session slot holds a cold/loaned aggregate wrapper, not a
second eqtb. Preserve necessary coarse generation/checkpoint owners, stable
branded IDs and cold slot admission lifetimes. No session/FFI API rewrite is
required merely to remove the old document's two-bank wording. Delete actual
duplicate captures/topology, not genuine cold owners. No second interpreter,
compatibility wrapper, old-format importer or parallel storage authority is
justified. Specialized node, PDF, provenance, resource, and
format contracts retain their actual semantic invariants; their session
ownership is governed here.

The hot machine reads packed tokens and meanings directly. TeX assignments
share one mutation policy. That policy writes two compact histories only when
their distinct semantic lifetimes require them:

- `GroupSaveStack`: a contiguous, reusable LIFO sequence for TeX local scope;
- `CheckpointUndoLog`: first-touch preimages for retained checkpoint intervals.

These are not two assignment engines. They have one owner policy and different
preimages. With checkpoint `x=0`, global `x=1`, then group-local `x=2`, group
exit needs `1`, whereas checkpoint restore needs `0`. Sharing one linked entry
between both histories would require multiple memberships and freeing rules
without eliminating those two values. Keep the sequential storage instead.

## Concrete state and mutation layout

A cell has a packed value and TeX assignment level. Its host checkpoint stamp
lives in a parallel indexed metadata bank, away from meaning-only reads. Use
one monotone `u64` host epoch per generation. Dense banks index metadata
directly; extended register and font-runtime pages allocate their corresponding
metadata page once. Box binding and scalar-lineage metadata use direct indexed
rows, replacing stamp hash maps. No assignment hashes a key or searches history.

`CellKey` uses the existing packed family/index scheme in
`tex-state/src/journal/cell.rs`, including code-table kind and font operands.
It must retain the complete coordinate; the old proposed `u32` key cannot
represent every current family. A common scalar restore record contains the
key, packed old value, and old TeX level. It contains no old host epoch,
checkpoint lease, owner serial, linked-list membership, or `Rc` per scalar.
Typed token/glue/font/box values preserve their real ownership: a box record
moves an exclusive region owner through a typed annex, never treats its naked
list coordinate as an owner. Fixed record slabs and sequential annexes retain
capacity after pop; growth is paid when a real save crosses capacity.

One validated assignment performs the following:

1. Resolve effective local/global scope, including `\globaldefs`, and validate
   the typed value and any required publication capacity before mutation.
2. If a retained restart needs undo and `stamp[cell] != host_epoch`, append the
   old value and TeX level to the checkpoint log and stamp the cell. The log
   owns that preimage. Without a retained restart, no checkpoint entry is
   produced.
3. For a local write below level one, save the old value and level only when
   `old_level != current_level`. The group entry independently owns its
   preimage. Repeated writes at the same level append no group entry.
4. Install the value with level one for a global write, otherwise current
   level. Acquire new semantic roots before releasing displaced roots. Alias
   and ownership-transfer operations use the same policy.

Compute the two predicates before replacing the cell; append/acquire owners
before any decrement can tombstone the old definition. When both histories
need a preimage, two shallow values are necessary. A repeated write still pays
one level predicate and one indexed stamp comparison; it performs no history
append. Root-level writes never need a group record.

At capture, restore, candidate admission, and candidate settlement, reserve a
fresh host epoch. Never serialize it into a format, hash it as semantic state,
or restore it from TeX history. On exhaustion, fail at the cold boundary before
mutation; do not wrap and revalidate old stamps.

Macro arguments use invocation-owned pooled region vectors in the retained
`ArgumentArena`. The macro body frame owns its invocation's argument region;
parameter frames borrow that region. Construct all argument words and their
at most nine ranges directly in this final owner, without a call-local range
array, separate parameter vector, per-argument allocation, second copy, or
lease. One scalar live parameter count supplies TeX capacity and high-water
bookkeeping; body retirement subtracts its region’s count. Named
checkpoint admission proves arguments empty. Returned vector capacities form
an allocator cache charged once to the engine, not semantic checkpoint state.
The `.10` implementation details may refine this physical pool without
creating argument undo history.

## The real TeX save stack

A group pushes one scalar boundary containing level, kind, source line,
lineage, journal start, and required diagnostic projections. Empty groups touch
no definition store, box group owner, fifteen-owner operation snapshot, or
checkpoint log. Warm empty push/pop allocates nothing. Typed builder groups
still own their actual box, math, alignment, insertion, or output state; this
does not require captures for an ordinary empty group.

The save-word projection keeps its actual total and the word count of the last
checked push. That count is zero before any push, one for a boundary,
aftergroup word, sparse marker or zero-level restore, and two for an old-value
or box restore. It fits directly in `u8`; no optional position/count pair is
needed. A boundary retains the prior total and prior last-push count for
unsave. Box-spec words remain with their actual box push/pop bookkeeping.

Chronology belongs to the contiguous native stack. Its length is its sole
position; journals, checkpoint cursors and accepted tails retain no duplicate
save-position counter. Native append still checks the real `u32` save-index
bound. Named checkpoint validation requires the empty save projection alongside
the existing root depth, full owner identity and retained checkpoint cut.

Tagged stack records cover local restores, `\aftergroup` tokens, typed builder
payloads, and e-TeX sparse restoration. A sparse save lazily creates its
TeX-required restore marker and links its preimages by stack index under that
one group owner. At that marker, restore the sparse chain in reference order.
Classic scalar and box saves share their actual push chronology. Simply
reversing all classic and extended register writes would change e-TeX tracing
order; preserve the dense/sparse boundary implemented today in
`env/restoration.rs`, without its scalar/box merge scratch or duplicate group
stacks. This sparse chain is language semantics, not a graph shared with the
checkpoint log.

Group close validates kind and boundary before mutation, decrements the effective assignment level at TeX82 §281, then walks the save
stack in TeX order. The physical closing boundary still supplies `cur_group` until §282 reaches that boundary. A current level-one binding suppresses a saved local value;
otherwise transfer the saved value and its old TeX level into the current
cell. Release each losing owner exactly once. A later local assignment after
a global write can create another save in the same group: restoring that
level-one value then suppresses the older local save, as TeX requires.

Group unsave stamps the restored cell with the _current_ host epoch. Proof:
checkpoint captures are forbidden while a group is open, so the epoch cannot
change between group entry and exit. Any cell the group restores was written
in that interval and already has the checkpoint preimage if one is required.
A group save restores value and TeX level, not checkpoint membership. Resource
or candidate restore abandons the entire group tail before advancing the epoch.
No old stamp belongs in a group payload or an extra unsave checkpoint record.

Emit real restoration diagnostics synchronously after each decision, using
that entry's now-current tracing/print parameters while saved and live owners
are still valid. The unobserved path constructs no restoration collection.
Delete `GroupRestorations` and its immediately consumed executor carrier.
Any genuinely retained diagnostic result must contain owned text or an explicit
payload owner, not an uncounted `ResolvedMeaning` coordinate.

Popped `\aftergroup` tokens go directly to ordinary backup input in TeX order;
multiple saved tokens execute in their original saving order. Do not build a
second token vector merely to consume it immediately. `\afterassignment` is
one pending semantic token, distinct from group history. Successful assignment
consumes it once at the established input point; failed scanning or assignment
does not consume it. Preserve box-construction application timing, group
mismatch recovery, source diagnostics, save-stack overflow, and high-water
projections.

Box dimension writes are unscoped scalar mutations of the current box lineage,
not local binding assignments. Restore binding owners before checkpoint scalar
preimages, and apply those scalars only to the matching restored lineage.
Destructive box/unbox moves preserve a closure only when an actual history
owner needs it; ordinary groups alone do not pin current boxes. See
[Durable box root mutation](durable_box_root_mutation.md) and
[Node-region ownership](node_region_ownership.md).

## Full checkpoint inventory and capture cost

`EngineCheckpoint` is the aggregate restart authority. Component marks cannot
be restored or settled independently. Existing legal boundaries remain
`JobStart` and eligible main-source `OuterParagraphEnd`. Shipout publishes
output evidence and no restart root.
Admission requires no active group, macro, scanner/build/attempt, alignment,
replay completion, pending diagnostic, or unfinished output operation, and one
empty outer vertical mode. Root-document/include source-role policy remains.
Open conditionals and non-macro backed-up/stored input may remain.

The inventory maps the actual fields of `tex-exec/src/checkpoint.rs`,
`tex-state/src/universe.rs`, and `tex-command/src/snapshot{,/boundary}.rs`:

| Owner                                          | Checkpoint content and retention                                                                                                                                                                                                                                                                           | Restore obligation                                                                                                                                                     |
| ---------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Session, generation, format, profile           | Coarse identity/capability, immutable format and explicit job metadata; undo interval position. No eqtb image per named checkpoint; keep stable generation brands/coarse owners.                                                                                                                           | Validate session, owner, profile/schema, branch ancestry, and every retained position.                                                                                 |
| Dense TeX environment                          | Meanings, all register/parameter/code banks, current/family fonts and mutable font-runtime cells through first-touch undo; box binding and dimension-lineage inverses.                                                                                                                                     | Restore values/levels and transfer their semantic owners; no bank census.                                                                                              |
| Immutable definitions, lists, glue, provenance | Current cells and history entries own semantic roots; frame/capture payloads own their real immutable storage. Publisher/annex marks retain compact glue/provenance rows under their specialized policy.                                                                                                   | Preserve roots before suffix release; never restore a tombstoned definition from an unowned coordinate.                                                                |
| Command and input                              | Actual cold `InputCapture`, source/stored/backup frame positions and owned transient words, conditional frames, alignment scalars, pending afterassignment, last diagnostic site. Groups/aftergroup/builders are empty. Job configuration and pending-source registrations have explicit session lifetime. | Install the complete capture; clear abandoned scanner, group, builder, replay, and diagnostic state. Preserve registered future sources and input identity high water. |
| Executor and modes                             | `ModeCheckpoint` outer scalar; executor root anchor and budget counters. No open builder, mode list, or save stack is retained.                                                                                                                                                                            | Replace outer mode before reclaiming backing nodes; reset transient executor state.                                                                                    |
| Page, nodes, durable boxes/forms               | `PageRegionCheckpointKey`, four PageBuilder roots, contributions/discards, insertions/marks/break state and node/descriptor positions; exact region ownership through history.                                                                                                                             | Transfer roots while backing regions remain attached; settle root inverses before payload suffixes.                                                                    |
| PDF                                            | `PdfStateSnapshot` general/color roots, scalars, row selections, payload frontier and form owners.                                                                                                                                                                                                         | Select roots, settle mutable rows, release lookup/version/row suffixes, then payloads. Preserve canonical object order.                                                |
| World and resources                            | `WorldSnapshot` effects, stream/input/artifact/publication marks, clocks/randomness, framing and host policy; host replay transaction prefix is paired with the root.                                                                                                                                      | Rewind semantic effects and streams before dropping abandoned payload; verified resource outcomes remain outside engine rollback.                                      |
| Source and fonts                               | `SourceMapMark`, `FontStoreMark`, immutable source/font owners and mutable font journals.                                                                                                                                                                                                                  | Restore all meaning/input/node/PDF roots before logical source/font suffix release.                                                                                    |
| Hyphenation                                    | Frozen initialized trie owner, exception/code undo position, capacities and initialization scalars.                                                                                                                                                                                                        | Restore mutable state and pattern initialization semantics; share immutable pattern payload.                                                                           |
| Dependencies and engine scalars                | Tracker root/mark, interaction mode, prepared magnification, engine-usage state, optional complete component identities.                                                                                                                                                                                   | Restore before observation or incremental publication; no active dependency recorder crosses capture.                                                                  |
| Output ledger                                  | `OutputLedgerCheckpoint`, root/effect/artifact prefixes and generated-file/artifact transaction marks.                                                                                                                                                                                                     | Restore the same visible prefix as the engine; output publication occurs once after settlement.                                                                        |
| Fuel and high water                            | One session fuel ledger, cancellation/error policy and TeX job-lifetime stack maxima stay outside rollback. Configured revision counters are explicit cold metadata.                                                                                                                                       | Never refund spent fuel or reset diagnostic maxima on replay/rejection.                                                                                                |

A command mark may reference an owned capture row; that row's vectors and
transient words still cost memory and copying. Capture/restore costs
`O(live source/input rows + conditional rows + owned transient words)` for
those cold values, plus component marks and required checkpoint publication
work. Source bytes and immutable stored-list payloads share their real owners.
Do not claim every component is O(1), every capture is a few hundred bytes,
or cold captures have zero allocations. Ordinary token delivery maintains no
input undo history for these captures.

Frozen JobStart owns an exact detached image and explicit metadata. Fallback
materializes it atomically into the sole engine after retaining whatever
accepted alternates rejection requires. It is not a permanent decoded format
engine, a second runtime eqtb, or a live journal floor at zero. Format loading
and import remain cold; internal representation changes do not require an
old-format compatibility reader.

## Restore, repeated roots, and candidates

### Aggregate admission and commit

Capture preflights both the command publication serial and the runtime epoch,
page boundary identity, paired node/annex lanes and page journal before either
owner publishes a mark or releases a rootless suffix. The synchronous aggregate
call cannot change those owners between admission and publication. Allocation
failure follows the process allocator policy; it is not a recoverable semantic
error after partial publication.

Public named restore includes the output ledger. Component-only restoration is
test-local and cannot publish an editor restart. Output preflight verifies its
selected lineage, whole-chunk mark and absence of an active builder or batch.
Its private pool never escapes the ledger. Prepared output application is
infallible under that exclusive owner invariant.

Repeatable restart roots require the existing retained World session mode.
`WorldSnapshot` owns bounded lineage marks, not another effect payload copy.
An eager host publication can retire a captured prefix and legitimately make
that root inadmissible; an exported retained session is likewise sealed against
rollback. Admission rejects those roots before mutating any aggregate owner.

The executor discards interrupted builder and diagnostic owners after every
component passes preflight. Command/input and rootless mode installation then
precede runtime suffix reclamation. Interrupted native group saves are retired
by the dense restore without normal unsave, aftergroup delivery or diagnostics.
The selected page/PDF/World roots retain their own backing through their
specialized restore; no selected command or mode capture names page/PDF nodes.
Source/font truncation follows installation of all external roots.

Production capture occurs only after the bounded driver returns. A paragraph
cut is an intent, not a capture inside a scanner, publication, character flush
or semantic application. `take_paragraph_checkpoint` rechecks command,
executor, mode and runtime quiescence to produce eligibility. The ledger then
collects prepared pages; cold capture rechecks command, mode and runtime
admission before publication.
Borrowed core loans independently prevent runtime checkpoint admission. No
per-command continuation flag or publication-root census is needed.

Incremental work telemetry counts completed bounded driver chunks. It must not
label those chunks as tokens or TeX commands; deterministic command fuel keeps
its existing work unit and remains outside rollback.

Preflight every fallible component admission before mutation.
A failed preflight leaves the aggregate unchanged. First unwind interrupted
Rust calls; their scanners, collectors, borrowed contexts, and temporary
commands cannot cross restore. Abandon interrupted group state without
executing its aftergroup tokens or rendering ordinary group-exit diagnostics.

Restore has one aggregate admission followed by owner-specific commits:

1. Discard interrupted executor scratch while its backing remains attached;
   install command/input and the empty selected outer mode.
2. Restore dense values and native group history, then page, PDF/forms, World,
   hyphenation, dependency and execution-usage state. Each specialized owner
   preserves its selected roots before reclaiming its own losing suffix.
3. Truncate logical source/font suffixes after external roots are installed;
   commit the prepared output rewind. No fallible semantic action or external
   publication occurs after aggregate admission.

The aggregate uses `validate -> discard interrupted executor owners -> install
external roots -> restore dense/specialized roots -> release suffixes`.
A source/font or definition suffix cannot drop before a restored input,
diagnostic, node, or meaning transfers its root. Identity
metadata follows the selected semantic roots; host epochs do not.

A normal named restore reverses undo entries to the root and drops the
abandoned tail after root transfer. Keep the selected capture and protected
undo prefix intact. It can be restored repeatedly after different subsequent
writes, each in a fresh epoch. Restore invalidates only newer roots on the
abandoned branch, never the retained root itself.

For an edit candidate, reverse-swap the accepted suffix into an owned
contiguous tail. Each log alternate becomes the forward value needed to
recover the accepted head; its semantic owner moves with it. Retain accepted
cold command/input captures and actual specialized page/PDF/World suffix
owners, then execute the candidate in the sole engine with a fresh epoch.
Rejection reverses the candidate suffix, transfers/drops candidate roots,
reattaches accepted backing, and forward-swaps accepted entries in forward
order. Acceptance drops the retained accepted alternates and promotes current
history. Both settle every owner before publication and advance the epoch.
Swapping two existing owning positions changes no definition root count;
creating or destroying a position does.

Use plain cold tail owners wherever sufficient; delete checkpoint-log
`ForkArena`, leases, slot adapters and copied topology when contiguous owned
tails provide the same exact identity. Specialized page-region or PDF version
structures remain where their real semantics require them. This is not a
blanket deletion of node topology or output ordering.

History pruning uses the earliest surviving restart as the undo floor.
Retained prefix keys/positions stay valid; prefix removal drops preimage
owners exactly once and releases whole consumed storage chunks. Charge cold
captures per root, shared substrate/payload once per actual owner domain,
page regions once across their boundary interval, and accepted/candidate
private tails explicitly. Dropping the last root releases its protected
history; detached accepted output retains only its own bytes/recipes.

Resource misses retain a typed need and matching full root/host prefix. Restore
that aggregate, admit the verified outcome via ordinary VFS/World state, and
parse again. No partial parser, matcher, Rust caller, or output continuation is
retained. Synchronous provider resolution may complete the current cold call.
The contract is [Resource-miss checkpoint replay](checkpoint_resource_replay.md).

## Definition roots and prompt reclamation

Keep one pinned immutable format bank and one monotone dynamic slot bank.
Local/global allocation namespaces and local-to-global copies are unnecessary:
scope is the meaning cell's TeX level. `\let` to a global binding aliases the
same dynamic key. Delete local-row group marks/truncation and
`DefinitionPromotion`, including its weak memo, rather than retaining an
invalidation subsystem.

Each dynamic slot contains its payload option and a direct non-atomic root
count. `DefinitionRef` stays a compact copyable _nonowner_. Central owner
operations increment/decrement the indexed count for live meaning cells,
independent group and checkpoint preimages, candidate/accepted alternates,
and actual retained cold command/request owners. A newly sealed definition
has an unpublished temporary owner until assignment consumes it or failure
drops it. Audit every insertion, replacement, restore, transfer and drop.
There is no eqtb/log census on assignment, activation, or checkpoint capture.

At zero roots, tombstone the slot and drop its arena payload owner immediately.
Slot coordinates are never reused or truncated to allocate new definitions,
including after rejection. Small tombstone metadata grows with definitions
issued; payload bytes follow actual liveness. Capacity exhaustion remains
bounded and explicit. A stale ref deterministically fails rather than
resolving a different definition. Formats are pinned and use no dynamic count.

Active matching/replacement frames retain one `Rc` of the full immutable
payload: packed words, compiled matcher and origin together. They read slices
without counts or slot lookup per token, and remain valid after the arena
slot tombstones. Count a frame's actual payload owner, not a separate slot
root when it never resolves that key again. Ordinary borrowed
`CurrentCommand` keys can outlive no owner mutation unless they
acquire the needed payload first. Cold read/input placeholders must be removed
with direct dispatch, or become explicit counted owners while they remain.
There is no second matcher-plan store.

The definition bank is shared only by the arena publisher, dense state and its
two owning histories, the frozen primitive registry, and actual unpublished definition owners. These coarse
owners admit the indexed bank on root insertion or retirement; packed meaning
and scalar history records contain no shared-bank owner. History truncation,
pruning, candidate settlement and destruction retire the roots of precisely
the records they discard. Swapping current and alternate meanings preserves
both owning positions. Native unsave keeps its popped preimage alive through
the synchronous observer, then retires it. No ordinary lookup acquires a
semantic root. A cold view may retain the immutable payload to end bank
admission before publication ownership is consumed or dropped.

`PublishedDefinition` is a move-only temporary root returned by both direct
definition sealing and attempt publication. Scanner results and the native
read request move this owner through diagnostic reporting, character flushing
and assignment. Assignment first acquires the installed meaning's root, then
consumes the publication owner. Failure drops that same temporary owner.
Formats publish directly into their pinned bank; dynamic scanning has no scope
destination, and alias assignment keeps the original key.

Canonical definition memory is charged once per immutable payload and released
on its final real payload-owner drop, including an active frame after slot
retirement. Slot root counts are not payload `Rc` counts: aliases need not
allocate or duplicate memory charges. Existing row-drop accounting must
migrate with this policy; replacing a row must neither undercount an active
body nor retain its charge forever. The accounting hook updates only the
existing scalar aggregate, not a registry or store traversal. Retained-byte
reporting separately charges slot/history capacity and shared payload once.

## Operation frequency and hot costs

There is no verified production read/write/group/save/checkpoint ratio. The
588-page production profile ran with detail counters disabled. Historical
`main_control_dispatch.md` counts (35.9M commands, 38M macros, 1.6M group commands) have
no available revision-scoped census receipt and are not frequency evidence.
Do not infer percentages or savings from those numbers.

The following are explicit workload inferences: token/meaning reads and macro
expansion dominate ordinary LaTeX body execution; assignments are sparser than
reads. Local style/environment/scratch assignments are common _within
assignments_. Global/root writes are common while loading packages, creating
definitions, and updating counters. Groups can be frequent while distinct
first-written cells are sparse; repeated same-level writes do not save again.
Named host boundaries are much rarer than token traffic; candidate settlement
and resource replay are exceptional. These observations justify putting
bookkeeping on mutations and cold boundaries without inventing ratios.

| Operation                                    | Necessary bookkeeping                                                                    | Allocation/copy cost                                                 |
| -------------------------------------------- | ---------------------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| Token/meaning read, replacement substitution | Zero checkpoint stamps, logs or root counts; direct read.                                | None after frame admission.                                          |
| Macro activation/retirement                  | One actual immutable payload retain/drop and frame push/pop.                             | No body/matcher copy; no checkpoint log.                             |
| Assignment, already touched/current level    | Level predicate, indexed epoch comparison; actual root replacement when meaning changes. | No history append; no allocation with available bank storage.        |
| First local write                            | Above plus one group preimage owner.                                                     | Sequential append, shallow scalar/key copy; typed owners transfer.   |
| First interval write                         | Above plus one checkpoint preimage owner.                                                | Sequential append; both histories may need independent roots.        |
| Empty group                                  | Scalar boundary/kind/source push/pop and exact TeX diagnostics projection.               | No eager owner capture; warmed stack capacity reused.                |
| Group unsave                                 | Walk actual saves in TeX order; owner transfers; real demanded diagnostics.              | No restore-results vector or extra checkpoint preimages.             |
| Named capture/restore                        | Validate component marks; capture/install cold values; undo changed state.               | Actual cold vectors/words copied and charged; no eqtb deep snapshot. |
| Candidate/replay/prune                       | Aggregate tail swap/transfer/drop and owned cold captures.                               | Cold storage may grow; no per-token fork metadata.                   |

## Source deletions and implementation order

| Tracked work                         | Source now                                                                                                                                                                                                                                                                                      | Required replacement and deletion                                                                                                                                                                                                                                                                                                                                                                   |
| ------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `.10` arguments                      | `tex-command/src/macro_call.rs` and input argument activation still move collected argument words into frame storage.                                                                                                                                                                           | Construct each argument once in final retained storage; preserve origins and matcher semantics. Checkpoint design adds no argument history.                                                                                                                                                                                                                                                         |
| `.6` groups/mutations                | `tex-state/src/journal/groups.rs` owns the native contiguous boundary/scalar/box/aftergroup stack and sparse chain. `journal/checkpoints.rs` owns plain dense undo tails. Specialized box checkpoint/dimension owners remain cold; native boundaries contain no definition allocation frontier. | Implemented: value/level-only preimages, current-epoch unsave, direct indexed stamps, last-root disarm and branch-valid cuts. Deleted chunk save sequence, duplicate dense/box/command group topology, box epoch maps, definition group hooks, scalar/box merge scratch, retained box group ancestry and restoration/token-result vectors. Counted definitions removed the last local-row frontier. |
| `.15` dispatch/aggregate integration | `tex-exec/src/main_control{,/direct,/semantic,/resources,/settlement}.rs`; `Universe`/retained-generation two-slot admission.                                                                                                                                                                   | Implemented: one packed mode/command/prefix loop with concrete scanners and native semantic owners; deleted eligibility split, operation unions, phase requests, prepare/apply handoffs and eager multi-owner captures. Preserve existing sole-core move/alternate-tail lifecycle and genuine coarse slot/brand ownership; simplify only real redundant adapters.                                   |
| `.13` definitions                    | `definition_arena.rs` pinned format and monotone dynamic slot bank, actual indexed semantic roots and move-only publication; full payload accounting in `body.rs`.                                                                                                                              | Implemented: zero-root tombstones, final payload accounting and central current/group/checkpoint/registry/publication roots. Deleted local/global allocation/promotion, group frontiers, definition cursor truncation and accepted row tails; ordinary undo alternates own roots.                                                                                                                   |
| `.12` closure                        | `tex-exec/src/checkpoint.rs`, `Universe` aggregate restore, command snapshot/capture, `tex-incr` history/resource drivers.                                                                                                                                                                      | Prove complete inventory, repeatable roots, validate-all/root-before-drop, candidate reject/accept, JobStart, pruning, replay, monotone fuel/high water and forwarded provenance; delete obsolete adapters remaining across the seams.                                                                                                                                                              |

`.14` settles the design only. `.10` completes arguments, then `.6` replaces
save ownership in reviewable logical chunks, `.15` replaces the direct dispatch engine, `.13` installs counted
reclamation against those central transitions, and `.12` audits the integrated
engine. Each implementation must delete its displaced representation and port
all real consumers in the same logical change. No shim or second engine may
remain; a design split is for review, not parallel authority.

### Implemented group and direct-control seams

The direct-control replacement (`.15`) selects commands from the packed meaning
and current mode. Each selected arm owns its synchronous operand scan, genuine
token or definition publication, semantic application, and ordered evidence.
Family helpers accept typed operands, without a universal operation union,
readiness mailbox, or eligibility handoff. Cold host-resource calls end command
admission and keep only real request and payload owners on the Rust stack.
Failure unwinds those owners and halts for full-checkpoint replay.

Ordinary expanded fetching and horizontal raw lookahead remain explicit fetch
labels. Prefix accumulation and `reswitch` consume the delivered command without
backup or redelivery. Diagnostic expansion and nested `do_assignments` enter
the same command arms. Observation selects evidence within those arms, never
another interpreter. Alignment retains command-owned delivery semantics and
executor-owned builders.

Shorthand scanners render a replaced meaning before their provisional `Relax`
write releases its cell owner. Final assignment needs no delayed nonowning old
meaning. Definition publication remains owned until assignment consumes it;
Assignment consumes its move-only `PublishedDefinition` at that lifetime boundary. Save capacity and its
monotone high-water projection use native stack metadata plus box-spec words
maintained at box push/pop transitions, without per-command collection walks.

Counted definitions remove the last allocation frontier from the scalar native
group boundary. Local/global scope is solely the meaning and save level;
group exit retires its actual preimages and displaced meanings. No definition
row stack, truncation hook, promotion or frontier capture remains.

Group close consumes native typed entries one at a time. The admitted state
context restores a scalar or box binding, invokes the synchronous restoration
observer while both values are valid, then releases the losing value. A saved
aftergroup token enters ordinary backup input directly at its pop position.
The executor's diagnostic renderer therefore needs no retained restoration
vector or second token vector. `CommandProcessor::end_group` is the execution
seam: its renderer callback observes one native restoration. e-TeX links later
popped words into the final backed-up input row and reverses that row once at
close; TeX82 pushes each word separately. There is no collected token result.
Named command captures prove the native stack empty and contain no group fields. Sparse entries use a lazy marker and a nullable packed `u32` stack-index
chain; their chain traversal remains distinct from ordinary reverse popping.

The `.15` direct control loop must preserve the two TeX fetch labels, current mode,
prefix flags and pending diagnostics as explicit control facts. It invokes
ordinary synchronous scanner and application leaves from the same dispatch
arm. Observation selects evidence production in those leaves, not a different
executor. Host-resource resolution and actual owned scanner roots remain cold
boundaries; no immediate-consumption universal command-operation union or
prepare/apply episode carries them between two dispatch engines.

Required proof cases include the `0 -> global 1 -> local 2` example, nested
local/global/local suppression, repeated same-level writes, repeated named
restore, fresh branch epochs, sparse/classic/box tracing order, aftergroup and
afterassignment timing, interrupted groups discarded on replay, payload
survival after slot tombstone, aliases and candidate tails, final-root drop,
box binding/dimension chronology, full output/resource rollback, and stale
handles. Source/provenance and fuel/high-water cases remain mandatory.
Implementation runs owning tests, `cargo test -q --tests`, authoritative
`scripts/check.sh`, byte-exact story and the canonical workflow; a
semantics-preserving stage must match the exhaustive existing 663 ordered
divergence signatures, not claim clean conformance. Performance measurements
and unchanged original targets remain evidence and architectural goals, not
arbitrary implementation-closure blockers.

### Journal capacity reporting

The live save stack and checkpoint log own their vector capacities. Retained
byte reporting reads those capacities directly with a fixed number of scalar
operations; neither group entries nor checkpoint deltas are traversed. The
journal stores no second byte-count cache and performs no capacity refresh
after append, unsave or checkpoint transitions. Existing save-stack high-water
accounting remains authoritative for TeX capacity diagnostics.

### Group exit without saved records

Closing a group first validates the actual innermost boundary and enters the
synchronous unsave guard. If the save-stack length equals that boundary's
record start, there are no cells, boxes, sparse markers or aftergroup words to
restore. Close that same boundary directly and poison the same dependency
timeline. Cursor and sparse-chain state belong only to the actual restoration
walk, which remains the sole saved-record path and keeps its physical order.
An empty inner group must preserve all saved records of its outer group.
