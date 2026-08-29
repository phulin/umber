# Page-region span validation

Page material has two deliberately different coordinate forms:

- `PageListId` is the compact transport coordinate stored in recursive nodes
  and at serialized, durable, pure-kernel, and cross-owner boundaries. It may
  cross a lifecycle boundary, so a receiving owner must validate it.
- `PageListSpan` is the checked traversal capability minted from one
  owner-admitted `PageListId` by the current `PageMaterialArena`. It records
  the descriptor endpoints established when construction or root admission
  fully validated the list. Its fields and constructor are private. Live
  page, mode, alignment-replay, and discretionary roots carry this type while
  they remain under that same page owner. Ordinary traversal, composition,
  slicing, and retained-range append then do not repeat raw-root admission or
  reconstruct endpoints through `descriptor_entry_at`.

The span is not an owner, cache, copied node list, or public unchecked handle.
It contains only the original coordinate and its validated descriptor
position. Payload remains in the one `NodePool`, and every actual descriptor or
payload borrow still passes through the pool's generation and arena-owner
check. A span whose descriptor was truncated, transferred, or retired therefore
fails closed instead of resolving replacement storage.

## Validation boundaries

| Boundary                 | Input                                                                                             | Validation and result                                                                                                                                                                                                                                                                                                                        |
| ------------------------ | ------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Construction             | Newly published payload and descriptors                                                           | Node publication rejects child coordinates outside the current arena. Builder finalization validates the complete canonical descriptor record once and returns a `PageListSpan`.                                                                                                                                                             |
| Existing-owner traversal | Opaque `PageListId` already accepted by the current semantic owner                                | Span admission checks the current arena, resolves the first and last descriptor keys, and verifies both generation/used bounds. `PageListSpan` then carries those endpoints through cursor construction, indexed reads, iteration, full-list append, and subrange append. No list-wide descriptor/range walk or copied payload is performed. |
| New root admission       | `PageListId` entering from a raw boundary or a `PageListSpan` already owned by the current region | The receiver validates the complete raw list once and stores the resulting span. Owner-produced spans transfer directly between live fields. No parallel region-id admission cache or second owner is retained.                                                                                                                              |
| Operation rollback       | Operation mark plus active builder                                                                | The active builder can roll back only its own appended suffix. Page and mode journal projections carry spans from the same region, while roots and arena marks settle atomically. Source spans admitted before the builder opened remain valid; a span into the discarded suffix fails its descriptor generation/used-bound check.           |
| Checkpoint candidate     | Private checkpoint row                                                                            | Page candidate rows carry the four root spans only while the row remains inside their `PageRegion`; begin validates the page-region key, node checkpoint mark, and builder mark before detaching any later owner. Mode named checkpoints remain rootless. No span is detached from its region or restored into a different generation.       |
| Page successor           | Complete `PageBuilder` root set and optional move-only closure build                              | Successor preparation fully validates all four roots in the old region before transfer or structural copy. The destination constructs new checked coordinates under its fresh `NodeRegionId`; no old span crosses succession. Commit either retains the entire old region for checkpoint history or retires it wholesale.                    |
| Shipout                  | Prepared page roots owned by the current page region                                              | Shipout traversal admits each root before staging. Successful publication commits the prepared successor; failed staging rolls back only speculative output work and retains the old root owner. A traversal span never becomes an artifact coordinate.                                                                                      |
| Retirement               | Exclusive `PageMaterialRegion` or durable closure owner                                           | Retirement first checks that there is no active builder or batch and validates all live chunks. Releasing chunks increments their generations, so stale ids and stale spans cannot alias later occupants.                                                                                                                                    |

## Negative controls

The raw-coordinate APIs remain the boundary tests for forged structure,
foreign pools, foreign arenas, stale chunks, truncated ranges, and descriptor
or payload spans crossing a region frontier. Checked-span construction must
reject every such input. Checked traversal has separate controls showing that a
span cannot be used with a foreign arena and that truncation invalidates its
descriptor before any retained range is published.

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

Paragraph line materialization admits each semantic or diagnostic source once
and threads the checked span through classification, direction tracking, and
retained-range append. Page and mode roots likewise retain checked endpoints
through traversal, composition, and slicing. The same source payload addresses
must appear in the result, `source_nodes_copied` must remain zero, and warmed
span operations must allocate nothing. Profiles should no longer show raw-list
admission below ordinary checked-root cursor, append, compose, or slice stacks.
`descriptor_entry_at` remains legitimate for actual descriptor topology walks
and one-time admission at the opaque boundaries listed above.
