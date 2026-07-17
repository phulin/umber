# bib-unicode Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns
versioned immutable Unicode, encoding, collation, transliteration, and TeX
recode resources. It must remain deterministic and wasm-compatible: never
consult the host locale, browser internationalization APIs, native libraries,
the filesystem, or mutable global state.

## File Map

- `src/lib.rs`: pinned compatibility identity and immutable table boundary.
