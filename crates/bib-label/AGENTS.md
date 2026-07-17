# bib-label Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns
static/contextual label fields, hashes, name visibility, extra fields, and
uniqueness. It consumes immutable model and Unicode resources and may not
depend on sibling workers, hosts, or mutable global state.

## File Map

- `src/lib.rs`: immutable labeling-stage context boundary.
- `src/labels.rs`: labelname/date/title selection and labelalpha templates.
- `src/hashes.rs`: normalized full, visible, raw, and per-name hashes.
- `src/extras.rs`: ordered extraalpha/date/title/titleyear collision passes.
- `src/uniqueness.rs`: per-list name, title, work, and primary-author uniqueness.
- `src/tests.rs`: crate-internal label-stage regression tests.
