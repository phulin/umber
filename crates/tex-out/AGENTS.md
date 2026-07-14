# tex-out Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns committed output artifact data and its compact binary representation.

## Crate Role

`tex-out` sits downstream of the commit barrier. It defines the page artifact model, artifact-local font resources, output effects, node representations suitable for drivers, content hashing, and the versioned binary reader/writer for committed page artifacts. Shipout code in `tex-exec` lowers frozen engine nodes into these types; later drivers consume the serialized artifact bytes.

Use this crate for stable, driver-facing artifact structures and serialization concerns that should not depend on live engine state.

## File Map

- `AGENTS.md`: Crate-local guidance, boundaries, validation expectations, and this file map.
- `Cargo.toml`: Crate manifest declaring shared arithmetic and content-identity dependencies.
- `src/binary.rs`: Versioned compact binary writer/reader, nested list and token streaming encoders/decoders, and parse error types.
- `src/dvi.rs`: Slice-compatible and incremental output-sink DVI APIs, one-page writer state, errors, and submodule wiring.
- `src/bin/texout-dvitype.rs`: Small host-side DVI disassembly binary for parity triage.
- `src/dvi/disasm.rs`: Bounded backpointer-graph validator and single-pass retained DVI command index/disassembler.
- `src/dvi/disasm/tests.rs`: Page-graph corruption, retained-index, disassembly, and command lookup tests.
- `src/dvi/extent.rs`: Page extent accounting for DVI postamble maximum dimensions.
- `src/dvi/fonts.rs`: Indexed page/global font selection, cross-page identity checks, first-use definitions, and postamble emission.
- `src/dvi/framing.rs`: Streaming DVI framing, page `bop`/`eop`, preamble/postamble, offsets, and one-page byte staging.
- `src/dvi/glue.rs`: TeX.web-style cumulative glue-set arithmetic and checked scaled-position helpers.
- `src/dvi/leaders.rs`: TeX.web hlist/vlist leader repetition loops for aligned, centered, expanded, rule, and degenerate leader cases.
- `src/dvi/movement.rs`: TeX.web-style DVI `movement()` lookback stack and w/x/y/z command optimization.
- `src/dvi/opcodes.rs`: Private DVI opcode and file unit constants shared by the writer modules and tests.
- `src/dvi/plan.rs`: Owned precompiled page bodies, scalar-event and v10-stream compilation, first-use font-definition relocations, and final plan assembly.
- `src/dvi/tests.rs`: Byte-level DVI writer tests for file structure, traversal, movement optimization, rules, fonts, glue, and specials.
- `src/dvi/traversal.rs`: TeX.web-style owned traversal plus the explicit-frame direct-emission state machine for boxes, rules, specials, glue, and movement synchronization.
- `src/html.rs`: deterministic coordinate-locked standalone HTML serializer, explicit web-font resolution, asset modes, escaping, and limits.
- `src/html/tests.rs`: deterministic-byte, exact-metadata, mapping-failure, and injection regression tests.
- `src/lib.rs`: Crate documentation, module wiring, tests module registration, and public re-exports.
- `src/model.rs`: Detached page artifact, font resource, node, glue, kern, and output effect data model.
- `src/positioned.rs`: public driver-neutral positioned-page event model and lowering API.
- `src/positioned/traversal.rs`: DVI-coordinate-equivalent box, glue, rule, leader, special, and browser-shaped text-run traversal.
- `src/positioned/tests.rs`: line-anchor, baseline, box-shift, rule, ligature, and kern-boundary coordinate tests.
- `src/tests.rs`: Round-trip, deterministic byte/hash, and binary rejection tests for artifact serialization.

## Boundaries

- Do not depend on `tex-state` or `Universe`; artifact data must be detached from live stores.
- Do not add engine mutation, page-builder logic, or file effects here.
- Keep binary format changes explicit, versioned, and covered by round-trip tests.
- Use `tex-arith::Scaled` raw values consistently for serialized dimensions.

## Validation

Run `cargo test --tests -p tex-out` after model, hash, or binary-format changes. For shipout integration, also run the focused `tex-exec` or `umber` tests that create artifacts.
