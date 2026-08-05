# bib-output Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns
detached deterministic serializers. Serializers consume only a frozen
`ProcessedBibliography`, an explicit request, and immutable compatibility
tables; they must not read files/options or mutate processing state.

## File Map

- `src/lib.rs`: closed output protocol exports and immutable compatibility context.
- `src/bbl.rs`: bounded BBL 3.3 writer, value/name codecs, and typed failures.
- `src/bibtex.rs`: bounded BibTeX writer, presentation policy, and typed failures.
- `src/dot.rs`: bounded DOT graph writer, inclusion policy, provenance and relationship edges.
- `src/router.rs`: `OutputPlan`, frozen projection, unified failures/finalization, and closed deterministic dispatch.
- `src/xml.rs`: bounded BibLaTeXML/BBLXML writers and deterministic Relax NG schemas.
- `src/tests.rs`: exact whole-file, typed-value, encoding, newline, and limit tests.
