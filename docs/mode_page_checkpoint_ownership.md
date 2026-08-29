# Mode and page checkpoint ownership

Status: implementation contract, 2026-08-27.

This document specializes the
[aggregate checkpoint component contract](aggregate_checkpoint_contract.md)
for executor modes and the page builder. It does not introduce an independent
ownership graph. [Node-region ownership](node_region_ownership.md) is the
authority for node closure lifetime: exclusive page regions, not raw list
coordinates or dependency-counted batches, keep page payload live.

## Two-lineage chunk representation

Every retained page list participates in at most two execution lineages:

1. immutable regions in the accepted prior lineage; and
2. append-only regions in the current candidate lineage.

Node payload is append-once in fixed-byte chunks inside one caller-owned coarse
pool. Typed `ForkArena` states contain coordinates and lifecycle metadata, not
the pool. Shared pool borrows yield stable direct node references; all physical
mutation takes the caller's exclusive mutable pool borrow. The only list
topology is a direct `ArenaRange` or one arena-owned nonrecursive sequence of
direct ranges. Payload chunks have no competing row-list identity, and there
is no parallel `NodePiece`, linked-node, or `Vec<Node>` owner.
Appending opens only current chunks. Front/tail consumption, prepend, split,
transfer, and discard movement publish compact canonical range-list records;
they never materialize a contiguous payload buffer.

One exclusive `PageRegion` owns the page-node and descriptor chunks for each
page-building period between shipouts. Paragraph checkpoints inside that page
store the region id, all four exact PageBuilder roots, sealed payload and
descriptor positions, scalar state, and journal position. They share the one
backing region and copy no node payload.

