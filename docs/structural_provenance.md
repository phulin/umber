# Structural and demand-driven provenance

Status: ownership contract for Beads issue `umber2-3v8z.4`.

This document replaces append-history ownership of diagnostic provenance. It
refines [Compact Source Spans and Token Provenance](source_spans_and_provenance.md)
without changing packed `OriginId`, source coordinates, rendered-source query
results, or diagnostic presentation.

## Invariant

Persistent provenance is owned only by a live diagnostic consumer, a live
source-map registration, a token position that may still be replayed, or a
detached artifact which exposes rendered-source information:

```text
typed origin root -> immutable origin atom -> typed child roots
                         ^
                         |
                   bounded weak index
```

An origin-store coordinate, weak candidate bucket, retry lease, allocation
serial, cache entry, or checkpoint watermark is not ownership. Provenance
does not confer semantic ownership on tokens, macro definitions, nodes, input
state, or output. It remains excluded from token equality, `\ifx`, formats,
semantic hashes, convergence, artifact bytes, and artifact content identity.

There is no copying compactor, retained-generation provenance arena,
post-acceptance graph scan, or history truncation pass. Every strong edge is
installed or removed at the typed transition which already knows the owner
and origin.

## Structural values

The first migration slice installs exact structural candidate reuse in the
existing packed store. `OriginRecord` candidates compare the complete record;
origin-list candidates use a compact hash only to select a bucket and compare
every `OriginId` before reuse. Rollback removes discarded candidates before
identity reuse, forks share inherited values while preserving sibling key
separation, and exact operation replay preserves its packed identity. This
foundation eliminates duplicate transition rows, including 10,000 identical
macro-frame allocations collapsing to one record. The following ownership
sections govern the subsequent replacement of the remaining append-watermark
authority with typed roots and weak reusable slots.

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

`OriginListRef` is one immutable exact sequence of `OriginRef` values. Its
`OriginListId` is a compact coordinate only. The empty list is an immortal
builtin. Dynamic lists use reusable generation-safe weak slots and a bounded
weak candidate index; candidate hash collisions perform exact id and child
comparison. Extending an untraced list preserves the empty-list contract,
while extending a traced list creates or reuses one complete structural list.

Inserted and synthesized transitions do not create a flattened wrapper for
every delivery. Expansion-control information which affects delivery remains
in the command token state (`NOEXPAND_FALLBACK` where required). Diagnostic
position follows the nearest structural parent, with an optional compact
role retained only by a consumer which can present that distinction.

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
| Frozen token positions      | `TracedTokenList` and stored `TokenPayload` own `OriginListRef`; the list structurally owns one `OriginRef` per nonempty position.                                              | Token-list freeze installs token and origin roots together. Stored replay, macro definition provenance, alignment templates, summaries, and final cursor retirement clone or drop the pair atomically.                                          |
| Transient command positions | Shared macro-argument, insertion, backup, scanner-recovery, pending-source, and inline command buffers own roots aligned with their packed traced words.                        | Push, prepend, replacement, and backup capture the root before publishing a cursor. Consumption, input retirement, snapshot rollback, or buffer replacement drops the same aligned root.                                                        |
| Expansion frames            | `MacroActivation`, the macro-body input level, `TracedExpansionToken`, and recoverable command diagnostic state own `ExpansionFrameRef`.                                        | Macro-call publication captures call, definition, and parent roots before the activation becomes visible. Retirement, failed matching, retry rollback, summary replacement, and continuation detachment release it.                             |
| Definition provenance       | A live `MacroDefinitionRef` occurrence owns optional definition, parameter-list, and replacement-list roots; the semantic macro body owns none.                                 | Definition scanning publishes the three roots with the occurrence. Redefinition/undo, group restoration, format-base release, and final occurrence release drop them. Loaded formats publish absent provenance.                                 |
| Nodes and mode/page state   | Owned character/ligature nodes and compact `NodeStorage` columns own aligned position roots; pending horizontal runs and persistent mode/page/node roots share them.            | Node construction captures the delivered root, freeze transfers it to the compact column, arena rollback truncates it, survivor promotion shares it, and final list/page/root release drops it.                                                 |
| Diagnostics                 | `DiagnosticSite` owns primary, related, and optional expansion-frame roots. Recoverable command and execution errors own the site until rendering or freezing.                  | Capture resolves raw projections to roots while the producing state is live. Retry either restores the prior site or freezes its presentation before releasing failed-operation roots.                                                          |
| Artifacts                   | Live render-origin sidecars own aligned `OriginRef`s; stable editor recipes own no runtime origin.                                                                              | Shipout clones node roots while the page is live. Output splice shares them, recipe conversion releases them, and output replacement/eviction drops the final roots.                                                                            |
| Checkpoints and generations | `CommandSummary`, source/input summaries, mode/page/node state, diagnostic state, and artifact prefixes structurally own the rows above.                                        | Snapshot/fork clones typed fields. Rollback restores destination roots before releasing the failed operation; pruning and final substrate release drop record-exclusive roots directly.                                                         |
| Private revision            | Provenance objects allocated during one aggregate operation are owned first by returned typed roots and by the revision's typed destinations; a retry-key lease owns no object. | Operation failure restores destinations then drops the exact operation suffix and leases its keys. Rejection drops the candidate roots. Acceptance keeps already-installed typed roots and clears weak private metadata without traversing ids. |

