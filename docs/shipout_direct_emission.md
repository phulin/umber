# Direct shipout emission (zero-IR redesign)

Status: implemented on `codex/shipout-direct-emission` (2026-07-14).

Ordinary fresh shipout no longer constructs `FrozenShipoutNode`, a
`PageNode` tree, or a fresh-path artifact decoder. A small compatibility
exception remains for box leader payloads because DVI repeats those subtrees;
only that localized payload is materialized for replay.

## Problem

`docs/dvi_artifact_fast_path.md` removed the whole-page `PageArtifact`
round-trip: fresh shipout now lowers one root child at a time into a typed
stream consumed by the canonical artifact encoder and the incremental DVI
compiler. What remains is the *per-child* intermediate representation. For
every direct root child, `stage_shipout` still:

1. materializes a `Vec<FrozenShipoutNode>` per node list — an owned snapshot
   whose only purpose is to break the borrow between reading
   `stores.nodes(list)` and mutating `stores` (whatsit effects, deferred-write
   expansion, math-list conversion, glue/metric reads);
2. lowers that snapshot into an owned recursive `PageNode` tree — one heap
   `Vec<PageNode>` per box/disc/insert/mark/leader payload, a `String` per
   mark control sequence and effect, a `font_char_metrics` store lookup per
   character, and `BTreeMap` font interning;
3. hands the tree to two independent recursive consumers:
   `V10ArtifactBuilder::push_node` re-walks it to serialize canonical bytes,
   and `DviPagePlanBuilder::push_node` re-walks it through
   `hlist_out`/`vlist_out` to compile DVI body bytes;
4. drops the tree.

The whole `PageNode` subtree is therefore built, walked twice, and freed per
root child. On top of that, the lowering loops re-create the `NodeList` view
and call `get(index)` per node instead of decoding the compact word stream
sequentially, and tex-out carries three parallel traversal implementations
(`hlist_out`/`vlist_out` over owned trees, `push_root_stream_child` for the
streamed root, `ship_streamed_box` over the incremental v10 decoder).

The 2026-07-14 Gentle profile attributes ~15% of inclusive samples to shipout;
exclusive slices are tex-exec shipout 4.16%, tex-out binary 4.30%, tex-out DVI
3.78%, plus a share of the tex-state `node_arena`/`stores` time that is view
churn and snapshot copying on behalf of shipout.

## Goal

Ship a page with **zero ordinary-path intermediate node materialization**: one sequential
decode of the compact arena word stream drives both byte products directly.
The only owned artifacts of shipout are the two things that must exist anyway:

- the canonical artifact bytes (durable currency; identity hash input), and
- the detached `DviPagePlan` body bytes (published output).

`FrozenShipoutNode`, the owned fresh-path `PageNode` tree, per-list `Vec`
allocations, mark/effect `String` clones, and per-char metric lookups are all
deleted from the hot path. Canonical bytes must remain byte-identical — page
content ids are durable identity and must not change.

## Architecture

### Traversal inversion

tex-out gains state-free scalar artifact and DVI emission contracts;
tex-exec drives them over the live arena. tex-out never sees `Universe`, node
ids, or store handles — it receives decoded scalars and borrowed slices,
preserving the state-boundary rule and the commit barrier.

```text
tex-out:
  V10NodeListWriter
    - scalar leaf writers
    - nested-list count backpatching
    - borrowed mark-token streaming

  DviPagePlanBuilder
    - scalar leaf events
    - explicit begin_box/end_box frame stack
    - localized owned-leader replay
```

The tex-exec driver feeds both contracts during the same arena traversal.
Canonical-only subtrees (disc, mark, insert, and adjust) are omitted from DVI
geometry. Durable v10 replay feeds its incremental decoder into the same
`DviPagePlanBuilder` scalar kernel as fresh shipout, while the owned
`PageNode`/`BoxNode` model remains for compatibility and tests.

### Phase A: normalize (the only `&mut Universe` pass)

