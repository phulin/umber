# Incremental memoization and execution-trace reuse

This document defines the long-term incremental-compilation design for Umber.
It supersedes folded-checkpoint-hash convergence as the primary way to reuse a
changed document's suffix. The named-boundary session in
[`incremental_v1.md`](incremental_v1.md) remains the restart, retention,
rollback, effect-virtualization, and cold-parity substrate.

The central decision is:

> General incremental reuse is constrained memoization over a hierarchical
> execution trace. Each reusable region records stable input identity, the
> semantic values it actually read, its detached result, and any state changes
> or virtual effects that must be replayed. Whole-state equality is an optional
> fast path, not the main invalidation mechanism.

The correctness criterion is byte-identical output and effect ordering versus
a cold execution of the same revision with the same pinned external inputs.
Every cache miss, unsupported construct, failed validation, or malformed
persistent entry falls back to ordinary execution.

## Why checkpoint-hash convergence is insufficient

The implemented checkpoint `state_hash` is a fold over the checkpoint
timeline:

```text
next_hash = combine(previous_hash, semantic_slice_hash)
```

This is useful for proving identical execution history under an identical
checkpoint schedule. It is not a fingerprint of only the current
future-relevant state. Once a changed paragraph contributes a different slice,
later identical slices do not normally make the folded hashes equal again.
Consequently, v1 convergence is effective for no-op, comment-only, and other
semantically identical edits, but it cannot be the general mechanism for
rejoining after changed typeset content.

A canonical current-state fingerprint could be sound if it included every
future-observable value: environment cells, input and mode state, page-builder
roots, insertions, marks, page counters, virtual streams, RNG, clocks, and
other state. Such a fingerprint remains valuable as an exact-state splice
shortcut, but it has two limitations:

1. unrelated state differences prevent reuse even when a computation never
   reads that state; and
2. page numbers, footnotes, marks, writes, and output routines commonly make
   current states differ for long spans.

The primary design therefore validates only the dependencies of each cached
computation. A paragraph that does not read the page number may reuse its
expansion or line breaking even while the page number differs. The output
routine that does read it must miss or replay a previously validated state
transition.

## Goals and non-goals

The design must:

- reuse unchanged physical input, expansion, paragraph construction,
  typesetting kernels, page-building work, and shipout lowering at the
  narrowest profitable sound boundary;
- make page numbers, insertions/footnotes, marks, streams, writes, diagnostics,
  included inputs, fonts, RNG, and output routines explicit dependencies or
  replayed effects;
- retain precise reuse after unrelated state changes through dynamic read
  constraints;
- preserve the single aggregate mutation boundary: instrumentation observes
  `Universe`, `InputStack`, `ModeNest`, and `World` capabilities rather than
  bypassing them;
- keep ordinary batch execution free of cache locks, atomics, and unbounded
  recording overhead;
- store no cache entry whose correctness depends on process-local handles,
  allocation order, provenance ids, or stale revision offsets;
- bound session memory and make every persistent cache entry schema-versioned,
  validated, and safely discardable; and
- measure cold overhead, edit latency, validation cost, replay cost, hit rate,
  retained bytes, and invalidation reasons independently for every layer.

The initial implementation does not include speculative parallel execution,
background workers, a distributed cache, or host-visible side-effect replay.
Shell escape and unpinned interactive input remain memoization barriers.

## Computation classes

Umber uses four explicit computation classes rather than one generic cache
policy.

### Pure queries

A pure query has immutable, explicit inputs and returns a detached semantic
value. It neither reads hidden engine state nor mutates the engine. Examples
include line breaking after all parameters and hyphenation inputs have been
captured, packing with explicit diagnostics parameters, math lowering with an
explicit style/font context, and DVI planning from a detached page artifact.

Pure queries use content keys and need no generic interpreter read set.

### Constrained read-only queries

A constrained query may read through a tracked aggregate facade but has no
observable mutations. The memo entry stores each tracked method's semantic key
and observed return value. A hit is valid when replaying those reads against
the current engine returns semantically equal values.