| Stratum                   | Strong owner                                                                                                                                              | Install and release boundary                                                                                                                                                                                          |
| ------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Source input              | Live source level and source/input checkpoint summary own the registration.                                                                               | Registration publishes the root before first delivery. Source retirement, rollback, checkpoint eviction, and generation replacement drop it.                                                                          |
| Stored token list         | `TracedTokenList` owns its `OriginListRef` beside `TokenListRef`.                                                                                         | List freeze captures both values atomically. Replay clones the list root; final replay/list owner release drops it.                                                                                                   |
| Macro definition          | One definition occurrence owns optional definition, parameter-list, and replacement-list roots.                                                           | Definition scan installs all roots with the macro occurrence. Redefinition, undo release, format-base release, or final definition-owner drop releases them. Loaded formats begin with absent provenance.             |
| Macro invocation          | `MacroActivation` and command summaries own `ExpansionFrameRef`. Argument positions are owned by the activation's structural position lists.              | Activation publication captures the frame before the body level is visible. Retirement, retry rollback, continuation detachment, checkpoint eviction, or final activation drop releases it.                           |
| Transient command input   | Source pending tokens, backed-up input, scanner recovery, alignment templates, and transient token buffers own the exact position roots they retain.      | Push/prepend/replace operations attach roots with words. Pop, consumption, rollback, or summary release drops the same slice. No command-state-wide provenance history exists.                                        |
| Nodes and mode/page state | Character and ligature node sidecars own roots aligned with their compact ids; persistent list/page roots share those sidecars.                           | Node freeze captures roots before publication. Arena rollback, survivor release, page reset, list replacement, and final node-root drop release them. Provenance never keeps the node alive.                          |
| Diagnostics               | A live diagnostic site owns primary, related, and optional expansion-frame roots until it is rendered or frozen to owned presentation data.               | Error capture clones only named roots. Resource rollback renders or freezes required evidence before dropping the failed operation. Accepted diagnostic DTOs own strings/ranges, not runtime handles.                 |
| Artifacts                 | Render-source sidecars own structural roots or detached stable recipes for exactly their slots.                                                           | Shipout attaches roots/recipes with the artifact. Output splice shares them; output replacement/eviction drops them. A query lazily builds only its page index.                                                       |
| Checkpoints and history   | Command, mode, page, source, and artifact aggregates above form the checkpoint closure.                                                                   | Snapshot/fork clones typed roots. Pruning drops record-exclusive roots and the final substrate owner releases shared roots. No provenance watermark makes unrelated history live.                                     |
| Continuations and memos   | Detached continuations encode logical source/range/frame recipes only when the continuation exposes diagnostics. Pure memos and formats strip provenance. | Materialization validates source identities and installs destination-local roots before publishing input state. Rejection publishes nothing.                                                                          |
| Private revision          | The candidate allocation domain owns each newly allocated atom/list until a typed destination captures it.                                                | Failed operation and `NeedResource` restore destinations, then release the exact suffix. Rejection drops the domain. Acceptance transfers typed live roots already held by state/output owners; it does not scan ids. |

Read-only origin resolution, token rendering, observation construction, state
hashing, and artifact lowering borrow roots. A raw `OriginId`,
`OriginListId`, source-map position, macro operand, or node ordinal is never a
strong owner.

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

An aggregate operation mark includes provenance allocation ownership. A
failed operation first restores command, mode, node, diagnostic, and artifact
roots, then releases atoms and lists allocated after the mark. Exact local
retry may lease the discarded packed keys in allocation order, but the lease
owns no accepted history and is abandoned at the first structural divergence.
Successful earlier candidate roots remain once.

Dropping `RevisionCandidate` or `RevisionTransaction` releases every private
provenance root and weak allocation lease. At acceptance, replacement state
already holds its structural roots. Convergence output already owns roots or
stable recipes, so the scratch Universe can be dropped directly. The existing
`retain_origin_graph_from` traversal is forbidden and removed.

## Capacity and accounting

Production uses explicit independent budgets for live structural atoms,
origin-list entries, weak slot metadata, weak candidate buckets, and detached
artifact recipes. Exhaustion degrades optional new provenance to unknown and
never aborts TeX. Weak indexes are bounded and may be cleared at any time.
Dead reusable slots are reclaimed at the next allocation; capacity must
plateau at the live-root and configured-cache high-water size.

`ProvenanceStats` reports live rooted atoms, live expansion frames, live
origin lists and entries, weak slot/index capacity, source registrations,
retry leases, and detached output charges separately. Measurement scans live
roots or weak slots on demand and adds no production expansion-path counters.

The focused plateau is 10,000 accepted/rejected bounded-live macro and token
position transitions. After warm-up, live objects and retained weak metadata
must remain constant within the explicit weak-index budgets. The negative
control retains every root and must grow by the exact object and logical-byte
charge. Repeated identical macro expansion must keep one structural frame for
one `(operand, invocation, definition, parent)` tuple.

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