The frozen-snapshot dance exists because traversal interleaves mutation. The
redesign discharges all mutation in one cheap preparatory pass so the main
traversal can hold a single long-lived `&Universe` borrow and decode
sequentially:

- **Effects.** Walk the page in document order and execute whatsit effects
  exactly as `lower_whatsit` does today (openout/closeout, deferred-write
  expansion via the input machinery), producing the complete
  `Vec<PageEffect>`. Anchor indices need no side table: pending pre-page
  effects occupy `0..k`, and every later pass counts anchoring whatsits in
  the identical document order (with the identical leader-payload suppression
  rule), so indices are assigned by counting.
- **Math.** Convert any `MathList` reaching shipout with
  `finish_math_list_node`, appending results as temporary frozen lists in the
  shipout epoch (released by the existing node mark on commit or rollback).
  Record substitutions in a small `(list, index) → NodeListId` overlay map.
- **Directions.** Resolve TeX--XeT segments into per-list index permutations
  instead of cloned node vectors, only for lists that contain direction tags.
- **Fonts.** Intern first-use font resources into the plan/artifact font
  table using a direct-mapped `Vec<Option<u32>>` keyed by `FontId` raw index
  (ids remain `raw - 1` exactly as today), and pin per-font width tables for
  the emit pass.

Phase A walks node references to discover nested lists and rare mutable nodes.
Direction detection itself is a raw compact-tag predicate, so direction-free
lists avoid an extra decoded prescan. Its output — effects, substitution
overlay, and permutations — is a small `PageOverlay` sized by rare nodes only;
font resources are interned during Phase B.

### Phase B: fused emit (single `&Universe` traversal)

The arena DFS enforces the existing 4096-depth budget and drives both sinks.
The DVI side uses an explicit frame stack; canonical nested-list emission uses
scoped writer callbacks so child counts can be backpatched:

- **v10 sink.** Today's `Writer` plus `begin_box() -> CountPatch` /
  `end_box(patch)` backpatching, generalizing the mechanism
  `V10ArtifactBuilder` already uses for the root child count. Post-lowering
  child counts are unknowable up front (math expansion, vanishing
  direction/language nodes), so every list length is backpatched. Mark
  control-sequence names are resolved via `stores.resolve(symbol)` and
  written straight into the byte stream — no `String`. Wire bytes are
  identical to today's encoder output by construction; validation remains
  by-construction (first-use font interning, sequential anchors, byte-domain
  character checks at width lookup) plus the codec limits.
- **DVI sink.** `RootStreamState` generalizes to a frame stack — exactly the
  locals of `hlist_out`/`vlist_out` made explicit:
  `Frame { save_loc, base_line/left_edge/top_edge, cur_g, cur_glue, glue_sign,
  glue_order, glue_set }`. `push_root_stream_child` already proves the
  event-driven form of this state machine; the redesign completes it for
  nested boxes.

Divergent consumer needs, handled by the driver contract:

- **Leaders.** The v10 sink encodes the payload once, structurally attached
  to its glue node. DVI repetition uses a localized compatibility `PageNode`
  payload. Ordinary boxes never materialize, and the leader-payload
  deferred-stream suppression rule remains exact.
- **Empty-box byte parity.** `output_box_in_hlist/vlist` take a materially
  different byte path for childless boxes (no `synch_v`, no PUSH). "Empty"
  today means *lowers to empty*, not raw-empty. The source answers this with
  the raw length when nonzero-vs-zero is decisive (virtually always) and a
  tag-only scan when the list contains only potentially-vanishing tags.
- **Char runs.** The dominant node kind (45.5% of appends) uses the arena's
  existing `char_run` view: one same-font run event carries the code slice;
  the emit loop fetches the font width table once per run and runs a tight
  set_char/advance loop for DVI while the v10 sink encodes the run without
  per-node dispatch. Missing-glyph detection stays exact (absent TFM width
  fails the page before commit, as today).

### Commit and assembly: unchanged

