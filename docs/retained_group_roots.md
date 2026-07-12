# Retained Group Roots

## Status and scope

This document specifies a state-layer extension that lets a named
`OuterParagraphEnd` engine checkpoint remain restorable after live execution
leaves the ordinary TeX group that enclosed the paragraph. It refines the
checkpoint contract in `core_state.md` §9; it does not change the rule that
only the outer executor may publish an `EngineCheckpoint`.

The feature covers a paragraph that has returned control to outer main control
but happens to have one or more ordinary groups open. This is common for prose
inside LaTeX-style environments. It does **not** make execution inside a
`\vbox`, insertion, alignment, scanner, math builder, output routine, or nested
shipout restartable. Those contexts require explicit resumable builder or
continuation state and remain separate work.

Before retained roots are enabled, paragraph checkpoint publication must reject
nonzero execution group depth. The current mode transition alone is not a
sufficient eligibility check: it can publish a snapshot whose enclosing group
later exits and invalidates it.

## Current mechanism and failure

`Env` owns the dense current value banks, a flat undo journal, group markers,
the `\aftergroup` payload vector, `\afterassignment`, write stamps, and box
journal bookkeeping. `EnvSnapshot` is a rollback cursor containing a journal
position, `\aftergroup` length, `\afterassignment`, group depth, and epoch.
Capture is cheap because it copies only these scalars.

Group exit is deliberately destructive:

1. local writes are restored from undo records;
2. global writes are refiled into the enclosing journal slice;
3. the exiting marker and journal suffix are truncated or compacted;
4. `\aftergroup` payloads are drained for delivery;
5. box-journal bookkeeping for the exiting depth is discarded; and
6. survivor ownership held by truncated records is released by `Stores`.

Consequently, an `EnvSnapshot` taken inside the group is not a retained state
root. After exit, its group depth differs from the live depth and its journal
position can lie past the new journal end. `Stores::assert_valid_snapshot`
correctly rejects it. Removing that validation would permit restoration from
missing undo history, consumed tokens, or released node-list ownership.

This limitation is confined primarily to `Env`. Code tables already retain
immutable roots and group history in `CodeTablesSnapshot`; token, node, font,
input, page, stream, and other store snapshots already expose roots or
watermarks that can participate in an aggregate snapshot. The design must not
weaken their existing owner, generation, and handle-liveness checks.

## Goals

- Keep group entry and engine-checkpoint capture O(1) or within the existing
  bounded root-copy budget, independent of the number of environment cells.
- Keep ordinary environment reads O(1) through the existing dense banks.
- Make a retained checkpoint inside nested ordinary groups restorable after any
  number of those groups have exited on the live branch.
- Preserve exact TeX local/global assignment, `\aftergroup`, and
  `\afterassignment` behavior after restoration.
- Retain every store resource reachable from environment values or history for
  as long as a checkpoint can restore that history.
- Restore `Env`, stores, `World`, input, modes, effects, and hash state
  atomically through the existing `Universe`/`EngineCheckpoint` boundary.
- Reclaim journal segments and referenced resources when checkpoint history
  releases the last root.
- Keep semantic hashes independent of allocation, segment shape, branch ids,
  and checkpoint-retention policy.

## Non-goals

- Arbitrary instruction-level continuation capture.
- Checkpoints while a scanner, alignment, box, math list, output routine, or
  nested shipout owns future-relevant Rust locals.
- Durable serialization of in-memory engine checkpoints. Formats remain
  separate, quiescent, validated DTOs and still require no open groups.
- Copying the complete `Env` or cloning a `Universe` per paragraph.
- Exposing raw roots, journal records, partial restore, or store handles outside
  the aggregate state facade.

## Ownership and layering

The existing authority boundaries remain intact:

```text
EngineCheckpoint
├── boundary kind and boundary-owned metadata
├── opaque Universe Snapshot
│   ├── Stores snapshot
│   │   ├── retained Env root
│   │   └── matching content/node/font/code-table roots
│   └── World/effect snapshot
├── rooted InputSummary
└── rooted ModeNest summary
```

