# tex-out Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns committed output artifact data and its compact binary representation.

## Crate Role

`tex-out` sits downstream of the commit barrier. It defines the page artifact model, artifact-local font resources, output effects, node representations suitable for drivers, content hashing, and the versioned binary reader/writer for committed page artifacts. Shipout code in `tex-exec` lowers frozen engine nodes into these types; later drivers consume the serialized artifact bytes.

Use this crate for stable, driver-facing artifact structures and serialization concerns that should not depend on live engine state.

## File Map

- `AGENTS.md`: Crate-local guidance, boundaries, validation expectations, and this file map.
- `Cargo.toml`: Crate manifest declaring the `tex-out` package, `tex-arith` dependency, and workspace lint settings.
- `src/binary.rs`: Versioned compact binary writer/reader for page artifacts plus parse error types.
- `src/dvi.rs`: DVI writer entry point, shared writer state, error type, and submodule wiring.
- `src/dvi/extent.rs`: Page extent accounting for DVI postamble maximum dimensions.
- `src/dvi/fonts.rs`: DVI font selection, first-use font definitions, and postamble font definition emission.
- `src/dvi/framing.rs`: DVI file framing, page `bop`/`eop`, preamble/postamble, and byte emission helpers.
- `src/dvi/glue.rs`: TeX.web-style cumulative glue-set arithmetic and checked scaled-position helpers.
- `src/dvi/leaders.rs`: TeX.web hlist/vlist leader repetition loops for aligned, centered, expanded, rule, and degenerate leader cases.
- `src/dvi/movement.rs`: TeX.web-style DVI `movement()` lookback stack and w/x/y/z command optimization.
- `src/dvi/opcodes.rs`: Private DVI opcode and file unit constants shared by the writer modules and tests.
- `src/dvi/tests.rs`: Byte-level DVI writer tests for file structure, traversal, movement optimization, rules, fonts, glue, and specials.
- `src/dvi/traversal.rs`: TeX.web-style committed page traversal, hlist/vlist output, rules, specials, and movement synchronization.
- `src/hash.rs`: Stable 32-byte content hash type and deterministic byte hashing helpers.
- `src/lib.rs`: Crate documentation, module wiring, tests module registration, and public re-exports.
- `src/model.rs`: Detached page artifact, font resource, node, glue, kern, and output effect data model.
- `src/tests.rs`: Round-trip, deterministic byte/hash, and binary rejection tests for artifact serialization.

## Boundaries

- Do not depend on `tex-state` or `Universe`; artifact data must be detached from live stores.
- Do not add engine mutation, page-builder logic, or file effects here.
- Keep binary format changes explicit, versioned, and covered by round-trip tests.
- Use `tex-arith::Scaled` raw values consistently for serialized dimensions.

## Validation

Run `cargo test --tests -p tex-out` after model, hash, or binary-format changes. For shipout integration, also run the focused `tex-exec` or `umber` tests that create artifacts.
