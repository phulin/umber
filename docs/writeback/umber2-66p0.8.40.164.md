# `umber2-66p0.8.40.164`: statically minimal arena capabilities

## Result

`ForkArenaBuilder` now retains the admitted destination block across ordinary
pushes. Block admission checks owner, lineage, incarnation, vacancy, and list
capacity once; resident pushes then write the final slot and advance only the
initialized prefix, root, and scalar cursors. `finish_unique(self)` is
infallible and consumes the construction owner. Its move-only result can be
published once.

Admitted list traversal likewise uses a mutable chunk cursor. Successful
resident reads perform one block-table lookup, one payload index, and cursor
increments. Root, owner, incarnation, and range admission occurs only when the
cursor is created; predecessor transitions admit one new block. Stale integer
coordinates therefore fail on readmission, not on every value access.

The reusable page builder moves its private open owner out at finish and
immediately returns to vacant state. Finish does not revalidate its arena,
root, predecessor chain, or initialized range. The representation remains the
single exact-64-KiB dense block table with stable logical coordinates; no
pointer cache, payload mirror, compaction, or new unsafe code was added.

Rollback, accepted/candidate settlement, cross-region move/copy, logical list
boundaries inside shared physical blocks, and native/Wasm coordinates retain
their existing behavior.

## Focused validation

- The capability and cursor tests pass, including compile-fail checks for
  builder reuse and repeated publication.
- `scripts/check.sh clippy`: both resolutions clean across 32 workspace
  members.
- The warmed arena gate reports checksum `202396794880`, seven logical blocks,
  zero allocations, and zero requested bytes. Versus exact baseline
  `1b358891b`, seven-run means move from 341,395,548 to 274,314,341
  instructions (-19.65%) and from 113,303,730 to 87,047,583 cycles (-23.17%).
- The public-copy census remains 18 `memcpy` calls / 153 startup bytes and two
  zero-byte `memmove` calls for both binaries; the arena workload adds none.
