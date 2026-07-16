# Incremental engine v1 — named-boundary session design

> **Status:** this document remains authoritative for restartable named
> checkpoints, retained editor-session effects/artifacts, edit mapping,
> generation substrates, and pruning. Its folded `state_hash` convergence is
> an identical-history optimization (no-op, comment-only, and other
> semantically identical edits), not a general way to rejoin after changed
> typeset content. General changed-document reuse is specified in
> [`incremental_memoization.md`](incremental_memoization.md).

This document fixes the v1 contract for an editor session that re-executes an
edited document from a retained checkpoint, detects convergence, and reuses the
unchanged suffix. It refines the incremental-engine overview in
[`architecture.md`](architecture.md) §11 and the normative state, effect,
checkpoint, and hashing rules in [`core_state.md`](core_state.md) §§8–10.
Where this document discusses retaining group roots, the representation and
ownership rules in [`retained_group_roots.md`](retained_group_roots.md) remain
normative.

The correctness criterion is byte-identical output to a cold execution of the
same editor-buffer revision with the same pinned external inputs. Reuse is an
optimization. Missing a checkpoint or a convergence match is allowed; resuming
state that is not restartable or accepting a false match is not.

## Scope and non-goals

V1 has exactly three executor-named boundary kinds:

- `JobStart`;
- eligible `OuterParagraphEnd`; and
- outermost `ShipoutComplete`.

Every record retained by the incremental session owns a complete,
schema-versioned `EngineCheckpoint`. There are no observation-only records and
no public way to manufacture a boundary or capture the executor between named
boundaries.

V1 explicitly excludes:

- `HashOnly` observations or checkpoints;
- resume kinds, resume fallbacks, and a protocol that tries a checkpoint and
  silently restarts elsewhere;
- arbitrary Rust-continuation capture;
- scanner, alignment, box-building, math-building, output-routine, or nested
  shipout checkpoints;
- inline- or display-math completion boundaries; and
- speculative page execution.

An edit inside any excluded construct re-executes the whole construct from the
latest preceding retained named boundary. Pure-kernel memoization is a separate
optimization and does not add whole-engine observations to this schedule.

## Boundary authority and exact schedule

`tex-exec` owns the only checkpoint-emission capability. `EngineCheckpoint`
fields and constructors remain private, `EngineSession` remains crate-private,
and callers receive completed checkpoints only through `CheckpointSink`.
Recursive scanners, builders, alignments, math conversion, output routines,
and shipout transactions are not passed this capability. A caller may choose
whether to retain a checkpoint after receiving it, but cannot request capture
at another instruction or relabel a checkpoint.

A logical revision has one ordered schedule. The schedule is part of the
meaning of its `state_hash`; it is not diagnostic telemetry.

### `JobStart`

The executor emits `JobStart` exactly once, before it consumes the first token
of a cold logical revision, after all of the following are true:

1. the format and job-start clock have been installed;
2. the root editor buffer and initial `World` input record are pinned;
3. source ids and the input stack are initialized;
4. the mode nest and page builder are in their initial outer-vertical state;
   and
5. the editor-session effect/artifact branch has been opened.

Its conservative root position is byte offset zero. Incremental execution that
restores a retained checkpoint does not create a second `JobStart`: the
restored record is the schedule anchor, and newly emitted records begin after
it. Restoring `JobStart` therefore means full TeX re-execution, not rebuilding
the session or repinning inputs halfway through a run.

### `OuterParagraphEnd`

The executor may emit `OuterParagraphEnd` only when an unrestricted horizontal
paragraph entered from the outer vertical list has:

1. ended and been packaged;
2. appended its result to the contribution/page lists;
3. completed page building and every output cycle triggered by that paragraph;
4. returned control to the outer main-control loop in outer vertical mode; and
5. reached TeX execution-group depth zero.

V1 deliberately restricts paragraph checkpoints to group depth zero. Ordinary
paragraphs inside an open TeX group do not publish a boundary, even when the
mode transition otherwise looks like paragraph completion. This prevents the
session from depending on an exited journal lineage and keeps the first
incremental implementation independent of enabling the broader grouped-
paragraph rollout in `retained_group_roots.md`. Reconsidering that policy
requires implementing and measuring that document's retained-lineage
capability; it is not an executor-side structural check to loosen.

