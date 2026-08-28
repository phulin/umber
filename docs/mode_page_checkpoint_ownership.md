# Mode and page checkpoint ownership

Status: implementation contract, 2026-08-27.

This document specializes the
[aggregate checkpoint component contract](aggregate_checkpoint_contract.md)
for executor modes and the page builder. It does not introduce an independent
ownership graph. The retained generation remains the sole lifetime authority;
mode and page marks name storage owned by that generation.

## Two-lineage sequence representation

Every open mode list and page-builder list is owned by at most two generation
lineages:

1. immutable regions in the accepted prior lineage; and
2. append-only regions in the current candidate lineage.

The regions are coordinates into coarse generation storage. They are not
owners, chunks, history entries, or values which can outlive that storage.
Most lists need one prior region followed by one current region. The
contribution deque may need a bounded three-region view--a current-lineage
front lane, the immutable prior region, and a current-lineage back lane--so a
candidate can implement both TeX prepend and append without adding an owner.
Appending material extends only a current region. Front consumption, tail
consumption, list transfer, and discard-list movement update scalar coordinates
and do not copy a region. APIs which consume or inspect a complete logical list
iterate the bounded view in semantic order; they do not materialize a
contiguous `Vec`.

Candidate rejection discards the private current suffix wholesale and restores
the prior roots. Candidate acceptance makes that suffix the current accepted
extent before the old prior lineage is released. A later edit still has only
the accepted prior and one private current suffix; no checkpoint, chunk, or
list creates a third generation.

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

List transfers follow the same two-lineage rule. Node payload lives once in
generation-owned immutable `PageNodeArena` segments. A logical sequence is
either one direct segment range or one range in the arena's append-only flat
`NodePiece` stream. Every piece names a payload segment and carries its
cumulative logical endpoint; pieces never name other composite sequences.
Mixed transforms append one descriptor for each unchanged or newly produced
run. Flattening a composite input copies only its compact direct-span
descriptors, never node payload. Random access binary-searches cumulative
endpoints, while a sequential cursor retains its current piece across
short-lived arena borrows.

A destructive Mode operation does not retain a copied `NodeSequence` inverse.
It moves coarse range or sequence coordinates through its destination and then
into the candidate arena suffix. The fixed inline multi-range carrier remains
only transaction-phase plumbing; it is not the arbitrary logical-sequence
representation. Paragraph breakpoint search, widths, protrusion, tracing, and
line materialization share one statically dispatched borrowed view over slices
and arena sequences. A coordinate-based `ParagraphTape` stores only the
sequence coordinate plus scalar/index lineage scratch and reborrows payload for
each execution step. The ordinary paragraph path moves its completed source
into the page arena once; a hyphenation fallback reverses that whole-range move
before transforming it. Kernels publish only semantically new output nodes and
move unchanged runs by descriptor.

Rooted settlement has three aggregate phases. Acceptance commits destination
page/layout ranges, releases source-side move bookkeeping, and only then closes
the transaction. Rejection first detaches candidate destination ranges and
returns their carriers, then Mode undoes the candidate suffix and forward-redoes
the saved accepted moves, and finally the page/layout owners reinstall those
accepted ranges. The reachability store is the sole phase coordinator; a
rooted component cannot use a one-shot accept/reject API or settle itself.

## Mode marks

A mode checkpoint records the mode-timeline lineage and serial, the semantic
journal position, the bounded mode depth, and one pair of list-span coordinates
per live TeX nest level. TeX's semantic nest is bounded by the existing 40-save
limit, so the complete root record has a fixed maximum size. Entry lines,
pending-character state, paragraph and display scalars, alignment state, and
the roots of any immutable token or page-node values are restored by the same
generation-owned reversible journal. The job-lifetime maximum nest depth is
operational telemetry and is never rolled back.

Append-only list changes restore by resetting span ends. A mutation which
cannot be expressed as range movement records exactly one first-before value in
the active semantic interval. It does not clone an accumulated node prefix.

## Page marks

A page checkpoint records the page-timeline lineage and serial, the semantic
journal position, and span roots for the contribution, current-page,
page-discard, and split-discard lists. Fixed page dimensions, integers,
contents, last-item facts, best-break coordinates, and fire-up coordinates are
stored directly in the mark. Insertions and sparse mark classes use
generation-owned append/journal lanes with scalar roots; the five class-zero
marks are journaled token-list roots. Neither the insertion-position index nor
the mark-class direct-lookup index is checkpoint ownership. They are rebuilt or
rewound as part of applying their canonical journal roots.

Validation checks every lineage, serial, range, font root, token root, and page
node root without mutation. Application follows the aggregate order: acquire
the coarse owner; restore dense and PDF state; install mode and page roots;
transfer external roots; truncate font, source, durable-node, and page-node
suffixes; then release the replaced owner.

## Retained-byte accounting

The generation is charged once for the capacity and initialized contents of
its mode and page sequence stores, insertion lane, mark lane, and reversible
semantic journals. A checkpoint is charged only for its fixed cursor/root
records and execution counters. Shared prefix bytes are never charged once per
checkpoint, and detached committed shipout artifacts are charged to the output
owner rather than the speculative page timeline.

Prepared DVI receipts have their own direct `OutputLedger` owner. An engine
checkpoint stores one fixed receipt-count mark into that accepted ledger.
Forking splits the accepted receipt tail at the mark and resumes the prefix;
the candidate appends a private suffix. Rejection drops that suffix and
reattaches the saved tail, while acceptance drops the superseded tail and keeps
the live prefix plus candidate suffix. Earlier receipts are never copied into a
candidate, and MainControl has no `Arc<Vec<_>>` copy-on-write receipt buffer.

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
