# Rendered Source Map

Status: authoritative contract for producer-bound per-page render source maps,
typed current/deleted/stale/cross-producer results, and retention accounting.

Builds on the edit-stable fragment-backed coordinates of
`edit_stable_source_coordinates.md` (umber2-hwtp): the map stores opaque
`OriginId`s, an edit never touches it, and all resolution goes through the
layout-aware resolver, which returns current-document offsets or a typed
`Deleted` result — never a stale offset. Source-coordinate caches are keyed
by `LayoutGeneration`, not revision.

## 1. Problem

`Session::rendered_source_location(page, event, unit, output, revision)`
(`crates/tex-incr/src/lib.rs`) currently answers one HTML click by:

1. deserializing the entire committed page artifact
   (`PageArtifact::from_bytes`);
2. re-running full positioned lowering (`lower_page`) to rebuild the event
   list;
3. indexing `events[event].sources[unit]` to obtain a
   `PositionedSourceRef { node_ordinal, source_index }`;
4. joining against the retained `render_origins` sidecar and resolving the
   `OriginId`.

The retained sidecar is a packed ragged table: one shared `u32` end offset per
artifact node and one shared flat `OriginId` buffer. Fresh shipout appends to
those two buffers directly, so artifact nodes do not allocate or retain
individual provenance vectors. Artifact clones remain O(1) through the two
shared buffers, and indexed lookup still returns the origin slice for one node.

Two structural defects:

- **O(page) work on every query.** Steps 1–2 repeat, per click or hover
  event, the traversal that `write_html` already performed at accept time.
  Nothing is retained between queries, so an interactive host that
  highlights on hover pays full page deserialization and lowering per mouse
  move.
- **Originally no producer/revision binding.** The `(page, event)` key is an index into the
  event list of whatever output is currently accepted. Nothing ties a DOM
  ordinal to the revision that rendered it; HTML from revision N queried
  after revision N+1 is accepted indexes into a different page set and
  returns a plausible but wrong location instead of a typed staleness error.
  A revision alone is insufficient because every independent session begins at
  revision 1.

Within one revision the index-based key itself is sound: the query path and
the HTML emitter run the same versioned lowering in the same binary, so
event order is deterministic by construction. The defects are the repeated
cost and the missing revision check, not the coordinate scheme.

## 2. Design

### 2.1 Lazily built per-page map, cached in the session

Keep the query API; make it cheap after first touch. On the first query
against a page, the session performs today's deserialize + lower pass
**once**, folds the result together with the `render_origins` sidecar into a
compact map, and retains it beside the accepted artifacts:

```rust
/// tex-incr: built on first query per page, dropped with the accepted
/// output it describes (next accept or rollback).
struct PageRenderMap {
    /// Prefix sums: event ordinal -> first unit slot. Non-text events
    /// contribute zero-width ranges, so `event_units[e+1] - event_units[e]`
    /// is the unit count of event `e`.
    event_units: Vec<u32>,   // 4 bytes per event
    origins: Vec<OriginId>,  // 4 bytes per unit slot; UNKNOWN for spaces
}
```

Every subsequent query on that page is a pure lookup:

```text
lookup(page, event, unit) =
    origins[event_units[event] + unit]      // O(1)
```

followed by the layout-aware resolver
(`edit_stable_source_coordinates.md` §5), whose own bookkeeping (fragment
and piece-table searches, the per-generation line-start index) is likewise
built lazily and memoized in Rust. No page bytes are touched after the first
query, and pages never queried cost nothing.
Origins outside the editor fragment space fall through to their engine source
descriptor; generated inputs use only their own optional logical path and
never a session-wide editor-root fallback.

The stored `OriginId`s name fragment content, so a cached map is immutable
for the lifetime of the accepted output it describes and is dropped with it
at the next accept or rollback. A unit whose source text was edited away
resolves to typed `Deleted`, never to an offset in an old snapshot.

Memory: four bytes per renderable source character plus one `u32` per event
boundary, for queried pages only, charged to live retained-output accounting
when built. The detached accepted output retains its point-in-time metrics;
the session getter includes maps constructed by subsequent queries.
The full positioned event list from the one-time lowering pass is _not_
retained — only the compact columns. Runs of consecutive direct origins
(ordinary text) may later be run-length encoded; that is an optimization,
not part of the contract.

### 2.2 Producer and revision binding

- Each incremental session mints one OS-random 128-bit rendered-output identity.
  Stamp its canonical lowercase hexadecimal form (`data-umber-output`) and the
  accepted revision (`data-umber-revision`) next to `data-umber-page`.
- The query API takes both values. It checks output identity first: a foreign
  session returns typed `output-mismatch` before page/event lookup. Only a
  matching producer proceeds to the revision check.
- The query API takes the revision the host read from the DOM. A mismatch
  with the currently accepted revision returns a typed `stale-revision`
  result instead of `None` or a silently wrong location.

This converts the implicit "HTML matches accepted output" contract into a
checked one and gives editors a precise invalidation signal.

Staleness has two independent axes, and this handshake covers only the
first:

- **Event ordinals** bind to the accepted revision whose lowering emitted
  them; a revision mismatch is a structural error (the DOM describes a
  different page set) and is rejected as above.
