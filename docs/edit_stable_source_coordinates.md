# Edit-Stable Source Coordinates

Status: implemented through phase 4. Per-revision whole-document source
regions have been replaced by fragment-backed coordinates so token, node, and
shipout provenance stays resolvable and correct across editor inserts and
deletes. Retired fragment bytes are pruned after checkpoint protection ends,
and long-session capacity and read-path costs are measured.

## 1. Problem

The provenance carrier chain is complete and cheap: tokens carry a packed
32-bit `OriginId` (`source_spans_and_provenance.md` §5), character and
ligature nodes retain origin columns through ligaturing, hyphenation, math
layout, packing, and line breaking (§6.3), and shipout emits the
`render_origins` sidecar that `rendered_source_map.md` turns into per-page
retained maps. None of that needs to change.

What breaks under editing is the coordinate space those ids point into. A
direct `OriginId` encodes a logical `SourcePos`, and a `SourcePos` resolves
through one `SourceRegion` to a byte offset in one immutable backing. For the
root editor document, `Session::advance` restarts through
`Universe::rebind_root_editor_input`, which registers the **entire new
document** as a fresh `GeneratedSource` region every revision. Three defects
follow:

- **Stale byte offsets on reused output.** Convergence and prefix reuse
  splice old pages into the accepted output. Their `render_origins` were
  minted against an older revision's region, so a rendered-source query
  resolves to offsets in that older snapshot but reports them against the
  live `source_path`. After any insert or delete earlier in the document,
  every reused page answers with a plausible, wrong location.
- **Dead origins on adopted scratch output.** In the convergence branch the
  session keeps the *old* accepted substrate and adopts the scratch window's
  pages. Those pages' origins reference the region registered on the
  discarded fork, which the retained substrate never saw. Direct fragment
  positions remain session-live, but scratch-only arena wrappers around those
  positions would degrade to unknown without explicit adoption.
- **Retained memory scales with revisions.** Each revision's rebind pins one
  full-document `Arc<[u8]>` for as long as any live region, checkpoint, or
  reused page can reference it.

Eagerly rewriting retained origins on each edit is not an option: origins
live in frozen origin lists, node columns, and committed artifact sidecars
that are immutable by design, and the update budget must not scale with
retained state.

## 2. Design overview

Invert the coordinate contract. Today a `SourcePos` names a location in a
document revision; the design makes it name a byte of **content** — a
position inside an immutable *source fragment* — and makes "where is that
byte in the current document" a derived, per-revision view:

- **`SourceFragment`**: an immutable byte string that entered the editor
  document at some point — the initial document is one fragment, and each
  accepted edit contributes exactly one fragment holding its (line-expanded)
  replacement text. Fragments are session-scoped and never mutated, moved,
  or renumbered. Each owns one disjoint range of the logical `SourcePos`
  space, exactly like today's regions.
- **`EditorLayout`** (a piece table): the ordered list of fragment
  *views* — `(fragment, byte sub-range)` — whose concatenation is the current
  document. Edits mutate only this table. A monotonically increasing
  `LayoutGeneration` stamps each accepted table.

Provenance records everywhere — tokens, origin-list entries, node columns,
`render_origins`, retained `PageRenderMap`s — keep storing opaque
`OriginId`s and are never touched by an edit. The layout table is the single
mutable structure, and it is O(pieces), not O(retained provenance).

```text
fragments (immutable, append-only)        editor layout (mutable, per revision)
F0: "The quick brown fox\n…intro…"        rev 7: [F0[0..14) F2[0..9) F0[23..410)]
F1: "jumped over\n"        (deleted)                 |         |        |
F2: "leaped at\n"                                 doc 0      doc 14   doc 23
```

A `SourcePos` inside `F0[23..410)` resolves to the same source text at every
revision that retains that view, and its *document offset* is recomputed
from the table at read time. A `SourcePos` inside `F1` resolves to a typed
`Deleted` result instead of a stale or aliased location.

### 2.1 Why this matches the cost profile

- **Construction** stays the existing single add per token: the lexer keeps
  calling `RegisteredSource::direct_origin(start, end)` with
  fragment-relative offsets. No table is consulted per token (§4).
- **Fragment metadata** costs O(log fragments) per accepted append and O(1)
  per immutable engine-generation snapshot. Piece replacement rebuilds the
  O(pieces) flat arrays plus the indexed layout described below. Nothing
  retained is rewritten, so there is nothing to go stale.
