# Runtime allocation audit

This audit covers the native execution paths exercised by the arXiv book
`2606.24937`, together with the owners behind those paths. It distinguishes
physical allocation from semantic ownership: using a `Vec` does not imply a
second runtime representation, and putting an object in an arena does not by
itself eliminate copying it.

## Current owners

| Runtime data            | Current production storage                                                                                       | Why other allocation remains                                                                                                                                    |
| ----------------------- | ---------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Nodes and node sidecars | Compact `NodeRecord` rows and typed annex rows in region-owned chunks; exclusive page and durable closure owners | TeX copies, retained group/checkpoint history, and closures that cannot transfer independently still need copies. Transient owned nodes build or detach values. |
| Macro definitions       | Packed `DefinitionRef` keys into format, global, or local-group regions                                          | An active local macro may hold a coarse region lease past group exit. There is no per-definition reference count.                                               |
| Macro arguments         | Reusable fixed scratch blocks, direct resident cursors, and sparse origin runs                                   | The active/spare block vectors own reusable capacity; they are not per-token allocations.                                                                       |
| Stored token lists      | Exact immutable `Rc<[TokenWord]>` payloads                                                                       | Token registers, journals, and input rows can independently retain the same list. Last-owner release maintains the semantic memory charge.                      |
| Input replay            | Fixed shared segments with copy-on-write checkpoint snapshots                                                    | The accepted input and candidate input can require independent cursor and segment histories.                                                                    |
| Provenance              | Dense generation-owned records and compact IDs                                                                   | Cold forks clone a generation; ordinary delivery uses direct IDs rather than allocating one shared owner per token.                                             |
| Mutable TeX state       | Direct dense banks and rollback journals                                                                         | These need indexed mutation and exact restoration, rather than immutable bump-only storage.                                                                     |
| Immutable fonts         | Coarse append-only payload chunks with separate dense mutable font parameters                                    | Metric programs and decoded resource bytes have font/resource lifetimes. The realized-identity index is derived lookup metadata, not another payload owner.     |
| PDF state               | Dense object maps and transaction-owned row vectors; shared immutable payload bytes                              | Accepted/candidate transactions and detached output can retain the same bytes. Color-stack byte vectors model independently mutable PDF state.                  |

The canonical arena/chunk storage is integrated. This is not a universal bump
allocator: exact shared payloads, dense mutable banks, and detached output have
different release requirements. No old/new command, definition, or node-storage
feature switch was found in the inspected production paths. The private
native-batch token program and frame stack were already removed; they are not
an alternative implementation to reconnect. The unreferenced
`page/sequence.rs` implementation of an owned-node page buffer was also removed;
it had no module declaration or callers and was not part of the compiled engine.

## Gaps found in execution

Destructive unboxing used the copying API followed by clearing the register,
even though a consuming transfer API existed. This bypassed the ownership
optimization. A transfer must still preserve group restoration and retained
history, and cannot move a region suffix whose other node blocks depend on
annex data outside that suffix. Those are real ownership constraints, not
compatibility fallbacks.

The consuming path also exposed a void-box rollback against a node block
shared with retained page history. The saved and current tail metadata were
identical, but rollback requested an exclusive write and failed. It now leaves
that unchanged tail alone; actual tail changes still require exclusive
ownership.

Region sealing previously completed missing child-dependency metadata by
scanning a construction suffix. Production typed-annex bulk writes marked
childless words incomplete, so this compatibility repair entered the book's
normal box path. Generic builders now record child floors when publishing each
value; the annex bulk path is restricted to childless scalar words and marks
its chunks complete during contiguous writes. Sealing no longer scans payload
to repair metadata, while incomplete reservations remain invalid until their
construction finishes or rolls back.

Conditional branch skipping requested rich command objects merely to inspect
a static command kind. It now uses the same compact raw delivery path as other
hot consumers. Reader advancement, fuel, alignment interception, scanner
recovery, and observation remain owned by that path.

Generated-font lookup constructed a projected font before looking for an
existing instance and rehashed every live font during the search.
[Immutable font identity lookup](font_identity_lookup.md) describes the indexed
lookup and allocation-on-miss replacement. The old `FontSourceIdentity` type
alias and forwarding accessor were removed at the same boundary without
changing identity bytes or font-resource equivalence.

Some apparent adapters are legitimate language boundaries: `read_toks` has
TeX's separate line-reading grammar, and scalar result/frame helpers preserve
caller-owned scanner state. Removing those based on names alone would not
simplify runtime ownership. PDF resource identity is likewise intentionally
different from realized-font identity.

Superblock retirement distinguishes values with drop glue from plain records.
For a type without drop glue, shortening the initialized prefix and adding the
removed count once is sufficient; visiting every removed slot only to update
an atomic diagnostic counter adds linear work with no ownership effect. Types
with destructors still drain in reverse order, including the remaining suffix
after a destructor panics.

