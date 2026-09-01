# tex-dense-prefix Guidance

Read the repository-level `AGENTS.md` before editing here. This crate is the
only raw-memory owner for the dense-superblock experiment. It may know about
checked allocation, initialized prefixes, construction, truncation, drop, and
deallocation only. It must not acquire block ids, cursors, forks, generations,
TeX groups, page attempts, journals, node regions, or output semantics.

Every unsafe operation needs a local safety argument. Safe callers must never
receive a raw pointer, `MaybeUninit<T>`, unchecked length mutation, or direct
deallocation capability.
