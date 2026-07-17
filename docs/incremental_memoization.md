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

Changed-at metadata is operational rollback state. Its key map is an immutable
shared root, so checkpoints and accepted-generation clones retain it in O(1).
Rollback restores that root while a monotonic clock remints only facts touched
on the abandoned branch; unrelated stamps remain valid. Group exit obtains the
deduplicated environment cells actually present in the popped journal slice,
marks only those restored facts, and marks only code-table generations whose
roots changed. Group level and group kind are the two unconditional group-exit
facts. The broad `World` mutation escape hatch remains a conservative cold-path
fallback; capability-specific World mutations keep their narrow keys.

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

The implemented paragraph redo subset records ordered count-register and
integer-parameter writes, including their local/global scope and their old and
new scalar values. A hit first simulates the complete log against current
values, including read-after-write chains, and performs no mutation on a red
precondition. Detached hlist import is also completed before any write. Only
then are ordinary `Universe` setters invoked in order. This preserves local
group restoration and global escape while avoiding a speculative snapshot,
whose checkpoint-fold side effects would perturb convergence identity.
Literal input and these explicitly recognized assignment commands share the
same bounded raw semantic-token transition. Unsupported commands—including
arithmetic assignment, token/glue/font/box mutation, input opens, generated
input, deferred writes, output, and shipout—bar the entire
paragraph rather than allowing a reusable suffix. Those families remain cold
until their detached value and ordered replay forms are implemented; they are
not silently treated as pure.

Literal `\message`/`\errmessage` regions are the first virtual-effect class.
Their ordered `StreamWrite` records are detached into sink, optional stream,
and UTF-8 payload, then appended through the ordinary `World` write boundary
on a hit. Stream open/close, deferred token writes, specials, PDF placeholders,
and shell escape are rejected when publishing an entry. The prepared hlist
stores one input-token ordinal per character or ligature source character;
reuse resolves those ordinals against the current trace, so command operands
and diagnostic arguments cannot shift rendered provenance.

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

Macro parameter substitution remains the existing lazy token-list replay.
An eager memo layer was removed after measurement showed that copying the
definition and arguments, hashing them, materializing a replacement, and
interning that replacement duplicated more work than it avoided.

Recursive expanded-stream caching was removed after its supported general-text
boundary repeatedly reported zero Gentle lookups, hits, entries, retained
bytes, and evictions. Expansion continues through ordinary lazy token-list
replay. Dynamic expansion dependency recording remains at the shared facade
boundary because accepted-generation paragraph traces consume those reads; it
is not coupled to a standalone result cache.

A repeated ABBA rerun after removing eager
substitution produced six stable 10-run blocks: disabled averaged 131.62 ms and
enabled averaged 131.24 ms. The 0.29% difference is noise-level parity, while
both enabled runs reported zero lookups and zero retained bytes; the earlier
roughly 4% sequential difference was not reproducible under stable load.
An additional eight-block order-balanced rerun measured 128.88 ms disabled and
129.82 ms enabled; all memo counters were again zero, so the 0.73% difference
is disabled-path noise rather than evidence about episode reuse.

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

Cold paragraph eligibility is now recorder-driven rather than decided by the
original bounded control-sequence whitelist. Ordinary execution records stable
root-piece spans, deduplicated semantic observations, supported escaping
count/integer writes, virtual stream writes, and the ending input summary, then
classifies the region at `\par`. `\everypar` is an ordinary token-parameter
dependency, and a symbol constructed by `\csname` contributes its observed
meaning directly to the paragraph recorder.
Display math, `\scantokens`, mid-paragraph input opens/`\endinput`, untracked
World effects, unsupported escaping writes, and nested output routines retain
explicit, counted barrier reasons. Group-closed work remains part of normal
cold execution and does not require a whitelist exception.

The pre-paragraph source region may also contain vertical-mode commands before
the paragraph actually starts. Because paragraph redo does not yet represent
group entry or exit, recording captures the execution-group depth and its
changed-at stamp at the stable candidate anchor and rejects a region after any
group-stack mutation, including a balanced exit/entry pair whose net depth is
unchanged. This explicit group-transition barrier prevents replay
from skipping a macro-expanded `\endgroup` (or `\begingroup`) and preserves the
group lineage required by retained named checkpoints. Regions that only read
or mutate ordinary state within an unchanged group remain eligible.

