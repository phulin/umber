# Node-region ownership

Status: normative replacement design for runtime node ownership and exact
paragraph restart, 2026-08-28.

## Scope and authority

This document defines the target ownership of runtime TeX node closures. It
specializes [Runtime storage lifetimes](runtime_storage_lifetimes.md),
[Aggregate checkpoint component contract](aggregate_checkpoint_contract.md),
and [Mode and page checkpoint ownership](mode_page_checkpoint_ownership.md).
Where an older document implies that a raw list coordinate owns storage, that
TeX box copy may alias an immutable node row, or that page-material liveness is
computed through dependency counts, this document takes precedence.

The design preserves the fixed-chunk work already completed for
`ChunkPool<Node>`, `ForkArena<Node, Lane>`, stable generation-checked chunk
keys, sealed payload marks, composable summaries, and atomic chunk-suffix
settlement. Lists now use direct `ListRoot { head, tail, length }` coordinates
over packed logical blocks carved from the same physical pool pages. There is
no list-descriptor lane or owner-local range lookup.

The following are explicitly rejected:

- `PageMaterialBatch` dependency graphs and transitive batch counts;
- `PageMaterialRoot`, `Rc`-backed page roots, or another clonable top-level
  node owner;
- root registries, root censuses, reachability scans, and per-checkpoint,
  per-list, or per-node reference counts;
- arbitrary node coordinates from one reclaimable region stored inside
  another;
- hidden promotion copies, first-write copies, and closure relocation on the
  ordinary list path;
- compaction, forwarding coordinates, or a third accepted/candidate
  generation; and
- changing the restart boundary to make reclamation easier.

The generic `SealedBatch<Lane>` already used to transfer one freshly built
typed lane is an operation result, not a page-liveness batch. It may remain
move-only. It must not acquire dependencies, reference counts, checkpoint
leases, or independent long-term ownership.

## Production cutover

As of 2026-08-28, ordinary execution uses these owners rather than retaining
the design as an unwired substrate:

- default and user-output completion wait until box255, detached output, and
  every live mode-list coordinate have settled, then rotate the complete
  four-root `PageBuilderState` owner;
- a suffix opened after box255 packaging moves the next-page contribution
  envelope when every nested child is suffix-local; an interleaved prefix
  child takes one exact recursive-copy fallback during the same bounded root
  traversal;
- an uncheckpointed old page region retires immediately, while a region with
  retained paragraph boundaries stays in the contiguous history interval;
- ordinary `\setbox` construction seals and rebrands its suffix into the box
  owner without relocating payload; `\box`/`\unhbox` use a rollbackable
  command-operation transfer loan, and `\copy`/`\unhcopy` remain the explicit
  recursive-copy operations; and
- PDF form creation consumes the same move-only construction envelope after
  taking its source box, so a unique source does not make a page-to-form copy.

The former `CompatibilityClosureBuildReceipt` retain seam is deleted. Lifecycle
counters distinguish envelope movement, bounded rebrand scans, TeX copies,
history-preservation copies, structural page-to-durable copies, held-over
fallback copies, region starts/retention/drops, and cross-region rejection.

## TeX82 ownership baseline

TeX82 implements its own fixed-address heap inside the global `mem` array.
`get_avail` and `free_avail` manage the one-word free stack, while `get_node`
and `free_node` manage the circular free-block list. See the allocator in
[`tex.web`](../third_party/texlive-source/src/texk/web2c/tex.web), lines
2612--2789.

Ordinary node lists are uniquely owned linked closures:

- `\box` takes the register pointer and clears the register at lines
  20949--20952;
- `\unhbox` links the child list into the destination, clears the register,
  and frees only the box wrapper at lines 21333--21350;
- `\copy` and `\unhcopy` call recursive `copy_node_list`, whose implementation
  at lines 3962--4043 duplicates nested box, insertion, leader, ligature,
  discretionary, adjustment, and math-list closures; and
- `flush_node_list`, at lines 3889--3954, recursively destroys a sole-owned
  closure. Glue specifications and token lists are the deliberate exceptions:
  they use the reference counts defined at lines 3898--3915 and incremented by
  `copy_node_list`.

