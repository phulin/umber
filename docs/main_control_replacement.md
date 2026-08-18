# Main-control replacement

Status: proposed architecture for Beads epic `umber2-awgc`.

Umber will replace the current scalar command object graph with one compact,
mutable, snapshot-native TeX core. The replacement preserves the existing
semantic tests, formats, incremental checkpoints, compact node builder, and
output authorities. It changes the representation and lifetime of work inside
the interpreter.

The design has three priorities, in order:

1. Ordinary TeX execution performs no heap allocation after bounded arena
   warmup. Values that must escape use chunk or region ownership rather than
   per-value `Arc`/`Weak` graphs.
2. State changes are direct and rollback is cheap. A snapshot is a fixed-size
   set of journal cursors, arena watermarks, and immutable-base identities;
   rollback is proportional to changes since the mark, never to live-state
   size.
3. The implementation is simple enough to audit. The hot interpreter, state
   storage, transactions, cold evidence, and barrier publication have separate
   modules with one owner for each invariant.

## Performance and correctness target

The primary performance authority is the pinned offline `2606.12566`
pdfLaTeX workload. Synthetic workloads may diagnose a mechanism but may not be
used as headline promotion evidence.

The final target is:

- complete the pinned paper in at most 20 seconds and at most 150 MiB peak RSS
  under its unchanged wall, fuel, RSS, source, format, distribution, cache,
  and interaction contract;
- preserve exact semantic state, effects, artifacts, DVI, PDF, transcript,
  diagnostics, and incremental boundaries;
- perform no per-token heap allocation, reference-count operation, weak-index
  lookup, content hash, or generation validation in ordinary source delivery,
  stored macro replay, scanning, assignment, and character-node construction;
  and
- create and discard a rollback mark in constant time and constant retained
  space.

Each migration stage must improve the same pinned boundary by at least 15% in
CPU or RSS, establish a necessary structural invariant for a later measured
stage, or be reverted. Full correctness gates remain mandatory before a stage
merges.

## Why replacement is necessary

The current `advance_episode` amortizes one aggregate savepoint across up to
256 scalar operations. It does not change the representations inside the
loop. One ordinary operation still:

1. constructs a command-processor borrow;
2. delivers a rich traced token and resolves owned meaning/provenance values;
3. scans into a large `ScannedStep` value;
4. ends the command borrow;
5. applies that value through `MainControl`;
6. updates journals, dependency state, diagnostics, and output sidecars; and
7. tests every possible barrier before repeating.

`CommandState::snapshot` clones the full rich command state. Token and macro
values use fine-grained strong owners and weak exact indexes. `CurrentCommand`
contains an owned origin, source metadata, resolved meaning, delivery identity,
and alignment state. A group-depth change ends the current episode because the
aggregate retry root cannot span the consumed group lineage.

This is safe and semantically explicit, but it prevents the compiler from
keeping the actual TeX machine in registers and cache. It also makes optional
evidence and incremental ownership shape the ordinary compile path.

The replacement is not a second batch engine. It becomes the sole canonical
engine and retains typed cold handlers for uncommon commands.

## Architecture

The engine is divided into a compact hot core and cold services:

```text
EngineSession
  |
  +-- HotCore
  |     +-- InputMachine
  |     +-- ExpansionMachine
  |     +-- DenseState
  |     +-- SaveJournal
  |     +-- ModeMachine
  |     +-- NodeListBuilder
  |     `-- EpisodeArenas
  |
  +-- ColdServices
  |     +-- Diagnostics
  |     +-- ProvenancePublisher
  |     +-- ObservationSink
  |     +-- DependencyRecorder
  |     +-- CheckpointPublisher
  |     `-- FormatAndOutputPublisher
  |
  `-- BarrierDriver
        +-- resource suspension
        +-- effects and readable output
        +-- diagnostics and observers
        +-- named incremental checkpoints
        +-- format and final output
        `-- cancellation and fuel
```

