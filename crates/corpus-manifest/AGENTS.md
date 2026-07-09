# corpus-manifest Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns the dependency-free parser and validator for `tests/corpus-manifest.txt`, which is consumed by host-side corpus acquisition and parity tooling.

## Boundaries

- Keep this crate free of third-party dependencies.
- Keep the format line-oriented and simple enough to parse with `std` only.
- Validate manifest safety here so all consumers reject unknown fields, duplicate fields, missing required fields, invalid SHA-256 values, unsupported URL schemes, and unsafe document names consistently.
- Do not add TeX engine runtime behavior here; this crate is host-side manifest infrastructure.

## Validation

Run `cargo test -p corpus-manifest --tests` after parser or format changes. If consumer behavior changes, also run the affected tool tests for `parity-harness` and `tools/corpus-sync`.
