# Structural and demand-driven provenance

Status: historical ownership contract for Beads issue `umber2-3v8z.4`.

The runtime ownership portions of this document are superseded by
[Runtime storage lifetimes](runtime_storage_lifetimes.md). The current
`tex-state` cold boundary accepts explicit diagnostic or rendered-source
demand, starts from generation-typed `ProvenanceId<G>` coordinates, and emits
owned source locations, summaries, strings, byte ranges, and artifact source
recipes. Those DTOs contain no runtime origin/source id, owner, reference, or
arena coordinate. Ordinary execution does not invoke this presentation build.

This document replaces append-history ownership of diagnostic provenance. It
refines [Compact Source Spans and Token Provenance](source_spans_and_provenance.md)
without changing packed `OriginId`, source coordinates, rendered-source query
results, or diagnostic presentation.

## Invariant

Structural origin records are owned only by a live diagnostic consumer, a live
source-map registration, a materialized token position, or a detached artifact
which exposes rendered-source information. Ordered origin lists instead live
in the aggregate runtime value regions:

```text
OriginRef -> immutable origin record -> typed child roots
OriginListRef -> generation-validated registry coordinate -> region span
```

An origin-store coordinate, retry lease, allocation serial, cache entry, or
checkpoint watermark is not structural record ownership. Provenance
does not confer semantic ownership on tokens, macro definitions, nodes, input
state, or output. It remains excluded from token equality, `\ifx`, formats,
semantic hashes, convergence, artifact bytes, and artifact content identity.

There is no provenance graph scan or refcount collection pass. Structural
record edges are installed at the typed transition which knows the owner and
origin. Origin-list storage follows the aggregate region lifecycle: marks
cover list identities and dense locations, rollback truncates the candidate
suffix, and forks copy the active suffix while sharing sealed regions.

## Structural values

`OriginRecord` candidates compare the complete record. Runtime origin-list
interning compares every ordered `OriginId` through borrowed registry views;
there is no hash bucket or parallel candidate authority. Rollback invalidates
discarded generations before raw slots can be reused, forks preserve inherited
coordinates while separating sibling suffix identities, and exact operation
replay preserves its packed identity. This also eliminates duplicate
transition rows, including 10,000 identical macro-frame allocations collapsing
to one record.

### Source registrations and ranges

One immutable source-registration value owns the descriptor, original backing
authority, line-start data, and registered logical range. `SourceMap` keeps
the current `SourceId` and position indexes, while a live source frame,
checkpoint input summary, diagnostic location, or detached artifact may share
the registration value independently.

An ordinary backed scalar remains a direct `OriginId` and allocates no origin
atom. A nontrivial spelling uses one immutable source-range atom containing a
strong source-registration reference and the validated relative half-open
range. Its packed id is only a nonowning coordinate. Equal ranges in the same
registration share one atom after exact comparison; ranges from different
registrations never alias even if their local offsets match.

Editor fragments retain their existing session-scoped stable piece identity.
Detached root-buffer artifacts prefer the current compact piece-anchor recipe.
Other detached source locations own an immutable source-registration recipe,
not a raw candidate `SourceId` or an unrooted arena id.

### Token positions and origin lists

`OriginRef` is the strong reference for one structural token position. It
projects an `OriginId` for `TracedTokenWord` and compact node columns, but the
id cannot upgrade a dead value. Direct positions use a zero-allocation
registration-relative owner when they must outlive the live source map.

`OriginListRef` is the `Copy` facade for one immutable exact sequence. Its
`OriginListId` selects a generation-validated dense registry location whose
`RuntimeOriginListCoordinate` names the list's span in the single
`RuntimeValueRegion` provenance column. The empty list is an immortal builtin.
Dynamic allocation and exact reuse go only through `Stores`; reads go through
borrowed `Stores`, `Universe`, or `CommandContext` views. Extending an untraced
list preserves the empty-list contract, while extending a traced list creates
or reuses one complete region value.

Inserted and synthesized transitions do not create a flattened wrapper for
every delivery. Expansion-control information which affects delivery remains
in the command token state (`NOEXPAND_FALLBACK` where required). Diagnostic
position follows the nearest structural parent, with an optional compact
role retained only by a consumer which can present that distinction.

#### Traced-list freeze invariant