Recursive expansion of a frozen token list is a candidate once all input-stack
transitions and semantic interning are either explicit results or proven
absent.

### Replayable transactions

A replayable transaction may read and mutate engine state. Its entry stores:

- external read constraints;
- an ordered semantic redo log;
- virtual effect and diagnostic records;
- a detached input-stack transition;
- detached semantic output; and
- a postcondition used by debug and shadow verification.

On a hit, validation occurs before mutation. Replay then uses ordinary
aggregate write APIs in the original order. A failed validation leaves the
live engine unchanged and executes the region normally.

Paragraph front ends, page-builder episodes, and eventually output routines
belong to this class.

### Barriers

A barrier executes normally and prevents a containing region from being
replayed until a specific design makes it deterministic. Initial barriers
include shell escape, unpinned terminal input, host clock reads not reduced to
the fixed job clock, unvirtualized filesystem effects, and unsupported input
continuations.

## Stable input and trace identity

Raw byte offsets and whole-file content hashes are insufficient trace
identities. Inserting one byte near the root shifts every later offset, and TeX
does not have a stable syntax tree because catcodes and macro expansion are
execution-dependent.

The editor root becomes an immutable rope or piece table. Each unchanged piece
retains a session-stable `PieceId`; revisions replace only pieces intersecting
the edit. A root source span is identified by:

```text
RootSpanId = (PieceId, local byte range, byte-content identity)
```

Included inputs remain content-addressed `InputRecord`s. Generated inputs such
as `\scantokens` use their semantic producer identity plus generated content
identity.

Derived delivery identities are separate from diagnostic provenance:

```text
Source delivery    = source span + token ordinal
Macro delivery     = definition semantic id + invocation id + argument path
Token-list replay  = token-list semantic id + index + parent invocation id
Synthetic delivery = inserted kind + stable parent delivery
```

`OriginId` remains a process-local diagnostic side channel and never becomes a
memo key. Cached results store relative provenance recipes that allocate fresh
origins against the current revision when reused.

Multiple identical source fragments may share pure cached values, while their
trace nodes retain distinct stable occurrence identities for diagnostics and
ordered replay.

## Dependency keys, observations, and validation

The typed `ReadDependency` vocabulary currently in `tex-expand` is the seed of
a state-layer `DependencyKey`. The complete vocabulary covers:

- environment cells: meanings, registers, parameters, boxes, current font,
  family fonts, and font parameters;
- individual code-table entries and table generations;
- immutable font metric and selected-font identities;
- hyphenation patterns, exceptions, language, and related parameters;
- input records, physical lines, streams, and terminal-input cursors;
- input-stack, conditional-stack, group, and mode facts exposed to TeX;
- page dimensions, page integers, contribution/current-page roots,
  insertion state, marks, discards, and break state;
- `World` virtual streams, effect-policy state, shell-escape policy, job clock,
  interaction mode, and loaded resources; and
- RNG and other future-observable engine scalars.

Each observation stores:

```text
ObservedDependency {
    key,
    changed_at stamp,
    semantic observation,
}
```

The semantic observation is the scalar value or a canonical content reference.
Process-local raw handles may be retained only as private lookup accelerators;
validation follows them to semantic content.

Validation uses a red/green-style fast path:

1. if `changed_at` has not advanced, the dependency is valid without reading
   or hashing its content again;
2. otherwise, read its current semantic value and compare it with the recorded
   observation; and
3. if the value is equal, backdate the dependency so downstream queries do not
   repeatedly invalidate on an allocation-only or changed-then-restored path.

Dependencies are deduplicated deterministically within a region. Nested query
hits become dependencies on the child query's changed-at result rather than
copying the entire child read set into every parent.

The disabled recorder path remains one predictable optional branch at facade
boundaries. Measurements decide whether very hot reads use per-cell
instrumentation, coarser bank generations, or explicit query parameters.

