# bib-input Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns
control/configuration and datasource parsing. It consumes immutable
`umber-vfs` snapshots, emits typed `bib-model` values, and must not perform
host I/O or processing-stage graph, sorting, labeling, or output work.

## File Map

- `src/lib.rs`: immutable input-stage context and boundary types.