The pre-node allocation audit for `umber2-3v8z.22.14` found that traced-list
freeze was not implementing the structural contract above. It materialized a
dense `Vec<OriginRef>`, materialized a second dense `Vec<OriginId>`, appended
the ids to the stored-list arena, and then published another dense id/root
pair. The following exact capped-prefix measurements use the pinned
`2606.12566` source, format, distribution, closure, and cache:

| Committed step | Traced positions | Direct positions | Rooted positions | Dense published bytes | Root scratch bytes | Id scratch bytes | Stored-arena capacity growth | RSS high water |
| -------------- | ---------------- | ---------------- | ---------------- | --------------------- | ------------------ | ---------------- | ---------------------------- | -------------- |
| 1,024          | 443,013          | 416,401          | 26,612           | 12,404,364            | 10,632,312         | 1,772,052        | 1,064,952                    | 220,496 KiB    |
| 2,048          | 990,732          | 919,094          | 71,638           | 27,740,496            | 23,777,568         | 3,962,928        | 4,128,760                    | 250,644 KiB    |
| 4,096          | 1,684,411        | 1,527,940        | 156,471          | 47,163,508            | 40,425,864         | 6,737,644        | 4,259,832                    | 295,952 KiB    |
| 6,144          | 2,665,505        | 2,370,640        | 294,865          | 74,634,140            | 63,972,120         | 10,662,020       | 8,257,528                    | 378,628 KiB    |

The published column is cumulative logical payload requested by the old
dense representation. Stored-arena growth is retained vector capacity.
Scratch columns are cumulative allocation churn and are not live after each
freeze; their contribution to RSS is allocator fragmentation and high-water
reuse, not semantic state. From step 1,024 through step 6,144 these four
producer columns grow by 131,651,130 bytes while RSS high water grows by
161,927,168 bytes. Thus this one producer explains 81 percent of the measured
phase growth before accounting for token payloads and allocator rounding.

The replacement invariant is falsifiable:

- The immutable list stores one compact `OriginId` per position in the existing
  shared provenance column. It has no per-list `Arc`, strong-owner set, or weak
  marker.
- Freeze consumes packed origins once into the final region span, publishes its
  identity and dense coordinate atomically, and returns only the copyable
  facade.
- Exact interning reuses a live coordinate only after complete ordered id
  comparison. Rollback, retry, and rejection discard the aggregate suffix;
  individual handle drop performs no collection.
- Continuation detachment reads an ordered borrowed registry view and stores
  handle-free recipes. Detached identity and rendered provenance do not depend
  on allocation order or retained capacity.

The post-change phase census separates cumulative production, exact peak live
payload, retained legacy capacity, and process high water:

| Committed step | Sparse published bytes | Peak live sparse bytes | Stored-arena capacity growth | Root/id scratch bytes | RSS high water |
| -------------- | ---------------------- | ---------------------- | ---------------------------- | --------------------- | -------------- |
| 1,024          | 2,405,868              | 676,264                | 1,064,952                    | 0                     | 236,424 KiB    |
| 2,048          | 5,671,776              | 2,011,372              | 4,128,760                    | 0                     | 262,436 KiB    |
| 4,096          | 10,377,580             | 3,749,420              | 4,259,832                    | 0                     | 301,076 KiB    |
| 6,144          | 17,581,772             | 6,851,080              | 8,257,528                    | 0                     | 379,164 KiB    |

At step 6,144 the structural list change removes 57,052,368 bytes of
cumulative published payload and 74,634,140 bytes of cumulative scratch
allocation. Only 6,851,080 bytes of sparse list payload were live at any one
time, and the retained legacy capacity was 8,257,528 bytes. RSS high water did
not fall reliably: a repeated post-change run reached 362,500 KiB, while the
phase-aligned run above reached 379,164 KiB against the 378,628 KiB baseline.
The removed 131,686,508 bytes were therefore cumulative churn already absorbed
by allocator high-water reuse, not 131 MiB of simultaneously live semantic
state. The remaining process RSS cannot be assigned to the sparse list or its
scratch. These receipts prove the producer-level amplifier and its structural
removal but do not constitute the epic's sub-400 MiB acceptance row.

The stored-list arena is now retired. The pre-retirement API audit found
`allocate_list` only behind list builders and tests, `allocate_repeated_list`
only in tests and a benchmark, and `resolve_stored_list`/`contains_list` only
behind raw reads, exact-candidate lookup, and handle validation. The runtime
owners were macro-definition provenance and input-frame summaries; both now
store `OriginListRef` directly. Transient and inline command buffers already
own sparse roots through `RootedTracedTokenBuffer`. Consequently there is no
parallel origin-list allocator, span hash, historical candidate table, or
raw-id-to-owner bridge. `OriginListId` remains only the compact projection of a
live region value and as a detached serialization key; payload resolution
requires aggregate registry admission.