## Evidence and limits

[Book performance investigation](arxiv_book_performance.md) records the original
production CPU profile, frozen binaries, source receipts, and run guards.
The initial samples attributed substantial time to token delivery, macro
expansion, and page/durable copying; they did not show speculative loading as
the active bottleneck. Inclusive profile percentages overlap and must not be
added as predicted savings.

The unbox and conditional changes eliminate demonstrated unnecessary work, but
neither individual book-prefix comparison established an overall speedup.
Ownership counters and semantic tests establish their local effects; matched
production runs are the separate performance evidence. Native tests do not
substitute for the original book: exercising the consuming path there exposed
a transfer preflight bug absent from the earlier routine suite.

## Fixed-fuel book heap attribution

The original `2606.24937` book was materialized from its authenticated source
archive using the command, format, distribution, and source-date epoch in the
[original result receipt](../target/arxiv-pdf-wave3-final/rows/2606.24937/result.json).
All runs retained the 120-second, 1,536-MiB, and 10-million-step guards and
used CPU 10. Every tabulated fixed-fuel run ended with the exact fuel-exhaustion
diagnostic. Profiling builds are used only for storage counters, not shipping
CPU comparisons. Full commands, hashes, logs, time records, and heaptrack data
are under `.worktrees/slot-3/target/perf-heap-retention/`.

| Profiling build | Fuel actions | Peak RSS KiB | Peak live annex blocks | Annex blocks at output census | Used annex words | Stranded annex words |
| --------------- | -----------: | -----------: | ---------------------: | ----------------------------: | ---------------: | -------------------: |
| Before packing  |   50,000,000 |      248,360 |                    578 |                           290 |           88,998 |            4,662,362 |
| Before packing  |  100,000,000 |      460,456 |                  2,031 |                         1,883 |          790,728 |           30,060,344 |
| Before packing  |  200,000,000 |      635,304 |                  2,031 |                         1,883 |          790,728 |           30,060,344 |
| Packed annexes  |  100,000,000 |      432,796 |                    271 |                           125 |          790,728 |            1,257,272 |
| Packed annexes  |  200,000,000 |      573,980 |                    271 |                            83 |        1,266,406 |               93,466 |

The 100-million-action output census holds the same 790,728 annex words in both
builds. Before packing, every one of its 1,883 physical annex blocks was
partial and no block was shared between logical chunks. After packing, all 125
sampled physical blocks are shared; stranded capacity falls from 30.1 million
to 1.26 million words. The 200-million-action output censuses select different
largest-output boundaries, so their used-word values are not a matched
semantic snapshot. The peak-live counter measures a different instant from the
output census and must not be added to its block count.

Heaptrack of the frozen shipping baseline identifies a separate growing owner:

| Fuel actions | Peak RSS KiB under heaptrack | Annex superblock allocation site | PDF version-index allocation site |
| -----------: | ---------------------------: | -------------------------------: | --------------------------------: |
|   50,000,000 |                      256,248 |                         37.88 MB |                  Below top report |
|  100,000,000 |                      470,496 |                        133.10 MB |                         100.66 MB |
|  200,000,000 |                      646,116 |                        133.10 MB |                         201.33 MB |

The version-index allocation is the persistent `Vec<PdfVersionIndexNode>` in
`PdfState`. Its reported peak grows by 100.67 MB between 100 and 200 million
actions, while the annex superblock site remains flat. Other measured growth
includes token-list payloads and verified resource artifacts. Heaptrack site
peaks are allocation-site observations, not simultaneously live values to sum
into RSS. The PDF index's 64-level path copying is a distinct follow-up from
annex packing.

The pool did return retired regions: from 100 to 200 million actions the old
annex pool made no new physical allocation, while reuse events rose from
26,460 to 63,358 and release events from 28,413 to 65,299. With packed
annexes, physical allocations stay at 271 over the same interval. The zero
live-block gauges printed on fuel failure are after engine teardown; the peak
and output censuses establish in-run retention. Warm vacant superblocks remain
available for reuse rather than being returned to the system at each region
retirement.

A shipping-build A/B/B/A sequence at 200 million actions used baseline binary
SHA-256 `da06fc9ad71202377133c74a50eb6f861309b7805a9483fa4652c6d75c55f4b7`
and packed binary SHA-256
`a47b912f2912d4108aea9ba13a33f74ae1797cb1098c967c9e7b0b4a750cb2a9`.
Peak RSS was 634,780 and 635,548 KiB before packing versus 574,624 and
574,876 KiB after packing: a median reduction of 60,414 KiB. Other builds
competed for CPU during this sequence, so its user and wall times do not
establish a speed change. The reported 4.7-GB run has no authenticated receipt;
these bounded runs do not reproduce it or identify every byte it may have held.
