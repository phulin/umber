# bib-model Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns the
versioned bibliography domain model. Public semantic values are immutable;
construction and mutation belong only to validating builders. Preserve
observable order explicitly and do not add host I/O, global mutable state, or
dependencies on bibliography worker crates.

## File Map

- `src/lib.rs`: public exports and compatibility version contract.
- `src/identifier.rs`: validated semantic identifiers.
- `src/value.rs`: typed field values and ordered field storage.
- `src/source.rs`: source locations and derived-value provenance.
- `src/options.rs`: immutable option layers and precedence resolution.
- `src/diagnostic.rs`: structured ordered diagnostics.
- `src/document.rs`: entries, lists, sections, frozen documents, and builders.
- `src/tests.rs`: crate-internal contract and invariant tests.
