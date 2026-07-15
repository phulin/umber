# test-support Guidance

Read the repository-level `AGENTS.md` before editing here. This crate contains host-side utilities for tests and fixture comparison; it is not part of the TeX engine runtime.

## Crate Role

`test-support` owns shared helpers used by workspace tests, especially committed corpus fixture assertions, normalized diagnostic/log comparison, DVI fixture setup/comparison, and small parsers used by regeneration tooling to cross-check reference tool output. It may depend on ordinary host libraries such as `anyhow` and diffing utilities because it runs only in tests and host tools.

Keep reusable test harness code here when multiple crates or integration tests need the same fixture, normalization, or reference-comparison behavior. Keep crate-specific assertions near the crate that owns the behavior unless they are clearly shared.

## File Map

- `AGENTS.md`: crate-specific guidance, boundaries, validation notes, and this file map.
- `Cargo.toml`: crate manifest, host-side dependencies, reference/DVI helper dependencies, and workspace lint settings.
- `src/compile_fail.rs`: Shared Cargo-check harness that gives each compile-fail fixture an independent temporary crate, points every crate at one reusable target directory, and checks stable stderr substrings.
- `src/corpus.rs`: shared committed-corpus discovery and support-file copy helpers.
- `src/dvi.rs`: shared DVI fixture setup and preamble-comment-normalized comparison helpers.
- `src/lib.rs`: public fixture assertion/read helpers, TeX/reference log normalizers, and PL font parsing utilities.
- `src/pdf.rs`: canonical PDF page/content plus document-object and dictionary structure normalizer.
- `src/tests.rs`: crate self-test that reads the committed hello fixture.

## Boundaries

- Do not make engine crates depend on `test-support` outside dev-dependencies.
- Do not put production TeX logic here; helpers in this crate may normalize, compare, or parse expected data, but they should not become an alternate implementation of runtime behavior.
- Keep reference-tool assumptions explicit and isolated in `scripts/regen-fixtures.sh`
  and tooling, not in cargo tests.

## Validation

Changes here usually need the tests that consume the helper plus `cargo test --tests -p test-support`. When fixture output changes, follow `tests/AGENTS.md` and regenerate with `scripts/regen-fixtures.sh`.