`HotCore` is the only owner of mutable TeX semantics. `ColdServices` may read a
barriered view or consume compact sidecars, but they do not mirror semantic
state. `BarrierDriver` is the only place an owned snapshot, detached DTO, or
structural provenance graph may be created.

## Compact representations

### Tokens and commands

`TokenWord` is a fixed-width packed value containing the TeX token class and
operand. Control-sequence tokens contain a dense symbol index. Source identity
and provenance are not embedded as owned Rust values.

`CommandWord` is a fixed-width decoded command containing an opcode, operand,
control-sequence index, and compact origin coordinate. It replaces the rich
`CurrentCommand` on the ordinary path. A diagnostic or observer can materialize
the existing public command value at a cold boundary.

The exact layout is versioned and asserted with size tests. It is runtime
state, not format ABI.

### Input frames

`InputFrame` is a POD record containing:

- a chunk or source identifier;
- current and limit offsets;
- frame kind and delivery flags;
- macro/argument or source-specific auxiliary data; and
- a compact trace coordinate.

The input stack owns chunk references at frame or stack granularity. Delivering
a token increments an offset and performs no clone, allocation, weak upgrade,
or hash lookup. Source refill may allocate one source chunk; it does not
allocate per token.

Backup, `\noexpand`, templates, inserted recovery tokens, and macro arguments
use the same frame type. A small inline frame covers one- and two-token replay;
larger replay is a span in an episode arena.

### Macro storage and arguments

Macro definitions are immutable records in an append-only chunk arena:

```text
MacroRecord {
  flags,
  parameter_program_span,
  replacement_span,
  provenance_coordinate,
}
```

The environment owns a compact macro coordinate. Reading a macro validates the
owning chunk at episode admission and then borrows its spans directly. It does
not hash or upgrade an owner on every invocation.

Arguments are spans in the current episode arena. An argument that becomes
future state is promoted by retaining or sealing its chunk, not by copying
each token into individually owned buffers.

Runtime definitions append. Exact-content interning is not part of ordinary
execution. If format capture or incremental convergence benefits from content
identity, it computes or reuses a chunk fingerprint at that publication
boundary.

### Dense state

Meanings and scalar registers remain packed words in dense banks. The hot core
uses direct indexed access. Sparse e-TeX overflow banks remain paged but use
compact pages and stable page coordinates.

Cells containing token, macro, glue, box, or font values store compact arena or
immutable-base coordinates. The coordinate's owning region is retained by the
state generation, group save record, or checkpoint root. A cell read does not
acquire a new owner.

### Nodes

The existing sole mutable `NodeListBuilder`, compact node rows, and immutable
`NodeListRef` publication remain. The hot core appends ordinary characters,
kerns, glue, penalties, and rules without intermediate `Node` allocation.
Complex nodes may use arena-backed side tables. Freeze occurs only at a page,
output, diagnostic, observer, format, or named-checkpoint barrier.

## Arenas and ownership

The engine uses a small number of typed segmented arenas:

- immutable accepted token and macro chunks;
- mutable episode token and argument chunks;
- compact provenance records and runs;
- uncommon scanned operands that must survive a handler call; and
- existing mutable node-builder storage.

An arena segment is the ownership unit. A segment is retained once by an
accepted generation, live input frame, group save entry, or published
checkpoint. Individual values are copyable coordinates. Segments never move
after publication.

Each coordinate contains a slot plus a generation or namespace sufficient to
reject stale and foreign use at admission and publication boundaries. The
ordinary loop does not repeat that validation for every access after it has
borrowed the admitted segment.

Arena growth is geometric and capacity is reused after rollback. Bounded-live
work must plateau both live bytes and registry metadata. All-live controls must
grow by the exact documented payload and segment overhead.

## Snapshot and rollback model

### Fixed-size mark

A `HotSnapshot` contains only:

- input, parameter, condition, group, save, and mode stack lengths;
- mutation-journal cursor;
- token, argument, provenance, and node arena watermarks;
- page/PDF/effect/output journal cursors;
- source and resource transaction cursors; and
- immutable-base and engine-owner identities.