Paragraphs built inside `\vbox`, `\hbox`, insertions, alignments, math, output
routines, or another nested main-control invocation are not outer paragraphs
and never publish this boundary.

### `ShipoutComplete`

`ShipoutComplete` denotes completion of one outermost shipout transaction, not
entry into `\shipout` and not artifact observation by a recursive routine. It
is emitted only after the artifact bytes and ordered page-effect slice have
been detached, the logical shipout has committed, all recursive shipout and
output-routine work has unwound, and the outer main-control loop owns the
complete input, mode, page, and `Universe` roots again.

A nested shipout never emits its own checkpoint. Its artifact id remains in the
ordered artifact prefix owned by the enclosing outer completion. If an
outermost transaction commits more than one artifact through recursive output
work, the single boundary records the entire newly committed prefix; artifact
count changes are not themselves additional schedule entries.

An outermost shipout may complete while TeX groups remain open, but v1 does not
publish a checkpoint in that case. Like `OuterParagraphEnd`,
`ShipoutComplete` is eligible only at execution-group depth zero. The logical
shipout and its artifact ordering are unchanged when publication is
suppressed. This explicit restriction keeps v1 on the current destructive
group-journal substrate: no v1 checkpoint can later be invalidated by group
exit.

The retained-lineage capability in `retained_group_roots.md` may later expand
both paragraph and shipout eligibility. That rollout must be expressed as an
aggregate state-layer capability and tested before `tex-exec` loosens the
depth-zero rule; executor-side inspection of group shape is not sufficient.

When one outer dispatch makes both a shipout and an outer paragraph eligible,
the order is `ShipoutComplete` followed by `OuterParagraphEnd`. This is the
order in which the outer executor observes the completed output cycle and then
the completed paragraph. Each checkpoint is captured independently, so the
schedule-relative hash advances twice.

## Restartable checkpoint tuple

A retained boundary record contains, as one ownership unit:

```text
BoundaryRecord {
    boundary kind,
    boundary occurrence key,
    conservative root-input position,
    accepted root-buffer revision and content hash,
    restartable EngineCheckpoint,
    ordered committed-artifact prefix position,
    schedule-relative state_hash,
}
```

The opaque `EngineCheckpoint` atomically owns the `Universe` snapshot, input
summary, mode summary, retained effect boundary, group lineage needed by that
boundary, and every content/node root reachable from those components. Input
and mode reconstruction is prepared and validated before `Universe` switches
branches; a failure leaves the live engine unchanged and is reported to the
session. There is no partial restore and no fallback field in the record.

Restoring the checkpoint alone reproduces the old revision exactly. Applying
an edit uses a separate aggregate session operation: after it proves the
unchanged root prefix, it prepares a root frame backed by the new editor input
record and substitutes that frame while restoring every other checkpoint root
unchanged. The substitution and restore commit atomically. This narrow
revision-rebind capability cannot alter included-source or token-list frames
and does not expose input-summary mutation to `tex-incr`.

There are exactly two authorities for changing the root revision named by a
checkpoint:

1. **Pre-edit restart rebind.** The session may rebind a selected checkpoint
   from the accepted revision to the in-progress revision only after proving
   that the root prefix through its conservative anchor is byte-identical.
2. **Post-convergence suffix rehome.** Once a new boundary has matched an old
   boundary under the convergence rules below, the engine may rehome the
   matched record and old suffix checkpoints onto the in-progress revision. The match proves equal
   semantic state at the splice point, and the edit map proves that root input
   from that point through the adopted suffix is unchanged. Rehoming maps each
   conservative anchor and occurrence key, substitutes only the root editor
   record and its mapped physical offsets, and preserves the checkpoint's
   boundary kind, semantic state, included-source records, and `state_hash`.

Both operations are engine-owned transformations of an already valid
checkpoint, not caller construction of a checkpoint. They validate every
mapped physical-line anchor and prepare all reopened input and mode state
before committing the aggregate root switch. Failure leaves the source
checkpoint and live engine unchanged.

