# State responsibility boundaries

This pass keeps the existing owner, admission, and checkpoint authority while
reducing a synchronized arena record and duplicate borrowed traversal paths.

| Owner            | Retained authority                                                                       | Method groups                                                                                                              |
| ---------------- | ---------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `World`          | One host backend, effect and input ledgers, and `WorldSnapshot`                          | `input_dependencies`, `effect_publication`, and `checkpoint_lifecycle`                                                     |
| `ForkArena`      | One typed lane of logical coordinates, topology, and settlement metadata                 | `checkpoint_lifecycle`, `batch_transfer`, and `whole_region_transfer`; physical `ChunkPool` remains the sole payload owner |
| `CommandContext` | One already-admitted borrow of session, retained generation, dense core, and page region | `pdf_commands`, `page_material`, and `page_builder`                                                                        |

The child modules contain inherent methods on these same types. They add no
service, owner, snapshot, or conversion. `Universe` still admits and settles
the aggregate checkpoint. `tex-dense-prefix` still owns raw initialized
superblocks, `tex-dense-arena` still owns physical block identities and safe
logical tables, `ChunkPool` still owns page payload, and `ForkArena` still owns
TeX list topology and lane lifetime. Page list reads continue through borrowed
views and cursors.

## Storage and paragraph traversal

`ForkArena` derives its live payload end and tail from the accepted or forked
`ChunkSet` at each admission boundary. Both reads are constant time: the end is
the released base plus at most two vector lengths, and the tail is one indexed
lookup. The tail's physical chunk still has to prove its owner, lineage, and
owner-relative position in `ChunkPool`. Keeping a second mutable frontier
beside those vectors would require every append, rollback, settlement, and
transfer path to update the same fact twice. `ChunkPool` remains the only
physical payload owner; the arena still owns logical coordinates and topology.

Borrowed slice and borrowed page-arena paragraphs use one `NodeCursor` source
in `ParagraphTape`. `NodeCursor::owned` borrows a slice and does not allocate or
take ownership of its nodes. The cursor retains the compact arena view for
resident paragraphs. The separate owned `NodeSequence` keeps detached semantic
and physical projections, while the production arena-ID tape keeps only
coordinates so execution can release its command borrow before the retained
range post-line sink runs. These are distinct lifetime and diagnostic
contracts, not duplicate copies of the same production payload.

`WorldSnapshot` remains the sole host-state rollback mark; its copied scalars
are the inverse information for one live `World`, not another mutable owner.
The public `NodeArenaError` and generic `NodeArena<L>` compatibility API remain
available. Modern page payload stays in `PageMaterialArena`, and command and
`Universe` result signatures still expose that error vocabulary.

Validation must cover same-lineage rollback, candidate accept/reject, whole
batch transfer, command page/PDF access, and the existing feature and compile
fail gates. No wire format, public signature, or test selection changes.