Staging still completes entirely before `ShipoutTransaction::commit`; the
byte buffer becomes a `VerifiedArtifact` (domain hash unchanged), storage and
effect commit remain atomic, plans publish only after success, and the final
assembler keeps its font-definition relocation copy. `DviPagePlan` is output,
not IR — it stays.

## What gets deleted

- `FrozenShipoutNode` and both snapshot loops in `shipout.rs`.
- The fresh-path `lower_node`/`lower_frozen_node`/`lower_box`/
  `lower_node_list`/`lower_nodes` tree materialization (the enum-mapping
  tables move into `ArenaSource`'s event construction).
- `reorder_direction_segments` over owned nodes (becomes index permutation).
- The owned-tree recursion in `hlist_out`/`vlist_out` and the parallel
  `ship_streamed_box` decoder traversal — one generic kernel remains.
- `BTreeMap` font interning and per-char `font_char_metrics` calls.

## Measured effect

Per-page heap traffic drops from O(nodes) to O(depth + rare nodes). The
compact word stream is decoded once (plus a tag scan and leader replays)
instead of snapshot-copied, tree-materialized, and walked twice. Removed
exclusive work spans the tex-exec shipout slice (4.16%), part of the tex-out
binary/DVI slices that walk owned trees, and the shipout-attributed share of
`node_arena`/`stores` view churn.

The 200-run sampled profile reduced the complete `shipout_node` subtree from
15.22% to 11.14% of whole-run samples, a 26.8% relative reduction. The direct
staging body is 10.35%; fused emission accounts for 6.47% and normalization
2.58%. The capture preceded the raw direction-tag optimization, so it still
includes 1.05% of whole-run samples in the decoded direction prescan that was
subsequently removed.

Thermally conditioned `BOOB` followed by `BOOBOBBO` five times measured
111.963 to 100.468 ms/run by raw mean. Removing one isolated 258 ms baseline
outlier gives 104.277 to 99.315 ms/run, a 4.76% default-workload improvement.
A post-optimization checkpoint-enabled repetition measured 135.664 to
131.800 ms/run (2.85% improvement); medians were effectively equal at 105.909
and 105.865 ms/run. Every invocation produced 97 pages and exactly 263,424 DVI
bytes, and checkpoint-enabled runs produced 1,108 checkpoints.

## Parity and safety gates

- Canonical artifact bytes byte-identical on all fixtures (content ids are
  durable identity; any wire drift is a hard failure).
- DVI byte-identical on Story, Gentle, TRIP, e-TRIP; effect text and
  log/terminal interleaving identical.
- During bring-up, a test-only dual-run mode encodes pages through both the
  old lowering and the new driver and asserts byte equality of v10 bytes and
  plan bodies before the old path is deleted.
- Snapshot allocation and latency budgets re-run in the performance tier.

## Phasing

1. Introduce scalar writer contracts in tex-out; port durable v10 replay onto
   the unified DVI kernel. Gate: all DVI fixtures and corpora byte-identical.
2. Implement Phase A (`PageOverlay`) and the fused arena driver in tex-exec.
3. Switch fresh shipout to fused emission, delete `FrozenShipoutNode` and the
   fresh-path owned lowering; keep `PageArtifact::from_bytes` as the decode
   boundary.
4. Add `char_run` batching and per-font width-table caching.
5. Matched Gentle profile against the pre-change commit; accept only
   with the parity gates green and no snapshot-budget regression.

## Rejected alternatives

- **Two independent single-pass traversals** (v10+effects, then DVI): simpler
  driver, but decodes the arena twice, doubles glyph-width and glue reads,
  and still needs the anchor-counting discipline. Kept as a fallback if the
  fused driver's descend/replay contract proves too coupled in practice.
- **Making DVI the canonical artifact**: DVI lacks marks, inserts, effect
  semantics, and validation structure; the durable format stays semantic.
- **Hash-without-bytes** (content id via traversal hashing, skipping byte
  materialization): the bytes must exist anyway for durable storage and
  replay; nothing saved.