Prepared paragraph hlists are anchored as survivor graphs owned by exactly one
accepted generation. These roots are deliberately independent of checkpoint
rollback pins, so a fork may roll back to an earlier paragraph boundary and
still import a later prior-generation result through the aggregate state
facade. Generation retention also preserves immutable glue content referenced
by those graphs and remaps it into the scratch timeline during import. A hit
validates stable consumed spans, semantic dependencies, the
ordered redo log, detached effects, and the revision-relative ending input
continuation before importing any nodes or replaying any mutation. Invalid or
stale metadata leaves the live state untouched and runs the paragraph cold.

The operational runtime keeps separate prior-accepted and speculative trace
vectors. Acceptance replaces the trace and its generation roots wholesale;
failed branches discard the speculative vector, while exact suffix convergence
continues to use the still-retained prior generation. Paragraph results no
longer occupy detached per-entry memo payloads, and a cold generation cannot
hit entries that it is still recording.

The accepted-generation result now has a second, dependency-tiered root for
finished line boxes, migrating material, and interline penalties. Its explicit
read set covers line dimensions and shape, scalar and e-TeX penalty arrays,
line-breaking and packing parameters, font metrics and hyphen characters,
language-local patterns/exceptions/saved codes, and the lccodes of paragraph
characters. If that set validates, replay imports current-provenance finished
lines and contributes them to the live vertical/page-builder boundary. If only
that set fails, replay imports the prepared hlist and performs line breaking
and materialization again. A front-end dependency, mutation, effect, input, or
barrier failure remains a completely cold paragraph. Both hit tiers publish
new generation-owned roots, so reuse remains available over later edits.

The second implementation allows semantic redo and virtual effects. The third
composes repeated command/expansion children beneath the paragraph trace node.

The pure layout key includes the prepared hlist's verified semantic content,
all `LineBreakParams` and `PostLineBreakParams`, hyphenation input identity,
font metric identity, language and `\hyphenchar` state, packing parameters,
and diagnostic thresholds. Cached decisions use stable node positions and
detached content rather than arena handles.

Migrating marks, insertions, and adjustments remain explicit outputs of line
materialization and inputs to page building.

The first measured pure-query implementation caches only the pretolerance
`BreakPlan`. It computes four independently domain-separated semantic
projections of the prepared hlist in one traversal, then canonically frames
every `LineBreakParams` and `LineShape` field. The compact 64-bit projection
selects a bucket; a 256-bit content identity verifies the candidate. The
session-local hot path retains a typed plan containing only break positions,
demerits, and detached last-line glue; schema encoding is reserved for a future
persistence boundary rather than paid on every hit. Hyphenation,
post-line materialization,
packing diagnostics, math lowering, and DVI planning remain ordinary execution
until their complete explicit keys are implemented by their owning phase.

The cache runtime is owned by the long-lived editor session and lent to each
scratch execution attempt, so accepted revisions reuse it without including it
in snapshots, formats, rollback state, or semantic hashes. It is bounded by
entry count and retained bytes and remains off by default. The disabled facade
is one `Option` branch with no hashing, lock, or atomic operation. On the
128-node `linebreak_memo` Criterion workload
(2026-07-15), raw pretolerance measured 3.99 ms, the disabled facade 3.55 ms
(benchmark noise, no measurable regression), and a strong-key-verified detached
hit 10.18 us, about 392x faster. A cache-on/off executor test with repeated
paragraph content verifies identical DVI plans, virtual effects, and final
semantic state. The `pure_memo_accepted_edit` benchmark edits the first of two
otherwise identical 128-rule paragraphs. Disabled execution measured 0.919 ms;
enabled execution measured 1.205 ms, about 31% slower. A rerun after fixing
cross-revision ownership measured 1.397 ms disabled and 2.025 ms enabled
(20-sample point estimates in a noisy run), so persistence alone does not make
this edit workload a win. Existing named-boundary
convergence skips the unchanged second paragraph, leaving only a strong-key
miss on the edited paragraph. The layer therefore remains off by default.
After the typed-plan and one-pass-key redesign, a short 20-sample rerun measured
2.048 ms disabled and 2.072 ms enabled at the point estimates, with overlapping
wide intervals and no detected difference. This removes the demonstrated
hot-path regression but does not establish a win. The epic nevertheless
continues into paragraph-front-end reuse; measurements select default
enablement at the release gate rather than stopping implementation phases.

