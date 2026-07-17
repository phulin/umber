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
- current-revision provenance rebinding recipes;
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
    provenance rebinding recipes,
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
3. validate its front-end dependencies, supported mutation preconditions, and
   virtual effects;
4. import its retained hlist before applying any mutation;
5. validate the separate break dependencies and import finished lines when
   green, otherwise line-break the imported hlist;
6. rebind provenance to current-revision source spans;
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

The implemented paragraph subset may replay ordered count-register and
integer-parameter writes and supported detached stream text. Replay first
validates the complete precondition sequence and imports retained nodes; only
then does it call ordinary aggregate mutation APIs in the recorded order.

Unsupported writes, group transitions, input continuations, output routines,
display math, `\scantokens`, mid-paragraph input opening, `\endinput`, and
untracked World access remain explicit barriers. During simplification, a
barrier must prefer ordinary execution over broadening redo semantics. New
mutation/effect families are added only when a measured corpus shows that the
barrier materially limits useful aligned replay.

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
   1,000-edit tier. Final balanced runs replayed 246 paragraphs per slow edit
   with exact cold parity, but lost 35.800--45.193 ms across the two slow edits
   and 58.240--70.547 ms including priming.
5. **Cleanup and release decision.** Collapse the generic memo runtime to the
   facilities still used, remove obsolete opt-in layers only after dependency
   checks, and decide paragraph default enablement from balanced release
   measurements. The accepted-history layer remains default-disabled: 628 of
   889 observed Gentle paragraphs were correctly barriered, chiefly because a
   complete group transition/redo substrate is not yet available.
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