## Mutation, input, and effect replay

Skipping interpreter execution requires more than validating reads. Every
observable state transition must either be reproduced or make the region a
barrier.

### Semantic redo log

All writes already cross aggregate state APIs. A region recorder observes
those writes and stores typed operations such as:

```text
SetCell { key, scope, semantic_value }
EnterGroup { kind }
LeaveGroup
SetModeField { field, semantic_value }
ReplacePageRoot { detached_page_transition }
AdvanceVirtualStream { stream, operation }
```

The first implementation validates the old semantic value of every written
location before replay. Later, a proven blind overwrite may omit that
dependency. Ordered group operations, `\aftergroup`, `\afterassignment`, box
consumption, and local/global assignment semantics may never be collapsed into
an unordered final-value map.

Values naming tokens, glue, macros, nodes, boxes, or fonts are stored as
detached semantic references. Replaying into a scratch `Universe` imports or
interns that content through one aggregate API. Old-generation handles never
cross the ownership boundary.

### Input transition

A replayable region stores the exact input it consumed and a detached ending
transition. Validation proves that every consumed source span, included input,
token-list body, and generated input is unchanged and that the entry input
summary is compatible. Applying the transition recreates current-revision
source frames and fresh diagnostic provenance atomically.

The transition is region-specific, not a general public input-summary mutation
API.

### Effects and diagnostics

Editor sessions already retain virtual effects and detached artifacts before
host materialization. Replay appends the recorded virtual effect and diagnostic
slice in its original order. Deferred writes remain split into construction
and shipout-time expansion: a cached whatsit does not imply that its eventual
expanded write is reusable.

Once effects have been materialized to the host, no memo entry may replay
across that one-way barrier.

## Memoization boundaries

### Input and line normalization

Physical-line indexing, unchanged rope pieces, UTF-8 validation, and TeX line
normalization are content-addressed. Normalization is keyed by physical bytes,
terminator metadata, `\endlinechar`, and `\scantokens` mode.

Whole-line tokenization is not assumed pure. TeX tokenizes on demand, and an
executed catcode assignment earlier on a physical line can change how later
characters on that same line tokenize. Token cache entries therefore record
the initial lexer state and the exact catcode/code-table observations used by
the delivered span. Fine-grained tokenization is adopted only if its measured
benefit exceeds validation overhead.

### Macro substitution and expansion

Pure macro parameter substitution is keyed by the macro-definition semantic
identity, semantic argument token lists, delimiter structure, and expansion
mode. It returns a detached substituted token list. Argument scanning remains
ordinary execution.

Recursive expanded-stream reuse is recorded as an expansion episode with a
stable input trace, dynamic read constraints, returned semantic tokens, and an
ending input transition. Episodes are initially bounded by outer executor
dispatch or a caller-owned scanner operation; the cache does not manufacture
arbitrary durable Rust continuations.

Expansion episodes that open inputs, perform untracked relaxed interning,
consume interactive input, or cross a barrier execute normally until those
operations gain explicit replay semantics.

### Paragraph pipeline

Paragraph completion is split into three independently reusable operations:

```text
consumed token trace + tracked entry state
    -> prepared horizontal list

prepared horizontal list + captured layout/hyphenation parameters
    -> break and line-materialization plan

vertical contribution chunk + page-builder entry state
    -> page-builder transition
```

The first paragraph-front-end implementation caches only dynamically proven
simple regions: no persistent writes, no virtual effects, no input opens, no
nested shipout/output routine, and a supported detached input transition. A
hit imports the prepared hlist and advances input; line breaking and page
building still execute.

The second implementation allows semantic redo and virtual effects. The third
composes repeated command/expansion children beneath the paragraph trace node.

The pure layout key includes the prepared hlist's verified semantic content,
all `LineBreakParams` and `PostLineBreakParams`, hyphenation input identity,
font metric identity, language and `\hyphenchar` state, packing parameters,
and diagnostic thresholds. Cached decisions use stable node positions and
detached content rather than arena handles.