The 2026-07-16 removal review compared paragraph-only against
paragraph-plus-pretolerance in a bounded ABBA sequence of two-run incremental
blocks. The first adjacent pair favored pretolerance on all four edits, while
the reverse pair lost heavily under visible host contention; the direction
therefore reversed instead of establishing neutral or positive removal.
Pretolerance remained active at 936/937 and 937/937 hits on the large and
inverse edits, retained about 200 KiB, and spent measurable time constructing
and validating keys. The layer remains opt-in pending conditioned evidence;
this inconclusive result does not justify deleting useful traffic.

The shipout profile was also rechecked before widening the cache boundary:
1,024-node ordinary lowering measured 269.75 us and deferred-math shipout
4.46 ms. The expensive case includes required math normalization and execution,
while the already-fused pure DVI planning slice is not independently dominant;
artifact/DVI caching therefore remains disabled rather than retaining a second
page representation without demonstrated benefit.

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

The implemented page layer keys the canonical allocation-independent page
root together with the narrow builder environment (`vsize`, `maxdepth`,
`topskip`, vertical-discard policy, and the count/dimen/skip/box tuple for each
observed insertion class). A successful cold transition detaches the complete
contribution, current-page, discard, insertion, break, and fire-up result as one
node graph plus scalar state. Replay imports that graph before atomically
replacing the page root. Character and ligature provenance is stored as input
ordinals and rebound to the current revision during import. Any diagnostic or
other world effect bars publication. Output-routine execution remains outside
the entry and therefore executes normally after a replayed fire-up boundary.

### Page numbers and output routines

A page number is ordinarily an environment register, not an implicit global
cache invalidator. A paragraph containing `\the\pageno` records that register
read and misses when it changes; unrelated paragraphs remain reusable.

An output routine is arbitrary TeX execution. It initially always executes as
a child barrier inside the page episode. Later it may become a replayable
transaction only after complete coverage exists for its environment, group,
input, box-255, mark, insertion, stream, effect, nested shipout, and diagnostic
behavior. Its transaction is validated and applied atomically.

The release implementation deliberately retains that execution barrier for
custom output routines: their invocation count is reported, and all state,
input, diagnostics, and nested shipouts execute through the ordinary engine.
The reusable boundary is placed later, after a shipout page has passed deferred
expansion and normalization. Effect-free, already-normalized box graphs use a
key over semantic node content, magnification, offsets, and the ten TeX page
counters; their canonical artifact bytes and render-provenance ordinal recipe
are detached. Replay publishes those bytes through the ordinary atomic shipout
transaction. Deferred writes, stream operations, specials, math normalization,
directions, insertions, shell escape, and other mutable lowering surfaces are
counted barriers and always execute. This narrower boundary avoids treating
arbitrary TeX execution or host effects as pure while removing repeated
artifact lowering and DVI planning where the equivalence proof is complete.

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

The retained named-boundary walk remains flat. `tex-incr::TraceSummary` now
provides the derived hierarchy over reusable leaves: it composes ordered read
and write operations, removes a child read only when an earlier child write has
the identical semantic value, and rejects conflicting external or internal
observations. Redo, input, effect, and output transitions remain in child
order. Validation of every external read completes before any transition is
applied, so a failed parent is an atomic miss. Focused tests replay nested
parents and the corresponding leaves independently and require identical
state, write order, input, effects, and outputs. Summaries own no correctness
state and may be discarded to recover leaf-by-leaf walking.

## Optional exact-state splice

An exact current-state comparison may stop trace walking and adopt the
remaining old trace when all future-relevant roots match. This identity is
separate from the folded history hash and includes every live continuation
root while excluding already detached output history.