- **Source coordinates** bind to the `LayoutGeneration` current at
  resolution time. Because the boundary is query-only, the host never holds
  a coordinate snapshot: every answer is resolved against the live layout at
  call time, so results are point-in-time truths the host displays and
  discards rather than state it must invalidate.

### 2.3 WASM boundary: queries only

One surface on `CompilerSession`:

```ts
renderedSourceLocation(page, event, unit, output, revision):
    | { kind: "current"; path; start; end; line; column }
    | { kind: "deleted"; mintedRevision: number }
    | { kind: "stale-revision"; accepted: number }
    | { kind: "output-mismatch"; acceptedOutput: string }
    | null                                   // unsourced or out of range
```

No mapping tables cross the boundary. Per-query cost after the first touch
of a page is one WASM call, an O(1) table lookup, and a memoized resolve —
comfortably within hover-tracking rates; the resolver's cold cost
(~45 us today) is paid once per generation for the line index, not per
query. All bookkeeping needed to make queries fast lives in Rust and is
constructed on demand.

### 2.4 DOM-side ownership

Rust owns `event/unit -> source`. The authored JS package owns
`DOM point -> (page, event, unit)`: a small `source-map.js` companion to
`html-preview.js` converts `caretPositionFromPoint` text offsets into unit
indexes using `data-umber-codes`, `data-umber-text-kind`, and the mapping
retained by the application's typed resource response. Encoding entries may map one
code to multiple scalars; direct OpenType runs instead carry full Unicode
scalar values and are counted through their actual UTF-16 length. This offset
arithmetic must live beside the encoding data, not
be re-derived by applications. This helper reads the HTML attributes plus the
accepted mapping; the runtime package exports no catalog or default encoding.
`renderedSourceKeyFromPoint` returns
`{ page, event, unit, output, revision }`; `renderedSourceLocationFromPoint` passes that
key through one `CompilerSession.renderedSourceLocation` call. Callers pass
either the accepted mapping or a map keyed by the HTML `data-umber-font` id.
Offsets are counted
in UTF-16 code units to match the DOM caret API, including encoding entries
that expand one TeX code to multiple Unicode scalars.

### 2.5 Reverse mapping (future)

The same cached per-page maps, indexed by `SourcePos` (stable fragment
space — not by resolved document offsets, which move on every edit), answer
the reverse query as another Rust-side lazy structure: an editor's
current-document range maps through the live layout to O(affected pieces)
fragment-space ranges, each binary-searched in per-page sorted indexes to
recover `(page, event, unit)`. This enables editor-to-preview sync (SyncTeX
"forward search") with exact per-character spans rather than SyncTeX's box
heuristics, exposed as another query (`sourceRenderedLocation(range)`), not
an export. Forward search must consider all pages, so its first use builds
the maps for every page; that pass is O(document) once per accepted output
and cached thereafter. Out of scope for the first implementation, but the
map layout must not preclude it (it does not).

## 3. Rejected alternatives

- **Bulk typed-array export of per-page span tables to JS.** Duplicates the
  mapping into host-held snapshots that go stale on every accepted edit and
  must be refetched, re-resolves whole pages that may receive one click, and
  widens the API surface. Queries are rare (clicks, hovers) and O(1) after
  the lazy map exists, so per-call FFI overhead is noise; keeping the state
  in Rust keeps one owner for staleness.
- **Eagerly emitting the map as a byproduct of the HTML pass.** Structurally
  attractive (same traversal writes ordinals and map), but charges 4 bytes
  per character for every page whether or not it is ever queried, and adds a
  provenance-shaped output to `tex-out`'s HTML path. Same-binary
  deterministic lowering plus the revision check (§2.2) already guarantee
  coordinate agreement; laziness wins. Revisit only if first-click latency
  on very large pages ever matters.
- **Embedding source offsets in HTML attributes.** Bloats the document by
  roughly an order of magnitude per character, and leaks absolute source
  paths into a shareable rendered artifact. The HTML stays free of source
  coordinates; the session answers the mapping.
- **Memoizing the full lowered page per query.** Retains complete positioned
  events (coordinates, fonts, glue) when only the two compact columns are
  needed; the map is the same information at a fraction of the retained
  bytes.

## 4. Invariants preserved

- Origins remain excluded from node semantic identity, state hashes, format
  images, artifact bytes, and artifact content identity; the map is
  display-only session state.
- Token delivery and provenance-arena allocation are untouched; the compact
  source contract in `source_spans_and_provenance.md` remains authoritative.
- Maps are dropped with the accepted output they describe (next accept or
  rollback) and are never queried across their revision boundary (checked,
  per §2.2).
- Maps store only opaque `OriginId`s; edit-stable resolution returns
  current-document offsets or typed `Deleted`/`Unknown`, never a stale or
  aliased offset.
- Resolution-derived bookkeeping (line indexes, layout searches) is keyed by
  `LayoutGeneration` and built lazily in Rust; nothing coordinate-bearing is
  exported to the host.
- Lazily built map memory is charged to retained-output accounting at
  construction.

## 5. Verification

Tests require one lowering pass for repeated queries, current-document offsets
for reused pages after an edit, typed deleted and stale-revision results, and
producer/session rejection before map lookup. Native, WASM, DOM, and browser
fixtures exercise the same query identity. Retention tests charge exact page-map
capacity to accepted `output_bytes`, charge the lazy layout line index to
`diagnostic_bytes`, and keep both caches outside snapshot capture.