Formats require no schema transition because schema 11 never serialized
provenance. Dumping continues to exclude definition and input provenance.
Loading constructs macro definitions with absent provenance, and any
definition scanned after load installs its `OriginRef` roots and
`OriginListRef` coordinates
directly. Detached command continuations keep DTO-local origin-list recipe
keys, validate and stage the entire recipe graph in a destination fork, and
publish the newly materialized `OriginListRef` values only with the completed
summary. Source and loaded execution therefore share the same structural
lifetime rules without a compatibility sidecar.

#### Transient command owner and transition matrix

Every command-owned sequence of packed traced words is one structural value:
the ordered words plus a sorted set containing exactly one `OriginRef` for each
distinct non-direct origin used by those words. Unknown, fallback, and ordinary
direct-source positions add no strong root. A slice shares that value and names
only its word range; it does not copy roots or consult a store. Clone, move, and
drop therefore retain, transfer, and release both projections together.
Mutable scanner and matcher values use `RootedTracedTokenBuffer`; immutable
cursor and argument values use the same word-plus-sparse-roots representation
behind their shared buffer owners.

| Concrete owner                                                                                                                    | Packed positions and roots owned                                                                                                                    | Install, mutation, and release transitions                                                                                                                                                                                                                                                                                                                                   |
| --------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `TokenPayload::Transient` and its one-word inline form                                                                            | The exact inserted, converted, recovery, or scanner-replay words and their distinct non-direct roots.                                               | Construction consumes rooted words; push and replacement publish the complete value; delivery borrows the aligned root; cursor retirement, rollback, retry replacement, and final drop release it.                                                                                                                                                                           |
| `TokenPayload::BackedUp` and its one-word inline form                                                                             | The exact backed-up spelling roots plus independent committed source-range metadata.                                                                | `back_input` moves the delivered command root into the payload. `back_list` and e-TeX aftergroup prepend merge structural values before publication. Redelivery borrows the root without lookup; consumption, stack conservation, rollback, and retirement drop it. Rehoming changes only source-range metadata.                                                             |
| `MacroArgumentBuilder`, `MacroArguments`, and `TokenPayload::ArgumentRange`                                                       | One shared structural value for all completed arguments; ranges are nonowning half-open views. Repeated argument positions share one distinct root. | Argument collection appends rooted deliveries. Group stripping and delimiter removal preserve the surviving aligned positions. Activation publication freezes once; parameter substitution clones the value and range; body retirement drops the activation after all parameter ranges retire. Failed matching and retry drop or restore the whole builder/activation value. |
| Scanner and expansion scratch, `ScannedToks`, and `LiveTokenBuilder`                                                              | The exact unfinished output words and distinct roots accumulated so far. Empty reusable capacity owns no roots.                                     | Append consumes a rooted delivery or a root-preserving transformation. Freeze moves the complete value into a stored traced list. Error, rollback, or builder retirement clears roots before word capacity may return to the bounded scratch pool. A checkpoint remains forbidden while a live builder exists.                                                               |
| Alignment preamble and row builders, pending recovery tokens, and u/v-template replay inputs                                      | Rooted transient words until they freeze into `TracedTokenList`; stored templates carry `OriginListRef`.                                            | Preamble append, token rewrite, row retry, template insertion, and delimiter recovery preserve roots with words. Successful freeze atomically publishes token and origin-list coordinates; aggregate rollback discards the corresponding region suffix.                                                                                                                      |
| Environment aftergroup save entries and backup replay                                                                             | The saved word's non-direct root from assignment until group exit transfers it to command input.                                                    | Save-stack insertion captures the current command root. Group rollback/restoration retains the saved value. Group exit moves values in save order into backup payloads; e-TeX prepend deduplicates roots in the destination. Discarded groups and final environment drop release them.                                                                                       |
| Live `CommandState`, typed blocked continuations, `CommandSummary`, tex-state `InputSummary`, and macro-argument checkpoint forms | Structural transient values already owned by cursors, source pending input, arguments, alignment state, and permitted quiescent builders.           | A resource boundary moves the exact unfinished scanner or expansion state into its typed continuation; it does not clone or restore an aggregate retry root. Summary projection preserves roots without upgrading ids. Checkpoint pruning and generation retirement drop the durable aggregate closure directly.                                                             |
| Detached command continuation DTOs                                                                                                | Handle-free logical origin recipes referenced by word-local DTO indices; no runtime `OriginId`, `OriginRef`, or store coordinate.                   | Detachment walks each structural value's aligned roots and emits recipes without lookup. Materialization validates all recipes, builds destination-local roots and structural buffers in staging, and atomically publishes the complete command summary. Failure drops staging and leaves the destination unchanged.                                                         |

