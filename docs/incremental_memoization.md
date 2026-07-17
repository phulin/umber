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
   values they read remain valid. The executor imports their retained hlists
   or finished lines in accepted order and feeds them through ordinary page
   building and shipout.

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
- keep paragraph artifacts tied to the accepted generation that owns their
  node roots and provenance recipes;
- make failure local: a red paragraph executes normally and later mapped
  paragraphs may re-align; and
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
- generation-owned paragraph hlists and finished lines;
- shared mountable survivor payloads with local provenance overlays and
  retained glue closures;
- compact output-reachable, current-revision provenance rebinding recipes;
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
    retained hlist and finished lines,
    line count,
    compact hlist and finished-line output-provenance recipes,
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
3. an equal value backdates the observation; and
4. a different value makes the paragraph miss.

Front-end dependencies are recorded dynamically during expansion and
supplemented with the known horizontal-mode facts such as current font,
`\everypar`, and used `\sfcode` values. Break dependencies are recorded
separately from the actual hlist and cover line parameters, shape and penalty
arrays, font metrics, language-local hyphenation state, and used lowercase
codes. This separation supports three outcomes:

```text
front-end red       -> execute the paragraph
front end green,
break set red       -> import hlist and re-break
both green          -> import finished lines
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
4. import its retained hlist before applying any mutation;
5. validate the separate break dependencies and mount finished lines when
   green, otherwise line-break the imported hlist;
6. resolve only the stable editor roots named by reachable hlist or line
   origins and bind them through the mount-local origin overlay;
7. replay supported ordered mutations/effects and atomically apply the input
   transition;
8. install the result through the ordinary vertical/page-builder boundary;
   and
9. publish the same `ShipoutComplete` and `OuterParagraphEnd` events as cold
   execution.

The next old record is then the only candidate. There is no token preflight,
content-key bucket search, paragraph census, probabilistic admission, or
per-paragraph discovery map on this aligned path.

If mapping, transition preparation, dependency validation, import, or a
barrier fails, the candidate leaves live state untouched and executes cold.
The cursor may resynchronize at a later accepted paragraph whose start maps
unambiguously; one miss does not invalidate the entire source suffix.

Read sets are validated just in time. Validating a union of every future read
at suffix entry would be unsound when paragraph mutations or ordinary page
building change values between paragraphs. Batching identical stable
projections inside one executor remains an implementation optimization, not a
new cache or correctness identity.

## Mutations, effects, and barriers

The implemented paragraph subset may replay count-register and
integer-parameter survivors plus supported detached stream text. Cold setters
feed an Env-owned recorder while paragraph recording is active. Paragraph
entry captures a lazy 64-bit aHash fingerprint of the complete count/integer
state without advancing the rollback epoch. On the first observed setter for a
cell, the recorder saves its paragraph-entry value. Global writes escape at
every group depth; local writes escape only at depth zero. At paragraph exit,
one final-value redo is retained for each escaping cell whose root value differs
from entry. Balanced-group locals and root writes restored to their entry value
therefore disappear without inspecting or depending on journal storage.

The common replay path compares the incoming fingerprint once and applies the
compact redo. If the fingerprint differs, only the surviving cells' recorded
entry values are checked before reuse. This admits the same root write script
after unrelated count/integer changes while keeping cold setter overhead to
cache invalidation. A rare 64-bit collision is an accepted performance tradeoff
for this experimental replay layer.

Unsupported writes, changed nonzero entry groups, input continuations, output
routines, display math, `\scantokens`, mid-paragraph input opening,
`\endinput`, and untracked World access remain explicit barriers. A paragraph
that starts and finishes at group depth zero may contain fully discharged
groups, including local count/integer writes: there is no entry frame to
replace, and the direct recorder omits writes that do not escape to the root.
Inside an open group, any surviving count/integer write remains conservative
because final values alone cannot reproduce assignment ownership.

Paragraph recording begins provisionally in outer vertical mode so expansion
reads made by the paragraph-starting token are captured. If the delivered
command leaves the engine in vertical mode, its recording and mutation
checkpoint are discarded immediately. Only the command that actually enters
horizontal mode owns the paragraph region. This keeps vertical glue,
penalties, assignments, and macro setup out of retained hlists. When a scanner
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

These are absolute rooflines, not release targets: the changed paragraph,
restart prefix, validation, import, provenance rebinding, and misses remain.
Fast-path suffix adoption has a different cost model and must never be averaged
with slow-path paragraph economics.

The previous opportunity-driven implementation measured 137 armed slow-path
opportunities, 27 hits, only 1.432 ms of validation/import, and a +67.594 ms
memo-enabled slow-path loss. It remains default-disabled. The new plan must win
by removing discovery, recording, and lifecycle work, not by weakening the
read-set or provenance contract.

### Mountable finished-line ownership experiment

The accepted-history implementation now shares immutable survivor payloads
between related Universes. A finished-line hit validates the retained graph and
its ordinary Gentle handle closure before mutation, mounts a local provenance
overlay, restores the retained hlist's glue closure into the restarted store,
and returns the unchanged `NodeListId`. It does not import, promote, re-freeze,
rehash, or recursively rewrite semantic nodes. The existing reused-paragraph
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

Paragraph recording no longer keeps the expanded-token trace. At hlist and
finished-line publication it traverses the ordinary node graph, follows only
reachable char and ligature origins to stable editor roots, and builds a
compact recipe. Each referenced editor piece contributes one full
`RootSpanId` anchor; distinct output ranges use `(piece ordinal, start, end)`
records, and depth-first origin slots use `u32` indexes. Unknown or non-rooted
output origins use the reserved unknown slot. Token values and origins that
produced no accepted node are not retained.

On a finished-line hit, replay prepares and validates the ordinary input
transition first, recreates origins only for the recipe's distinct output
ranges, and installs them in the existing survivor mount overlay. Every
ordinary `Universe::nodes` traversal therefore observes current-revision
provenance immediately, including page building, output-routine inspection,
node diagnostics, and shipout. An hlist fallback mounts the same sidecar before
the existing recursive epoch import, so copied child graphs also receive the
current origins. There is no shipout-only map and no second node model.

The focused scaling regression expands 4,096 `\relax` tokens after paragraph
entry but produces only two characters; both retained recipes contain at most
three roots and three slots. Current-layout resolution after a finished-line
hit and typed deletion after replacing the referenced fragment are covered
separately. Thus retained metadata and replay origin allocation scale with
reachable output provenance, while the scalar delivered-token count remains
only avoided-work telemetry.

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

### Direct root-state delta recording

Paragraph mutation recording no longer opens a rollback epoch or derives redo
from a journal suffix. The count-register and integer-parameter setters record
entry values only while a paragraph recorder is active; final root equality
removes no-op and restored cells. The complete count/int fingerprint remains
the common replay validation. Only a fingerprint mismatch reads the compact
recorded entry preconditions before replaying final values.

The focused state tests prove that starting a recorder leaves the Env epoch
unchanged, nested globals survive, balanced nested locals disappear, root
writes restored to entry disappear, abandonment releases recorder state, and
writes made by a paragraph entering inside an open group remain conservative.
The incremental balanced-depth-zero group regression records zero group
barriers and produces a later line hit; it verifies an actual replay candidate,
not just mutation counts.

The 2026-07-17 instrumented two-pair Gentle pass found identical priming
state-hash journal work with paragraph recording disabled and enabled: 1,084
hash calls examined 11,822 journal entries and projected 2,212 changed cells in
both modes. On each slow edit, replay examined 9,104 entries versus 9,180 for
disabled execution. Thus recording adds no journal-scan volume, while reuse
avoids 76 examined entries. The Gentle corpus recovered no additional group
candidates: its 420 group barriers are unchanged conservative nonzero-entry
ownership cases, rather than journal-rewind rejection. Slow edits still mounted
132 finished-line hits, skipped 42,183 commands, and retained 1,997,576 bytes
of paragraph metadata.

A six-pair optimized AB/BA run preserved every named-boundary schedule and cold
DVI. Paragraph-enabled minus disabled medians were -9.292 ms for the combined
slow edits, +25.577 ms including priming, -1.005 ms for interaction, and
-0.160 ms for the independent fast path. Disabled/enabled priming medians were
240.270/278.726 ms. This keeps the layer default-disabled: the steady slow path
won in this run, but priming remains a net cost.

### Central paragraph validation boundary

An aligned candidate now crosses one executor-owned, fail-before-mutation
entry boundary. Source anchors and the complete raw input transition are
prepared first. The common same-timeline path then compares an exact
paragraph-relevant identity: the canonical front-end read order's changed-at
stamps plus the complete count/integer entry fingerprint. This identity is not
a full-`Universe` hash and excludes page state that the slow path rebuilds.

If either component differs, validation falls back only to the typed semantic
observations for the recorded read set and, when count/integer state differs,
the compact surviving-cell preconditions. Semantically equal observations are
backdated and refresh the exact identity carried into the new accepted
generation. Detached effects and retained hlist liveness are checked at the
same boundary. Only after all checks succeed may replay advance input, apply
the root delta, reproduce effects, or mount provenance.

The audited treatment of future-relevant state is:

| State                                                                                                                                              | Treatment on a hit                                                                                                                 |
| -------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| Raw source coverage and ending input stack                                                                                                         | Validated, then advanced atomically                                                                                                |
| Expansion reads, current font, `\everypar`, used `\sfcode`, `\parindent`, `\spaceskip`, and `\xspaceskip`                                          | Exact-stamp match or typed semantic fallback                                                                                       |
| Metrics and hyphen character of fonts reachable from the prepared hlist                                                                            | Front-end validation, because they affect character existence, ligature/kern construction, and materialized spaces before breaking |
| Surviving count-register and integer-parameter writes                                                                                              | Entry preconditions validated on divergence; final root values replayed in order                                                   |
| Detached stream text                                                                                                                               | Shape validated, then replayed in original order                                                                                   |
| Line parameters, shapes, penalty arrays, font/hyphenation facts, and used lowercase codes                                                          | Post-redo exact-stamp match or typed semantic fallback; mismatch recomputes lines from the validated hlist                         |
| Baseline/interline glue, `prev_graf`, normal-paragraph reset, page building, output routines, and shipout                                          | Recomputed by the ordinary paragraph epilogue and page pipeline                                                                    |
| Nonzero-entry group ownership, unsupported writes/input transitions, display math, `\scantokens`, input opening/ending, and untracked World access | Explicit barrier; the 420 measured Gentle open-group cases remain ineligible                                                       |

Finished-line failure therefore remains an hlist fallback rather than a whole
transaction failure. No conservative whole-state identity is added: facts are
named at their actual read tier, and missing seams use typed observations.

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
5. **Cleanup and release decision.** Collapse the generic memo runtime to the
   facilities still used, remove obsolete opt-in layers only after dependency
   checks, and decide paragraph default enablement from balanced release
   measurements. The accepted-history layer remains default-disabled. In the
   corrected Gentle run, 132 regions replay and 525 recorded regions hit
   barriers; macro-generated paragraph starts that have no clean root-source
   alignment after vertical setup are not recorded. Recovering those requires
   a separate late-alignment design with an explicit cold-prefix identity, not
   a weaker vertical/paragraph boundary.
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