Suffix rehoming is eager when a revision is accepted. Every record in accepted
history therefore names that accepted revision directly; the session never
keeps a chain of revision maps and never asks a later edit to restore a
checkpoint rooted in an older accepted buffer. Rehoming may share unchanged
checkpoint storage internally, but its public ownership unit has current
revision metadata and a root frame that reopens the current editor record.

The artifact prefix position is session metadata, not TeX semantic state and
not part of `state_hash`. It identifies exactly which artifacts precede the
boundary so a converged run can splice the old suffix without walking page
nodes.

## Generation substrates and restart forking

Every checkpoint of a retained generation shares one frozen `Universe`
substrate. Records are O(1) owner-exact watermark snapshots into that
substrate, and a retained substrate is never mutated or rolled back in place.
Strong canonical identities are not part of checkpoint capture. During an
advance, the resume sink requests them only after the mapped occurrence key
proves that a boundary will actually be compared. The corresponding accepted
record computes its identity on that first later comparison and caches it in
derived record metadata shared by record clones; cold history and boundaries
that are never compared retain only their O(1) roots. Canonical store identity
separates append-only interned content from mutable checkpoint state. Stable
font data is strongly identified once at load, and new token-list, macro,
name, glue, and font entries add only canonical leaves and prefix roots to a
cache shared by related forks. Allocator ancestry prevents a divergent
post-rollback suffix from reusing the wrong derived root. This cache is not
semantic state and does not change rollback or exact-match results. Mutable
environment state contributes its journal-maintained persistent Merkle root;
code-table, hyphenation, page, input, World, interaction, and PDF components
contribute cached canonical roots or rolling semantic fingerprints. A single
versioned checkpoint identity composes those components. Exact comparison does
not serialize the full mutable store or page graph, and therefore visits only
component roots dirtied since their prior projection.
Restart uses one validated aggregate fork operation: clone the retained
substrate, retarget ownership internally, and roll the clone back to the
selected checkpoint atomically, rebinding the root frame to the in-progress
revision as specified above. Restore atomicity follows by construction: input,
mode, and root-frame state are prepared and validated against the fork, and
the fork is swapped into the private executor only on success. Snapshots stay
owner-exact and there is no general snapshot re-owner API; per-`Universe`
cloning happens once per restart, never per checkpoint.

The session therefore holds at most two substrates — the accepted frozen
`Universe` and one in-progress scratch fork — and only while an edit is
executing. Both terminal outcomes return to one substrate:

- **Convergence.** The match proves the old record at the splice point
  hash-equal to the new one, so the accepted history keeps the old records at
  and after the match, rehomed onto the new accepted revision. The scratch
  fork is discarded after the accepted substrate imports only the diagnostic
  origin graph reachable from newly adopted artifacts. Process-global origin
  keys preserve the artifacts' ids, while owned locations cover scratch-only
  engine sources; neither operation adopts semantic scratch state. The new
  artifacts and detached effect slices are session-owned and survive. A
  later edit inside the diverged span restarts from the restart anchor and
  replays at most the span the previous edit already re-executed.
- **Job end without convergence.** The fork becomes the accepted substrate.
  Records before the restart anchor are retargeted onto it through a second
  validated aggregate operation that requires the fork's journal prefix to be
  bit-identical below the anchor, which the fork operation guarantees; the old
  substrate is then dropped.

Rare partial-adoption outcomes may transiently leave accepted records split
across both substrates; the next terminal outcome or ordinary eviction
normalizes back to one. Semantic state is never spliced between substrates:
adoption is by reference plus metadata rehoming only. The one exception is the
validated, diagnostic-only reachable-origin import above; arbitrary handles
and raw substores still cannot cross substrate ownership.

Because retained substrates are frozen, v1 needs no per-checkpoint pinning of
journal, node, or content spans: watermark prefixes below a retained
checkpoint stay intact by construction. Fine-grained per-span retention (the
`retained_group_roots.md` capability) remains a measured follow-up, not a v1
prerequisite. The fork's O(state) cost is paid once per `advance` and is
reported by the session's restart-latency metrics, separately from O(1)
checkpoint capture.

## Effects and artifacts across shipout

Batch mode keeps the existing eager rule from `core_state.md` §8: shipout
materializes the committed effect prefix, records the content-addressed
artifact, drops the flushed effect records, and releases unretained page-local
nodes.

