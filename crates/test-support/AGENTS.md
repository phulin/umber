# test-support Guidance

Read the repository-level `AGENTS.md` before editing here. This crate contains host-side utilities for tests and fixture comparison; it is not part of the TeX engine runtime.

## Crate Role

`test-support` owns shared helpers used by workspace tests, especially committed corpus fixture assertions, fixture update behavior, normalized diagnostic/log comparison, and small parsers used to cross-check reference tool output. It may depend on ordinary host libraries such as `anyhow` and diffing utilities because it runs only in tests.

Keep reusable test harness code here when multiple crates or integration tests need the same fixture, normalization, or reference-comparison behavior. Keep crate-specific assertions near the crate that owns the behavior unless they are clearly shared.

## File Map

- `AGENTS.md`: crate-specific guidance, boundaries, validation notes, and this file map.
- `Cargo.toml`: crate manifest, host-side dependencies, `refexec` dev-dependency, and workspace lint settings.
- `src/lib.rs`: public fixture assertion helpers, TeX/reference log normalizers, and PL font parsing utilities.
- `src/tests.rs`: crate self-test that runs the hello corpus through reference TeX and checks the normalized log fixture.

## Boundaries

- Do not make engine crates depend on `test-support` outside dev-dependencies.
- Do not put production TeX logic here; helpers in this crate may normalize, compare, or parse expected data, but they should not become an alternate implementation of runtime behavior.
- Keep reference-tool assumptions explicit and isolated so parity tests remain understandable.

## Validation

Changes here usually need the tests that consume the helper plus `cargo test --tests -p test-support`. When fixture output changes, follow `tests/AGENTS.md` and use `UPDATE_FIXTURES=1` intentionally.