- **Layout construction** rebuilds the flat piece and document-start arrays
  and a static fragment/offset index. For fragment `f` with `v_f` views, the
  index costs O(`v_f log v_f`) time and storage; total index cost is
  O(Σ `v_f log v_f`). This remains accepted-edit work and the resulting
  layout is immutable and shareable.
- **Read** pays O(log fragments + log views-of-the-hit-fragment) to select the
  first covering piece, independent of preceding document-order pieces, plus
  lazy line indexing of the current document. Reads are rare (clicks, hovers,
  diagnostics), and subsequent line/column queries reuse the per-generation
  line index.

## 3. Data structures

```rust
/// tex-state: session-scoped, append-only, immutable entries.
pub struct FragmentStore {
    fragments: PersistentTree<SourceFragment>, // O(log n) indexed append
    sources: HashMap<FragmentId, FragmentSource>, // session owner only
    append_lineage: u64,              // fresh on every writable clone
}

pub struct SourceFragment {
    id: FragmentId,                   // lineage tag + dense slot
    region_start: SourcePos,          // disjoint logical range, + end anchor
    byte_len: u64,
    minted_revision: RevisionId,      // diagnostic metadata
}

pub struct FragmentSource {
    bytes: Option<Arc<[u8]>>,         // absent from immutable engine views
    removed_revision: Option<RevisionId>,
    live_generation: LayoutGeneration,
}

/// tex-incr: the accepted document as an ordered sequence of fragment views.
pub struct EditorLayout {
    generation: LayoutGeneration,
    pieces: Vec<Piece>,               // document order
    doc_starts: Vec<usize>,           // prefix sums, rebuilt per accept (O(pieces))
    fragment_index: Vec<FragmentPieceIndex>, // sorted by FragmentId
}

pub struct FragmentPieceIndex {
    starts: Vec<u32>,                 // views sorted by fragment start
    ends: Vec<u32>,                   // compressed end-offset domain
    roots: Vec<NodeId>,               // persistent range-min prefix roots
    nodes: Vec<RangeMinNode>,         // earliest document piece per end range
}

pub struct Piece {
    fragment: FragmentId,
    range: Range<u32>,                // fragment-relative byte view
}
```

Positions and fragment ids are process-unique and never reused, extending
the existing rollback invariant. A clone shares the persistent metadata root
in O(1) but receives a fresh append lineage, so sibling copy-on-write appends
at the same dense slot mint different handles and reject one another rather
than aliasing. Appends path-copy O(log fragments) metadata nodes. Mutable byte
retention lives only in the session owner; pruning marks the layout's live
source entries in place and drops retired entries without cloning metadata or
allocating a fragment-count bitmap. The `FragmentStore` is owned by the
session's retained root and shared with every engine generation as an O(1)
metadata snapshot installed together
with its validated layout at rebind. It therefore survives fork discard: a
fragment minted for an edit stays resolvable no matter which substrate —
scratch or converged — wins the revision, which fixes the dead-origin defect
directly.

For each fragment, index construction sweeps views by start offset. Every
prefix root is a persistent range-min tree over end offsets and stores the
earliest document-order piece index. Resolution binary-searches the last
eligible start prefix and range-mins the end-offset suffix that covers the
requested high offset. This is O(log views) even when one fragment has many
repeated or overlapping views. Selecting the minimum piece index preserves
the former linear scan's first-covering-view semantics, including zero-width
anchors (whose end threshold is the anchor itself).

Convergence also transfers the diagnostic graph reachable from each adopted
artifact's `render_origins` through the `GenerationSubstrate` aggregate
facade. Arena keys are process-global, so artifacts keep their existing ids;
only missing scratch records are appended to the retained provenance index.
Scratch-only engine sources are captured as owned resolved locations. This is
diagnostic-only retention: semantic state, source stores, checkpoints, and
artifact identity are unchanged, and raw substores never cross the facade.

Engine-registered sources (World input files, non-editor generated sources)
keep today's substrate-owned, watermark-rolled-back regions unchanged. The
logical position allocator remains a single session-lifetime high-water mark
so fragment ranges and engine ranges never collide, and discarded forks
never re-hand out positions.

### 3.1 Line-aligned fragments

Fragment boundaries always fall on physical line boundaries of the document.
`advance` expands each incoming `Edit` outward to whole lines before minting
the replacement fragment: the fragment holds the complete new text of every
line the edit touches.

This buys two structural simplifications:

- Every physical line the lexer reads lies inside exactly one fragment, so
  per-token position minting inside a line stays pure offset arithmetic, and
  every arena `SourceSpan` (control sequences, `^^` spellings — which cannot
  cross lines) keeps its single-region invariant.
- Invalidation granularity equals re-lex granularity. Positions in the
  untouched portion of an edited line become `Deleted`, but that text is
  re-lexed by the same `advance` and its surviving output carries fresh
  positions in the new fragment; nothing reusable is lost.

The root document keeps **one stable `SourceId` for the whole session**
(today it burns one per revision); fragments, not source ids, provide
per-revision identity.

## 4. Construction path (hot)

The root editor input adapter still reads the contiguous current-document
`String`, and `next_source_offset`, checkpoint anchors, and `EditMap`
rehoming all remain document-offset-based operational state — unchanged.

The only change is how a physical line's positions are minted. At rebind,
instead of registering a whole-document region, the session installs a
frozen **`LayoutCursor`**: the per-revision table of
`(document line start, fragment registration, fragment-relative base)`
derived from `EditorLayout` (built once per `advance`, O(pieces)). When the
reader takes the next physical line, the cursor advances monotonically to
the piece containing it — amortized O(1) per line, one comparison in the
common case — and hands the frame that line's `RegisteredSource` capability
plus base offset. Per-token work is exactly today's
`registration.direct_origin(start, end)` single-add path; there are no new
per-token branches, no allocation, and no store writes.

Everything downstream is untouched by construction: origin lists, macro
replay, node origin columns through line breaking, page building, and the
shipout `render_origins` sidecar all copy opaque 4-byte ids exactly as they
do now. This design adds zero bytes and zero writes to those paths.

## 5. Read path (cold)

Resolution gains a layout-aware entry point used by session queries and
diagnostic rendering:

```text
resolve(origin, &FragmentStore, &EditorLayout) ->
    Current { path, doc_offset_lo, doc_offset_hi, line, column }
  | Deleted { minted_revision }          // fragment view no longer in layout
  | Foreign { … }                        // World/static/generated regions, as today
  | Unknown
```

1. Decode `OriginId` to a `SourcePos` or arena `SourceSpan` (unchanged).
2. Binary-search the fragment store's region ranges. A miss falls through to
   the substrate source map (engine-registered sources) and resolves as
   today.
3. On a fragment hit, binary-search the fragment-id table, then use its
   persistent start-prefix/end-suffix range-min index to select the earliest
   document-order view covering the complete fragment-relative span. No
   covering view means the text was edited away: return typed `Deleted`
   (optionally with the nearest surviving neighbor for UX) — never a stale
   offset. This is O(log pieces) in the worst case and does not scan earlier
   document pieces.
4. Covering view: `doc_offset = doc_starts[piece] + (pos - view.start)`,
   then line/column via a lazily built line-start index of the **current**
   document, cached and keyed by `LayoutGeneration`.

Every cache derived from resolution (the per-page resolved-span cache in
`rendered_source_map.md` §2.4 included) must be keyed by
`LayoutGeneration`, which supersedes revision-keying for source-coordinate
caches. The `data-umber-revision` staleness handshake for event ordinals is
unaffected — and reused pages' retained maps now stay *correct* across
edits instead of merely detectably stale, because they store `OriginId`s and
resolve through the live layout at query time.

The reverse mapping (`rendered_source_map.md` §2.6, editor→preview forward
search) composes cleanly: a current-document range maps through the layout
to O(affected pieces) fragment-space ranges, each binary-searched in the
retained per-page tables sorted by `SourcePos`.

## 6. Rollback, forks, and pruning

- **Within a compile**, engine snapshot/rollback semantics are unchanged:
  the fragment store is read-only during execution, so snapshot capture
  stays O(1) and rollback has nothing new to truncate.
- **Across revisions**, a failed or discarded `advance` may leave its newly
  minted fragment in the store as an orphan: no layout view, resolves as
  `Deleted`, bytes droppable. Correctness never depends on unwinding the
  store.
- **Byte pruning**: when the last layout view of a fragment disappears and
  no retained checkpoint's revision precedes the removal, the fragment's
  bytes drop; its metadata row (region range, `minted_revision`) is retained
  forever in the persistent metadata tree so old ids stay typed-`Deleted`
  rather than aliasable. Session
  memory becomes O(initial document + live inserted text + metadata per
  edit) instead of one full document per revision. `self.source` remains the
  single contiguous current-document copy.

