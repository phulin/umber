# bib-label Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns
static/contextual label fields, hashes, name visibility, extra fields, and
uniqueness. It consumes immutable model and Unicode resources and may not
depend on sibling workers, hosts, or mutable global state.

## File Map

- `src/lib.rs`: immutable labeling-stage context boundary.
