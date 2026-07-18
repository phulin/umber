# Incremental paragraph replay

Status: authoritative forward design for changed-document paragraph reuse.

The named-boundary session in
[`incremental_v1.md`](incremental_v1.md) remains the restart, retention,
rollback, effect-virtualization, and fast suffix-adoption substrate. This
document defines the one additional mechanism used when an edit changes page
state and full-state convergence does not occur: ordered replay of unchanged
paragraphs from the prior accepted execution while the ordinary page pipeline
rebuilds output.

The design deliberately has two paths and two reuse tests:

1. **Fast path — full boundary convergence.** A mapped named boundary has the
   same canonical future-state identity and mode state as the accepted record.
   The session stops execution and adopts the accepted effect, artifact,
   checkpoint, and paragraph suffix.
2. **Slow path — aligned paragraph replay.** Full state differs, usually
   because pagination changed, but later paragraph inputs and the semantic
   values they read remain valid. The executor mounts retained finished lines
   in accepted order and feeds them through ordinary page building and
   shipout.

There is no reverse paragraph-suffix hash. Stable source mapping determines
alignment, and dependency validation determines semantic validity. A suffix
hash would duplicate the accepted record order without eliminating either
check.

## Goals and non-goals

The design must:

- restart each edit from one retained named checkpoint with one `Universe`
  fork;
- retain the existing full-state suffix splice as an independent fast path;
- replay an unchanged paragraph after an earlier paragraph miss or pagination
  change without searching for content globally;
- validate every replayed paragraph before mutation or input advancement;
- preserve the cold named-boundary schedule, effect order, page artifacts,
  DVI bytes, and final state;
- keep paragraph artifacts tied to accepted-history handles that own their
  shared node roots, resource closures, and lazy provenance resolvers;
- make front-end failure local so later mapped paragraphs may re-align, while
  a changed line-breaking dependency switches the revision to one cold pass;
  and
- measure height/page-preserving and pagination-changing edits separately.

This phase does not design page-artifact patching, output/shipout reuse,
cross-run persistence, a hierarchical execution tree, speculative execution,
or a distributed cache. Page and shipout work remain ordinary slow-path work
until a later measured design addresses them directly.

## Implemented substrate

The repository already provides the required correctness substrate:

- stable root pieces and `RootSpanId` source intervals across revisions;
- an edit map for accepted boundary and source-anchor positions;
- restartable `JobStart`, `OuterParagraphEnd`, and `ShipoutComplete`
  checkpoints;
- immutable accepted-generation substrates and one validated restart fork;
- retained logical effects and page artifacts;
- accepted-history-owned finished-line mounts;
- shared immutable survivor payloads with mount-owned glue closures, ordinary
  rollback-local root pins, and lazy provenance resolvers;
- immutable chunked origin-record snapshots, attached only when paragraph
  history is accepted and decoded only for a diagnostic query;
- typed dependency keys, semantic observations, changed-at stamps, and
  backdating; and
- explicit paragraph mutation, effect, input-transition, and barrier records.

Canonical boundary identities are now captured while each accepted boundary's
`Universe` is live. Candidate comparison reads the retained identity directly;
it never forks or rolls the accepted substrate back merely to reconstruct an
old identity.

## Accepted history

An accepted revision conceptually owns:

```text
AcceptedRevision {
    substrate,
    boundaries: [BoundaryRecord],
    paragraphs: [ParagraphRecord],
    effects,
    artifacts,
    page_plans,
}
```

A boundary record contains:

```text
BoundaryRecord {
    key: (mapped root position, boundary kind, same-position ordinal),
    restartable checkpoint,
    canonical future-state identity,
    mode summary,
    effect prefix,
    artifact prefix,
}
```

The canonical identity includes every future-relevant `Universe` root,
including input, page-builder, World, interaction, and PDF state. Effects and
artifacts already preceding the boundary are represented by their ordered
prefixes and are excluded from the identity. Equality is the existing
session-local, fixed-seed 64-bit aHash contract; the accepted rare collision
risk does not change durable content identities.

A paragraph record contains the accepted result of one normally executed
outer paragraph:

```text
ParagraphRecord {
    starting and ending source anchors,
    consumed source spans and ending input transition,
    front-end dependency observations,
    line-breaking dependency observations,
    supported ordered mutations and virtual effects,
    replay barriers,
    retained finished lines,
    line count,
    opaque finished-line origins plus an accepted-history provenance resolver,
}
```

Paragraph records are ordered execution history, not globally keyed cached
computations. Identical text elsewhere in the document is not a candidate
unless it is the mapped continuation of the corresponding accepted paragraph.

## Read-set tracking

Each dependency observation stores:

```text
ObservedDependency {
    key,
    changed_at,
    semantic value,
}
```

The vocabulary covers meanings, registers and parameters, code tables, fonts,
hyphenation, input facts, group/conditional/mode facts, page queries, PDF
state, virtual streams, resources, clocks, and randomness. Structured values
use allocation-independent semantic projections.

Validation is red/green:

1. an unchanged stamp validates without rereading the value;
2. an advanced stamp rereads the semantic value;
3. an equal value validates the observation; mutable observations may also
   backdate their stamp; and
4. a different value makes the paragraph miss.

Front-end dependencies are recorded dynamically during expansion and
supplemented with the known horizontal-mode facts such as current font and
`\everypar`. Line-result dependencies cover line parameters, shape and penalty
arrays, font metrics, language-local hyphenation state, the complete mutable
font-parameter vector, and canonical roots for the complete `\sfcode` and
lowercase-code tables. The aggregate roots make the common validation path
constant-size; they conservatively invalidate paragraphs after an unrelated
change in the same code table. This separation supports two outcomes:

```text
front-end red       -> execute the paragraph
front end green,
line result green   -> mount finished lines
line result red     -> disable paragraph replay and execute the revision cold
```

Source identity and input transition are validated separately from the read
set. Changed-at stamps accelerate validation but are not semantic identities
and are never used as cross-generation hashes.

## Aligned replay cursor

After restart, the session maps prior paragraph anchors through the edit. Once
execution reaches a stable accepted paragraph start after the changed range,
it may arm an ordered cursor into the prior paragraph records.

For each candidate record, in order:

1. prove that its starting anchor and consumed source spans map unchanged;
2. prepare, but do not yet apply, its ending input transition;
3. validate its front-end dependencies, count/integer mutation state, and
   virtual effects;
4. validate line-result dependencies and prove that the retained finished-line
   graph and complete resource closure are mountable;
5. atomically apply the prepared input transition and replay the supported
   ordered mutations/effects;
6. mount the accepted finished lines;
7. attach the accepted generation's shared provenance resolver without walking
   the line graph, resolving editor roots, or allocating live origins;
8. install the result through the ordinary vertical/page-builder boundary;
   and
9. publish the same `ShipoutComplete` and `OuterParagraphEnd` events as cold
   execution.

The next old record is then the only candidate. There is no token preflight,
content-key bucket search, paragraph census, probabilistic admission, or
per-paragraph discovery map on this aligned path.

Rooted and input-frame fallback alignment share one monotonic accepted-history
cursor. Each fallback bucket is ordered by accepted index and keeps a direct
position that only advances. Entries behind the global cursor are skipped at
most once per revision, so bucket traversal is amortized constant time and
never rescans an accepted prefix. Every successful alignment advances the
global cursor. Mixed rooted/macro starts therefore cannot select an earlier
paragraph after later effects or mutations were consumed.

If mapping, transition preparation, front-end dependency validation, mount
validation, or a barrier fails, the candidate leaves live state untouched and
executes cold. The cursor may resynchronize at a later accepted paragraph
whose start maps unambiguously. A true line-result dependency change is
different: there is no cheaper retained artifact below finished lines, so the
executor abandons recording and replay for the remainder of that revision
after the first typed miss.

Read sets are validated just in time. Validating a union of every future read
at suffix entry would be unsound when paragraph mutations or ordinary page
building change values between paragraphs. Batching identical stable
projections inside one executor remains an implementation optimization, not a
new cache or correctness identity.

## Mutations, effects, and barriers

The implemented paragraph subset may replay count-register and
integer-parameter survivors plus supported detached stream text. Cold setters
feed an Env-owned recorder while paragraph recording is active, without
advancing the rollback epoch or scanning its journal. At root group depth, the
recorder retains one final-value redo for each escaping cell whose root value
differs from entry. Balanced-group locals and root writes restored to their
entry value disappear.

