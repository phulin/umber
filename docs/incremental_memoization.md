# Incremental paragraph replay

Status: authoritative contract for changed-document paragraph reuse.

Umber reuses finished line results from the accepted editor history. Reuse is
ordered by edit-stable source alignment, validated against the exact facts read
by the original paragraph, and mounted from accepted-history-owned immutable
roots. It is not a global content-keyed cache, a reverse suffix index, or a
hierarchical execution trace.

## Goals and non-goals

The paragraph path must:

- skip material expansion, execution, and line-breaking work after edits that
  change pagination;
- remain byte-identical to a fresh cold run;
- preserve the named-boundary schedule used by ordinary incremental execution;
- fail before mutation when a candidate cannot be reused;
- keep accepted-history ownership and retained-memory accounting explicit; and
- keep the memo-disabled path dormant and free of recording-only state.

It does not reuse page building, output routines, shipout, arbitrary scanner or
builder continuations, or paragraphs with unsupported surviving state. The
existing height/page-preserving suffix-adoption path remains independent.

## Accepted history

An accepted revision owns an ordered sequence of paragraph records. A record
contains:

- stable starting and ending source anchors;
- the raw input transition consumed by the paragraph;
- canonical front-end and line-breaking dependency observations;
- supported surviving mutations and detached effects;
- one retained finished-line mount;
- lazy diagnostic provenance; and
- boundary metadata required to publish the same cold checkpoint.

Records are owned by accepted history, not by an independent cache generation.
Carried records keep their original immutable dependency and provenance tables.
Newly executed records belong to the scratch revision until acceptance.

At rest a session owns one accepted substrate. During `advance` it owns that
substrate and one scratch fork. Convergence discards scratch semantic state and
adopts the accepted suffix. A nonconvergent edit promotes scratch and its new
paragraph history. There is no separate begin/accept/discard cache protocol.

An unchanged-root external-input rerun is another scratch fork of the accepted
substrate. It restores only the executor-owned `JobStart` checkpoint, preserves
the accepted editor revision and identity source layout, and reuses the same
ordered paragraph history. It never restores a later checkpoint. Its current
conservative sink also declines suffix adoption because equality before a
changed external input is consumed is not equality of future execution; the
ordinary typed paragraph observations remain the only selective replay test.

## Source alignment

Replay uses edit-stable fragment coordinates from
`edit_stable_source_coordinates.md`. The executor walks prior records in source
order with one monotonic cursor. Unchanged paragraph anchors map into the new
layout without hashing the remaining document or searching a global cache.

Deleted, split, reordered, or otherwise unmappable fragments are alignment
misses. Alignment only identifies a candidate; dependency validation still
decides whether its result is reusable.

## Dependency observations

Each observation stores a typed key, its recording-time changed-at stamp, and
the recorded semantic value. Validation accepts an unchanged stamp without
projecting current semantic state. When the stamp changed, it compares only the
typed value for that observation. Read-only validation does not backdate or
copy accepted metadata.

One generation-owned append-only table stores observations. Paragraph records
refer to canonical `u32` ordinals, and acceptance freezes the table into shared
immutable storage. Real mutations populate changed-at state; reads do not.
Broad invalidation is a scalar stamp. Scalar code-table facts share one clock
per table and retain exact values for semantic fallback.

The tracker remains dormant until recording or explicit tracked reads begin.
The memo-disabled cold path therefore allocates no dependency map and performs
no recording-only work.

## Mutations, effects, and barriers

Root-level count-register and integer-parameter setters record entry values
while a paragraph recorder is active. Final equality removes no-op and restored
writes. A paragraph inside an ordinary live group records the exact ordered
entry-frame/global setter sequence and validates only the touched entry cells.

Detached stream text is replayed in original order after its shape is
validated. Baseline/interline glue, normal-paragraph reset, page building,
output routines, and shipout are recomputed by the ordinary paragraph epilogue
and page pipeline.

Explicit barriers reject unsupported future-relevant state, including:

- entry-frame replacement or unbalanced group transitions;
- unsupported surviving assignments or box consumption;
- input opening, ending, `\scantokens`, or untracked World access;
- inline-math or nested builder state not represented by the record; and
- output effects that cannot be reproduced by the detached effect contract.

Balanced child groups whose local effects are fully discharged remain
reusable.

## Finished-line ownership

Each record owns a cloneable mount handle containing an immutable survivor
payload and its deduplicated glue closure. Cloning or dropping the handle is
O(1) in graph size. A hit validates the handle, installs its payload through
the restarted Universe's ordinary rollback pin log, restores the resource
closure, and returns the unchanged `NodeListId`.

Replay does not import, promote, refreeze, rehash, or recursively rewrite the
line graph. The ordinary reused-paragraph installation materializes only the
top-level contributions and sends them through vertical append, baseline glue,
`prevdepth`, page building, and output-routine behavior. Unsupported foreign
children, marks, whatsits, leaders, unset nodes, or unresolved font/glue
handles conservatively miss.