The fast comparison uses immutable root identity and a versioned, fixed-seed
64-bit aHash over canonical semantic projections. Hash equality is authoritative
for suffix adoption: the session-local path deliberately performs no SHA-256 or
structural verification. A rare 64-bit collision may therefore produce
incorrect reuse; this is an accepted performance tradeoff rather than a durable
integrity guarantee. Domain/schema changes invalidate compatibility, while the
fixed seeds preserve deterministic identities across forks and rollback within
one compatible build/session. Durable content and persistence identities remain
cryptographic and separate.

Snapshots retain a fixed-size cache of component projections keyed by immutable
roots. Rollback restores those derived roots but not journal scratch, so exact
comparison composes unchanged input, page, stream, code-table, hyphenation, and
font-selection roots without traversing their contents. Append-only immutable
stores retain a bounded lineage cache per collection; accepted and scratch
forks therefore extend separate persistent roots, and allocator ancestry turns
divergence into a cache miss rather than unsafe reuse.

Failure to match merely continues trace validation and replay.

The implemented splice walks the flat ordered named-boundary trace after the
restart point. Every mapped boundary is attempted even after an earlier miss,
so a semantic edit can retype its changed pages and still adopt later pages.
Adoption now requires the 64-bit session-local identity of canonical store state, the
allocation-independent detached page transition, an exact future-input
comparison that ignores only revision-relative coordinates, exact mode state,
and exact future-relevant World scalars. Existing detached effect and artifact
prefixes remain outside that comparison and are composed in order. Boundaries
inside open groups, or any boundary whose canonical projection cannot be
formed, are safe misses. The folded `state_hash` remains diagnostic telemetry
and is not consulted by the splice decision. Exact session-local identities are requested
only by incremental history sinks; ordinary rollback and profiling checkpoints
retain the O(1) snapshot path.

## Cache ownership, trust, and eviction

The detached cache is session-local, single-threaded, and byte-budgeted. It
owns detached semantic values independently of one `Universe` generation.
Recording policy is configured per layer. The default records only paragraph
regions, which are not detached-cache entries: their trace metadata and node
roots belong to the prior accepted generation and are replaced wholesale on
acceptance. Pretolerance, page, and shipout recording are opt-in experiments.

Detached admission is scan resistant through a first-reuse protection rule.
Once admitted, an entry cannot be evicted until it has received its first
lookup opportunity. If the protected working set fills the byte or entry
budget, later values are not admitted instead of evicting earlier values before
they can possibly hit. Focused state tests cover this budget-fit invariant.
After first lookup, deterministic CLOCK eviction applies normally.

Checkpoint-root retention, detached memo-result retention,
generation-anchored paragraph metadata, and required accepted-output retention
are reported separately. Evicting a memo entry never evicts accepted output,
paragraph-generation roots, or named checkpoints required for fallback.

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

- lookup count, hit count, and misses split into not-attempted, barrier (with
  barrier kind), key miss, validation failure (with first failing dependency
  family), evicted-before-reuse, and import failure;
- bytes mapped, tokens delivered, commands skipped, paragraphs reused,
  line-breaking plans reused, page episodes reused, and pages adopted;
- record, lookup, dependency-validation/key-construction, import, replay,
  execution, page-building, splice, and output-write latency;
- detached-cache entry count, logical bytes, retained bytes, evictions,
  generation paragraph-metadata bytes, and protected output/checkpoint bytes;
  and
- disabled-path overhead under order-balanced paired interleaved measurement.

Trace telemetry distinguishes named nodes walked, adopted page leaves, adopted
suffix subtrees, shallow retained trace bytes, exact dependency-validation
time, and ordered suffix-replay time. Prefix-retained, re-shipped, and adopted
page counts are separate, so their sum can attest that a verified splice
accounts for every output page.

The Gentle runner measures four accepted revisions in one session—the pinned
edit, a follow-up edit, removal of that follow-up, and the equal-width
height-preserving substitution—and verifies every mode against a fresh cold
DVI for the corresponding revision. Adjacent disabled and enabled samples
alternate AB/BA order, and conclusions use their paired differences rather
than independent sequential means. The fourth edit must reconverge at a
`ShipoutComplete` boundary, re-ship exactly the three changed pages, account
for the complete retained prefix, and adopt every remaining page as one suffix
subtree; the report prints both incremental-to-cold latency ratios.

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

