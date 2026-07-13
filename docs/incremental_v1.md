# Incremental engine v1 — named-boundary session design

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

Unlike paragraph boundaries, an outermost shipout may complete while TeX
groups remain open. The session-state implementation must retain that full
group lineage before such a checkpoint is published. If the state layer cannot
prove the lineage retainable, the executor must suppress the checkpoint while
preserving the logical shipout and its artifact ordering. It must never publish
a checkpoint that a later group exit invalidates.

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
    root-buffer revision and content hash,
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

The artifact prefix position is session metadata, not TeX semantic state and
not part of `state_hash`. It identifies exactly which artifacts precede the
boundary so a converged run can splice the old suffix without walking page
nodes.

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

Logical shipout therefore releases only node epochs unreachable from every
live engine root and every retained checkpoint. Retained pre-shipout roots pin
their node spans and referenced token/glue/font content. Pruning the last
checkpoint that reaches such a span releases it through the aggregate
`Universe`/`Stores` ownership path. Neither `tex-incr` nor `tex-exec` receives
raw node marks, survivor handles, or arena rollback controls.

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
conservative line-end anchor, it is not a schedule match. Multiple edits are
composed in revision order with the same rules.

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
generation and the older generation is dropped. Failed or cancelled execution
drops only the in-progress branch.

Within an accepted generation, records are ordered by schedule and never
mutated in place. `JobStart` is always retained. The host supplies a checkpoint
memory budget; when retention exceeds it, the session evicts restart roots in
this deterministic order:

1. oldest `OuterParagraphEnd` records first;
2. oldest non-final `ShipoutComplete` records next; and
3. never `JobStart` or the newest boundary while that generation is accepted.

Artifact ids and detached artifact bytes needed to assemble the accepted
output are revision output metadata and survive checkpoint-root eviction.
Eviction removes the complete restart record and asks one aggregate session
API to release its environment, group, input, effect, node, and content roots.
It cannot leave a hash-only record behind. An edit before the oldest useful
remaining checkpoint simply restarts at `JobStart`, so pruning changes latency
but not output.

Dropping the previous generation after convergence also drops any old suffix
roots not adopted into the new history. Root/reference accounting performs
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
3. its schedule-relative `state_hash` equals the prior record's hash.

The second rule is required because the current hash is a fold over checkpoint
slices, not a canonical fingerprint of state at an arbitrary instruction. A
changed boundary partition therefore causes missed reuse, never permission to
reinterpret the hash. Hash collisions retain the ordinary documented 64-bit
risk; parity tests remain the correctness oracle.

The first matching candidate wins. For a no-op edit this is the first eligible
named boundary emitted after the selected restart anchor. On a match the
session stops re-execution, keeps the new records through the match, adopts the
old records and artifact ids strictly after the matching record, and adopts the
old completed revision's detached effect/output suffix. The live executor at
the match need not pretend it ran to job end: the accepted session state is its
ordered named-checkpoint history plus completed output metadata, and the next
edit always restores one of those named checkpoints. The resulting artifact
sequence and deferred effect sequence must equal a cold run. No unnamed
terminal continuation is captured or resumed.

If schedule keys or hashes never match, execution continues to normal job end
and replaces the old revision. There is no fixed “pages retyped” correctness
threshold and no fallback protocol hidden behind a failed restore.

## Verification obligations

Implementation is not complete until tests prove all of the following:

- executor-only construction and compile-fail rejection of checkpoint forgery;
- the exact boundary order, including a paragraph-triggered shipout;
- no publication from scanners, alignments, boxes, math, output routines, or
  nested shipouts;
- group-depth-zero paragraph eligibility and retained-lineage enforcement for
  any grouped shipout checkpoint;
- rollback across logical shipout restores effects, streams, artifacts, nodes,
  input, modes, groups, and semantic state atomically;
- pruning releases exactly the roots no remaining checkpoint reaches;
- stale editor revisions and changed included files cannot reuse old roots;
- no-op edits converge at the first eligible candidate;
- schedule changes cause only missed reuse; and
- incremental artifacts, deferred effects, and final DVI bytes equal a cold
  run across the committed fast corpus and the 1,000-edit scripted fuzz tier.

Run focused `tex-state`, `tex-lex`, `tex-exec`, and `tex-incr` tests, then
`cargo test --workspace --tests`, `scripts/check.sh`, the snapshot budget gate,
and the relevant parity corpora before enabling editor-session mode by default.
