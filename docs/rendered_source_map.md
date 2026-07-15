# Rendered Source Map

Status: phases 1-4 implemented. Accepted HTML is revision-stamped and the
native `tex-incr`/`umber` query path uses a lazily built, session-cached
per-page render source map with typed current, deleted, and stale-revision
results. The WASM boundary exposes those typed results and the authored DOM
helper converts browser caret positions into revision-bound queries; no mapping
tables cross into JavaScript.

Builds on the edit-stable fragment-backed coordinates of
`edit_stable_source_coordinates.md` (umber2-hwtp): the map stores opaque
`OriginId`s, an edit never touches it, and all resolution goes through the
layout-aware resolver, which returns current-document offsets or a typed
`Deleted` result — never a stale offset. Source-coordinate caches are keyed
by `LayoutGeneration`, not revision.

## 1. Problem

`Session::rendered_source_location(page, event, unit)`
(`crates/tex-incr/src/lib.rs`) currently answers one HTML click by:

1. deserializing the entire committed page artifact
   (`PageArtifact::from_bytes`);
2. re-running full positioned lowering (`lower_page`) to rebuild the event
   list;
3. indexing `events[event].sources[unit]` to obtain a
   `PositionedSourceRef { node_ordinal, source_index }`;
4. joining against the retained `render_origins` sidecar and resolving the
   `OriginId`.

Two structural defects:

- **O(page) work on every query.** Steps 1–2 repeat, per click or hover
  event, the traversal that `write_html` already performed at accept time.
  Nothing is retained between queries, so an interactive host that
  highlights on hover pays full page deserialization and lowering per mouse
  move.
- **No revision binding.** The `(page, event)` key is an index into the
  event list of whatever output is currently accepted. Nothing ties a DOM
  ordinal to the revision that rendered it; HTML from revision N queried
  after revision N+1 is accepted indexes into a different page set and
  returns a plausible but wrong location instead of a typed staleness error.

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

The stored `OriginId`s name fragment content, so a cached map is immutable
for the lifetime of the accepted output it describes and is dropped with it
at the next accept or rollback. A unit whose source text was edited away
resolves to typed `Deleted`, never to an offset in an old snapshot.

Memory: four bytes per renderable source character plus one `u32` per event
boundary, for queried pages only, charged to live retained-output accounting
when built. The detached accepted output retains its point-in-time metrics;
the session getter includes maps constructed by subsequent queries.
The full positioned event list from the one-time lowering pass is *not*
retained — only the compact columns. Runs of consecutive direct origins
(ordinary text) may later be run-length encoded; that is an optimization,
not part of the contract.

### 2.2 Revision binding

- Stamp the page container in the emitted HTML with the accepted revision
  (`data-umber-revision`) next to the existing `data-umber-page`.
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
renderedSourceLocation(page, event, unit, revision):
    | { kind: "current"; path; start; end; line; column }
    | { kind: "deleted"; mintedRevision: number }
    | { kind: "stale-revision"; accepted: number }
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
indexes using `data-umber-codes` and the font encoding tables the package
already ships (`cm-fonts.js`). Encoding entries may map one code to multiple
scalars, so this offset arithmetic must live beside the encoding data, not
be re-derived by applications. This helper reads only attributes already in
the HTML; it needs no exported tables. `renderedSourceKeyFromPoint` returns
`{ page, event, unit, revision }`; `renderedSourceLocationFromPoint` passes that
key through one `CompilerSession.renderedSourceLocation` call. OT1 is the
default encoding, while callers rendering other supplied faces pass either one
encoding or a map keyed by the HTML `data-umber-font` id. Offsets are counted
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
- Token delivery and provenance-arena allocation are untouched; the source
  throughput matrix in `provenance_performance.md` is unaffected.
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

## 5. Implementation phases

1. **tex-out:** add the `data-umber-revision` stamp beside
   `data-umber-page`. Existing HTML byte output changes only by the one
   attribute.
2. **tex-incr / umber (implemented):** add the lazy `PageRenderMap` cache (build on first
   query per page from one deserialize + lower pass joined with
   `render_origins`; drop on accept/rollback), route
   `rendered_source_location` through it and the layout-aware resolver, and
   add the revision check with the typed `stale-revision` and `deleted`
   results. Regression tests: repeated queries on one page lower it exactly
   once; after an edit, a reused page's map resolves to current-document
   offsets; an edited-away unit reports `deleted`; a stale revision is
   rejected.
3. **umber-wasm + js (implemented):** extend the typed query result, add the authored
   `source-map.js` DOM helper and Node tests, extend the browser integration
   fixture with a click-to-source assertion.
4. **Docs and budgets (implemented):** update `source_spans_and_provenance.md` §6.3,
   `provenance_performance.md` rendered-source follow-up, and
   `persistent_compile_sessions.md`; verify
   `scripts/check-snapshot-budgets.sh` still meets retained-allocation
   budgets with the lazily built maps included in accounting. Live retention
   tests charge the exact page-map capacities to `output_bytes`, independently
   charge the lazy layout line index to `diagnostic_bytes`, and preserve the
   point-in-time accepted metrics. The 2026-07-15 snapshot gate met every
   latency and retained-allocation budget.