| Edit                       |   Fork | Re-execute | Splice | Bytes / tokens / dispatches |       Exact checks | Pages retyped / reused | Retained checkpoint / memo / diagnostic / output bytes |
| -------------------------- | -----: | ---------: | -----: | --------------------------: | -----------------: | ---------------------: | -----------------------------------------------------: |
| comment-only               | 109 us |     267 us | 111 us |                  53 / 2 / 2 |            1 match |                  1 / 9 |                            166,130 / 0 / 1,772 / 3,500 |
| semantic rule-width change | 122 us |     498 us | 111 us |                 106 / 4 / 4 | 1 miss, then match |                  2 / 8 |                            164,378 / 0 / 1,805 / 3,500 |

The semantic case now demonstrates the intended hierarchy: the changed page
misses, the next exact boundary matches, and the remaining eight-page suffix is
adopted. The comment case adopts at the first comparison. The scenario matrix
beside the baseline also checks changed paragraph content, page-number reads,
marks, deferred writes, page-count changes, output routines, and insertions
against a fresh cold execution.

### Final release evaluation

The 2026-07-15 release pass kept the cache session-local and opt-in. A paired
20-sample `pure_memo_accepted_edit` run measured 1.844 ms at the disabled point
estimate and 2.582 ms enabled, about 40% slower for that workload. The memo
layers therefore remain available for explicit experiments but are not enabled
by default. Cross-run persistence is deferred: session-local reuse has not yet
shown an end-to-end benefit that justifies an untrusted persistent format,
compatibility policy, integrity validation, and disk I/O.

The optimized Gentle runner produced the pinned 97 pages and 263,424-byte DVI.
Twenty measured default runs averaged 215.591 ms; the ordinary named-checkpoint
path averaged 221.272 ms and captured 1,108 checkpoints, a 2.6% observation on
this run. These are profiling builds with instrumentation and are diagnostic,
not thresholds. The explicit snapshot gate reported all budgets met, including
flat 208 ns ordinary input/page/stream/hyphenation/provenance/code-table
snapshots in that run. The explicit 1,000-edit incremental-versus-cold tier,
the complete workspace test/check gate, and Story, Gentle, TRIP, and e-TRIP
parity all passed.

A later long-document edit stress measurement inserted 1,792 words into one
paragraph 19.66% through Gentle. The output grew to 98 pages, invalidating the
remaining 84-page suffix. Five interleaved optimized samples measured 3.986
seconds mean with memoization disabled, 7.304 seconds with memoization enabled,
and 2.875 seconds for a cold edited compile. Thus memoization was 83% slower
than disabled incremental execution and 154% slower than cold on the means.
Only 385 of 7,140 lookups hit; the 64 MiB cache retained 67,008,455 bytes and
evicted 6,475 entries. This confirms that the current hierarchy does not pay
for itself when a large changed paragraph shifts pagination through the rest of
the document; the opt-in default remains appropriate.

The paragraph-generation verification pass on 2026-07-16 found and repaired
one release-path defect hidden by that earlier measurement: a nonempty
`\everypar` still bypassed prior-generation paragraph lookup, and reused
paragraph installation scheduled `\everypar` a second time even though its
recorded result was already present. Plain TeX's nonempty `\everypar` therefore
made Gentle attempt zero paragraph lookups before the repair. Focused
cross-revision coverage now requires a nonempty `\everypar` paragraph to hit
and remain cold-DVI-identical; the installer does not leave duplicate replay
tokens behind.

The repaired 1,792-word stress run remained cold-DVI-identical and attempted
32 paragraph lookups, with 24 hits, while retyping the 84-page changed suffix.
The 75% attempted hit rate and 6.3-second enabled versus 2.5-second disabled
single-sample observation do not satisfy the paragraph release gate. A smaller
28-word semantic insertion at the same 19.66% position retained 97 pages but
still walked 83 pages: three interleaved samples averaged 10.124 seconds
enabled, 3.561 seconds disabled, and 3.739 seconds cold, again with 24 of 32
paragraph lookups hitting. The limiting defect is candidate selection, not
recorder eligibility: lookup still recognizes only the old bounded raw-token
preflight, so macro-bearing downstream paragraphs never attempt validation.
`umber2-vfqs.15.5` implements the required stable raw-delivery candidate key.

