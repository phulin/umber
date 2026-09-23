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

## World live state and checkpoint marks

The World audit found no further live owner record that can be removed under
the current public API and constant-time dependency-limit check. For example,
`input_dependency_len` counts distinct paths across the accepted dependency
parent and the live map. An accepted parent containing `a` with a live update
to `a` has one distinct path; the same parent with a live insertion of `b`
has two. Both live maps have length one. Deriving the count by merging maps at
each new admission would repeatedly traverse and allocate for a set bounded
at 8,192 paths. The stored count and its snapshot inverse preserve a bounded
constant-time limit check through rollback and fork.

The artifact hash, committed artifact, and publication columns are aligned,
but each has a public borrowed-slice accessor. A combined record could not
return those slices without keeping duplicate columns or changing that API.
The hash also appears in `CommittedArtifact`, yet the separate hash slice is
used as the ordered shipout notification and page cursor. Publication records
can be linked to an effect after artifact commit, so they are not immutable
projections of the artifact bytes. Effect sequence, domain, and ordinal columns
similarly support public `Arc<Vec<_>>` access and separate installation or
claim operations; merging them would change those contracts or recreate the
columns on demand.

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
conversion path. The World-specific storage aspiration is therefore settled
at this API and performance boundary, rather than deferred as an unexplored
audit.

The public `NodeArenaError` and generic `NodeArena<L>` compatibility API remain
available. Modern page payload stays in `PageMaterialArena`, and command and
`Universe` result signatures still expose that error vocabulary.

Validation must cover same-lineage rollback, candidate accept/reject, whole
batch transfer, command page/PDF access, and the existing feature and compile
fail gates. No wire format, public signature, or test selection changes.