Inside a live entry group, the recorder instead retains the exact ordered
sequence of local and global setters that affect the entry frame. Local writes
inside a deeper balanced child group are omitted because they are fully
discharged; deeper global writes remain in the sequence. Admission checks the
paragraph-entry scalar only for each touched cell, then replays the same setter
sequence against the current group. Unrelated count/integer divergence is
irrelevant. This operation-log rule reproduces both values and local/global
ownership without a whole-state fingerprint or journal-ownership scan.

The same depth rule applies before unsupported group-scoped assignments become
barriers. Local macro/register/code-table/box assignments made strictly below
the paragraph's entry group are allowed to execute and disappear with their
balanced child group; an effective global assignment or a write at the entry
depth remains an escaping-write barrier. Expansion tracking excludes a meaning
first supplied by such a local definition while the group is active, but keeps
any read made before the definition, any external right-hand-side meaning read,
and any read after the group exits. This lets retained output depend on local
macro machinery without hiding dependencies on the state that supplied it.

Escaping unsupported writes, entry-frame replacement or `\aftergroup` payload changes,
unsupported input continuations, output routines, `\scantokens`,
mid-paragraph input opening, `\endinput`, box-register reads/consumption,
unsupported nested vertical construction, and untracked World access remain
explicit barriers. Balanced child groups are permitted at every entry depth;
entering and leaving them does not mutate the entry frame. Shifted fresh boxes
remain eligible, while a nested `\box`, `\copy`, `\vsplit`, or `\lastbox`
marks the containing paragraph at the scanner that observes it.

Inline math stays inside the retained finished-line graph. Its front-end read
set adds exact `\mathcode` and `\delcode` characters, math parameters, and a
64-bit mask naming only the `(size, family)` bindings actually consulted by
Appendix G conversion. Semantic metrics, parameters, and skew facts are then
retained only for fonts reached through those bindings. This avoids hashing
whole code tables or validating all 48 family cells. A display interruption
publishes the preceding paragraph with a typed display continuation; replay
skips the proven `$$` transition and re-enters display mode before ordinary
display processing resumes.

Recording begins provisionally in outer vertical mode at every group depth so expansion
reads made by the paragraph-starting token are captured. If the delivered
command leaves the engine in vertical mode, its recording and mutation
checkpoint are discarded immediately. Only the command that actually enters
horizontal mode owns the paragraph region. This keeps vertical glue,
penalties, assignments, and macro setup out of retained lines. Once an
explicit barrier is observed, expansion read tracking stops immediately; the
rest of the paragraph executes normally without accumulating a doomed read
set. When a scanner
has read one source token ahead, input alignment uses that pending token's
rooted start anchor and validates the full raw source transition before
discarding the token on a hit.

## One execution loop

The complete incremental algorithm is:

```text
select latest retained boundary before the edit
fork accepted substrate once at that checkpoint
rebind edited root input

while execution remains:
    if the next accepted paragraph maps here:
        validate and replay it, or execute it on a miss
    else:
        execute normally

    publish each named boundary with a live canonical identity
    if mapped boundary key, mode state, and full identity match:
        splice the accepted suffix and stop       // fast path

finish the document and accept the scratch fork  // slow path
```

At rest the session owns one accepted substrate. During `advance` it owns that
substrate and one scratch fork. A converged edit discards scratch semantic
state and adopts the old suffix. A nonconvergent edit promotes scratch and its
new paragraph history. Paragraph artifacts follow accepted history naturally;
there is no separate begin/accept/discard cache generation protocol in the
target architecture.

## Simplification boundary

The target paragraph path removes or collapses:

- global paragraph content-key lookup and raw-token preflight;
- census, seeding, probation, and admission policy;
- an independently owned paragraph-cache generation lifecycle;
- a reverse paragraph sequence or suffix hash;
- hierarchical parent trace summaries; and
- paragraph dependence on generic pretolerance, page, or shipout memo layers.

Persistent component hashes, dependency changed-at state, stable input
identities, and execution-local deduplication remain. They make required
identity or validation work cheaper; they are not additional reusable result
layers.

