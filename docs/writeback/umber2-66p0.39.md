# `umber2-66p0.39`: ordinary command-context admission

## Evidence and acceptance

The originating exact authenticated 20-million-command capture attributes
270,070,463 disjoint ancestry cycles, or 1.5427% of the whole capture, to
repeated `Universe::command_context` admission below ordinary preparation,
application, save-stack accounting, page-output selection, and local glue
classification. Per the fixed queue override, this issue does not run paired
CPU profiling; the acceptance evidence is architectural deletion, exact
behavior, and deterministic allocation gates.

## Resulting ownership

`CommandContext` remains a stack-local borrowed facade. Main-control entry,
named-list publication, file-framing retirement, capability refresh, and
command delivery now share one admission until the processor borrow retires.
The output-routine path likewise shares one admission across page selection,
named-list publication, output-routine command setup, and group entry. Default
shipout explicitly drops that context before entering the outer World/shipout
transaction.

Glue reassignment and e-TeX redundant-assignment classification now receive a
shared reborrow of the cold-apply admission instead of reconstructing the
facade independently. Save-stack accounting also receives only a shared
reborrow. No `CommandContext` is stored in `MainControl`, retained in an
operation frame, moved into rollback or suspension state, or carried across a
resource or executor barrier. The change adds no owner, cache, fast path, heap
indirection, compaction, or lifetime mechanism.

## Verification

The focused `tex-exec` suite passes 695 unit tests, 4 fixture-parity tests, and
23 integration tests. The complete `cargo test -q --tests` routine suite
passes. `scripts/check.sh` reports dprint, Biome, rustfmt, and both clippy
resolutions clean.

The standalone `canonical_episode` allocation gate cannot build at this base:
its pre-existing synthetic-font constructor supplies the new 32-byte
`ContentHash` where `LoadedFont::new` still requires an 8-byte identity. The
failure occurs in `benchmarks/tex-exec/src/lib.rs:153`, outside this issue's
production diff and before the gate executable is produced. No production
allocation, container, copy, cache, or owner was added by this change.
Repair and execution of the three documented rows is tracked separately as
`umber2-z55f`.
