# bib-sort Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns
list construction, filtering, sort templates and keys, and stable ordering.
It consumes immutable model and Unicode resources; map iteration, host locale,
and dependencies on sibling worker crates must never affect observable order.

## File Map

- `src/lib.rs`: immutable sorting-stage context boundary.