`Env` implements assignment and group semantics. `Stores` owns `Env` and the
resources referenced by its cells and journal records; it validates handles and
manages survivor ownership. `Universe` is the only production authority that
may capture or restore the complete store/world aggregate. `EngineCheckpoint`
adds the synchronized executor roots and proves that capture occurred at a
named safe boundary.

An `EnvRoot` is therefore not an independently restorable public capability.
It is a private component of a `StoreSnapshot`, and its owner/timeline must
match every other root in the containing `Snapshot`.

## Proposed representation

Use a hybrid representation: mutable dense banks for the checked-out live
state, plus immutable copy-on-write history roots for branching and restore.
Group entry never copies a value bank.

The illustrative types below are structural, not an API commitment:

```rust
struct Env {
    values: ValueBanks,              // O(1) current reads and writes
    current: EnvRoot,
    stamps: StampBanks,
    box_write_index: BoxWriteIndex,
}

#[derive(Clone)]
struct EnvRoot {
    journal: JournalCursor,
    groups: GroupStackRoot,
    afterassignment: Option<Token>,
    epoch: Epoch,
}

#[derive(Clone)]
struct JournalCursor {
    segment: Arc<JournalSegment>,
    offset: u32,
}

struct JournalSegment {
    parent: Option<JournalCursor>,
    changes: Box<[ChangeRecord]>,
    retained: RetainedResources,
}

struct ChangeRecord {
    cell: CellId,
    before: RawValue,
    after: RawValue,
    assignment: AssignmentScope,
    global_generation: GlobalGeneration,
}

#[derive(Clone)]
struct GroupStackRoot(Option<Arc<GroupFrame>>);

struct GroupFrame {
    parent: GroupStackRoot,
    kind: GroupKind,
    entry: JournalCursor,
    aftergroup: PersistentTokenQueue,
    entry_afterassignment: Option<Token>,
}
```

Segments are append-only once shared. The live writer may keep a small unique
tail buffer; capture seals or shares that tail and clones only root handles.
Segment boundaries, compaction, and branch identity are representation details
and never enter semantic hashes.

The dense banks are not copied at group entry. They are the materialized state
of the currently checked-out `EnvRoot`. Journal changes must contain enough
information to move the banks both backward and forward between retained roots.
Repeated writes to one cell within a segment may be collapsed to the first
`before` and final `after` value when no observable group/global boundary lies
between them.

If measurements show that long-distance branch switching dominates, selected
dense banks may later use fixed-size COW pages or periodic private materialized
anchors. That is an optimization behind `EnvRoot`; it must preserve bounded
capture and the same semantic journal.

## Operations

### Capture

Capturing an environment root seals the current journal tail if necessary and
clones `JournalCursor`, `GroupStackRoot`, and scalar state. `Stores::snapshot`
binds this root to matching arena generations, survivor roots, code-table roots,
and owner identity. Capture performs no traversal of environment cells or open
groups.

### Enter group

Push an immutable `GroupFrame` containing the current journal cursor, group
kind, empty persistent `\aftergroup` queue, and entry scalar state. The live
dense banks are unchanged. Entry is O(1).

### Local assignment

The write barrier validates any embedded handle through `Stores`, records the
cell's `before` value on its first semantically relevant write, updates the
dense bank, and records the final `after` value. Existing stamps/write sets
remain the mechanism for avoiding duplicate undo records and dependency noise.
Records that carry store handles add them to the segment's retained-resource
summary before publication.

### Global assignment

Global assignment must not destructively delete or rewrite history visible to a
retained branch. Give global writes a monotonic semantic generation within the
Universe timeline. A local restore record notes the global generation it
observed; leaving a group restores the local `before` value only when no later
global generation for that cell supersedes it.

