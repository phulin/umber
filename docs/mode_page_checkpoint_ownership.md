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

A shipout transaction created inside a candidate records the accepted roots
and the lengths of every candidate-private region in its fixed page mark.
Rollback restores those coordinates after applying its move-only private
inverses. It therefore cannot leave an accepted contribution, current-page, or
discard root consumed merely because artifact lowering aborted.

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

The storage may retain bounded spare capacity up to the generation's observed
high-water mark. That capacity is reusable storage, not live semantic payload.
No compaction, per-value owner, root registration, ordinary-path copy-on-write,
or deferred prefix clone is permitted.