Pretolerance, page, and shipout experiments remain default-disabled while this
migration is in progress. Removing their obsolete public/runtime surface is a
separate cleanup after the aligned paragraph path no longer depends on the
generic memo runtime. Direct page-artifact patching is explicitly deferred.

## Performance model

An optimized cold Gentle profile on 2026-07-17 measured 210.252 ms per compile:

| Work                                     |    Time | Share |
| ---------------------------------------- | ------: | ----: |
| Paragraph hlist/front end                | 50.6 ms | 24.1% |
| Paragraph finalization and line breaking | 30.2 ms | 14.4% |
| Page builder, output, and shipout        | 64.9 ms | 30.9% |
| Other execution and driver work          | 64.6 ms | 30.7% |

Only 0.8 ms of the page pipeline was page-break selection; about 60.4 ms was
shipout/page lowering and artifact construction. Therefore perfect,
zero-overhead finished-line replay with page rebuilding has an approximate
129.5 ms floor, or a 1.62x whole-document ceiling. Hlist-only replay has an
approximate 159.7 ms floor, or a 1.32x ceiling.

These are absolute cold-work rooflines, not release targets: the changed paragraph,
restart prefix, validation, mount/provenance installation, and misses remain.
Fast-path suffix adoption has a different cost model and must never be averaged
with slow-path paragraph economics.

The previous opportunity-driven implementation measured 137 armed slow-path
opportunities, 27 hits, only 1.432 ms of validation/import, and a +67.594 ms
memo-enabled slow-path loss. It remains default-disabled. The new plan must win
by removing discovery, recording, and lifecycle work, not by weakening the
read-set or provenance contract.

The completed accepted-history path changes the practical roofline. On the
2026-07-18 Gentle slow edit, 863 of 873 eligible paragraphs mount retained
finished lines, 101,166 commands are skipped, and only nine candidates miss
validation. Across all 912 paragraph executions, including output-routine
paragraphs, 94.6% replay. In an optimized 50-iteration isolated slow-path
sample, `try_reuse_aligned_paragraph` owns only 0.07% of weighted samples. The
remaining executor work is dominated by the ordinary page pipeline:
`drain_pending_output` owns 20.55%, alignment 19.21%, shipout 16.86%, and
`stage_shipout` 11.58% of samples; direct output emission contributes roughly
another 8%. Line breaking itself is 1.78% and is limited to misses and
output-routine paragraphs.

An instrumented optimized ten-pair path-separated run measured the
representative slow edits at -44.832 ms enabled-minus-disabled, or -21.081 ms
after charging the one-time accepted-history priming cost. A final
uninstrumented twenty-pair release run measured -29.703 ms for the slow edits
and -7.846 ms with priming charged. The independent fast path was +1.006 ms,
forced rebreak was +3.918 ms, and priming plus the complete five-edit history
was -3.909 ms. This means further provenance, read-set, or replay-loop tuning
cannot yield a large slow-path improvement: the next material ceiling is
page/output reuse, which remains deliberately outside this paragraph design.

### Mountable finished-line ownership experiment

The accepted-history implementation now owns cloneable retained-root handles
whose immutable survivor payload and glue closure are shared between related
Universes. A finished-line hit validates the handle before mutation, installs
its payload under the restarted Universe's ordinary rollback pin log, mounts a
shared lazy diagnostic resolver, restores its glue closure, and returns the unchanged
`NodeListId`. It does not resolve editor roots, allocate live origins, import,
promote, re-freeze, rehash, or recursively rewrite semantic nodes. The existing reused-paragraph
installation still materializes only the mounted top-level contributions and
feeds them through ordinary vertical append, baseline glue, prevdepth, page
building, and output-routine behavior. Marks, whatsits, leaders, unset nodes,
and unresolved font/glue/foreign child handles conservatively miss the mount.

The 2026-07-17 optimized ten-pair AB/BA Gentle run retained 132 finished-line
hits on each slow edit, skipped 42,183 commands, and reported zero imported
semantic bytes. All four revisions preserved the disabled schedule and were
DVI-byte-identical to cold. The combined slow-edit enabled-minus-disabled
delta was -8.219 ms mean/-6.196 ms median; executor deltas were -11.195 and
-9.716 ms for the two slow edits. Interaction was +0.876 ms mean and the
independent fast path was -0.191 ms mean/+0.842 ms median. Recording still
costs: slow-plus-priming remained +25.987 ms mean/+26.489 ms median, so this is
not a default-enablement claim.