Taking or discarding the mark is O(1) in live-state size and performs no heap
allocation after mark-stack warmup.

### Mutation journal

Every mutable state write records the cell coordinate and previous packed
value on first write after the active rollback mark. Repeated writes coalesce
within the mark. Group semantics use the same underlying save records but have
their own TeX group boundary and restoration behavior.

Rollback walks journal records backward, restores packed values, truncates
stacks and arena suffixes, and restores external-ledger cursors. Its cost is
O(changes since the mark). Commit drops the mark and retires unreachable
journal history when no group or checkpoint owns it.

### Narrow resource transactions

Ordinary commands do not run beneath an aggregate retry snapshot. A command
that may suspend must either:

1. determine and acquire its resource before semantic mutation; or
2. open a narrow `HotSnapshot` around the mutation sequence that depends on
   the resource.

This lets episodes cross group entry and exit. A group change is semantic work,
not an external commit barrier.

### Durable checkpoints

A named incremental checkpoint is created only when scanners and transient
builders are quiescent. Mutable arena suffixes are sealed into immutable
segments; the checkpoint retains segment roots plus compact stack and journal
coordinates. Format and continuation DTOs remain handle-free.

## Persistent interpreter

The ordinary interpreter borrows `HotCore` once and retains its input cursor,
meaning banks, stack cursors, and arena writers across commands. Its loop is:

```text
while budget remains {
  token = input.next_raw()
  command = expansion.next_command(token)
  dispatch(command, hot_core)
  if a real barrier is pending {
    break
  }
}
```

Expansion and scanners are methods over the same input and state views.
Common primitives scan operands and apply their mutation directly. They do not
construct a universal `ScannedStep` value.

Rare or structurally large primitives may call a typed cold handler. The
handler receives scalar values or arena spans and mutates the same `HotCore`.
It is not a second semantic executor. A cold handler that may publish an
effect returns a typed barrier request.

Fuel remains charged at the same canonical logical points. An attached
observer selects an observed interpreter instantiation with an event sink. The
ordinary production instantiation has a zero-sized sink, allowing evidence
branches and builders to compile away.

## Cold evidence

### Provenance

Ordinary tokens carry compact source coordinates or provenance-run indexes.
Macro expansion appends one compact frame record and refers to it by index.
Direct source runs require no structural allocation.

At a diagnostic, rendered-source, observer, checkpoint, or output boundary,
`ProvenancePublisher` traverses the compact records and materializes the
existing structural roots required by that consumer. Publication is budgeted,
atomic, and cached per sealed segment. With no consumer, it performs no work.

### Observations and dependencies

The unobserved interpreter does not allocate observation names, mutation keys,
records, or dependency projections. The observed interpreter writes compact
typed records to an append-only sidecar and renders them after the command's
semantic mutation commits.

Dependency recording is selected once at episode admission. Direct state
getters are used when no tracked region is active; there is no atomic check on
every getter in that instantiation.

### Diagnostics

Input frames always retain enough compact coordinates to reconstruct TeX's
error context. Strings and structural input descriptions are created only when
an error or tracing parameter demands them. Ordinary execution does not create
empty diagnostic vectors or format trace values.

## Module boundaries

The replacement should converge on modules of roughly these responsibilities:

- `hot_core/mod.rs`: aggregate semantic owner and admitted borrowed views;
- `hot_core/layout.rs`: packed words, coordinates, and size/layout assertions;
- `hot_core/arena.rs`: typed segmented arenas and sealing;
- `hot_core/input.rs`: source and token frames plus raw delivery;
- `hot_core/expand.rs`: macro expansion and condition delivery;
- `hot_core/scan/`: scalar and structured scanners over the persistent input;
- `hot_core/dispatch.rs`: opcode dispatch and common direct handlers;
- `hot_core/state.rs`: dense banks and compact value coordinates;
- `hot_core/journal.rs`: group save records, transaction marks, and rollback;
- `hot_core/mode.rs`: mode stack and the sole node builder;
- `cold/diagnostics.rs`: context and trace rendering;
- `cold/provenance.rs`: structural provenance publication;
- `cold/observation.rs`: canonical observation publication;
- `cold/checkpoint.rs`: named snapshot sealing and DTO construction; and
- `barrier.rs`: typed resource, effect, output, checkpoint, and cancellation
  exits.

