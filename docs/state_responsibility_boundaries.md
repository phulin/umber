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

The page-material arena is the only node-list owner. `PageListId` and
`PageListSpan` are coordinates and admitted proofs over that owner; the
borrowed `NodeCursor`/`NodeView` projection reads compact records without
constructing a parallel list. Paragraph analysis retains page coordinates and
scalar semantic/physical boundary and lineage evidence so execution can end
its command borrow before the retained-range post-line sink runs. Pure
typesetting tests use borrowed cursors over their own page-material test owner.
Distinct semantic and physical diagnostic channels remain observable; they do
not require an owned duplicate of the entire paragraph.

## World live state and checkpoint marks

The World audit keeps independent live facts and the inverse information
needed by checkpoints. For example,
`input_dependency_len` counts distinct paths across the accepted dependency
parent and the live map. An accepted parent containing `a` with a live update
to `a` has one distinct path; the same parent with a live insertion of `b`
has two. Both live maps have length one. Deriving the count by merging maps at
each new admission would repeatedly traverse and allocate for a set bounded
at 8,192 paths. The stored count and its snapshot inverse preserve a bounded
constant-time limit check through rollback and fork.

Each `CommittedArtifact` already owns its content hash. World therefore keeps
one ordered committed-artifact column and derives hash notifications and
artifact cursors from that column without storing a parallel hash vector. The
publication column remains distinct: a publication can be linked to an effect
after artifact commit, so it is not an immutable projection of the artifact
bytes. Effect sequence, domain, and ordinal columns likewise record distinct
installation or claim events. Their structure-of-arrays layout supports the
current hot ledger access pattern; an old borrowed-slice API alone is not a
reason to retain duplicate authority.

Other apparent duplicates encode different history. The page-effect artifact
cursor distinguishes live effects already embedded in a committed page from
pending effects at the same ledger length. Closed output paths remain in the
committed-path set after their write-stream slots are empty. Monotone identity
counters survive publication and fork sequencing; the optional incremental
reachable-state identity avoids rescanning effects, inputs, and scalars for a
checkpoint root. These are live facts, not interchangeable representations.

`WorldSnapshot` remains the sole host-state rollback mark. Its copied cursors,
counters, and scalars are inverse information for one live `World`, not another
mutable owner. Its effect position can be computed from base and length, but
removing that one private copied scalar would not reduce a live owner or
conversion path. The remaining World-specific storage boundary is its current
event and checkpoint model, not compatibility with retired API shapes.

The generic `NodeArena<L>` scratch/page/durable storage and its lifetime,
range, and sequence coordinates are retired. Page methods report failures in
the vocabulary of their actual owner: invalid or foreign page coordinates,
invalid region marks, and failed promotion. Error translation must preserve
these distinctions and cannot turn every rejected operation into an old
`InvalidList` or `ForeignCursor`. Tests for stale handles, rollback, transfer,
and output continue through the page-material arena and aggregate facades.
Cold node materialization is restricted to real detachment and output demand;
successor root validation traverses compact records directly.

Validation covers same-lineage rollback, candidate accept/reject, whole batch
transfer, command page/PDF access, paragraph semantic/physical diagnostics,
and the existing feature and compile-fail gates. This removal changes public
Rust signatures but no TeX semantics or artifact wire format.
