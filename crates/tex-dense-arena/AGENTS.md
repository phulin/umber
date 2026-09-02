# tex-dense-arena Guidance

Read the repository-level `AGENTS.md` before editing here. This crate is the
safe proof layer above `tex-dense-prefix`. It owns checked block identities,
flat tables, cursors, reuse, and lifetime-specific wrappers. Unsafe code is
forbidden here.

The node-integration prerequisite in
`../../docs/fork_arena_dense_prefix_emplacement.md` requires physical
`BlockStore<T>` ownership to separate from pool-stable logical tables, exactly
two borrowed accepted/candidate views, and prepared whole-block transfer
receipts. Physical `BlockId` must remain private to this crate. This crate does
not learn TeX lists, node children, annex codecs, or semantic region roles.

Keep generation fork-copy restricted to `Copy` payloads. Group, scratch,
journal, and speculative-output wrappers must remain distinct and nonforking;
do not give owned production records fork semantics for allocator convenience.

Source ownership is split as follows:

- `store.rs`: caller-owned typed physical blocks and private `BlockId` reuse;
- `logical.rs`: public logical coordinates, accepted tables, direct lookup,
  cursor/truncate, rotation, and the accepted/candidate transaction;
- `transfer.rs`: semantic-neutral whole-block owners, detached loans, prepared
  transfer, and exact-frontier rollback receipts;
- `generation.rs`: the convenience owner combining a store and table for
  isolated measurements;
- `nonforking.rs`: dense group, scratch, journal, and output policies; and
- `metrics.rs`: exact safe-layer measurement rows.