Page construction likewise relinks owned nodes. Held-over insertions are
moved back to the contribution list before default shipout, and `ship_out`
recursively flushes the shipped box after emitting it; see lines 19895--19913
and 12681--12720.

Umber keeps those semantic move-versus-copy rules. Fixed chunks modernize the
allocator and allow whole-suffix settlement. Exact incremental checkpoints add
historical owners that TeX82 does not have. A semantic move must therefore copy
only when moving the live closure would invalidate one of those historical
owners.

## Restart eligibility is unchanged

There are exactly two retained restart kinds:

1. one `JobStart` bootstrap produced before job execution; and
2. a root-main-file outer paragraph end, after horizontal depth returns to the
   sole outer vertical mode, at TeX group level zero, with command delivery,
   scanner, expansion, resource, diagnostic, alignment, mode, and operation
   state quiescent.

An included-file paragraph, a paragraph inside a group, a live alignment, a
nested mode, shipout completion, and arbitrary page boundaries remain
ineligible. The eligibility receipt must still prove the complete barrier
before a node-region checkpoint can be sealed. This design does not add or
remove boundaries and never replays from a page boundary instead of the
selected paragraph boundary.

## Coarse owners

`NodeRegion<Lane>` is an exclusive, move-only owner of a self-contained node
closure domain. It owns the packed logical-block envelopes allocated to that
domain, its stable `NodeRegionId`, and the private roots needed to resolve its
lists. It is neither `Clone` nor `Copy` and has no shared-owner constructor.

`PageRegion` is the page-building specialization. One exists for each period
from the start of page building through the corresponding shipout. It owns:

- the fixed-chunk page-node blocks allocated during that period;
- the contribution, current-page, page-discard, and split-discard roots;
- PageBuilder scalar state and its reversible operation journal; and
- the contiguous interval of retained boundary rows published while that
  region was current.

Multiple paragraph checkpoints in one page share this one backing region. A
checkpoint does not copy nodes, descriptors, roots, or a region owner. Its
private row inside checkpoint history stores:

```text
PageRegionCheckpoint
  region_id
  contribution_root
  current_page_root
  page_discard_root
  split_discard_root
  sealed_payload_position
  page_builder_scalar_state
  page_builder_journal_position
```

Payload tails are sealed before publication. Later execution only appends and
changes current roots through the reversible journal, so an earlier row
remains exact.

`DurableNodeRegion` is the corresponding exclusive owner for a box register,
PDF form, or another durable node closure whose lifetime is independent of the
current page. A durable region is also self-contained. It may refer to
session- or generation-owned immutable non-node values such as fonts, stored
token lists, and glue specifications through their existing typed contracts,
but it cannot contain a node-list coordinate into another reclaimable node
region.

## Raw coordinates are borrowed capabilities

An `ArenaRange`, `ArenaListId`, `PageListId`, or future raw list coordinate is
not ownership. Production code may use it only while statically admitted
through the matching `NodeRegion` owner.

The Rust boundary must enforce these rules:

- a raw coordinate constructor remains private to `tex-state`;
- resolving a coordinate requires `&NodeRegion` or a branded admitted region
  borrow and returns a view whose lifetime cannot outlive that borrow;
- a production top-level state carrier cannot store a raw coordinate as if it
  were an owning root;
- PageBuilder raw roots live inside `PageRegion`, and checkpoint raw roots live
  inside checkpoint rows owned by the corresponding history `PageRegion`;
- a box register or PDF form stores a move-only `OwnedNodeClosure`, not an
  unaccompanied `DurableListId`;
- a journal entry either owns an `OwnedNodeClosure` or stores an owner-relative
  coordinate under the same enclosing region owner; and
- detached formats, memos, artifacts, continuations, process messages, and
  thread messages contain no runtime coordinate.

One suitable API shape is:

```rust
struct NodeRegion<Lane> { /* exclusive chunk envelopes and roots */ }
struct OwnedNodeClosure<Lane> { region: NodeRegion<Lane>, root: RegionRoot<Lane> }
struct RegionList<'region, Lane> { /* borrowed coordinate capability */ }
struct PageRegionCheckpointKey { region: NodeRegionId, boundary: BoundarySerial }
```