Migrating marks, insertions, and adjustments remain explicit outputs of line
materialization and inputs to page building.

### Page builder, insertions, and marks

Page building is a sequential transducer, not a pure function of a paragraph.
A page episode consumes an immutable vertical contribution chunk against an
explicit page root and tracked environment reads. Its result contains:

- the next page root and remaining contribution queue;
- insertion-box and page-register mutations;
- mark transitions;
- output-fire metadata;
- diagnostics; and
- any produced output-routine child trace.

The page root includes page totals and glue orders, page goal/depth, last-item
state, best break, insert penalties, dead cycles, current-page and contribution
roots, insertion state, mark classes, and page/split discards.

Insertion processing explicitly observes `\count`, `\dimen`, `\skip`, and box
state for the insertion class. A cached paragraph may still hit when insertion
capacity differs; only its page episode must miss. Footnote content is reusable
immutable material, while footnote placement and splitting are page-state
dependent.

Mark reads performed during expansion and mark updates performed at page fire
are typed dependencies and mutations. A changed mark invalidates only queries
that observe it or page transitions that consume it.

### Page numbers and output routines

A page number is ordinarily an environment register, not an implicit global
cache invalidator. A paragraph containing `\the\pageno` records that register
read and misses when it changes; unrelated paragraphs remain reusable.

An output routine is arbitrary TeX execution. It initially always executes as
a child barrier inside the page episode. Later it may become a replayable
transaction only after complete coverage exists for its environment, group,
input, box-255, mark, insertion, stream, effect, nested shipout, and diagnostic
behavior. Its transaction is validated and applied atomically.

### Shipout and output encoding

Shipout-time deferred-write expansion remains a tracked execution episode.
After expansion and leader suppression, the detached page artifact, page effect
slice, font resources, magnification, and output schema form explicit pure
inputs to artifact lowering and DVI planning.

Content-addressed artifact bytes may be reused immediately. Revision output
ordering remains session metadata and is never inferred from cache lookup
order.

## Hierarchical execution trace

The accepted revision owns an ordered persistent trace whose leaves are input,
expansion, paragraph, page, output, and shipout computations. Parent nodes
summarize children without changing their semantic order.

After an edit, execution restarts from the latest retained named checkpoint
whose consumed prefix is unchanged. From there the session walks the mapped old
trace:

1. map and validate the node's stable input identity;
2. validate its external dependency constraints;
3. on a hit, import its detached result, replay its ordered state/effect delta,
   and advance input;
4. on a miss, execute normally and record a replacement node; and
5. continue, allowing later nodes to hit even when an earlier node missed.

This is the general suffix-reuse mechanism. An upstream page miss does not
force downstream paragraphs to miss unless their own dependencies changed.

Parent trace nodes reduce internal read-after-write dependencies: a read
satisfied by an earlier child write is internal to the parent. The parent's
external constraints contain only values required from before the subtree;
its redo log and outputs compose children in order. Once composition is proven,
an entire unchanged subtree can validate and replay at once.

The trace begins flat and gains hierarchy only after leaf correctness and
overhead are measured. Parent summaries are derived accelerators and may be
dropped without invalidating leaf entries.

## Optional exact-state splice

An exact current-state comparison may stop trace walking and adopt the
remaining old trace when all future-relevant roots match. This identity is
separate from the folded history hash and includes every live continuation
root while excluding already detached output history.

The fast comparison may use immutable root identity and versioned strong
semantic digests, but a cache hit cannot rely on an unverified 64-bit collision.
Where roots are not shared, structural comparison or a stronger canonical
identity verifies equality before suffix adoption.

Failure to match merely continues trace validation and replay.

## Cache ownership, trust, and eviction

The first cache is session-local, single-threaded, and byte-budgeted. It owns
detached semantic values and stable input metadata independently of one
`Universe` generation. It uses deterministic LRU or clock eviction with
per-kind retained-byte and hit/miss accounting.

