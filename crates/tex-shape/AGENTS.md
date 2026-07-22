# tex-shape Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns
pure, backend-neutral Unicode/OpenType shaping kernels.

## Crate Role

`tex-shape` converts one immutable Unicode text run into positioned glyphs.
It consumes validated OpenType data from `tex-fonts`, uses `tex-arith` for
font-unit conversion, and carries no engine state or output-backend concepts.

## Boundaries

- Do not depend on `tex-state`, `tex-exec`, `Universe`, or output drivers.
- Keep APIs pure list-in/list-out and host-neutral.
- Shape only caller-delimited runs here; line breaking and run integration
  belong to later stages.
- Preserve rustybuzz byte cluster indices so glyphs map to source text.

## File Map

- `Cargo.toml`: crate metadata and shaping/itemization dependencies.
- `src/lib.rs`: public single-run shaping API and backend translations.
- `src/tests.rs`: deterministic fixture-based shaping tests.
- `tests/fixtures/`: pinned OFL fonts, licenses, and provenance for complex-script tests.

## Validation

Run `cargo test -q --tests -p tex-shape` and verify the crate for
`wasm32-unknown-unknown`. Use `scripts/check.sh` for rustfmt and clippy.
`scripts/check-hb-shape-fixtures.sh` is an optional local comparison with C
HarfBuzz; it skips successfully when `hb-shape` is unavailable and is not a
build or CI dependency.
