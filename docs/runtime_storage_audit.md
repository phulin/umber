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
