# bib-output Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns
detached deterministic serializers. Serializers consume only a frozen
`ProcessedBibliography`, an explicit request, and immutable compatibility
tables; they must not read files/options or mutate processing state.

## File Map

- `src/lib.rs`: serializer interface and immutable output context.
