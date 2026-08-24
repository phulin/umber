# Incremental engine v1 — named-boundary session design

Cross-subsystem resource suspension and admission must follow
[Canonical resource identity and lifecycle](resource_lifecycle.md). This
document remains authoritative for incremental restart and acceptance.

> **Status:** this document remains authoritative for restartable named
> checkpoints, retained editor-session effects/artifacts, edit mapping,
> generation substrates, and pruning. Its canonical reachable live-state
> identity drives the fast full-state suffix splice; the folded `state_hash`
> remains schedule-relative lineage telemetry. Changed-document work resumes ordinary command
> execution from an accepted checkpoint's `CommandSummary`, or from `JobStart`
> when no checkpoint is eligible. The deleted paragraph-replay design is
> recorded only as history in
> [`incremental_memoization.md`](incremental_memoization.md).

This document fixes the v1 contract for an editor session that re-executes an
edited document from a retained checkpoint, detects convergence, and reuses the
unchanged suffix. It refines the incremental-engine overview in
[`architecture.md`](architecture.md) §11 and the normative state, effect,
checkpoint, and hashing rules in [`core_state.md`](core_state.md) §§8–10.
Where this document discusses retaining group roots, the representation and
ownership rules in [`retained_group_roots.md`](retained_group_roots.md) remain
normative.

Paragraph construction and line breaking follow ordinary execution after
restart. V1 has no paragraph recorder, paragraph memo, retained-node mount, or
paragraph-specific validation path; reuse is only the generic checkpoint
restore and convergence described below.

The observed correctness criterion is byte-identical output to a cold execution
of the same editor-buffer revision with the same pinned external inputs. Reuse
is an optimization. Dependency-validated replay must fail closed before
mutation, but session-local suffix adoption deliberately trusts a 64-bit aHash
identity and therefore carries the accepted rare risk of a collision-induced
false match. Missing a checkpoint or a convergence match is allowed; no other
false-match mechanism is accepted.

## Supported replacement and performance budget

Cross-revision reuse consists only of restoring an accepted named checkpoint
and adopting an accepted suffix after the schedule and canonical-state checks
below. The session does not promise paragraph-replay hits, paragraph-local
retention, or unchanged latency for an edit that misses convergence. A miss
continues ordinary execution to another eligible named boundary or job end.

The fixed synthetic comparison, byte identities, historical receipt, and
numeric acceptance ratios are normative in
[`paragraph_replay_deletion_baseline.md`](paragraph_replay_deletion_baseline.md).
The hard behavioral budgets are cold-equivalent artifacts and effects, at
least one generic suffix adoption in the fixed `suffix` pair, zero retained
paragraph-replay state or work, at most one generation substrate at rest and
two during an advance, and deterministic pruning under the host's soft
checkpoint-root budget. There is no pages-retyped correctness budget and no
fallback to a paragraph-specific path.

## Scope and non-goals

V1 has exactly three executor-named boundary kinds:

- `JobStart`;
- eligible root-main-file `OuterParagraphEnd`; and
- eligible root-main-file outermost `ShipoutComplete`.

The root-main-file restriction is a temporary conservative source policy.
`JobStart` remains available, but paragraph and shipout boundaries formed while
any nested external input is active are not retained, including user includes,
class/package inputs, and distribution inputs. General source-role
classification and broader eligibility are deferred to Bead
`umber2-66p0.11`; v1 does not infer a role from a file name or extension.

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
meaning of its `state_hash`; it is lineage telemetry rather than semantic
state equality. Canonical live-state equality is the separate optional identity
captured from current reachable roots.

For paragraph and shipout boundaries, `tex-exec` freezes source eligibility at
the operation that forms the boundary. It compares the active external file
frame's compact source identity with the one root main source registered by
the host. Token-list and macro frames do not replace that external frame: a
macro defined by a package but invoked while the root document is active is
root-originated, while commands being read from the package file are not.
Token provenance, definition sites, paths, names, and extensions do not enter
the decision. A queued nested-input boundary remains ineligible if input
retirement, structural unwinding, or resource resumption later exposes the
root frame before checkpoint publication.

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