Transformations which change token spelling but retain diagnostic position keep
the same root while repacking the word. A transformation which creates an
unknown or direct position supplies the corresponding rootless value. No API
may accept an arena-tagged raw `OriginId` as sufficient ownership, recover a
root from `ProvenanceStore`, register a buffer in a lifetime side table, or
scan command state after publication to repair missing roots. The legacy
stored-list/raw-record arena remains compatibility authority only until
`umber2-3v8z.22.15`; this matrix neither retires nor extends it.

### Expansion frames

An active macro invocation owns one immutable `ExpansionFrameRef` containing
the nonowning macro observation operand, invocation position, definition
position, and optional parent frame. The parent is a strong structural edge.
Exact repeated calls at the same call site, definition site, and parent share
one frame through a weak collision-safe pool.

No general `OriginRecord::MacroInvocation` row is appended during expansion.
The input stack carries the active frame reference in O(1). A diagnostic which
requests an expansion trace retains the current frame reference and walks the
parent chain only while rendering. Trace labels, depth selection, paths,
lines, excerpts, and strings are therefore created only on the error path.
Runs with no diagnostic observer allocate no detailed trace rows.

## Audited owners and transitions

The reachability cutover uses the following concrete owner matrix. Compact
`OriginId` and `OriginListId` fields remain projections; each row names the
parallel typed owner which must move at the same transition. Store slots,
candidate buckets, retry-key leases, and allocation journals are weak or
operation-local and are absent from this matrix.

| Ownership stratum           | Concrete typed roots                                                                                                                                                            | Exact install, restoration, and release transition                                                                                                                                                                                              |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Source registrations        | `SourceMap` regions, live command `SourceLevel`s, source-frame summaries, and detached source recipes own `SourceRegistrationRef`.                                              | Registration publishes the root before the first token. Source pop drops the command owner; rollback or source-map replacement drops the region owner; checkpoint clones share both.                                                            |
| Frozen token positions      | `TracedTokenList` carries `OriginListRef`; stored `TokenPayload` copies exact origins from a borrowed registry view into its transient packed buffer.                           | Token-list freeze installs token and origin-list coordinates together. Replay borrows the region span; rollback invalidates a discarded list generation before slot reuse.                                                                      |
| Transient command positions | Shared macro-argument, insertion, backup, scanner-recovery, pending-source, and inline command buffers own roots aligned with their packed traced words.                        | Push, prepend, replacement, and backup capture the root before publishing a cursor. Consumption, input retirement, snapshot rollback, or buffer replacement drops the same aligned root.                                                        |
| Expansion frames            | `MacroActivation`, the macro-body input level, `TracedExpansionToken`, and recoverable command diagnostic state own `ExpansionFrameRef`.                                        | Macro-call publication captures call, definition, and parent roots before the activation becomes visible. Retirement, failed matching, retry rollback, summary replacement, and continuation detachment release it.                             |
| Definition provenance       | A live `MacroDefinitionRef` occurrence owns optional definition, parameter-list, and replacement-list roots; the semantic macro body owns none.                                 | Definition scanning publishes the three roots with the occurrence. Redefinition/undo, group restoration, format-base release, and final occurrence release drop them. Loaded formats publish absent provenance.                                 |
| Nodes and mode/page state   | Owned character/ligature nodes and compact `NodeStorage` columns own aligned position roots; pending horizontal runs and persistent mode/page/node roots share them.            | Node construction captures the delivered root, freeze transfers it to the compact column, arena rollback truncates it, survivor promotion shares it, and final list/page/root release drops it.                                                 |
| Diagnostics                 | `DiagnosticSite` owns primary, related, and optional expansion-frame roots. Recoverable command and execution errors own the site until rendering or freezing.                  | Capture resolves raw projections to roots while the producing state is live. Retry either restores the prior site or freezes its presentation before releasing failed-operation roots.                                                          |
| Artifacts                   | Live render-origin sidecars own aligned `OriginRef`s; stable editor recipes own no runtime origin.                                                                              | Shipout clones node roots while the page is live. Output splice shares them, recipe conversion releases them, and output replacement/eviction drops the final roots.                                                                            |
| Checkpoints and generations | `CommandSummary`, source/input summaries, mode/page/node state, diagnostic state, and artifact prefixes structurally own the rows above.                                        | Snapshot/fork clones typed fields. Rollback restores destination roots before releasing the failed operation; pruning and final substrate release drop record-exclusive roots directly.                                                         |
| Private revision            | Provenance objects allocated during one aggregate operation are owned first by returned typed roots and by the revision's typed destinations; a retry-key lease owns no object. | Operation failure restores destinations then drops the exact operation suffix and leases its keys. Rejection drops the candidate roots. Acceptance keeps already-installed typed roots and clears weak private metadata without traversing ids. |

