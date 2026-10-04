# Current semantic mode ownership

Main control reads the current mode and pending horizontal run on every
ordinary terminal. ModeNest currently obtains both from the last element of
a vector containing every live semantic level. Its complete level occupies
616 bytes; the reader repeats the length check, row-address calculation and
list projection even for relax. This is actual semantic ownership work that
does not belong in ordinary dispatch.

Use TeX82 §216's ownership split: one actual current level and a bounded vector
of suspended levels. The current level always exists, including the outer
vertical base. It owns its mode, diagnostic entry line, final list builder,
pending character run and all mode-specific material. There is no duplicate
list header, cached pointer, pending-run flag or second list owner. Ordinary
mode/list access borrows this level directly.

Push validates the existing semantic nest bound before changing owners,
records the same pre-push high-water mark, suspends the previous current level
and installs the new empty level. Pop rejects the base level and uncommitted
pending characters before restoring its suspended parent and returning the
actual completed level. Horizontal scratch remains one pool owned by the
nest; no mode transition adds an allocator or new accounting scalar.

Cold diagnostics traverse suspended levels oldest-first and then the current
level. Validated parent-list coordinates access their actual owner directly.
The old live-level slice API is deleted, with no collected replacement slice.
Explicit detached summaries still own their requested ordered values.

Full restart checkpoints retain the existing single empty outer vertical
admission. Capture copies that actual current owner; restore releases displaced
pending sources, clears suspended levels and installs the captured outer level.
Candidate identity, preflight, accept/reject and page-region barriers retain
their existing ownership and ordering. Semantic equality and fingerprints use
the same logical level order. The obsolete identity-enabling no-op is deleted;
identity remains computed at the actual requested checkpoint boundary.

Move the mode nest and checkpoint implementation into its own source module.
Keep the list payload and its mutation authority separate from stack ownership.
Validate the real command/execution/checkpoint tests, complete native suite,
authoritative quality gates and exhaustive independent canonical comparison.
Measure both dense ordinary terminals and real mode transitions; fix any
regression in the actual owner or consumer instead of adding a compatibility
front or numerical completion threshold.
