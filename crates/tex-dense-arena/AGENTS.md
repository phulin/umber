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