The exact names may change, but the ownership shape may not. A compile-fail
gate must reject placing `ArenaListId`, `PageListId`, or `RegionList` in a
production top-level owner without the matching region.

## Ordinary construction and list processing

Ordinary list processing is packed-block movement plus append-only output:

- a uniquely owned whole chain is represented by move-only `UniqueArenaList`
  or `UniquePageList` authority and is spliced by consuming that authority;
- the right-head predecessor is write-once, so a copyable shared root can
  never silently mutate retained topology;
- a genuinely generated or rewritten node is appended exactly once;
- a shared slice uses an explicitly named counted-copy path until its semantic
  owner is migrated to the move-only handoff;
- page and mode accumulation, saved-discard and unbox insertion, alignment row
  and display completion, math lowering, migrated line material, and the plain
  post-line path consume move-only whole roots at their semantic handoffs;
- packing, line breaking, paragraph post-processing, alignment setting, math
  lowering, and page breaking borrow short-lived `ArenaListView` or
  `NodeCursor` views; long forward consumers use the callback traversal that
  follows the sole predecessor chain once and retains its continuation on the
  Rust stack, mutation-interleaved operation consumers retain only a
  coordinate-valued chunk continuation and end each node borrow before an
  append, compatibility iterators retain one admitted owner-relative cursor
  within each packed block, and genuinely positional semantic reads remain
  explicit;
- paragraph breakpoint analysis settles discardable-run successor positions
  and prefix widths during that same forward callback; diagnostic breakpoint
  coordinates perform positional reads only when paragraph tracing is
  requested; and
- source identities compose from packed-block summaries without a descriptor
  lookup.

TeX82 §§914--918 constructs an automatic discretionary's pre-break,
post-break, and replacement closures before linking the discretionary into the
reconstituted main list. Post-line hyphenation therefore seals the preceding
main-list segment before publishing those child closures, then resumes a fresh
main-list segment and consumes the unique segments into one direct chain. An active main-list
builder never becomes a second owner around nested child publication, and the
reconstituted word's generated nodes are still appended exactly once.

There is no ordinary node closure copy, source-node republish, or ownership
scan. Active paragraph, math, alignment, and box builders remain move-only.
Their operation marks may name partial tails for local failure, while retained
checkpoint marks may name only sealed whole-chunk boundaries.

Copying or a closure scan is permitted only at an explicit semantic lifetime
transition:

- TeX `\copy` or `\unhcopy`;
- preservation of a historical owner across an otherwise consuming TeX move;
- evacuation of the exact held-over closure into a new page region;
- copying a nested closure whose source must remain independently owned; or
- cold handle-free format or continuation materialization.

The copy should be fused with the traversal already required by TeX copy,
page-break held-over extraction, packaging, or cold materialization. A
separate promotion scan is forbidden.

Paragraph transforms that must copy a shared source coalesce adjacent
unchanged nodes into one semantic range before entering the counted-copy
boundary. They do not resolve and copy a separate one-node slice for every
ordinary source node.

## Exact edit fork

Selecting a checkpoint inside page region `R` creates exactly this state:

```text
unchanged accepted page regions
selected R prefix
detached accepted suffix of R + later accepted page regions
private current suffix of R + new candidate page regions
```

The region and checkpoint owners move into one transaction. The selected
region forks at its sealed payload-and-descriptor boundary. Its arena state is
the existing two-lineage state:

```text
Accepted
Forked { prefix, detached_prior, current }
```

There is no independently cloneable accepted tail. Page regions after `R`
detach wholesale into the same accepted-suffix owner; they are not visited to
copy roots or payload. Candidate execution appends only to the private current
suffix of `R` until shipout, then creates new candidate `PageRegion` owners.

Rejection performs this order:

1. validate the return destination and every root/journal coordinate;
2. undo candidate PageBuilder root and scalar changes;
3. drop candidate page regions created after the selected page;
4. truncate the selected region's current payload and descriptor suffix
   atomically;