Checkpoint-root retention, memo-result retention, and required accepted-output
retention are reported separately. Evicting a memo entry never evicts accepted
output or the named checkpoints required for correctness fallback.

Cross-run persistence is considered only after session-local measurements show
material benefit. Persistent entries include:

- query kind and schema version;
- key and dependency schema versions;
- engine/format compatibility identity;
- canonical detached inputs and results;
- integrity identity; and
- no raw handles, paths without pinned content, or provenance ids.

Every persistent entry is untrusted. Decode, bounds, dependency, and semantic
validation failures are cache misses, never execution errors. No persistent
cache entry may authorize a host-visible effect.

## Instrumentation and measurements

Every accepted revision reports per layer:

- lookup count, hit count, miss count, and invalidation reason;
- bytes mapped, tokens delivered, commands skipped, paragraphs reused,
  line-breaking plans reused, page episodes reused, and pages adopted;
- lookup, dependency-validation, semantic-comparison, import, replay,
  execution, page-building, splice, and output-write latency;
- cache-entry count, logical bytes, retained bytes, evictions, and protected
  output/checkpoint bytes; and
- disabled-path overhead under paired interleaved measurement.

The edit corpus includes:

- no-op, comment, ignored-space, and same-semantic-token edits;
- word edits preserving and changing line breaks;
- edits changing page count and edits whose pagination later stabilizes;
- footnote insertion, removal, movement, and splitting;
- marks and running headers;
- page-number reads and output-routine increments;
- immediate and deferred writes, labels, and auxiliary streams;
- preamble macro, register, font, language, and hyphenation changes;
- catcode and `\endlinechar` changes within and across physical lines;
- included-input changes and generated input;
- alignments, displays, boxes, math, grouped paragraphs, output routines, and
  nested shipouts; and
- multi-revision edits before and after reused trace regions.

The primary performance workloads are multi-page Story/Gentle-class documents
with edits near the start, middle, and end. A one-page edit with zero reused
pages is a restart-latency measurement, not evidence of suffix reuse.

## Correctness and verification

For every memo layer, cache-on and cache-off execution must agree with a fresh
cold run on:

- DVI bytes and ordered artifact identities;
- virtual and materialized effect records;
- terminal/log diagnostics and ordering;
- final environment, input, mode, page, stream, RNG, and resource state;
- liveness and ownership validation; and
- source mapping after current-revision provenance rebind.

Tests include targeted mutation invalidation for every dependency key,
write/effect replay for every mutation class, collision candidates, malformed
persistent entries, memory-budget eviction, cancellation before replay commit,
and second/third edits through previously reused trace nodes.

The fast committed tier and an explicit 1,000-edit scripted/fuzz tier compare
incremental and cold results. Relevant corpus parity, snapshot budgets,
profiling, `cargo test --tests`, and `scripts/check.sh` gate each
rollout phase.

### Named-checkpoint baseline before memoization

The committed `tex-incr` multi-page baseline uses 20 independently shipped
pages and edits page 11 after restoring the preceding `ShipoutComplete`.
An optimized macOS run on 2026-07-15 recorded the following diagnostic sample;
the work counters are deterministic, while the timings are observations rather
than performance gates:

| Edit | Fork | Re-execute | Splice | Bytes / tokens / dispatches | Hash checks | Pages retyped / reused | Retained checkpoint / diagnostic / output bytes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| comment-only | 110 us | 58 us | 135 us | 53 / 2 / 2 | 1 match | 1 / 9 | 159,666 / 1,772 / 3,500 |
| semantic rule-width change | 222 us | 251 us | 212 us | 530 / 22 / 22 | 10 mismatches | 10 / 0 | 158,954 / 1,805 / 3,500 |

The semantic case demonstrates the limitation this design addresses: after
the edited page, every later schedule entry is comparable but its folded
history hash remains divergent. The comment case preserves identical history
and adopts the suffix at the first comparison. The scenario matrix beside the
baseline also checks changed paragraph content, page-number reads, marks,
deferred writes, page-count changes, output routines, and insertions against a
fresh cold execution.