The separate paragraph-generation mark, pin vector, recursive epoch importer,
semantic refreeze path, and imported-byte telemetry are not part of this
design. Generic detached page/shipout memo APIs remain isolated from paragraph
replay.

## Lazy output provenance

Retained line nodes keep their raw `OriginId`s. While recording is provisional,
provenance is pending; acceptance attaches one shared immutable origin resolver
without traversing the graph.

The resolver owns a snapshot of origin records plus metadata-only editor
fragments. Sealed chunks are shared, the bounded live tail is copied at
snapshot time, and rollback truncates the live archive. A replay mount carries
the resolver into page promotion. Direct shipout stores a packed lazy reference
containing the resolver ordinal and raw origin id. Current/deleted editor-layout
resolution happens only when a host query consumes that origin.

Dependency observations follow the same generation-owned sharing model. Both
provenance and observations scale with accepted generations and records rather
than replay-time graph traversal.

## Validation and replay order

One executor-owned boundary performs every check before mutation:

1. Align and prepare source anchors and the complete raw input transition.
2. Validate front-end observations by stamp or typed semantic fallback.
3. Validate touched-cell mutation preconditions and detached effects.
4. Validate line-breaking observations and retained-root liveness.
5. Start the reused paragraph in cold order behind a rollback guard.
6. Advance input, apply the supported mutation/effect record, and mount lines.
7. Run the ordinary paragraph epilogue and publish the cold-equivalent boundary.

Starting the paragraph before applying its recorded body is essential: a
changed prefix can make `\parskip` fire the output routine at paragraph start.
If starting produces page fire-up or effects not present in the record, the
guard restores page, store, World, dependency, and mode roots and resumes cold
dispatch.

The principal validation treatments are:

| State                                                                  | Treatment on a hit                                                                     |
| ---------------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| Raw source and input transition                                        | Validate, then advance atomically                                                      |
| Expansion reads, current font, `\everypar`, spacing parameters         | Exact stamp or typed semantic fallback                                                 |
| Supported count/integer writes                                         | Validate touched entry values, then replay compact root delta or ordered group setters |
| Inline-math codes, used families, font facts                           | Exact stamp or typed semantic fallback                                                 |
| Line parameters, penalties, shapes, hyphenation and mutable font facts | Validate before redo; mismatch selects cold execution                                  |
| Baseline glue, page/output/shipout state                               | Recompute through ordinary execution                                                   |
| Unsupported input, group, box, or builder state                        | Barrier                                                                                |

A real line-breaking dependency mismatch disables paragraph replay and
recording for the remainder of that revision and follows ordinary cold
execution. Prior accepted history is retained for a later inverse edit. Umber
does not retain a prepared-hlist rebreak tier.

## Performance boundary

Finished-line replay deliberately rebuilds pages. Its remaining cost is
therefore dominated by alignment, output draining, page construction, shipout,
and artifact staging rather than line mounting. Page or shipout reuse requires
a separate design and must not be inferred by extending paragraph records.

Fast suffix adoption, slow pagination-changing replay, cross-generation
interaction, cold priming, and forced line-breaking fallback are reported
separately. Historical measurements and rejected cache experiments remain in
Git history; `profiling.md` defines the current measurement workflow.

Accepted revision telemetry identifies cold execution, ordinary fast and slow
edits, unchanged-root external-input replay, and forced `JobStart` fallback.
Revision-local paragraph lookup, hit, and typed-validation-miss counts sit
beside that path attribution, while `PureMemoStats` remains the cumulative
generic runtime view. This separation does not introduce generated-input or
label-specific cache machinery.

## Verification obligations

Every change must preserve:

- incremental DVI, artifacts, effects, diagnostics, and final state versus a
  fresh cold run;
- identical named-boundary keys and order between replay and execution;
- full-state fast convergence from equivalent histories;
- no accepted-substrate rollback after the single restart fork;
- fail-before-mutation behavior on every miss;
- current-revision input and rendered provenance after replay;
- owner/liveness checks for retained node, token, glue, font, and source roots;
- bounded accepted paragraph metadata and deterministic pruning;
- separate fast, slow, interaction, priming, and fallback measurements; and
- the focused incremental suites, snapshot budgets, repository static gate,
  committed fast matrix, and explicit incremental/cold fuzz tier.

## Related contracts

- [`incremental_v1.md`](incremental_v1.md): named boundaries, restart,
  canonical full-state splice, effects, substrates, and pruning.
- [`core_state.md`](core_state.md): aggregate ownership, snapshots, identities,
  and changed-at state.
- [`edit_stable_source_coordinates.md`](edit_stable_source_coordinates.md):
  editor fragments and revision mapping.
- [`profiling.md`](profiling.md): current Gentle workloads, telemetry, and
  measurement method.