5. reattach its detached accepted suffix and later accepted page regions;
6. forward-redo the saved accepted PageBuilder journal; and
7. return the settled exclusive region/history authority to accepted state.

Acceptance performs this order after complete validation:

1. remove checkpoint rows and journal roots which name the superseded accepted
   suffix;
2. drop later accepted page regions as whole owners;
3. prune the selected region's detached accepted payload and descriptor chunks
   atomically;
4. promote its current suffix and candidate page regions into accepted
   history; and
5. publish boundary/accounting metadata only after every aggregate owner has
   settled.

Unwind, cancellation, and explicit rejection use the same rejection path.
Exactly the accepted lineage and one private current lineage may exist. No
compaction, relocation, copied prefix, sibling bank, or third generation is
allowed.

## Shipout and page succession

After shipout, page construction starts a new `PageRegion`. Before the old
region can be dropped, the page-breaking traversal handles its three possible
escapes in this order:

1. lower the selected page into handle-free output;
2. evacuate the exact held-over closure into the new page region; and
3. retain or drop the old region according to checkpoint history.

Handle-free DVI/PDF page plans, artifact bytes, effects, and detached source
recipes own no runtime node. Once output construction has borrowed the runtime
closure and committed its detached result, output does not keep the old page
region alive.

Held-over material must become self-contained in the new page region. Output
opens a sealed construction boundary before it builds the next-page material.
When the old region has no historical checkpoint owner and every live
PageBuilder root, including nested children, belongs to that suffix, commit
consumes the predecessor and adopts the suffix under the same physical arena
identity. The predecessor prefix drops, the semantic region generation
advances so old handles become stale, and the successor retains the suffix's
sealed chunks plus its mutable partial tail. Adoption changes only one chunk
index per retained payload or descriptor chunk, so ownership transfer is O(1)
per adopted chunk and copies or rebrands no payload.

Prepare only proves that ownership shape and records the build mark; cancel
re-arms the same suffix without changing chunks or counters. If a checkpoint
keeps the predecessor live, a self-contained sealed successor suffix is shared
at arena granularity. The source and destination keep separate chunk lists and
one of two fixed lineage-position slots per shared chunk. The suffix is an
immutable prefix in the destination, and both regions allocate subsequent
values only in private tails. Direct child-position floors accumulated during
publication prove suffix closure from chunk metadata; succession performs no
node-tree traversal, payload clone, rebrand, census, or per-node reference
count.

Dropping a lineage visits its chunk keys only. It clears that lineage's slot
and returns an exclusive chunk immediately; a shared chunk remains live until
the other lineage drops it, at which point payload destructors run and the
chunk incarnation advances. Reuse therefore rejects stale keys, and retiring
prior before current or current before prior releases exactly the unreachable
chunks. A third lineage is rejected by the bounded metadata. A successor root
which crosses the build boundary or was not published through the checked
dependency-folding seam retains the explicit structural-copy fallback. The
shipped page, complete old region, and unrelated checkpoint material are never
copied into the new owner. Page-to-durable box255 publication remains a
separate lifetime boundary and retains its explicitly counted copy while page
and durable owners coexist.

An old page region is retained precisely when at least one retained restart
row belongs to its contiguous boundary interval. If a page contains no
retained boundary, output has detached, and held-over material has evacuated,
the region drops immediately. When pruning removes the last boundary in an old
region's interval, history drops the entire region. No list-by-list liveness
calculation participates.

## Checkpoint-history ownership and memory cost

Checkpoint history directly owns page regions in document order:

```text
AcceptedNodeHistory
  PageRegion A -> boundary rows 0..4
  PageRegion B -> boundary rows 4..11
  PageRegion C -> boundary rows 11..15
```

Each page/epoch interval is contiguous. Appending a checkpoint extends only
the current interval. Prefix pruning and accepted-suffix replacement remove
whole intervals when possible; the one selected page may settle a suffix at
its sealed checkpoint mark. There is no per-checkpoint or per-region reference
count and no ownership inferred from raw root occurrence.

