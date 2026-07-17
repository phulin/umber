# bib-graph Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns
sourcemaps, dependency closure, aliases, relationships, inheritance, sets,
and data-model validation. Preserve declared and diagnostic order explicitly;
do not depend on other worker crates, `bib-engine`, hosts, or mutable globals.

## File Map

- `src/lib.rs`: immutable graph-stage context boundary.