Paragraph candidate selection now records a zero-width `RootSpanId` at the
live physical-source cursor immediately before expansion begins. The input
stack prepares the next physical line when necessary, so an unchanged line
keeps its fragment-local anchor even when the preceding edited line was
reminted. This pre-delivery lookup neither tokenizes the cold paragraph nor
allocates diagnostic provenance. A matching prior-generation anchor first
validates dependencies, redo logs, and effects, then maps every recorded raw
source span through the current editor layout. The opaque prepared transition
can advance the physical reader and restore the recorded lexer cursor without
delivering tokens or observing catcodes; a validation miss leaves the live
input stack untouched. Paragraphs whose ending continuation still contains a
token-list or conditional frame carry `UnsupportedInputTransition` and remain
cold, because those arena-backed continuations cannot cross a generation
rollback. The recorded expanded trace also keeps its source ancestry so
imported node provenance is rebuilt from current root spans rather than
assuming raw and expanded token ordinals coincide.

The pinned 1,792-word Gentle edit now attempts the pre-delivery lookup for the
unchanged downstream macro-bearing population. In the optimized verification
run, 121 source-transition-eligible macro paragraphs hit; including the eight
conservatively unsupported token-frame continuations gives 121 of 129
otherwise-valid downstream candidates (93.8%). The memo-enabled incremental
DVI was byte-identical to both memo-disabled incremental execution and the cold
edited 98-page, 278,000-byte result.

An independent five-sample rerun on 2026-07-16 confirmed the 121 full-line
paragraph hits, zero hlist fallbacks, zero import failures, and the same exact
98-page DVI parity, but did not satisfy the latency gate. Memo-disabled
incremental execution measured 6.409 seconds mean and 2.967 seconds median;
memo-enabled measured 9.446 seconds mean and 8.562 seconds median. The enabled
path therefore lost by 47.4% on the means and 188.6% on the medians. Its
remaining detached cache made 8,754 lookups, retained 66,899,304 bytes, and
evicted 6,721 entries; page episodes alone made 5,378 lookups for 30 hits while
the pagination-shifting edit necessarily retyped 84 pages. Paragraph telemetry
reported 45 validation misses and no import failures. Of 845 recorded barrier
events, the runner attributed 103 to display math, one to a mid-paragraph input
open, and 104 to output routines; the remaining reason distribution is not yet
printed by the runner. Separated lookup, validation, import, and recording
latencies plus the complete reason taxonomy remain tracked by
`umber2-vfqs.16`, so this run cannot independently prove the per-paragraph cost
criterion even though it demonstrates that the stable-start candidate repair
engages the intended population.

These runs also strengthen the removal case for the standalone expansion
episode, which has no useful Gentle traffic. The pretolerance plan is
architecturally overlapped by accepted-generation finished-line reuse, but its
measured traffic requires a separate marginal verdict. Paragraph memoization
and the remaining detached experiments stay opt-in; the measured release
criteria are not met.

A post-main rerun on 2026-07-16 used six optimized AB/BA pairs, three accepted
edits per session, and a fresh cold DVI comparison for every revision. The
first large edit measured 1,700.926 ms disabled and 1,776.852 ms enabled by the
means (a 75.926 ms, 4.5% loss). The follow-up insertion measured 1,671.180 ms
disabled and 1,567.323 ms enabled (a 103.857 ms, 6.2% win). Removing the
follow-up measured 1,689.310 ms disabled and 2,043.332 ms enabled (a 354.022
ms, 21.0% loss). All modes matched the corresponding cold DVI exactly: the
three revisions emitted 100 pages and 279,176, 279,248, and 279,176 bytes.

The first two enabled revisions retained the expected macro-paragraph traffic:
121 and 122 finished-line hits, with no hlist fallback or import failure.
Dependency validation plus import cost 9.959 ms and 10.008 ms respectively,
well below their roughly 1.5--1.6 second reexecution phases. The first edit's
45 validation failures were 29 cell, 13 mutation, two input-transition, and one
meaning failures; the second edit's 22 failures were 20 cell and two mutation
failures. Each of those revisions recorded 571 barrier-reason events: 255
unsupported input transitions, 219 unsupported writes, 50 displays, and 47
output routines. Paragraph generation metadata retained 19,234,304 and
19,799,668 bytes, detached retention was zero, and no eviction occurred.