| Stratum                   | Strong owner                                                                                                                                                                                                                                                           | Install and release boundary                                                                                                                                                                                                                             |
| ------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Source input              | Live source level and source/input checkpoint summary own the registration.                                                                                                                                                                                            | Registration publishes the root before first delivery. Source retirement, rollback, checkpoint eviction, and generation replacement drop it.                                                                                                             |
| Stored token list         | `TracedTokenList` carries `OriginListRef` beside `TokenListRef`; aggregate region roots own storage.                                                                                                                                                                   | List freeze captures both coordinates atomically. Replay copies the facade and borrows payload; individual replay/list drop performs no liveness work.                                                                                                   |
| Macro definition          | One definition occurrence owns optional definition, parameter-list, and replacement-list roots.                                                                                                                                                                        | Definition scan installs all roots with the macro occurrence. Redefinition, undo release, format-base release, or final definition-owner drop releases them. Loaded formats begin with absent provenance.                                                |
| Macro invocation          | `MacroActivation` and command summaries own `ExpansionFrameRef`. Argument positions are owned by the activation's structural position lists.                                                                                                                           | Activation publication captures the frame before the body level is visible. Retirement, retry rollback, continuation detachment, checkpoint eviction, or final activation drop releases it.                                                              |
| Transient command input   | Source pending tokens, backed-up input, scanner recovery, alignment templates, and transient token buffers own the exact position roots they retain.                                                                                                                   | Push/prepend/replace operations attach roots with words. Pop, consumption, rollback, or summary release drops the same slice. No command-state-wide provenance history exists.                                                                           |
| Nodes and mode/page state | Character and ligature node sidecars own roots aligned with their compact ids; persistent list/page roots share those sidecars.                                                                                                                                        | Node freeze captures roots before publication. Arena rollback, survivor release, page reset, list replacement, and final node-root drop release them. Provenance never keeps the node alive.                                                             |
| Diagnostics               | A live diagnostic site owns primary, related, and optional expansion-frame roots until it is rendered or frozen to owned presentation data.                                                                                                                            | Error capture clones only named roots. Resource rollback renders or freezes required evidence before dropping the failed operation. Accepted diagnostic DTOs own strings/ranges, not runtime handles.                                                    |
| Artifacts                 | Render-source sidecars own structural roots or detached stable recipes for exactly their slots.                                                                                                                                                                        | Shipout attaches roots/recipes with the artifact. Output splice shares them; output replacement/eviction drops them. A query lazily builds only its page index.                                                                                          |
| Checkpoints and history   | Command, mode, page, source, and artifact aggregates above form the checkpoint closure.                                                                                                                                                                                | Snapshot/fork clones typed roots. Pruning drops record-exclusive roots and the final substrate owner releases shared roots. No provenance watermark makes unrelated history live.                                                                        |
| Continuations and memos   | Detached command continuations encode source-relative ranges, origin lists, macro provenance, and expansion-frame ancestry through DTO-local recipe indices; they retain no source, token, macro, origin-list, or frame root. Pure memos and formats strip provenance. | Materialization validates the complete logical graph, builds destination-local roots in a staged Universe fork, and swaps only after the complete command summary exists. Any validation or destination conflict leaves the original Universe unchanged. |
| Private revision          | The candidate allocation domain owns each newly allocated atom/list until a typed destination captures it.                                                                                                                                                             | Failed operation and `NeedResource` restore destinations, then release the exact suffix. Rejection drops the domain. Acceptance transfers typed live roots already held by state/output owners; it does not scan ids.                                    |

