# bib-graph Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns
sourcemaps, dependency closure, aliases, relationships, inheritance, sets,
and data-model validation. Preserve declared and diagnostic order explicitly;
do not depend on other worker crates, `bib-engine`, hosts, or mutable globals.

## File Map

- `src/lib.rs`: public graph-stage exports.
- `src/maps.rs`: ordered sourcemap predicates and actions.
- `src/processor.rs`: bounded section closure, aliases, relationships, sets,
  inheritance, and cycle handling.
- `src/validation.rs`: post-inheritance data-model constraints.
- `src/tests.rs`: graph-stage contract and adversarial-limit tests.