This replaces current in-place group-slice compaction with a branch-safe rule
while preserving TeX cases such as a local assignment followed by a global
assignment to the same cell. Global generations are runtime ordering metadata;
hash the resulting semantic value, not the generation number.

### `\aftergroup` and `\afterassignment`

Store `\aftergroup` payloads in the current immutable group frame or a
persistent queue rooted by it. Group exit returns the logical FIFO payloads for
insertion on the live branch but does not erase them from a retained historical
root. A checkpoint restored inside that group therefore sees the original
pending payload sequence.

`\afterassignment` is a scalar in `EnvRoot`. Taking it creates a new root with
the field cleared; retaining an older root retains the pending token. Both token
forms must be validated before ingress and their symbol/content dependencies
must participate in semantic hashing.

### Leave group

Leaving a group creates a new live root rather than truncating the only history:

1. walk changes since the frame's entry cursor in reverse;
2. restore local cells unless superseded by a later global generation;
3. retain global resulting values;
4. pop the live group-stack root;
5. schedule the frame's `\aftergroup` sequence through the normal input API;
6. append a compact branch transition or begin a new journal segment; and
7. release nothing still reachable from another retained root.

Cost remains proportional to writes in the exiting group, as it is today.
Historical roots inside the group remain valid because their segments, group
frames, payloads, and referenced resources are immutable and retained.

### Restore and branch switching

`Universe::rollback` first validates the entire aggregate snapshot without
mutation. For `Env`, locate the nearest shared journal ancestor between the
checked-out root and target root, apply `before` values while walking backward,
then apply `after` values while walking forward to the target. Install the
target group root, pending tokens, scalar state, stamps, and resource ownership
only after every component is known to be valid.

If any input reopening, mode reconstruction, handle validation, or resource
retention check fails, no component may remain partially restored. The engine
checkpoint restore path should prepare input and mode roots first, validate the
Universe target, then commit the aggregate switch.

Restoration cost is proportional to divergent mutations, not total bank size.
Periodic anchors or COW pages may bound pathological old-root switching after
measurement, but engine-checkpoint capture must remain bounded.

## Resource retention and reclamation

Journal values can contain token, glue, macro, font, provenance, source, and
node-list handles. `Env` may enumerate those references but must not decide
their liveness independently. `Stores` attaches a retained-resource summary to
each sealed segment/root and pins the corresponding survivor/content roots.

In particular, box-register records must no longer release survivor ownership
merely because the live branch exits a group. Ownership is released when the
last live environment root and retained checkpoint that can observe the record
are both gone. Reclamation must be iterative and bounded; dropping a long
checkpoint history must not recursively overflow the stack.

The engine session owns checkpoint retention policy. Evicting a checkpoint
drops its aggregate root; ordinary `Arc`/root reference accounting then makes
unreachable journal segments and store resources reclaimable. Runtime root ids,
reference counts, and segment allocation are excluded from semantic equality.

## Semantic hashing and convergence

The hash represents the target root's resulting TeX-semantic state, pending
group actions, and future-relevant group stack—not its journal path. Equal
states reached through different assignment orders, segment layouts, or
checkpoint schedules must hash equally.

State-hash cursors therefore become root-aware. A cursor records the semantic
root it summarizes and can be retargeted when branches share ancestry. Journal
compaction may cache hash deltas, but a retained root's hash cannot change when
another branch exits a group or is reclaimed. Group kind, ordered
`\aftergroup` payloads, `\afterassignment`, and behaviorally relevant open-group
state are included; epochs, global generation counters, segment ids, ownership
counts, and allocation history are excluded.

Convergence may compare checkpoints only at the same named boundary schedule.
Retained group roots expand paragraph eligibility; they do not permit matching
an inner builder state against an outer paragraph boundary.

## Eligibility rollout

Implementation must proceed conservatively:

1. **Fix current publication.** Require `OuterParagraphEnd` to have group depth
   zero while snapshots inside exited groups remain invalid. Add a regression
   proving grouped prose produces no invalid durable checkpoint.