A matched ten-run sampled profile reduced `try_reuse_aligned_paragraph` from
391 samples (1.08%) to 120 (0.32%). The former recursive origin-refreeze
subtree was 152 samples (0.42%), including 90 SHA-256 compression samples, and
the retained-list clone subtree was 78 samples (0.21%); neither appeared in
the after profile. The replacement mount subtree was 24 samples (0.06%).

A final two-pair counter pass observed 1,161 recycling releases on each
memo-enabled slow edit versus 1,162 disabled, while 1,836--1,894 local roots
used the O(1) shared-payload drop path. Thus the mounted hits add neither
semantic promotion volume nor survivor recycling work.

### Output-provenance closure

Paragraph recording no longer keeps the expanded-token trace or constructs a
stable-span recipe. Retained line nodes keep the raw `OriginId`s they already
carry. While recording is provisional this provenance is `Pending`; accepting
paragraph history attaches one shared `ParagraphOriginResolver` to the
accepted generation without traversing a line graph.

The resolver owns an immutable origin-record snapshot and metadata-only
fragment store. Origin records use a persistent chunked archive: sealed chunks
are shared by `Arc`, the bounded tail is copied at snapshot time, and rollback
truncates only the live archive. Snapshot construction is therefore O(1) in
the sealed history plus a bounded tail copy, and it replaces rather than
duplicates the live record vector. It follows an origin chain to a stable
`RootSpanId` only when a diagnostic consumes that origin.

On a finished-line hit, replay attaches the resolver to the survivor mount.
Page promotion remaps only the lazy resolver range. Direct shipout stores one
packed lazy reference containing the resolver ordinal and raw 32-bit
`OriginId`; it does not decode an origin or resolve an editor root. A committed
artifact performs that resolution in `render_origin` only when queried, then
returns the same stable source identity used by current/deleted editor-layout
resolution. Existing eager and stable-recipe representations remain for live
input provenance and shipout memo, but paragraph replay does not construct
either.

The earlier recipe implementation's focused scaling regression expands 4,096
`\relax` tokens after paragraph entry but produces only two characters; both
retained recipes contain at most three roots and three slots. Current-layout
resolution after a finished-line hit and typed deletion after replacing the
referenced fragment are covered separately. That experiment established
output-scaled sparse recipes. The current resolver instead keeps one
generation snapshot alive and avoids all publication-time graph traversal;
its retained lifetime is generation-scaled rather than output-reachability
scaled. Replay still performs no origin allocation, and the scalar
delivered-token count remains only avoided-work telemetry.

On the 2026-07-18 stable 721-hit Gentle run, the earlier recipe deferral reduced paragraph import
from 2.565 to 0.209 ms on the forward slow edit and from 1.793 to 0.203 ms on
the inverse edit. A rejected first implementation performed a survivor lookup
and binary search for every glyph: that function alone was 2.59% of samples
and expanded shipout from 9.16% to 13.49%. The per-list monotonic cursor reduced
the lookup to 0.18% plus 0.11% cursor setup; matching sampled runs no longer
showed shipout growth. The tradeoff is explicit: cold line-provenance recording
rose by 0.807 ms and retained paragraph-cache bytes by 7.5% for the sparse node
index. All five representative edits remained DVI-byte-identical to cold.

On the q02h.58 baseline, the Gentle edit history retained 3,916,504 metadata
bytes. The compact closure retains 1,999,076 bytes for the same 132 line hits,
42,183 skipped commands, zero imported semantic bytes, barriers, schedules,
and DVI output: a 48.96% reduction. A ten-pair optimized run under substantial
host outliers reported median paragraph-enabled minus disabled deltas of
+1.366 ms for the combined slow edits, +23.753 ms including priming,
+2.032 ms interaction, and +1.832 ms fast. Disabled/enabled priming medians
were 267.313/282.726 ms. A separate instrumented telemetry pass charged
0.65--0.75 ms total import/mount work to each 132-hit slow edit. These costs
preserve the default-disabled decision; the evidence establishes bounded
output-scaled provenance rather than a new enablement claim.