The selected region's arena state is exactly `Accepted` or `Forked { prefix,
detached_prior, current }`. Candidate rejection drops current chunks and
reattaches the prior suffix. Candidate acceptance drops the detached prior and
promotes current. Accepted page regions after the selected checkpoint detach
and settle as whole owners. A later edit still has only the accepted lineage
and one private current suffix; no checkpoint, list, or operation creates a
third generation.

When an incremental session requests convergence identity before job start,
each ordered node lane also maintains a version-1 domain-separated polynomial
identity beside its ordinary extent. Append, prepend, and end consumption use
scalar arithmetic; the accepted/current (and contribution front/prior/back)
regions compose from their fixed roots without walking payload. Immutable page
list coordinates carry the semantic identity of their published child list,
so relocation, arena owner, row, generation, and cursor changes do not change a
mode or page root. This is derived state inside the existing coarse lane, not a
cache, registry, per-node owner, or second ownership graph.

A shipout transaction created inside a candidate records the accepted roots
and the lengths of every candidate-private region in its fixed page mark.
Rollback restores those coordinates after applying its move-only private
inverses. It therefore cannot leave an accepted contribution, current-page, or
discard root consumed merely because artifact lowering aborted.

Active paragraph, math, alignment, and box builders own exclusive typed chunk
regions. Their persistent `ActiveListBuilder` state is coordinate-only: every
mutation temporarily presents the one caller-owned pool and lane, and an open
builder prevents checkpoint sealing. Appending an immutable page/durable list
adds only its canonical ranges; replacement nodes are appended once. Sealing a
complete output produces a move-only `SealedBatch`; lane
promotion transfers its whole chunk envelopes and canonical range descriptors
to page-material ownership while retaining stable raw chunk keys. Mixed
transforms reuse unchanged ranges and append replacement nodes once. Random
access binary-searches cumulative range endpoints, while a sequential cursor
retains its scalar position across short-lived arena borrows.

A destructive Mode operation does not retain a copied `NodeSequence` inverse.
Candidate-only active material restores an `OperationMark` and truncates its
partial chunk suffix on local failure. Legal restart checkpoints contain no
active Mode node list. Paragraph breakpoint search, widths, protrusion,
tracing, and line materialization share one statically dispatched borrowed
view over test slices and canonical arena lists. A coordinate-based
`ParagraphTape` stores only the list coordinate plus scalar/index scratch and
reborrows payload for each execution step.

Production post-line breaking consumes that coordinate into a detached
page-material active builder. It appends unchanged source ranges and frozen
discretionary subranges, generates only nodes whose semantics actually change,
and publishes the completed line once. Semantic and TeX-physical `Vec<Node>`
channels are test-adapter concerns, not runtime ownership. Only a genuinely
distinct hyphenation diagnostic source can create an optional detached
diagnostic projection; an ordinary line has one list topology. Reusable
direction and lineage vectors contain scalar evidence only. `new_semantic_nodes`
therefore measures generated line nodes, while `source_nodes_copied` remains
zero after paragraph publication and is backed by an explicit nonzero negative
control plus source-address retention tests.

Automatic hyphenation follows TeX82 §§914--918's nested-list order. Before it
publishes a discretionary's pre-break, post-break, or replacement child, it
seals the preceding main-list segment; after the children are complete, it
resumes the main-list builder and composes the segments by coordinate. Thus one
page-arena builder owns the mutable suffix at every instant without copying
the retained paragraph prefix or republishing generated word nodes.

Raw `PageListId` and `ArenaListId` values are borrowed capabilities under the
matching region owner. They cannot be stored as production top-level owning
roots. PageBuilder roots live inside `PageRegion`; a box/form owner carries its
exclusive durable region with its root. Compile-fail coverage must reject a
naked coordinate escaping that owner.

Rooted settlement has three aggregate phases. Acceptance commits destination
page/layout ranges, releases source-side move bookkeeping, and only then closes
the transaction. Rejection first detaches candidate destination ranges and
returns their carriers, then Mode undoes the candidate suffix and forward-redoes
the saved accepted moves, and finally the page/layout owners reinstall those
accepted ranges. The reachability store is the sole phase coordinator; a
rooted component cannot use a one-shot accept/reject API or settle itself.

The retained mode mark is rootless, but the live mode owner still has a typed
candidate lifecycle. Fork construction labels the restored nest as a candidate;
accept and reject each consume that capability and make a second disposition
impossible. A normally dropped unresolved candidate is an invariant failure,
not an implicit rejection path. Main-control completion parks the live command
owner and moves the mode capability into `PreparedCheckpointControl`; Session
consumes that receipt before aggregate state acceptance or rejection. Candidate
acceptance preserves the already-built topology and current roots without
replay, while rejection discards only that candidate topology. The fixed mark
and accepted page owner remain unchanged, so sibling marks stay seedable.
While CandidateRun temporarily detaches its runtime and MainControl, an owned
attached-control guard keeps their sidecar slots exclusively borrowed and
parks both owners from `Drop` during unwind. The outer owned generation guard
then performs the same complete dependency-ordered rejection as an explicit
Session rejection; it never relies on ModeNest suppressing an unresolved-drop
assertion during a panic.

## Mode marks

A live `ModeList` pairs every nonempty page-list coordinate it carries with the
identity of the one admitting `PageRegion`. Node publication validates each
direct parent/child edge against that region; admission can therefore validate
the top-level coordinate in constant time without walking payload or building a
root census. Nested horizontal, vertical, math, and alignment levels are
move-only operation-local state. Popping or packaging a level transfers its
exact list root, and the private rollback journal records the owner identity and
coordinate together. Each journal frame carries a cumulative scalar summary of
whether any active rollback projection can restore a page root, so page-region
succession remains constant-time and cannot retire that root's region. No
production summary clones a rooted mode level.

An operation which both rewrites a mode root and opens a different lifetime
owner orders those transitions source-first. In particular, TeX82 §§1074 and
1077's `\setbox<n>=\lastbox` settles the shortened mode list before opening the
durable destination suffix, so destination sealing cannot capture the live
page-owned source descriptor.

A retained mode checkpoint is legal only at a quiescent root-main-file
paragraph boundary with one empty outer vertical level. It stores that fixed
rootless scalar level, the mode-timeline lineage and serial, and the semantic
journal position; it does not retain the live nest or any page-list root. Entry
lines, pending-character state, paragraph and display scalars, and alignment
state are restored by the same generation-owned reversible journal. The
job-lifetime maximum nest depth is operational telemetry and is never rolled
back.

Append-only list changes restore by resetting span ends. A mutation which
cannot be expressed as range movement records exactly one first-before value in
the active semantic interval. It does not clone an accumulated node prefix.

The candidate restored from that mark starts with exactly the stored outer
level and a fresh operation journal. Nested levels, pending characters,
alignment, fraction, display, scalar, and page-list state created afterward
belong only to its current suffix. Accepting does no mode-journal replay or page
publication; rejecting never visits an accepted mode prefix. Page-region
settlement remains the owner of candidate chunk reclamation and runs before the
mode capability is consumed.

After shipout has consumed all live and rollback-restorable mode-list roots,
the executor issues a move-only same-region preflight receipt which `Universe`
must consume before preparing page-region succession. The combined seam
deliberately remains outside the production shipout tail until durable box and
form carriers have the same owner-relative lifecycle; mode lists no longer
block that later cutover.

## Page marks

A page checkpoint stores its exclusive page-region id, the sealed payload and
descriptor positions, the semantic journal position, and exact owner-relative
roots for the contribution, current-page, page-discard, and split-discard
lists. Fixed page dimensions, integers, contents, last-item facts, best-break
coordinates, and fire-up coordinates are stored directly in the mark.
Insertions and sparse mark classes use
generation-owned append/journal lanes with scalar roots; the five class-zero
marks are journaled token-list roots. Neither the insertion-position index nor
the mark-class direct-lookup index is checkpoint ownership. They are rebuilt or
rewound as part of applying their canonical journal roots.

Validation checks every lineage, region generation, serial, range, font root,
token root, and page-node root without mutation. Application follows the
aggregate order: acquire the exclusive region/history owner; restore dense and
PDF state; install mode and page roots; transfer external roots; settle payload
and descriptor suffixes atomically; then release replaced whole regions only
after no restored root can borrow them.

## Retained-byte accounting

Checkpoint history owns page regions directly in document order. Boundary rows
for one page form one contiguous interval; no per-checkpoint or batch reference
count determines liveness. A page region is charged once for its chunks,
descriptors, PageBuilder state, and reusable capacity. A checkpoint is charged
only for its fixed cursor/root record and execution counters. Shared prefix
bytes are never charged once per checkpoint, and detached committed shipout
artifacts are charged to the output owner rather than the speculative page
timeline.

Shipout starts a new page region. Handle-free output keeps no runtime node.
The page-breaking traversal evacuates only the exact held-over closure into the
new region, moving self-contained whole envelopes when no historical owner
needs them and otherwise copying that bounded closure. An old region remains
live only while its checkpoint interval remains retained, then drops wholesale.

Prepared DVI receipts have their own direct `OutputLedger` owner. An engine
checkpoint stores one fixed receipt-count mark into that accepted ledger.
Forking splits the accepted receipt tail at the mark and resumes the prefix;
the candidate appends a private suffix. Rejection drops that suffix and
reattaches the saved tail, while acceptance drops the superseded tail and keeps
the live prefix plus candidate suffix. Earlier receipts are never copied into a
candidate, and MainControl has no `Arc<Vec<_>>` copy-on-write receipt buffer.
At terminal completion, the unforgeable terminal-revision receipt authorizes a
borrowed visit of that same ledger. A loaded job serializes the exact retained
DVI plans before TeX prints its byte-count report, then terminal capture closes
the ledger after the report and moves the already-aligned pages into the run
result. The runner never consults the drained MainControl page queue or builds
a second page-plan vector.

The storage may retain bounded spare capacity up to the generation's observed
high-water mark. That capacity is reusable storage, not live semantic payload.
No compaction, per-value owner, root registration, ordinary-path copy-on-write,
or deferred prefix clone is permitted.

Identity maintenance is selected once, before execution, by the incremental
history session. Batch and other non-incremental sessions leave it disabled:
node-sequence mutation, page-list publication, alignment/pending-run mutation,
insertions, and marks then perform none of the new semantic hash work. In an
enabled session, each coarse lane owns only fixed scalar roots and list ids
reuse the identity computed while their immutable payload is published;
checkpoint demand merely copies/composes those roots and allocates nothing.
Identity demand changes only whether scalar semantic roots are maintained.
Mode, page, move-carrier, and output-ledger ownership and settlement are the
same with identity enabled or disabled; there is no separate rootless
lifecycle.
