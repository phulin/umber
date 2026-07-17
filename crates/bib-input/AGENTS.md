# bib-input Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns
control/configuration and datasource parsing. It consumes immutable
`umber-vfs` snapshots, emits typed `bib-model` values, and must not perform
host I/O or processing-stage graph, sorting, labeling, or output work.

## File Map

- `src/lib.rs`: immutable input-stage context and boundary types.
- `src/xml.rs`: bounded pure-Rust XML tree parsing and VFS-only include expansion.
- `src/control.rs`: BCF 3.11 validation, options, sections, templates, and data model parsing.
- `src/config.rs`: configuration validation, typed values, templates, and precedence layers.
- `src/biblatexml.rs`: typed BibLaTeXML entries, names, dates, ranges, lists, aliases, and annotations.
- `src/tests.rs`: parser, validation, include, precedence, and adversarial-limit tests.