The final opaque-resolver implementation eliminates this remaining cold graph
walk. In the representative 863-hit run, `line_provenance_ns` is zero during
both priming and replay, where the previous matched priming run charged
111.215 ms and the slow edit charged 14.457 ms. Accepted metadata fell from
9,959,318 to 7,773,494 bytes. The focused snapshot budget reports a 542 ns
median for both small and multi-chunk histories with zero retained allocation;
focused current-layout, deleted-layout, and rollback-survival tests preserve
diagnostic behavior. This is the intended policy for rarely consumed
diagnostic provenance: retain enough immutable information cheaply and decode
only the requested origin.

Accepted dependency observations are likewise generation-owned. One append-only
table stores each `(key, changed_at, value)` observation once per speculative
history; paragraph front-end and break read sets retain only canonical `u32`
ordinals. Acceptance freezes the table into one shared `Arc`. Carried regions
continue to reference their prior generation's table, while newly executed
regions receive the new table. This keeps exact stamp/value validation but
reduces Gentle metadata from 8,500,706 to 3,101,194 bytes after cold priming and
from 7,773,494 to 2,960,838 bytes after a slow edit. Clean cold timings were
neutral to about 1 ms favorable; the representation is accepted for its 62--64%
retained-memory reduction, not as a claimed latency breakthrough.

### Direct root-state delta recording

Paragraph mutation recording no longer opens a rollback epoch or derives redo
from a journal suffix. The count-register and integer-parameter setters record
entry values only while a paragraph recorder is active; final root equality
removes no-op and restored root cells. A live-group paragraph records its exact
ordered entry-frame/global setter sequence. Replay validates only the touched
cells' entry scalars, so unrelated count/int state does not force semantic
fallback or invalidate an otherwise identical setter script.

The focused state tests prove that starting a recorder leaves the Env epoch
unchanged, nested globals survive, balanced nested locals disappear, root
writes restored to entry disappear, abandonment releases recorder state, and
local/global writes inside an open group retain exact order and ownership.
The incremental balanced-depth-zero group regression records zero group
barriers and produces a later line hit; it verifies an actual replay candidate,
not just mutation counts.

Instrumented Gentle runs found no additional state-hash journal work from
paragraph recording. Dependency observations are read-only: an absent stamp is
`NEVER`, and after tracking activates only a real mutation inserts into the
shared `AHashMap`. Broad
invalidation advances one scalar stamp, while scalar code-table facts share one
mutation clock per table and retain exact values for semantic fallback. This
tracker stays dormant until a dependency region or explicit tracked read is
opened, so memo-disabled cold execution retains the old zero-stamp-map path. It
also avoids both read-side copy-on-write and one stamp entry per Unicode scalar;
recorded read sets remain canonically sorted. Per-slow-edit paragraph
validation is approximately 2.7 ms. The final slow edits mount 450 finished-line
results, skip 24,896 commands, and retain 3,029,160 bytes of accepted paragraph
metadata.

The final twenty-pair optimized AB/BA run preserved every named-boundary
schedule and cold DVI. Paragraph-enabled minus disabled deltas were -15.299 ms
mean/-15.004 ms median for the combined slow edits and -2.005/-3.492 ms after
charging the complete one-time priming cost. Interaction was -0.217 ms median;
the independent fast path was +0.898 ms and the forced cold rebreak path was
+2.680 ms. The complete five-edit baseline-inclusive median was +1.310 ms, so
the follow-on optimization work must reduce priming and non-replay overhead
rather than treating the slow-path result as whole-session enablement.

### Central paragraph validation boundary

An aligned candidate now crosses one executor-owned, fail-before-mutation
entry boundary. Source anchors and the complete raw input transition are
prepared first. The common same-timeline path compares the canonical
front-end read set's changed-at stamps. This identity is not a full-`Universe`
hash and excludes page state that the slow path rebuilds.

Each changed-at stamp lives directly beside its typed observation. There is no
parallel paragraph stamp vector: one ordered pass accepts exact stamps without
projecting semantic state and projects only observations whose stamps changed.
Read-only validation deliberately leaves accepted-history observations at
their recording stamps, so a restored value may take that semantic path again
without copying shared metadata.

