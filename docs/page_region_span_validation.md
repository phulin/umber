# Page-region span validation

Page material has two deliberately different coordinate forms:

- `PageListId` is the compact transport coordinate stored in recursive nodes
  and at serialized, durable, pure-kernel, and cross-owner boundaries. It may
  cross a lifecycle boundary, so a receiving owner must validate it.
- `PageListSpan` is the checked traversal capability minted from one
  owner-admitted `PageListId` by the current `PageMaterialArena`. It records
  the direct-root owner proof established when construction or root admission
  fully validated the chunk chain. Its fields and constructor are private. Live
  page, mode, alignment-replay, and discretionary roots carry this type while
  they remain under that same page owner. Ordinary traversal, composition,
  slicing, and move-only suffix append then do not repeat raw-root admission.

The span is not an owner, cache, copied node list, or public unchecked handle.
It contains only the original direct root and its validated owner stamp.
Payload remains in the one `NodePool`, and every block borrow still passes
through the pool's generation and arena-owner check. A span whose block was
truncated, transferred, or retired therefore fails closed instead of resolving
replacement storage.

`UniquePageList` is the separate move-only construction result. Consuming it
permits one write to the whole right chain's previously unset head predecessor.
This is the O(1) production append seam. Publishing it as a copyable
`PageListId` permanently gives up that authority; shared roots and slices must
then use an explicitly counted copy path.

## Validation boundaries

| Boundary                 | Input                                                                                             | Validation and result                                                                                                                                                                                                                                                                                                                  |
| ------------------------ | ------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Construction             | Newly published packed blocks                                                                     | Node publication rejects child coordinates outside the current arena. Builder finalization validates the direct chain once and returns either move-only `UniquePageList` authority or a published `PageListSpan`.                                                                                                                      |
| Existing-owner traversal | Opaque `PageListId` already accepted by the current semantic owner                                | Span admission checks the current arena, direct head/tail generations and bounds, and the complete predecessor chain once. `PageListSpan` then carries the owner proof through cursor construction, indexed reads, iteration, slicing, and O(1) unique-suffix append.                                                                  |
| New root admission       | `PageListId` entering from a raw boundary or a `PageListSpan` already owned by the current region | The receiver validates the complete raw list once and stores the resulting span. Owner-produced spans transfer directly between live fields. No parallel region-id admission cache or second owner is retained.                                                                                                                        |
| Operation rollback       | Operation mark plus active builder                                                                | The active builder can restore only its exclusive tail-used cursor or abandon newly allocated logical blocks. It never reopens a published block, and generation increments prevent ABA when an abandoned block slot is reused.                                                                                                        |
| Checkpoint candidate     | Private checkpoint row                                                                            | Page candidate rows carry the four root spans only while the row remains inside their `PageRegion`; begin validates the page-region key, node checkpoint mark, and builder mark before detaching any later owner. Mode named checkpoints remain rootless. No span is detached from its region or restored into a different generation. |
| Page successor           | Complete `PageBuilder` root set and optional move-only closure build                              | Successor preparation fully validates all four roots in the old region before transfer or structural copy. The destination constructs new checked coordinates under its fresh `NodeRegionId`; no old span crosses succession. Commit either retains the entire old region for checkpoint history or retires it wholesale.              |
| Shipout                  | Prepared page roots owned by the current page region                                              | Shipout traversal admits each root before staging. Successful publication commits the prepared successor; failed staging rolls back only speculative output work and retains the old root owner. A traversal span never becomes an artifact coordinate.                                                                                |
| Retirement               | Exclusive `PageMaterialRegion` or durable closure owner                                           | Retirement first checks that there is no active builder or batch and validates all live chunks. Releasing chunks increments their generations, so stale ids and stale spans cannot alias later occupants.                                                                                                                              |

## Negative controls

The raw-coordinate APIs remain the boundary tests for forged structure,
foreign pools, foreign arenas, stale chunks, truncated ranges, and block spans
crossing a region frontier. Checked-span construction must
reject every such input. Checked traversal has separate controls showing that a
span cannot be used with a foreign arena and that truncation invalidates its
generation before any retained root is published.

## Live-root lifetime audit

| Root field                                                                                     | Span proof lifetime                                                                                                                                                                                                                         |
| ---------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `PageBuilderState` contribution, current-page, page-discard, and split-discard roots           | `PageListSpan` under the enclosing `PageRegion`; page inverse rows, payload projections, and private checkpoint rows remain inside that same owner and settle with its arena marks.                                                         |
| `ModeList.nodes`, incomplete-fraction numerator, display equation, and display prototype lists | `PageListSpan` under the current page region; operation rollback restores the same-region span projection. Retained named mode checkpoints are rootless, so no proof crosses restore into another page generation.                          |
| Active replay-alignment row migrations and active discretionary parts                          | `PageListSpan` while the main-control frame remains attached to the current page owner. Completion converts only at the recursive-node or pure-output boundary; abort drops the frame before page succession.                               |
| `PageNodeCarrier` and page succession source projections                                       | The carrier keeps a span while it borrows the current owner. Cross-region succession transports an opaque id or exact closure and explicitly admits the destination coordinate after move/rebrand/copy; the source span never crosses over. |

Recursive `Node<PageListId>` children, durable closures, serialized formats,
memo/artifact payloads, shipout scratch, and pure typesetting data-transfer
objects remain opaque coordinates. Synchronous packing, alignment, output,
and diagnostic projections may borrow a raw id, but they do not become stored
semantic roots. These are deliberate trust or lifetime crossings, not missed
span storage. A rollback/checkpoint projection carries a span only when its
arena mark and root projection are settled atomically by the same owner.

## Performance and accounting contract

Page and mode append-heavy owners retain checked left roots and consume fresh
`UniquePageList` suffixes. Tail and append are O(1); reverse traversal is
O(nodes inspected + actual block crossings), with no descriptor lookup or
binary search. Forward sequential iteration resolves its starting position,
then advances the admitted owner-relative cursor directly inside each packed
block and re-enters positional resolution only at an actual block boundary.
This is traversal state, not a successor cache: persistent checkpoint forks
continue to use the sole predecessor topology. Genuinely indexed reads remain
the explicit `owned_node` path. Shared and sliced transforms increment `source_nodes_copied`
exactly; they are not permitted to hide a whole-list copy behind generic
concatenation. The counted fallback collects one reverse traversal and replays
it once, so it visits and hashes every copied node exactly once and never
revisits an accumulated prefix. Demand-enabled copies maintain summaries on
their destination blocks before a generated suffix can extend the tail.
Physical pages contain sixteen logical blocks, so many small lists share one
bump allocation. The pool reports payload capacity and block metadata
separately, while `unused_sealed_bytes` records exact tail slack.

The parameterized direct-root gates exercise 1, 64, and 4,096 nodes. Unique
suffix construction allocates exactly one logical block per deliberately
single-node suffix and copies zero nodes at every size. The explicit shared
fallback reports exactly 1, 64, and 4,096 copied nodes respectively. These
deterministic counters, rather than a wall-clock threshold, prove that neither
path recopies an accumulated prefix.
