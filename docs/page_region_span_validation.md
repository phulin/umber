# Page-region span validation

Page material has two deliberately different coordinate forms:

- `PageListId` is the compact transport coordinate stored in nodes, mode
  state, page-builder state, journals, and checkpoint rows. It may cross a
  lifecycle boundary, so a receiving owner must validate it.
- `PageListSpan` is the checked traversal capability minted from one
  owner-admitted `PageListId` by the current `PageMaterialArena`. It records
  the descriptor endpoints established when construction or root admission
  fully validated the list. Its fields and constructor are private. Ordinary
  traversal and retained-range append accept this type and do not repeat
  `descriptor_entry` or `validate_raw_range`.

The span is not an owner, cache, copied node list, or public unchecked handle.
It contains only the original coordinate and its validated descriptor
position. Payload remains in the one `NodePool`, and every actual descriptor or
payload borrow still passes through the pool's generation and arena-owner
check. A span whose descriptor was truncated, transferred, or retired therefore
fails closed instead of resolving replacement storage.

## Validation boundaries

| Boundary                 | Input                                                                | Validation and result                                                                                                                                                                                                                                                                                                                          |
| ------------------------ | -------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Construction             | Newly published payload and descriptors                              | Node publication rejects child coordinates outside the current arena. Builder finalization validates the complete canonical descriptor record once. Callers may explicitly admit the resulting `PageListId` as a `PageListSpan` before a traversal.                                                                                            |
| Existing-owner traversal | Opaque `PageListId` already accepted by the current semantic owner   | Span admission checks the current arena, resolves the first and last descriptor keys, and verifies both generation/used bounds. `PageListSpan` then carries those endpoints through cursor construction, indexed reads, iteration, full-list append, and subrange append. No list-wide descriptor/range walk or copied payload is performed.   |
| New root admission       | `PageListId` entering a mode list or page-builder root               | The receiver validates the complete list against the current `NodeRegionId`, then stores the region admission beside its roots. Repeated reads of unchanged roots use that admission instead of rescanning all roots.                                                                                                                          |
| Operation rollback       | Operation mark plus active builder                                   | The active builder can roll back only its own appended suffix. Source spans admitted before the builder opened remain in the prefix. Any span into the discarded suffix fails the descriptor chunk generation/used-bound check. Aggregate rollback restores page roots and arena marks together and re-admits roots at the aggregate boundary. |
| Checkpoint candidate     | Private checkpoint row                                               | Candidate begin validates the page-region key, node checkpoint mark, and builder mark before detaching any later owner. Accept/reject settles the complete arena and builder aggregates; traversal spans are not stored in checkpoint rows.                                                                                                    |
| Page successor           | Complete `PageBuilder` root set and optional move-only closure build | Successor preparation fully validates all four roots in the old region before transfer or structural copy. The destination constructs new checked coordinates under its fresh `NodeRegionId`; no old span crosses succession. Commit either retains the entire old region for checkpoint history or retires it wholesale.                      |
| Shipout                  | Prepared page roots owned by the current page region                 | Shipout traversal admits each root before staging. Successful publication commits the prepared successor; failed staging rolls back only speculative output work and retains the old root owner. A traversal span never becomes an artifact coordinate.                                                                                        |
| Retirement               | Exclusive `PageMaterialRegion` or durable closure owner              | Retirement first checks that there is no active builder or batch and validates all live chunks. Releasing chunks increments their generations, so stale ids and stale spans cannot alias later occupants.                                                                                                                                      |

## Negative controls

The raw-coordinate APIs remain the boundary tests for forged structure,
foreign pools, foreign arenas, stale chunks, truncated ranges, and descriptor
or payload spans crossing a region frontier. Checked-span construction must
reject every such input. Checked traversal has separate controls showing that a
span cannot be used with a foreign arena and that truncation invalidates its
descriptor before any retained range is published.

## Performance and accounting contract

Paragraph line materialization admits each semantic or diagnostic source once
and threads the checked span through classification, direction tracking, and
retained-range append. The same source payload addresses must appear in the
result, `source_nodes_copied` must remain zero, and warmed span admission and
traversal must allocate nothing. Profiles should no longer show
`ForkArena::descriptor_entry`, `ForkArena::validate_raw_range`, or
`ForkArena::validate_list` below the ordinary post-line cursor and append
stacks.