Replay validation and mount phase timers are compiled only with
`profiling-stats`. Production builds retain hit/miss/work counters but do not
read the host clock around every paragraph hit.

If a stamp differs, validation falls back only to the typed semantic
observations for the recorded read set. Mutation admission separately checks
the compact touched-cell preconditions. Paragraph history's large immutable
shared read sets validate read-only: they retain their old stamps and may take
the semantic path again, avoiding copy-on-write merely to refresh metadata.
Detached effects, line-result dependencies, and retained
finished-line liveness are checked at the same boundary. Only after all checks
succeed may replay advance input, apply the root delta, reproduce effects, or
mount provenance.

The audited treatment of future-relevant state is:

| State                                                                                                                                                                     | Treatment on a hit                                                                                             |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| Raw source coverage and ending input stack                                                                                                                                | Validated, then advanced atomically                                                                            |
| Expansion reads, current font, `\everypar`, `\parindent`, `\spaceskip`, and `\xspaceskip`                                                                                 | Exact-stamp match or typed semantic fallback                                                                   |
| Surviving count-register and integer-parameter writes                                                                                                                     | Touched entry scalars validated; compact root redo or exact live-group setter sequence replayed                |
| Detached stream text                                                                                                                                                      | Shape validated, then replayed in original order                                                               |
| Inline-math codes, parameters, used family bindings, and reached font facts                                                                                               | Exact-stamp match or typed semantic fallback; unused families and unobserved code points are irrelevant        |
| Aggregate `\sfcode`/lowercase-code roots, line parameters, shapes, penalty arrays, conditional `prev_graf`, font/hyphenation facts, and aggregate mutable font parameters | Pre-redo exact-stamp match or typed semantic fallback; a real mismatch switches the revision to cold execution |
| Baseline/interline glue, normal-paragraph reset, page building, output routines, and shipout                                                                              | Recomputed by the ordinary paragraph epilogue and page pipeline                                                |
| Entry-frame replacement, unsupported writes/input transitions, box-register scans, `\scantokens`, input opening/ending, and untracked World access                        | Explicit barrier; balanced child groups and their discharged local writes remain reusable                      |

No conservative whole-state identity is added: facts are named at their actual
read tier, and missing seams use typed observations. Pre-redo line validation
is sound because supported paragraph mutations are checked for overlap with
the line-result read set before reuse.

### Superseded prepared-hlist experiment

The prepared-hlist fallback was implemented and measured, but it was not an
economic reuse tier. Rebreaking mounted hlists saved too little executor work
to pay for retaining a second graph, a second provenance recipe, validation,
and accepted-history publication. It also forced every cold paragraph to build
two artifacts. The q02h.66 implementation therefore removes prepared hlists
from `RecordedParagraphRegion` and keeps only finished lines.

The fifth Gentle revision still changes `\tolerance`, but now verifies the
fallback policy: the first aligned paragraph reports one typed
`BreakDependency` miss, abandons its provisional recording checkpoint, and
disables paragraph replay and recording for the rest of that execution. The
revision then follows the ordinary cold executor and publishes no doomed
paragraph history. It preserves the previous accepted history instead of
freeing its retained graphs; a later inverse edit may make that history valid
again. This is intentionally a conservative whole-revision fallback for a rare
changed-linebreaking-state edit, not another cache tier.

### Retained-root lifecycle cleanup

Issue `umber2-q02h.63` made the shared mount the ownership boundary rather than
an adapter over the earlier paragraph-generation machinery. Each accepted
`RecordedParagraphRegion` now owns a cloneable finished-line mount handle. The
handle contains the immutable survivor payload and its deduplicated
glue closure; cloning or dropping it is O(1) in graph size. Mounting installs a
local survivor root plus one ordinary rollback pin and restores only the small
resource closure. Scratch rollback therefore treats paragraph consumers like
other survivor-backed engine state, while accepted-history drop releases its
shared payload without a graph walk.

