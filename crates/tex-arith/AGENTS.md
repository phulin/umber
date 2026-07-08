# tex-arith Guidance

Read the repository-level `AGENTS.md` before editing here. This crate is the shared arithmetic substrate for TeX-compatible fixed-point values and conversion routines.

## Crate Role

`tex-arith` owns arithmetic-only TeX data types and helper routines: scaled-point values, TeX dimension limits, physical-unit ratios, font-size specifications, `xn_over_d`/`x_over_n` style fixed-point operations, badness-adjacent arithmetic helpers, and TFM metric conversion math. It is intentionally small and dependency-free so scanners, state storage, font parsing, and typesetting kernels can share exact TeX arithmetic without creating dependency cycles.

Prefer putting exact TeX.web arithmetic here when it is domain-neutral and reused by multiple crates. Port rounding, overflow, sentinel values, and error cases exactly; approximations here tend to cause late parity failures.

## File Map

- `AGENTS.md`: crate-local guidance, boundaries, validation notes, and this file map.
- `Cargo.toml`: package metadata for the dependency-free `tex-arith` crate.
- `src/lib.rs`: public scaled-point types, dimension/unit conversions, arithmetic helpers, TFM conversion math, and module tests hook.
- `src/scaled_tests.rs`: crate-internal tests for scaled arithmetic, unit conversion, rounding, overflow, and TFM sizing behavior.

## Boundaries

- Do not add dependencies on state, lexing, expansion, font loading, execution, I/O, or drivers.
- Do not hide TeX range checks in callers when the invariant belongs to the arithmetic operation itself.
- Keep APIs value-oriented and deterministic; this crate should have no access to mutable engine state or host effects.

## Validation

Run `cargo test --tests -p tex-arith` for local changes. For arithmetic that affects scanners, fonts, packing, or state parameters, also run the dependent crate tests that exercise the changed operation.