2. **Introduce private retained roots.** Add immutable group/journal roots and
   bidirectional change records without changing public checkpoint eligibility.
3. **Retain resources.** Bind environment roots to store-owned content and
   survivor ownership; add deterministic eviction and reclamation.
4. **Make restore branch-aware.** Restore retained roots atomically through
   `Stores` and `Universe`, including hash cursors and failure rollback.
5. **Enable grouped paragraph boundaries.** Replace the depth-zero condition
   with an explicit `Stores`/`Universe` capability proving the full active group
   lineage is retained and contains no unsupported execution context.
6. **Measure and tune.** Add anchors or COW bank pages only if branch-switch and
   retained-memory measurements justify them.

The eligibility API should express capability rather than duplicate structural
rules in `tex-exec`, for example `Universe::can_publish_grouped_checkpoint()`.
The state layer remains authoritative about whether its current lineage is
durably retainable.

## Validation

Correctness tests must cover:

- restoration inside one and many nested groups after the live branch exits;
- local, global, and repeated mixed-scope writes to the same cell;
- every value bank, including meanings, registers, font selectors, and boxes;
- group kinds and mismatched-close recovery;
- ordered `\aftergroup`, pending/consumed `\afterassignment`, and rollback;
- code-table global/local interaction across independently retained roots;
- box survivor pinning, checkpoint eviction, and exact final reclamation;
- stale, foreign-timeline, post-rollback-reused, and malformed handles;
- input, mode, page, stream, effect, and `World` restoration atomicity;
- equal semantic hashes and byte-identical output versus uninterrupted replay;
- branching twice from one checkpoint without handle or history revival;
- format export continuing to reject open groups; and
- compile-time enforcement that downstream crates cannot restore `EnvRoot`.

Performance gates must demonstrate:

- group entry performs no payload-sized allocation or value-bank copy;
- engine-checkpoint capture stays within the documented snapshot budget;
- reads remain O(1) with no group-depth traversal;
- group exit scales with writes in that group;
- restore scales with mutations between roots, with an adversarial bound or
  measured anchor policy;
- retained memory scales with unique changed cells/resources, not complete
  environment size times checkpoint count; and
- eviction returns journal segments and survivor roots to the expected
  baseline.

Run focused `tex-state` normal/shadow/replay suites, `tex-expand` and `tex-exec`
checkpoint/group tests, `cargo test --workspace --tests`, `scripts/check.sh`,
the snapshot budget gate, and relevant Gentle/corpus parity before enabling the
new boundary policy.

## Rejected alternatives

**Copy all environment banks at capture.** Correct as a prototype, but capture
and retained memory scale with total environment size and violate the snapshot
contract.

**Clone the complete Universe per checkpoint.** Duplicates unrelated stores,
complicates timeline identity, and is substantially more expensive than the
changed state.

**Remove snapshot invalidation checks.** Restores from destroyed history and
released resources; this is unsound.

**Layer persistent maps directly in the read path.** Makes reads depend on group
depth or persistent-map lookup cost. Dense current banks are a deliberate hot
path and should remain.

**Reconstruct retained branches from undo-only records.** Current records and
global compaction do not guarantee the final forward value needed to move from
a common ancestor to a sibling branch. Retained history must be bidirectional
or use COW materialized roots.

**Serialize arbitrary Rust continuations.** Expands scope to scanners and
builders, couples durable state to control-flow implementation, and is not
needed for ordinary grouped prose.

## Expected outcome

After this design is implemented, ordinary paragraphs inside stable open TeX
groups can publish durable `OuterParagraphEnd` checkpoints. Restoring one after
the original execution has left the environment recreates the exact local
values, group stack, pending tokens, referenced resources, input/mode roots,
effects, and semantic hash. Specialized nested builders remain construct-level
incremental regions until separately modeled as explicit resumable state.