### Dependency-recorder baseline

The state-layer recorder has an explicit disabled branch and no lock or atomic.
An optimized arm64 macOS run on 2026-07-15 interleaved disabled and enabled
recording over 4,096 reads per sample. One committed Criterion command
`cargo bench --manifest-path benchmarks/tex-state/Cargo.toml --bench
state_budgets -- dependency_recording --warm-up-time 1 --measurement-time 2
--sample-size 20` measured a median 2.546 us for the disabled batch (0.622
ns/read), 42.871 us for enabled deterministic deduplication (10.47 ns/read),
and 51.807 us for the paired interleaved batch. The separate `dependency_gate`
uses `black_box` at the aggregate facade to prevent specialization of the
known-disabled state. Twelve rotated samples over 2,000,000 reads measured a
0.683 ns/read control, 0.949 ns/read disabled facade, a 0.266 ns/read
incremental disabled cost, and 49.446 ns/read enabled cost over a rotating
32-key set. These are diagnostic observations, not latency gates; later query
layers must repeat the paired end-to-end comparison because their key
distributions differ.

## Implementation sequence

1. **Contract correction and baseline.** Reclassify folded hashes as
   identical-history checks, add genuine semantic-divergence workloads, and
   measure current restart/re-execution behavior.
2. **Stable revision input.** Add piece-based root identity, incremental line
   indexing/normalization, mapped delivery identities, and provenance recipes.
3. **Memo runtime and dependency validation.** Move dependency keys to the
   state layer, add observed values and changed-at/backdating, and install
   region-scoped recorders with a measured disabled path.
4. **Detached semantic cache values.** Add schema-versioned import/export for
   token, glue, macro, node, box, font, input-transition, and page-transition
   values without cross-generation handles.
5. **Pure-kernel memoization.** Land and measure line breaking,
   post-line-break/packing, math/box kernels, and shipout/DVI planning.
6. **Expansion memoization.** Add pure macro substitution followed by tracked
   expanded-token episodes and supported input transitions.
7. **Paragraph-front-end reuse.** Cache simple effect-free paragraphs, then add
   semantic redo and virtual-effect replay.
8. **Page-builder episodes.** Record and replay page transitions including
   insertion/footnote and mark behavior, initially treating output routines as
   barriers.
9. **Output and shipout execution episodes.** Add complete output-routine and
   deferred-write tracking/replay only after their dependency and mutation
   audits pass.
10. **Hierarchical trace composition.** Compose validated leaves into subtree
    summaries and add the optional exact-current-state suffix splice.
11. **Persistence and release gate.** Add byte-budgeted eviction, consider
    World-backed persistence from measurements, run the complete edit/corpus
    matrix, and enable editor-session memoization only after independent review.

Each phase is independently useful and may stop if end-to-end measurements do
not justify its complexity. Later phases depend on the correctness contracts of
earlier phases, not merely on their API presence.

## Related work

- Typst's constrained memoization records the state methods a computation
  actually observes and validates their outputs before reuse:
  <https://laurmaedje.github.io/posts/comemo/>.
- `comemo` also demonstrates replay of mutations through a tracked mutable
  argument, subject to strict impurity boundaries:
  <https://docs.rs/comemo/latest/comemo/attr.track.html>.
- Salsa's red/green algorithm supplies changed-at validation and backdating of
  semantically unchanged recomputations:
  <https://salsa-rs.github.io/salsa/reference/algorithm.html>.
- _Fast Typesetting with Incremental Compilation_ discusses page counters,
  footnotes, cyclic document dependencies, and constraint-based layout caches:
  <https://doi.org/10.13140/RG.2.2.15606.88642>.
- The TeX `memoize` package documents why opaque externalized boxes cannot
  participate normally in line and page breaking:
  <https://ctan.org/pkg/memoize>.