The separate paragraph generation mark, pin vector, glue reference table,
accept/drain transition, recursive epoch importer, semantic refreeze path, and
paragraph imported-byte/import-failure telemetry are removed. Carried hits
keep the existing retained handles and do not retain or promote them again.
The generic detached import API in `tex-state::memo` remains for the independent
page and shipout experiments; it owns handle-free DTO reconstruction and is not
part of paragraph replay. Compact piece/root ordinals also remain because they
encode the live output-provenance recipe, not the removed import lifecycle.

The earlier dual-artifact measurements in this section are historical. The
current finished-line-only result and release decision are recorded below.

## Implementation sequence

1. **Live boundary identities.** Capture accepted canonical identities while
   boundaries are live; remove lazy accepted-snapshot fork/rollback identity
   reconstruction. This is implemented by commit `b2fbbb84`.
2. **Accepted-history cursor.** Expose ordered prior paragraph records to the
   executor, align them by stable source anchors, and replay sequentially with
   current per-record validation. Preserve cold boundary publication. This is
   implemented by commit `a9bfee13`.
3. **Remove lookup/admission machinery.** Delete global paragraph key lookup,
   token preflight, census/seeding/probation, and redundant paragraph
   generation ownership once the cursor covers accepted-history reuse. The
   implementation now retains only ordered accepted-history publication and
   carry-forward telemetry; the old discovery and admission layers are gone.
4. **Path-separated verification.** Re-run slow pagination-changing,
   cross-generation interaction, and fast height/page-preserving Gentle
   cases. Require cold parity, exact page accounting, and the explicit
   1,000-edit tier. Earlier runs replayed 246--257 paragraphs but incorrectly
   allowed a vertical prelude to share the following paragraph's region. The
   corrected boundary replays 132 finished-line paragraphs per slow edit and
   preserves exact cold DVI across the full four-edit Gentle matrix.
5. **Cleanup and release decision.** The paragraph graph generation/import
   lifecycle has been removed in favor of accepted-history-owned shared mounts;
   generic detached page/shipout import remains isolated. The q02h.66 follow-up
   removes the uneconomic prepared-hlist tier, shares immutable metadata,
   validates aggregate code-table and mutable-font-parameter roots, and replays
   exact live-group setter sequences. The final twenty-pair balanced run
   replays 450 finished-line paragraphs and skips 24,896 commands on each
   representative slow edit. The combined slow path wins by 18.244 ms
   mean/15.800 ms median, and still wins by 6.917 ms mean/5.976 ms median after
   charging the complete one-time priming cost. The independent fast path
   remains noise-level. A true
   line-result dependency change deliberately falls back to cold execution for
   the rest of that revision; prepared-hlist rebreaking is not retained as a
   second cache tier.
6. **Deferred page work.** Design direct page/shipout artifact patching only
   after paragraph replay reaches its measured slow-path ceiling. It is not a
   blocker for this plan.

## Verification obligations

Every change must preserve:

- incremental DVI, artifacts, effects, diagnostics, and final state versus a
  fresh cold run;
- identical named-boundary keys and order between replay and execution;
- full-state fast convergence from equivalent histories;
- no accepted-substrate rollback after the single restart fork;
- fail-before-mutation behavior on every paragraph miss;
- current-revision input and rendered provenance after replay;
- owner/liveness checks for retained node, token, glue, font, and source roots;
- bounded accepted paragraph metadata and deterministic pruning;
- separate fast, slow, and interaction performance reports; and
- `cargo test --workspace --tests`, `scripts/check.sh`, snapshot budgets, the
  committed fast matrix, and the explicit 1,000-edit incremental/cold tier.

The release criterion is not merely a high validation-eligible hit rate. The
slow path must beat paragraph-disabled incremental execution in balanced
optimized measurements without regressing the fast path or cold priming beyond
the documented budget.

## Related contracts

- [`incremental_v1.md`](incremental_v1.md): named boundaries, restart,
  canonical full-state splice, effects, substrates, and pruning.
- [`core_state.md`](core_state.md): aggregate state ownership, snapshots,
  canonical identities, and dependency changed-at state.
- [`edit_stable_source_coordinates.md`](edit_stable_source_coordinates.md):
  stable editor fragments and revision mapping.
- [`profiling.md`](profiling.md): current Gentle workloads, telemetry, and
  measured release evidence.