Read-only origin resolution, token rendering, observation construction, state
hashing, and artifact lowering borrow roots. A raw `OriginId`,
`OriginListId`, source-map position, macro operand, or node ordinal is never a
strong owner.

For character material delivered from a macro body, construction captures the
innermost live `ExpansionFrameRef`, not merely the macro-definition token's raw
position. The pending horizontal run keeps that root through shaping and
ligature formation. Direct shipout then appends the roots borrowed from the
char or ligature node to the artifact sidecar. It neither upgrades raw ids nor
scans the provenance graph after page traversal.

## Demand policy

Source registration and direct scalar coordinates are required for exact
input and restart behavior. Detailed provenance has two explicit consumers:

- diagnostic sites need primary/related positions and the active expansion
  frame only when an error is consumed; and
- rendered-source output needs node position roots only when the selected
  output/session contract exposes rendered-source queries.

The engine receives an immutable provenance-demand policy at job creation.
The policy cannot change between accepted revisions. With no rendered-source
consumer, node and artifact provenance columns remain absent. With no
diagnostic event, expansion-frame presentation rows and strings remain
absent. TeX tracing text remains TeX execution output and is unaffected.

Disabling a consumer degrades only that optional diagnostic surface. It does
not alter token delivery, source ids, source registration, lexer coordinates,
command observations, execution effects, or artifact bytes.

## Retry, rejection, and acceptance

An aggregate operation mark covers the compatibility origin-record archive and
the runtime registry's origin-list identities, dense locations, and region
span. A failed operation first restores command, mode, node, diagnostic, and
artifact destinations, then truncates the rejected aggregate suffix. Exact
local record retry may lease discarded packed keys in allocation order, but
the lease owns no accepted history and is abandoned at the first structural
divergence. Successful earlier candidate roots remain once.

Dropping `RevisionCandidate` or `RevisionTransaction` releases every private
provenance root and allocation lease. At acceptance, replacement state
already holds its structural roots. Convergence output already owns roots or
stable recipes, so the scratch Universe can be dropped directly. The existing
`retain_origin_graph_from` traversal is forbidden and removed.

## Capacity and accounting

Production uses explicit independent budgets for structural origin records,
runtime origin-list coordinates, origin-list entries, and detached artifact
recipes. Exhaustion degrades optional new provenance to unknown or the empty
list and never aborts TeX. Origin-list admission performs collision-safe full
ordered comparison through the registry, then checks list and entry limits
before reserving its identity, location, and region span. There is no sweep,
candidate cache, or second liveness limit.

The common expansion frame owns exactly three child positions, so those roots
reside inline with the immutable origin atom rather than in a second heap
allocation. Full ordered `OriginId` comparison is the origin-list collision
authority. A rooted transient buffer already proves its owner/word alignment,
so optimized freeze does not repeat the debug-only owner-membership audit.
These representation choices change neither packed ids nor strong edges.

`ProvenanceStats` reports structural records and frames, runtime origin lists
and entries, retained region/location/identity capacity, source registrations,
retry leases, and detached output charges separately. The retained provenance
column charge is shared with token and macro sparse origins because those rows
physically occupy the same `RuntimeValueRegion` column.

The focused controls exercise 10,000 accepted/rejected macro and token-position
transitions. Rollback must restore logical list and entry counts while retained
region capacity may remain warm. Repeated identical macro expansion must keep
one structural frame for one `(operand, invocation, definition, parent)` tuple.

## Compatibility and validation

The migration preserves:

- exact direct and arena `OriginId` packing boundaries and unknown fallback;
- exact diagnostic primary ranges, related labels, macro trace order, paths,
  excerpts, line and column presentation;
- edit-stable current/deleted/foreign/unknown resolution;
- artifact node/source ordinals and rendered-source query results;
- source, loaded-format, checkpoint restart, continuation, and retry output;
  and
- byte-identical semantic effects, artifacts, DVI, PDF, and HTML.

Focused controls cover direct and wide source ranges, collisions, repeated
macro frames, nested trace lifetime after input pop, traced-list final-owner
release, transient backup/recovery, node and artifact ownership, lazy page-map
construction, loaded formats, continuation materialization, source and editor
restart, resource retry, candidate rejection, convergence acceptance, weak
slot reuse, bounded-live plateau, and exact all-live growth.

Run the affected `tex-state`, `tex-command`, `tex-exec`, and `tex-incr` tests
serially, then `cargo test -q --tests` and `scripts/check.sh`. Corpus and
profiling validation remain the final epic tier.