Exact paragraph rollback necessarily retains the backing state for every
restartable paragraph. Consequently accepted-history node memory grows with
the node material of pages containing retained checkpoints. This design stores
each accepted node once per page region, not once per checkpoint. That cost is
irreducible without pruning restart boundaries, replaying from a coarser
boundary, or serializing retained state. None of those tradeoffs is authorized
here.

Logical retained-byte accounting follows the direct owner:

- each live `PageRegion` is charged once for its pool pages, live chunk
  envelopes, descriptors, journal storage, and reusable capacity;
- each checkpoint is charged only for its fixed boundary row and aggregate
  non-node metadata;
- each durable node region is charged once to its register, form, or history
  journal owner; and
- detached output is charged to its output owner and never to a page region.

Accounting counters are observations, not liveness authority. A count becoming
zero never causes runtime release; removing the owning history interval does.

## Boxes, forms, and nested closures

Box registers, PDF forms, leaders, discretionary children, insertions, and
other stored node closures follow TeX's move-versus-copy semantics.

| Semantic operation      | Node-region transition                                                                                                                                                                                                                             |
| ----------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Build a new box or form | Seal one exclusive region and move it into the destination owner.                                                                                                                                                                                  |
| `\box<n>`               | If no historical/save owner needs the source, transfer its region and clear the register. If history must preserve it, move the old region into that history owner, clear the register, and copy exactly its closure into the current destination. |
| `\unhbox<n>`            | Apply the same rule to the child closure; a unique source transfers, while a historically retained source is preserved and copied before the wrapper is discarded.                                                                                 |
| `\copy<n>`              | Deep-copy the exact recursive closure into a fresh exclusive region and leave the source region untouched.                                                                                                                                         |
| `\unhcopy<n>`           | Deep-copy the child closure into the destination region and leave the source box untouched.                                                                                                                                                        |
| Local assignment/save   | Move the old exclusive region into the save-journal entry before installing the new owner. Group restoration moves it back; a superseding global assignment drops the saved owner in TeX order.                                                    |
| Register/form overwrite | Drop the old region only after no save or retained checkpoint can restore it; otherwise move it into that history journal.                                                                                                                         |
| Nest a unique child     | Transfer or merge its whole self-contained chunk envelopes into the consuming parent region.                                                                                                                                                       |
| Nest a retained child   | Copy the exact child closure into the parent region and preserve the independent source owner.                                                                                                                                                     |

A copied closure retains TeX's selected immutable aliases: glue and stored
token values use their existing explicit shared owners. Node payload and node
topology are duplicated. No copied or moved result retains an arbitrary node
coordinate into its source region.

TeX82 §§1074 and 1077's `\setbox<n>=\lastbox` first removes the selected
tail box from the current mode list and only then stores that box in the
register. The page-region source-list rewrite therefore settles before the
destination durable-closure build mark opens. Opening the destination mark
earlier would put the rewritten live source descriptor in the destination
suffix and transfer it out of page ownership when the box is sealed.

The historical-preservation copy is not a hidden optimization accident. It is
the explicit cost of adding rollback history to a TeX operation that otherwise
consumes its sole owner. The counter family below distinguishes it from a TeX
`\copy` and from held-over evacuation.

## Ownership and drop order

Every transition validates its complete destination, owner generations, roots,
journal positions, and chunk positions before moving an owner. After that
preflight, the infallible drop order is:

1. remove or restore the top-level semantic roots;
2. settle PageBuilder and dense/save-journal entries that can name node owners;
3. retire child `OwnedNodeClosure` values and candidate active builders;
4. settle or remove canonical list descriptors;
5. release payload chunk envelopes; and
6. recycle empty pool slots with fresh generations.

For shipout, detached output is built first, held-over material is established
in the next page region second, and only then may the old page region drop. For
TeX assignment, the old box/form region moves into the save or checkpoint
journal before the current cell changes; restoration moves that owner back
before dropping the overwritten current value. For generation retirement,
suspended execution and active builders retire first, checkpoint rows and save
journals retire next, durable box/form owners and page-region history retire
after them, and the physical pool retires last. A runtime coordinate can
therefore never observe that its region or pool has already dropped.

## API invariants

The storage facade must make these properties structural:

1. `NodeRegion` and `OwnedNodeClosure` are move-only.
2. A region id includes a reuse generation; a stale id or coordinate cannot
   alias a recycled chunk slot.
3. Every node child coordinate resolves under the same region as its parent.
4. Payload and descriptor changes validate together and settle atomically.
5. A retained boundary can be created only from a consumed sealed boundary and
   the quiescent restart-eligibility receipt.
6. An operation mark cannot convert into a retained checkpoint mark.
7. An open active-list builder blocks checkpoint sealing, shipout transfer, and
   region drop.
8. Beginning a candidate consumes the one accepted region/history authority;
   neither side remains independently usable.
9. Every fallible validation precedes root removal or chunk ownership change.
10. Region destruction occurs only after all roots and journal entries that
    can resolve through it have been removed.
11. Production top-level node roots are owner-relative fields inside a region
    or move-only owner-plus-root aggregates, never naked copy-only coordinates.
12. Identity summaries, source range summaries, and memory counters move and
    drop in the same envelope as payload and descriptors.

The essential APIs are conceptually:

```text
seal_page_checkpoint(eligibility, page_region) -> PageRegionCheckpointKey
fork_page_region(checkpoint_key, history_owner) -> PageRegionFork
reject_page_region(PageRegionFork) -> AcceptedNodeHistory
accept_page_region(PageRegionFork) -> AcceptedNodeHistory
finish_shipout(PageRegion, held_over) -> (DetachedPage, PageRegion)
move_closure(OwnedNodeClosure) -> OwnedNodeClosure
copy_closure_into(&OwnedNodeClosure, &mut NodeRegion) -> RegionRoot
drop_region(NodeRegion)
```

The implementation may refine names and split validation from infallible
settlement, but it must not expose one-shot component settlement that can
leave PageBuilder roots and chunk ownership in different lineages.

## Required counters and gates

The existing arena counters remain authoritative:

- `new_semantic_nodes`;
- `source_nodes_copied`;
- `identity_nodes_hashed` and `identity_summaries_combined`;
- `chunks_sealed` and `unused_sealed_bytes`;
- `chunks_promoted`;
- `candidate_chunks_truncated`;
- `accepted_chunks_reattached`; and
- `obsolete_chunks_pruned`.

Add region-transition counters at the semantic facade:

- `page_regions_started`, `page_regions_retained`, and
  `page_regions_dropped`;
- `page_region_forks` and `later_page_regions_detached`;
- `held_over_nodes_copied` and `held_over_envelopes_moved`;
- `tex_copy_nodes_copied`;
- `history_preservation_nodes_copied`;
- `nested_closure_nodes_copied`;
- `node_closure_scan_nodes`; and
- `cross_region_node_reference_rejections`.

Counters must be demand-free scalar updates. They cannot register roots or
control reclamation.

The deterministic performance gates are:

- publishing 1 versus 4,096 paragraph checkpoints in one page allocates and
  copies no node payload at capture, keeps one page region, and increases only
  fixed boundary rows plus sealed-tail slack;
- an ordinary paragraph/alignment/math/page pipeline reports
  `source_nodes_copied == 0` and preserves exact addresses for unchanged
  ranges;
- early and late edits scan or release only the selected detached suffix and
  later region owners, never the unchanged prefix;
- rejection returns exact roots, chunk generations, identities, counters, and
  region order; acceptance makes superseded coordinates stale;
- repeated pages without retained boundaries reach a stable pool high-water
  plateau after output and held-over evacuation;
- a unique `\box`/`\unhbox` transfer performs zero node copies, an explicit
  `\copy`/`\unhcopy` copies exactly the selected closure, and a move protected
  by history increments only `history_preservation_nodes_copied`;
- held-over work is proportional to the evacuated closure and independent of
  shipped-page and retained-history size; and
- retained bytes equal each live page/durable region once plus fixed boundary
  metadata, with no multiplier for boundaries in the same page.

Elapsed-time ratios remain profiling diagnostics. Zero-copy, exact counter,
stable-prefix, stale-coordinate, and allocation-count assertions are routine
deterministic gates.

## Required tests

The implementation must add or adapt these exact test families:

1. `restart_eligibility_is_unchanged` proves only `JobStart` and quiescent
   root-main-file group-level-zero outer paragraph ends can produce a region
   checkpoint.
2. `paragraph_checkpoints_share_one_page_region` publishes many boundaries in
   one page and proves one payload/descriptor owner and no node copy.
3. `checkpoint_restores_all_four_page_roots` mutates every PageBuilder list,
   scalar family, insertion/mark state, and journal after a selected boundary,
   then restores the exact state.
4. `page_region_reject_reattaches_exact_accepted_suffix` covers candidate
   boundaries, shipout-created regions, and suspension before rejection.
5. `page_region_accept_drops_superseded_suffix_wholesale` proves old suffix and
   later-page coordinates are stale while unchanged-prefix addresses remain
   stable.
6. `alternating_early_and_late_edits_keep_two_lineages` repeats accepted and
   rejected edits for thousands of cycles and observes one generation at rest,
   two during edit, and no third region lineage.
7. `shipout_without_checkpoint_drops_page_region` proves a bounded physical
   plateau across hundreds of pages.
8. `shipout_checkpoint_retains_then_prune_drops_whole_region` proves handle-free
   output owns no runtime nodes and pruning the last boundary releases the old
   region.
9. `held_over_material_is_self_contained_in_next_region` proves exact
   evacuation, no cross-region child, and no shipped-prefix copy.
10. `tex_move_transfers_unique_region` and
    `tex_copy_deep_copies_recursive_closure` cover boxes, leaders,
    discretionaries, insertions, marks, glue, and token aliases.
11. `historical_owner_turns_move_into_bounded_copy` proves the old region moves
    into history, the register clears, the destination is independent, and
    rollback restores the original owner.
12. `nested_closure_transfer_or_copy_is_region_local` covers both unique and
    retained nested children.
13. `payload_and_descriptor_settlement_is_atomic` injects every validation
    failure and proves no partial root/chunk change.
14. `stale_region_and_chunk_generations_are_rejected` covers reuse without ABA
    aliasing.
15. a compile-fail test such as `raw_page_list_root_escape_forbidden.rs` proves
    a raw list coordinate or borrowed `RegionList` cannot become a production
    top-level owner or outlive its `NodeRegion` borrow.

Semantic output, DVI/PDF parity, format round trips, source identity, and
aggregate checkpoint tests remain the acceptance authority; topology-only
tests may supplement but never replace them.

## Migration order

The migration preserves the completed alignment, math, borrowed-range, and
range-summary work. It changes ownership around those paths rather than
reintroducing `Vec<Node>` or rebuilding their transforms.

1. Introduce move-only `NodeRegion`, `PageRegion`, `OwnedNodeClosure`, region
   ids, and borrowed coordinate capabilities above the existing
   `ChunkPool`/`ForkArena` substrate. Add the raw-root compile-fail boundary.
2. Put the PageBuilder's four roots, scalar mark, and journal under the current
   `PageRegion`. Publish checkpoint rows owner-relative to that region.
3. Make accepted checkpoint history own contiguous page-region intervals and
   implement selected-region suffix plus later-region detach/reattach/drop.
4. Start a new region after shipout, lower handle-free output, and fuse
   held-over evacuation with the existing page-break traversal. Prove the
   uncheckpointed-page plateau.
5. Convert box registers and PDF forms to move-only durable region owners.
   Implement unique moves, explicit TeX copies, historical-preservation copies,
   save-journal movement, and region-local nested closures.
6. Remove production APIs that store naked `PageListId`/`DurableListId` roots,
   conservative monotonic page/durable bounds used as ownership substitutes,
   and any remaining arbitrary cross-region node reference.
7. Delete or permanently reject `PageMaterialBatch`, `PageMaterialRoot`, batch
   dependencies/counts, root registries/censuses, and compatibility
   materialization seams. Keep generic operation-local `SealedBatch` promotion
   only if it remains exclusive and dependency-free.
8. Add the counters, allocation gates, exact lifecycle tests, full semantic
   suite, and quality gates before declaring the node lifetime migration
   complete.

No step changes restart eligibility, adds replay, or creates a temporary second
production node topology.