An editor session uses a retained logical-commit mode instead. At logical
shipout it performs all TeX-semantic work at the normal time—deferred writes
are expanded in node order against shipout-time state, leader suppression is
applied, stream state advances, artifact bytes are finalized, and the
schedule-relative hash sees the effect slice—but it does not perform
irreversible host-visible output. Instead:

- the ordered, detached effect records and resulting virtual stream state are
  retained by the session branch;
- the immutable artifact bytes may be placed in content-addressed storage
  immediately, while the revision's artifact ordering remains session-owned;
- filesystem writes, terminal/log publication, stream materialization, and
  other externally visible effects are deferred until an explicit export or
  finalization operation; and
- shell escape remains disabled in an incremental editor session.

Retained session effect, stream, and artifact history is owned by the session
outside any single `Universe`'s `World` state, or shared through immutable
references, so forking a substrate never duplicates it and discarding a
scratch fork never drops it.

Thus rollback across a logical shipout switches to an earlier retained effect,
stream, and artifact prefix; it never tries to undo bytes already exposed to a
host. A later branch may produce a different effect/artifact suffix without
duplicating the old one. The current accepted revision may be exported once,
in order, only after the session commits to discarding every checkpoint that
precedes host materialization. Further edits start a new retained session (or
a cold run) rather than rolling back across that external commit.

Immediate TeX writes are still immediate with respect to TeX execution: they
enter the virtual effect sequence at the instruction that produces them.
“Deferred host materialization” does not change their ordering relative to
deferred whatsits or page artifacts.

### Shipout nodes

Artifact bytes and page effect records must be detached semantic values; a
post-shipout checkpoint does not keep the shipped page graph merely to render
or replay its artifact. However, a checkpoint retained from before the
shipout may still own page-builder, contribution-list, box-register, deferred-
write, or group-journal references into those node/content arenas.

Logical shipout's node release stays transaction-local: it releases only
epochs allocated inside the shipout transaction, which lie above every
published checkpoint watermark, so records retained on a frozen substrate stay
restorable without per-span pins. Whole-substrate retention keeps every root a
record could reach; releasing a substrate's last record releases that storage
through the aggregate `Universe`/`Stores` ownership path. Neither `tex-incr`
nor `tex-exec` receives raw node marks, survivor handles, or arena rollback
controls.

## Root-buffer revisions and input positions

Each `Session::advance` names the exact revision it edits and supplies the
expected old content hash. Revisions are immutable, monotonically identified
values. A stale base revision or mismatched hash is an actionable error; v1
does not guess how to rebase concurrent editor edits. A no-op edit may create a
new revision identity, but preserves the content hash and has an identity
offset map.

The mutable v1 source is the root editor buffer. Included files and other
`World` inputs remain pinned by their `InputRecord` content hashes. If any
non-root input changes, the session invalidates incremental history and starts
a cold revision at `JobStart`; it does not restore a checkpoint whose dormant
source frames name old bytes.

Every boundary records a conservative root byte position. It is derived by the
lexer from the dormant or active root source frame, never from token
provenance:

- while a physical root line is loaded, use that line record's physical
  `terminator_end` (the frame's `next_source_offset`);
- if the root frame is between lines, use the next unread physical offset;
- while an included source or token list is active, use the suspended root
  frame's `next_source_offset`; and
- at root EOF, use the root buffer length.

Rounding past the complete physical line is intentional. The checkpoint owns
the old normalized line, including unread characters after the token that
caused a boundary; selecting it for an edit anywhere on that line would retain
stale future input. Line normalization, `^^` processing, catcodes, macro
expansion, and a suspended include make a token-level cursor unsafe as an edit
boundary. The conservative position is monotonic for one schedule and means
that edits at or after it cannot change the root line image already stored in
the checkpoint. It is not a promise that the current top input frame is the
root source.

To restart an edit whose old half-open byte range begins at `edit_start`, the
session chooses the latest retained checkpoint with position less than or
equal to `edit_start` for which the old root prefix `[0, position)` is byte-
identical in the new revision. If none remains, it uses `JobStart`. An edit in
a scanner, alignment, box, math list, output routine, grouped paragraph, or
other non-boundary construct therefore selects the boundary preceding the
construct and replays it completely.

Old boundary positions at or after the end of the edited range are mapped to
the new revision by the edit's byte delta. Positions inside the replaced range
have no mapping and cannot be convergence candidates. The complete-physical-
line rule is reapplied in the new revision; if the mapped point is not the same
conservative line-end anchor, it is not a schedule match. Multiple edits
supplied in one `advance` are composed in order before restart. Across accepted
revisions, mappings are collapsed by eager suffix rehoming; accepted history
never incurs mapping work proportional to session age.

The root revision id and whole-buffer content hash are validation and mapping
metadata, not inputs to semantic convergence. The aggregate revision-rebind
operation retargets the input hash cursor at the restart anchor so future hash
slices observe newly consumed root input without hashing the unread remainder
of the editor buffer. The active normalized line and its cursor remain semantic
checkpoint state, and included-file `InputRecord` content hashes remain
semantic. Without this distinction, changing one future root byte would poison
every later folded hash and make middle-document convergence impossible even
after TeX state had rejoined.

## History and pruning

The session owns two generations while an edit is running: the accepted
revision used as the comparison/splice source and the in-progress revision.
Once the new revision either finishes or converges, it becomes the accepted
generation and the session returns to one substrate: at job end the fork
replaces the old substrate, and on convergence the scratch fork and its
diverged-span records are discarded while the accepted history is rehomed in
place. Failed or cancelled execution drops only the in-progress fork.

Host composition may keep a fully executed revision in an opaque prepared
state before publishing it. A prepared revision owns its candidate source
layout, effects, artifacts, checkpoints, and either the replacement substrate
or the convergence scratch data required at commit. It may materialize
detached output for validation, but it does not change accepted session state.
Dropping it is rollback; accepting it performs the existing pruning and
generation transition once. This is the boundary used to compose editor
acceptance with VFS build transactions.

Within an accepted generation, records are ordered by schedule and their
restart roots and revision metadata are never mutated in place. The one
derived mutation is publication of a previously absent canonical comparison
identity into the record's shared one-time cache. Rehoming creates a new
accepted record wrapper rather than mutating an old generation's checkpoint.
`JobStart` is always retained.

The host supplies a soft checkpoint-root memory budget. The aggregate state
layer reports opaque retention units and their charged bytes; `tex-incr` never
walks stores or estimates their contents. In v1 the dominant retention unit is
a generation substrate, charged once and shared by every record retained on
it. A unit is charged once when the session first pins it, even if several
checkpoints or both live generations share it, and is uncharged when the last
session pin is released. Charges
include checkpoint records, input/mode summaries, journal and group-history
blocks, retained effects, and content/node/store blocks kept alive as restart
roots. Allocation ids, sharing counts, and charged sizes are runtime retention
metadata and never enter semantic hashes.

Detached artifacts and the effect/output metadata required to export the
accepted revision are not checkpoint-root retention: they remain necessary
even if every optional restart point is evicted and are accounted by the
session's separate output-retention total. The session reports both totals.
`JobStart` and the newest boundary are protected, so the checkpoint-root total
may exceed the requested budget; the reported overage makes the budget
explicitly soft rather than silently discarding the only useful roots.

When charged checkpoint-root retention exceeds the budget, the session evicts
restart roots in this deterministic order:

1. oldest `OuterParagraphEnd` records first;
2. oldest non-final `ShipoutComplete` records next; and
3. never `JobStart` or the newest boundary while that generation is accepted.

Artifact ids and detached artifact bytes needed to assemble the accepted
output are revision output metadata and survive checkpoint-root eviction.
Eviction removes the complete restart record and asks one aggregate session
API to release its roots: record-exclusive metadata is released immediately,
and substrate storage is released when the substrate's last record goes.
It cannot leave a hash-only record behind. An edit before the oldest useful
remaining checkpoint simply restarts at `JobStart`, so pruning changes latency
but not output.

Discarding the scratch fork after convergence drops its diverged-span roots;
replacing the substrate at job end drops the old generation's storage once its
surviving records are retargeted. Root/reference accounting performs
iterative reclamation; history length must not turn checkpoint destruction
into recursive stack growth.

## Schedule-relative convergence and suffix splice

Boundary occurrence keys are `(mapped root position, boundary kind,
same-position occurrence ordinal)`. The ordinal distinguishes, for example,
two outermost shipout completions while the root cursor is suspended at the
same include command. It is assigned only by the executor schedule and is not
editable caller metadata.

Re-execution starts with the restored record as its hash and schedule anchor.
A newly emitted checkpoint is a convergence candidate only when:

1. its occurrence key equals the prior revision's key after revision mapping;
2. every named boundary from the restart anchor through the candidate has the
   same mapped key in the same order; and
3. its complete canonical future-state identity equals the prior record's
   identity, computed lazily at this comparison.

The second rule is required because the inexpensive `state_hash` remains a
fold over checkpoint slices, not a canonical fingerprint of state at an
arbitrary instruction. It remains telemetry and a schedule-relative
accelerator; suffix adoption uses the stronger canonical identity so changed
content may rejoin once every future-relevant root is equal. A changed boundary
partition still causes missed reuse, never permission to reinterpret a hash.
Parity tests remain the correctness oracle.

The first matching candidate wins. For a no-op edit this is the first eligible
named boundary emitted after the selected restart anchor. On a match the
session stops re-execution and keeps the new artifacts and detached effect
slices through the match. Because the match proves the old record hash-equal
at the splice point, the accepted history adopts the old records at and after
the match, eagerly rehomed onto the new accepted revision, discards the
scratch fork together with its diverged-span records, and adopts the
corresponding artifact ids and detached effect/output suffix. Rehoming is permitted only when the edit map proves the root interval
from the matching anchor through each adopted anchor unchanged; otherwise that
record and everything after it are not adopted and execution continues.

The executor stopped at the matching boundary and must not pretend it ran to
job end. After a splice, `Session` exposes only accepted history, detached
artifacts/effects, revision metadata, and reuse measurements. It does not
expose a readable "final" `Universe`, input stack, mode nest, or executor.
Export/finalization consumes detached accepted output, never live executor
state. A later `advance` first restores one accepted named checkpoint into the
private executor and then resumes execution. This state-machine boundary makes
the accepted session coherent without capturing an unnamed terminal
continuation.

The resulting artifact sequence and deferred effect sequence must equal a cold
run. No unnamed terminal continuation is captured or resumed.

If schedule keys or hashes never match, execution continues to normal job end
and replaces the old revision. There is no fixed “pages retyped” correctness
threshold and no fallback protocol hidden behind a failed restore.

## Verification obligations

Implementation is not complete until tests prove all of the following:

- executor-only construction and compile-fail rejection of checkpoint forgery;
- the exact boundary order, including a paragraph-triggered shipout;
- no publication from scanners, alignments, boxes, math, output routines, or
  nested shipouts;
- group-depth-zero eligibility for both paragraph and shipout checkpoints;
- rollback across logical shipout restores effects, streams, artifacts, nodes,
  input, modes, groups, and semantic state atomically;
- at most one generation substrate is retained at rest and two while an edit
  executes: convergence discards the scratch fork and job end replaces the
  substrate with retargeted prefix records;
- substrate forking and record retargeting are validated aggregate operations
  unavailable to `tex-incr`, retargeting requires a bit-identical journal
  prefix, and cross-substrate handle use is rejected;
- pruning releases record-exclusive roots deterministically, and releasing a
  substrate's last record releases its storage;
- stale editor revisions and changed included files cannot reuse old roots;
- adopted suffix records are eagerly rehomed, survive a second edit, and never
  accumulate revision-map chains;
- no-op edits converge at the first eligible candidate;
- schedule changes cause only missed reuse;
- checkpoint-root and output-retention accounting charge shared roots once,
  report protected-root overage, and return to baseline after eviction; and
- incremental artifacts, deferred effects, and final DVI bytes equal a cold
  run across the committed fast corpus and the 1,000-edit scripted fuzz tier.

Run focused `tex-state`, `tex-lex`, `tex-exec`, and `tex-incr` tests, then
`cargo test --tests`, `scripts/check.sh`, the snapshot budget gate,
and the relevant parity corpora before enabling editor-session mode by default.