## 7. Capacity

Direct-encodable position space is 2^31. Fragment space consumption is
session-cumulative: initial document + Σ line-expanded replacement lengths.
A pathological character-at-a-time session re-mints one line per keystroke
(~10^2 bytes); 10^5 keystrokes consume ~10^7 positions, two orders of
magnitude below the boundary, before any host-side edit coalescing.
Exhaustion degrades exactly as today: arena `SourceSpan` fallback, then
`OriginId::UNKNOWN` — never an abort, never a wrong location.

Piece count grows by ≤2 per edit and is a layout-build, retained-diagnostic,
read-path, and per-advance-cursor cost. The fragment/offset index makes reads
logarithmic but adds O(Σ `v_f log v_f`) retained nodes; a worst-case 10^5
views of one fragment is roughly 17 tree levels and tens of MiB rather than
the few MiB of the flat tables alone. If it ever matters, an **epoch rebase**
is reserved: mint one
fragment covering the whole current document, reset the layout to one piece,
and rewrite each live fragment's metadata with a remap into the new epoch so
lookups chase at most one hop. Deferred until measurements demand it.

## 8. Rejected alternatives

- **Eager rebasing of retained records.** Rewriting node columns, origin
  lists, and artifact sidecars per edit is O(retained state) work on the
  edit path and violates the immutability of committed artifacts and frozen
  lists.
- **Per-revision regions + composed `EditMap` chains at read time.** Keeps
  construction untouched but read cost and pinned snapshots grow with
  revision count, delta GC is required, and the compacted fixed point of
  composing edit deltas *is* a piece table — with extra steps. Also does not
  fix fork-dead scratch origins.
- **Editor-style marker/anchor trees per position.** Per-token allocation
  and pointer chasing on the delivery path; violates the zero-write
  invariant that motivated packed origins.
- **Re-deriving provenance by content search.** Matching rendered text back
  to source by n-gram search is heuristic, ambiguous under repetition, and
  O(document) per query with no staleness guarantee.
- **Arbitrary-boundary fragments.** Letting piece boundaries fall mid-line
  preserves a few more positions per edit but forces segmented-line position
  minting and multi-region spans into the lexer hot path.

## 9. Invariants preserved

- Origins remain excluded from token equality, `\ifx`, interning, semantic
  hashes, memo keys, format images, and artifact content identity.
- Ordinary source-character delivery performs zero provenance-store writes;
  `TracedTokenWord` stays 64 bits with a 32-bit origin field.
- Snapshot capture stays O(1) by sharing the immutable metadata root;
  positions and fragment ids are never reused
  across rollback or fork discard.
- Resolution degrades to typed `Deleted`/`Unknown`, never to a silently
  wrong offset; diagnostic exhaustion never aborts execution.
- Node origin columns and `render_origins` accounting are unchanged; the
  fragment store and layout are display-only session state charged to
  retained-diagnostic memory.

## 10. Implementation phases

1. **tex-state substrate (complete).** Add `FragmentStore`, `FragmentId`,
   fragment-backed region resolution behind the aggregate facade, the
   session-lifetime position allocator split, and the layout-aware resolver
   with typed `Deleted`. Unit-test region disjointness, deletion, liveness,
   and allocator monotonicity across simulated fork discard.
2. **tex-lex cursor (complete).** Add the frozen `LayoutCursor` and per-line
   registration handoff to the root frame; prove per-token minting is
   unchanged (existing provenance benchmarks; the
   `provenance_performance.md` throughput matrix is the gate).
3. **tex-incr adoption (complete).** Line-expanded edit application, fragment minting,
   layout maintenance and generation stamping in `advance`; replace
   whole-document `rebind_root_editor_input` with cursor installation;
   route `rendered_source_location` (and the retained maps from
   `rendered_source_map.md`, if landed first) through the layout-aware
   resolver. Regression tests prove an edit-before-reused-page scenario
   resolves reused-page origins to *current* offsets and a
   convergence-adopted scratch page resolves at all.
4. **Pruning and measurement (complete).** Fragment byte pruning, retained-memory
   accounting, long-session capacity tests (keystroke storm, alternating
   insert/delete, pathological piece growth), and a
   `provenance_performance.md` update recording construction parity and
   read-path costs.

Each phase keeps Story/Gentle parity, fixture parity, and
`scripts/check-and-test.sh` green; phase 3 is where the three §1 defects
become regression tests.
