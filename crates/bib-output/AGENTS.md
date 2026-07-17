# bib-output Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns
detached deterministic serializers. Serializers consume only a frozen
`ProcessedBibliography`, an explicit request, and immutable compatibility
tables; they must not read files/options or mutate processing state.

## File Map

- `src/lib.rs`: serializer interface and immutable output context.
- `src/bbl.rs`: bounded BBL 3.3 writer, value/name codecs, and typed failures.
- `src/bibtex.rs`: bounded BibTeX writer, presentation policy, and typed failures.
- `src/tests.rs`: exact whole-file, typed-value, encoding, newline, and limit tests.
