# tex-dense-arena Guidance

Read the repository-level `AGENTS.md` before editing here. This crate is the
safe proof layer above `tex-dense-prefix`. It owns checked block identities,
flat tables, cursors, reuse, and lifetime-specific wrappers. Unsafe code is
forbidden here.

Keep generation fork-copy restricted to `Copy` payloads. Group, scratch,
journal, and speculative-output wrappers must remain distinct and nonforking;
do not give owned production records fork semantics for allocator convenience.