The removal revision did not preserve that behavior. It attempted no paragraph
lookup, reported 1,781 not-attempted regions, retyped all 100 pages, and retained
23,589,481 bytes of new generation metadata. Its 631 barrier-reason events were
285 unsupported input transitions, 238 unsupported writes, 54 output routines,
53 displays, and one input open. This complete lookup loss, together with the
end-to-end losses on two of the three revisions, leaves the release gate open.
No tuning was performed before recording this result.

The same post-main pass refined the isolated-cache decision. Expansion memo
remained entirely idle across two disabled and two enabled ten-run Gentle
blocks: zero lookups, hits, entries, bytes, and evictions, with exact
97-page/263,424-byte parity. Its apparent wall-time variation therefore
contains no cache signal, and the removal follow-up remains justified. The
pretolerance cache was not idle: a two-pair diagnostic run reported 834/835,
833/834, and 1,054/1,054 hits across the three edits, about 200 KiB retained,
and no eviction. That small sample is not an enablement verdict; it means
`umber2-vfqs.17` must isolate the cache's marginal end-to-end value before
removing it rather than treating removal as mechanically free.

`umber2-vfqs.17` subsequently removed the expansion cache, its public
configuration/statistics surface, and profiler flag while retaining paragraph
dependency recording. Its bounded ABBA pretolerance comparison reversed
direction under host contention, so pretolerance remains as an opt-in
experiment rather than being removed on inconclusive evidence.

The apparently inflated memo-enabled command count was actual main-control
work. `ExecutionStats::main_control_dispatches` increments only immediately
before scalar token dispatch; paragraph validation and import do not increment
it, and a successful paragraph replay contributes only to the separate
paragraph `commands_skipped` telemetry. On a paragraph key or validation miss,
preflight restores the scanned traced tokens to the input stack for ordinary
execution. Those physical-source words now retain a dedicated transient replay
kind, and horizontal main control consumes maximal `Letter`/`Other` runs with
their existing traced origins. Expansion, alignment, input ordering,
provenance, and paragraph recording remain unchanged; only the original first
vertical-mode character stays scalar before horizontal mode begins. Incremental
metrics continue to expose macro and source text-span tokens beside scalar
commands so this delivery accounting remains auditable.

The final default-policy verification on 2026-07-16 kept only paragraph
recording enabled and excluded the unresolved pretolerance experiment. Four
same-process AB/BA pairs preserved cold-DVI parity on every accepted revision,
the height-preserving edit re-shipped three pages and adopted the complete
83-page suffix, and paragraph validation plus import stayed below front-end
execution cost. The matching validation-eligible paragraph candidates cleared
the 90% hit target, detached retention and eviction stayed zero, and generation
metadata retained 19,112,980 bytes. Nevertheless, enabled-minus-disabled
paired mean deltas were +768.447, +207.602, +923.535, and +266.583 ms across
the large insertion, follow-up, inverse removal, and equal-width edit. The
committed matrix, external corpus parity, explicit 1,000-edit tier, snapshot
budgets, and full workspace gates passed, but the end-to-end speed criterion
did not. Paragraph memoization therefore remains a measured experiment rather
than a release-enabled policy; the concrete blocker is the enabled execution
loss, not parity, import cost, suffix correctness, retention, or pretolerance.

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
6. **Expansion memoization experiment.** Measure pure macro substitution and
   tracked expanded-token episodes; remove standalone layers without useful
   traffic while retaining shared dependency recording.
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

Each phase remains independently gated for correctness, atomic fallback, and
retention. Measurements inform the final default-enablement and persistence
decision, but do not stop implementation of later phases. Later phases depend
on the correctness contracts of earlier phases, not merely on their API
presence.

### 2026-07-15 continuation decision

Implementation continues through paragraph transactions, page-builder replay,
output/shipout replay, hierarchical trace composition, and the complete release
gate even though the first isolated memo layers did not improve end-to-end edit
latency. Caches remain off by default during development, with no lock or atomic
work on ordinary execution. The release phase decides which layers to enable
and whether World-backed persistence is justified after the complete
architecture and edit matrix exist.

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