An otherwise eligible paragraph is retained only when its frozen active
external file frame is the root main input. A paragraph read from any nested
input is omitted from incremental history without changing ordinary paragraph
execution, input retirement, rollback, or resource suspension.

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

The shipout boundary also requires the frozen root-main-file origin described
above. A nested-file shipout still commits its artifact and effects normally;
only its named incremental checkpoint is filtered. This source restriction is
independent of the existing outermost-output, mode, output-routine, and group
safety checks.

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
    canonical reachable future-state identity,
}
```

The opaque `EngineCheckpoint` atomically owns the `Universe` snapshot,
tex-command `CommandSummary`, mode summary, explicit execution-budget state,
retained effect boundary, group lineage needed by that boundary, and every
content/node root reachable from those components. Command and mode
reconstruction is prepared and validated before `Universe` switches branches;
a failure leaves the live engine unchanged and is reported to the session.
There is no partial restore, alternate lexer/expander continuation, or fallback
field in the record.

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
substrate. Records are owner-exact watermark snapshots into that substrate,
and a retained substrate is never mutated or rolled back in place.
Incremental history sinks capture the session-local canonical comparison
identity while each accepted boundary's `Universe` is live. The identity is
stored with the checkpoint and preserved by record clones and revision
rehoming. A later comparison reads both retained identities directly; it never
forks or rolls the accepted substrate back to materialize an old boundary.
Ordinary non-incremental snapshot consumers still use the bounded snapshot
path without requesting this optional projection. Mutable environment state
contributes its journal-maintained commutative live-cell accumulator. Each live value
resolves referenced token, macro, glue, font, and node handles into canonical
content, so unreferenced append-store entries never enter the identity.
Code-table, hyphenation, page, input, World, interaction, and PDF components
contribute cached canonical roots or rolling semantic fingerprints. Fixed-size
component projection roots are retained with each snapshot and restored on
rollback, while journal scratch remains transient. These caches are not
semantic state and do not change rollback or exact-match results. A single
versioned, domain-separated, fixed-seed 64-bit aHash checkpoint identity
composes those components. Equality is authoritative for suffix adoption, with
no SHA-256 or structural fallback; the accepted rare collision risk is confined
to this session-local optimization. Fixed seeds make fork and rollback results
deterministic within a compatible build/session, and a schema change invalidates
retained compatibility. Durable content and persistence identities remain
unchanged. The session-local aHash comparison does not serialize the full
mutable store or page graph. Root-key mismatch is the invalidation signal, so it visits only
component roots dirtied since their retained snapshot projection.
Restart uses one validated aggregate fork operation: clone the retained
substrate, retarget ownership internally, and roll the clone back to the
selected checkpoint atomically, rebinding the root frame to the in-progress
revision as specified above. Restore atomicity follows by construction: input,
mode, and root-frame state are prepared and validated against the fork, and
the fork is swapped into the private executor only on success. Snapshots stay
owner-exact and there is no general snapshot re-owner API; per-`Universe`
cloning happens once per restart, never per checkpoint.

Profiling builds split revision setup, restart forking, executor work,
detached diagnostic/effect snapshots, checkpoint-history transition,
splice/history construction, accepted-substrate publication/drop, and
acceptance/pruning into additive session timings. DVI materialization remains
a driver-owned timer outside `Session::advance`. These measurements are
operational telemetry only; they do not enter snapshots, session-local aHash
identity, revision acceptance, or output semantics.

The session therefore holds at most two substrates — the accepted frozen
`Universe` and one in-progress scratch fork — and only while an edit is
executing. Both terminal outcomes return to one substrate:

- **Convergence.** The match proves the old record at the splice point
  hash-equal to the new one, so the accepted history keeps the old records at
  and after the match, rehomed onto the new accepted revision. Newly adopted
  artifacts already own their typed diagnostic roots or stable source recipes,
  so the scratch fork is discarded directly; acceptance imports no origin
  graph and adopts no semantic scratch state. The new artifacts and detached
  effect slices are session-owned and survive. A
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
adoption is by reference plus metadata rehoming only. Typed diagnostic roots
are ordinary detached ownership, and arbitrary handles and raw substores still
cannot cross substrate ownership.

Because retained substrates are frozen, v1 needs no per-checkpoint pinning of
journal, node, or content spans: watermark prefixes below a retained
checkpoint stay intact by construction. Fine-grained per-span retention (the
`retained_group_roots.md` capability) remains a measured follow-up, not a v1
prerequisite. The fork's O(state) cost is paid once per `advance` and is
reported by the session's restart-latency metrics, separately from O(1)
checkpoint capture.

## Effects and artifacts across shipout

Effect reconciliation uses a snapshot-owned semantic record ordinal allocated
inside a typed restart domain. Publication-boundary records map their left and
right publication endpoints into the accepted restart domain before competing
with retained records. Their output-attempt identity remains typed candidate
ownership for final transaction-winner arbitration; it is deliberately not
part of the mapped boundary record key. Thus a replacement attempt can displace
the exact retained boundary with the same mapped endpoints and local ordinal,
while distinct ordinals and unrelated endpoint gaps remain separate records.
Neither effect values nor ledger positions participate in this identity. A
separate snapshot-owned placement intra-order survives rollback and generation
forks. Reconciliation maps the semantic correspondence first, inherits its
retained sequence and intra-order, and only then merges boundary, owned, and
unowned winners. Failed allocations remain gaps; vector position, effect kind,
text, counts, and accepted/live offsets never reconstruct placement.

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

The consuming native-finalization handoff has one additional publication
barrier for TeX82 §§1373--1375. The ordered page suffix beginning with the
first deferred `OpenOut` is transferred as `PreparedPageSuffix`, not exposed
through the committed artifact or PDF-page prefixes. An authoritative open
failure retains that suffix together with the failed effect and following
effects. Prompt retry rewrites the `PageEffect::OpenOut` target before artifact
hashing; successful effect export then publishes the whole prepared suffix.
Dropping the retry plan discards it, and an already-drained effect prefix is
never replayed.
The suspended open is named by its absolute effect-log position. Stream and
path validate that identity but never select an artifact occurrence.
Retargeting first prepares a replacement suffix, then validates and changes
the exact pending World effect, and only then swaps in the prepared suffix;
an absent or stale identity leaves both histories unchanged. Each fallible
page also retains the canonical §530 input display captured at shipout so
commit-time interaction does not reconstruct context from consumed input.

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

Logical shipout keeps structural references only for the duration of the
transaction. Successful artifact detachment drops those operation-local refs;
rollback restores the cloned pre-shipout aggregate and drops the candidate.
Whole-substrate retention keeps every root a record could reach through its
ordinary `NodeListRef` fields. Neither `tex-incr` nor `tex-exec` receives raw
node marks, promotion handles, pins, or arena rollback controls.

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

The private candidate and prepared transaction also own one disposable
allocation domain. Resource suspension retains the same domain after rolling
back only the blocked operation. Dropping either owner rejects the complete
domain. Acceptance asks the aggregate state owners for explicit typed roots,
moves only the distinct immutable payloads named by those roots, and then
drops the domain; it never preserves an arena or searches a graph because one
payload survived. Store-specific root projections land with their dedicated
representation migrations under the generic contract in
[Private revision allocation domains](patch_allocation_domains.md).

Within an accepted generation, records are ordered by schedule and their
restart roots, canonical comparison identities, and revision metadata are
never mutated in place. Rehoming creates a new accepted record wrapper rather
than mutating an old generation's checkpoint. `JobStart` is always retained.

The host supplies a soft checkpoint-root memory budget. The aggregate state
layer reports opaque retention units and their charged bytes; `tex-incr` never
walks stores or estimates their contents. In v1 the dominant retention unit is
a generation substrate, charged once and shared by every record retained on
it. A unit is charged once when the session first pins it, even if several
checkpoints or both live generations share it, and is uncharged when the last
session pin is released. Charges
include checkpoint records, command/mode summaries, journal and group-history
blocks, retained effects, and content/node/store blocks kept alive as restart
roots. Allocation ids, sharing counts, and charged sizes are runtime retention
metadata and never enter semantic hashes.

Detached artifacts and the effect/output metadata required to export the
accepted revision are not checkpoint-root retention: they remain necessary
even if every optional restart point is evicted and are accounted by the
session's separate output-retention total. The session reports both totals.
The public `AcceptedOutput` copies only materialized effects, committed
artifacts, detached DVI plans, and telemetry. It never copies a
`BoundaryRecord` or `EngineCheckpoint`; restart history is visible only through
the live session and cannot be kept alive accidentally by a published output.
`JobStart` and the newest boundary are protected, so the checkpoint-root total
may exceed the requested budget; the reported overage makes the budget
explicitly soft rather than silently discarding the only useful roots.

When charged checkpoint-root retention exceeds the budget, the session evicts
restart roots in this deterministic order:

1. oldest `OuterParagraphEnd` records first;
2. oldest non-final `ShipoutComplete` records next; and
3. never `JobStart` or the newest boundary while that generation is accepted.

Cold and replacement candidates apply the same policy as each named boundary
is published. The live generation charge is the opaque lower bound until the
completed generation is frozen and charged precisely at acceptance. This
prevents a long candidate from retaining an unbounded speculative checkpoint
timeline only to discard those same roots during acceptance.

Artifact ids and detached artifact bytes needed to assemble the accepted
output are revision output metadata and survive checkpoint-root eviction.
Eviction removes the complete restart record and asks one aggregate session
API to release its roots: record-exclusive metadata is released immediately,
and substrate storage is released when the substrate's last record goes. The
executor checkpoint store validates generation and capture serial, marks only
the requested live keys, and walks only its exact live-slot index while
pruning. Freed physical slots are reused without relocating survivors; a fresh
serial prevents an old key from aliasing a replacement checkpoint. It cannot
leave a hash-only record or a second command-timeline root behind. An edit
before the oldest useful remaining checkpoint simply restarts at `JobStart`,
so pruning changes latency but not output.

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
3. its live-captured canonical reachable future-state identity equals the identity
   retained on the prior record.

The second rule is required because `state_hash` remains a fold over checkpoint
slices, not a canonical fingerprint of state at an arbitrary instruction. It
records schedule-relative lineage; suffix adoption uses the authoritative
session-local 64-bit aHash identity over current reachable roots, with the
accepted collision risk described above, so changed content may
probabilistically rejoin when the projections of every future-relevant root
hash equally. Append cursors, physical handles, dead immutable entries,
provenance, and cache membership cannot create or prevent a match. A changed
boundary partition still causes missed reuse, never permission to reinterpret
a hash. Parity tests remain the observed correctness oracle.

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

Except for the accepted possibility of an authoritative 64-bit aHash collision
on suffix adoption, the resulting artifact sequence and deferred effect
sequence must equal a cold run. No unnamed terminal continuation is captured or
resumed.

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
- `JobStart` retention plus root-main-file-only paragraph and shipout
  retention, using the active external file frame rather than token
  provenance;
- frozen rejection across nested-input retirement and resource suspension,
  followed by ordinary retention of later root-file boundaries;
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
- accepted boundary comparison never forks or rolls back the accepted
  substrate after the one restart fork;
- checkpoint-root and output-retention accounting charge shared roots once,
  report protected-root overage, keep restart history out of published output,
  and return to baseline after eviction;
- dropping a prepared revision removes its provisional effects, artifacts,
  plans, checkpoints, and private generation without changing accepted output;
- explicit memo/render-cache eviction and an over-budget ephemeral render
  lookup preserve state, source answers, effects, artifacts, and DVI bytes; and
- 2,048 accepted patches and 2,048 resource-retried rejected patches preserve
  exact live-owner plateaus, exact checkpoint/diagnostic charges after the
  64-row fragment-history budget is warm, and bounded weak/node metadata at
  equal-work milestones, with process RSS retained only as a diagnostic; and
- incremental artifacts, deferred effects, and final DVI bytes equal a cold
  run across the committed fast corpus and the 1,000-edit scripted fuzz tier,
  providing empirical coverage rather than an absolute suffix-adoption
  guarantee.

Run focused `tex-command`, `tex-state`, `tex-exec`, and `tex-incr` tests, then
`cargo test --tests`, `scripts/check.sh`, the snapshot budget gate,
and the relevant parity corpora before enabling editor-session mode by default.
