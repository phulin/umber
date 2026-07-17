# bib-unicode Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns
versioned immutable Unicode, encoding, collation, transliteration, and TeX
recode resources. It must remain deterministic and wasm-compatible: never
consult the host locale, browser internationalization APIs, native libraries,
the filesystem, or mutable global state.

## File Map

- `src/lib.rs`: pinned compatibility identity and immutable table boundary.
- `src/annotations.rs`: ordered annotation values and replacement/merge rules.
- `src/collation.rs`: pinned root collation identity and deterministic keys.
- `src/date.rs`: bounded extended-date, range, season, script-digit, and time parsing.
- `src/encoding.rs`: labeled legacy byte codecs with explicit failures.
- `src/langtag.rs`: bounded BCP-47 language-tag parsing.
- `src/recode.rs`: base, full, and null TeX recoding sets.
- `src/transliteration.rs`: pinned transliteration schemes.
- `src/utils.rs`: compatibility hashes, normalization, ranges, and string helpers.
- `src/tests.rs`: malformed/bounded and pinned-identity unit tests.