`MainControl` becomes a small lifecycle facade over the persistent interpreter
and barrier driver. Scanner and executor modules remain testable separately,
but their production path is not separated by a heap-owning universal DTO.

## Migration plan

The work is tracked by dependency-ordered Beads children:

| Issue           | Deliverable                                                       |
| --------------- | ----------------------------------------------------------------- |
| `umber2-awgc.1` | Pinned arXiv baseline and structural allocation/episode census    |
| `umber2-awgc.2` | Typed arenas and fixed-size snapshot marks                        |
| `umber2-awgc.3` | Packed token, input-frame, macro, and argument representation     |
| `umber2-awgc.4` | Journaled narrow transactions that span group changes             |
| `umber2-awgc.5` | Persistent fused expansion, scanning, and dispatch                |
| `umber2-awgc.6` | Cold provenance, observation, dependency, and diagnostic sidecars |
| `umber2-awgc.7` | Full cutover, legacy deletion, and final pinned validation        |

Migration is bottom-up. A new representation becomes canonical storage before
the old representation is deleted. Temporary adapters may exist only at a
barrier and must have counters proving they are absent from the pinned hot
path. No stage may introduce a runtime engine selector.

During migration, differential tests compare the new core with the current
canonical engine at bounded quiescent boundaries. Promotion requires exact
state, effects, artifacts, DVI/PDF, diagnostics, fuel, and rollback outcomes.
Once a family is promoted, its old storage and dispatch path are deleted in the
same or immediately following commit.

## Measurement gates

The child-1 receipt fixes the comparison authority. Every later stage records:

- full-process and engine-only wall and CPU time;
- peak and milestone RSS;
- command fuel and logical work vectors;
- heap calls and requested bytes by subsystem;
- arena committed, live, and high-water bytes;
- delivered tokens, macro calls, arguments, scanner entries, and state writes;
- episode length distribution and stop reasons;
- snapshot count, rollback count, changed cells, and rollback work;
- `Arc` clone/drop, weak lookup, content hash, and provenance publication
  counts; and
- exact semantic and output identities.

The current-core structural fields come from the profiling-only
`HOT_CORE_CENSUS` report documented in
[`profiling.md`](profiling.md#main-control-hot-core-structural-census). Its
owner, stop-reason, command-family, and phase vocabularies are fixed-width and
exhaustive, and its episode histogram preserves every length from zero through
the canonical 256-operation bound. The production feature resolution contains
none of its counters, scopes, allocator wrapper, fields, or calls.

The same optimized profile, CPU affinity policy, source, format, distribution,
offline cache, environment, and guards are used for before/after measurements.
Synthetic benchmarks are retained for focused asymptotic controls only.

## Non-goals

- No JIT is required for this replacement.
- No unsafe code is required merely to obtain compact layouts or direct
  indexing.
- The format ABI does not serialize runtime coordinates, arena capacities, or
  journals.
- Snapshot equality is not weakened.
- Diagnostics, provenance, incremental execution, and observers are not
  deleted; they are moved to explicit cold publication paths.
- The replacement does not preserve compatibility wrappers after their final
  consumer migrates.

## Completion criteria

The epic closes only when:

- every supported TeX82, e-TeX, and pdfTeX profile uses the same `HotCore`;
- the rich scalar input, `CurrentCommand`, universal `ScannedStep`, blanket
  retry snapshot, and per-value weak-ownership hot paths are absent;
- group changes do not force episode publication;
- bounded-live arenas and journal metadata plateau while all-live controls grow
  exactly;
- source/loaded formats, TRIP/e-TRIP, exhaustive command tracing, incremental
  convergence, the full routine suite, and `scripts/check.sh` pass; and
- the pinned paper meets the 20-second and 150-MiB gates under unchanged
  guards.
