# tex-out Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns committed output artifact data and its compact binary representation.

## Crate Role

`tex-out` sits downstream of the commit barrier. It defines the page artifact model, artifact-local font resources, output effects, node representations suitable for drivers, content hashing, and the versioned binary reader/writer for committed page artifacts. Shipout code in `tex-exec` lowers frozen engine nodes into these types; later drivers consume the serialized artifact bytes.

Use this crate for stable, driver-facing artifact structures and serialization concerns that should not depend on live engine state.

## File Map

- `AGENTS.md`: Crate-local guidance, boundaries, validation expectations, and this file map.
- `Cargo.toml`: Crate manifest declaring shared arithmetic and content-identity dependencies.
- `src/binary.rs`: Versioned compact binary writer/reader, nested list and token streaming encoders/decoders, and parse error types.
- `src/dvi.rs`: Slice-compatible and incremental output-sink DVI APIs, the private body compiler and file writer, one-page writer state, errors, and submodule wiring.
- `src/bin/texout-dvitype.rs`: Small host-side DVI disassembly binary for parity triage, enabled by the opt-in `dvi-tools` feature.
- `src/dvi/disasm.rs`: Bounded backpointer-graph validator and single-pass retained DVI command index/disassembler.
- `src/dvi/disasm/tests.rs`: Page-graph corruption, retained-index, disassembly, and command lookup tests.
- `src/dvi/fonts.rs`: Indexed page/global font selection, cross-page identity checks, first-use definitions, and postamble emission.
- `src/dvi/framing.rs`: Streaming DVI preamble/postamble, offsets, and one-page byte staging.
- `src/dvi/glue.rs`: TeX.web-style cumulative glue-set arithmetic and checked scaled-position helpers.
- `src/dvi/leaders.rs`: TeX.web hlist/vlist leader repetition loops for aligned, centered, expanded, rule, and degenerate leader cases.
- `src/dvi/movement.rs`: TeX.web-style DVI `movement()` lookback stack and w/x/y/z command optimization.
- `src/dvi/opcodes.rs`: Private DVI opcode and file unit constants shared by the writer modules and tests.
- `src/dvi/plan.rs`: The common page-plan currency, operation-local fresh-shipout co-emitter, owned and canonical-artifact adapters, first-use font-definition relocations, and final file assembly. The co-emitter consumes the artifact encoder's scalar events without retaining nodes; subtree-replaying leaders switch that operation to the canonical streaming-byte adapter.
- `src/dvi/tests.rs`: Byte-level DVI writer tests for file structure, traversal, movement optimization, rules, fonts, glue, and specials.
- `src/dvi/traversal.rs`: The sole explicit-frame DVI body traversal for boxes, rules, specials, glue, leaders, movement synchronization, and coordinate inspection.
- `src/geometry.rs`: Shared geometry authority for artifact ordinals, snap lookahead, checked coordinates, and exact leader placement.
- `src/html.rs`: deterministic coordinate-locked standalone serializer derived from the detached keyed render document, plus retained realized-program reuse, compatibility font validation, asset modes, escaping, and limits.
- `src/html/markup.rs`: bounded standalone HTML page/text/math/accessibility markup writer with direct numeric, code, hex, escaping, and byte-limit emission.
- `src/html/markup/tests.rs`: mixed exact-byte markup goldens and warmed allocation/copy controls.
- `src/html/incremental.rs`: the canonical detached `RenderDocument`, keyed render revisions, stable cross-revision identity reuse, ordered resources, and bounded artifact/positioned-page builders shared by full and incremental output.
- `src/html/incremental/digest.rs`: versioned canonical render hashing and key derivation.
- `src/html/incremental/patch.rs`: deterministic bounded typed diff planning from one canonical render revision to its successor.
- `src/html/tests.rs`: deterministic-byte, exact-metadata, mapping-failure, and injection regression tests.
- `src/lib.rs`: Crate documentation, module wiring, tests module registration, and public re-exports.
- `src/model.rs`: Detached page artifact, versioned font-layout/classic/OpenType identities, node, glue, kern, and output effect data model.
- `src/node_cursor.rs`: Canonical explicit-stack artifact node/list event order shared by codec emission and validation.
- `src/pdf.rs`: validated detached PDF object/page/resource graph, canonical ordering, and semantic identity.
- `src/pdf/finalization.rs`: complete host-neutral PDF finalization input, including committed pages/forms, realized fonts/programs, images, metadata/navigation, allocation state, and explicit limits.
- `src/pdf/finalize.rs`: pure detached form validation, artifact positioning, page/content lowering, font-object emission, object allocation, graph validation, and deterministic serialization.
- `src/pdf/graph.rs`: private canonical graph-role and nested-value cursor shared by validation, hashing, preflight, and serialization.
- `src/pdf/import.rs`: sole bounded pure selected-page PDF resource importer used by detached external-image lowering.
- `src/pdf/paint.rs`: private compact/ordered PDF paint program and shared graphics/text-state interpreter.
- `src/pdf/tests.rs`: PDF graph validation, canonical identity, and budget tests.
- `src/pdf/serialize.rs`: deterministic `pdf_writer` adapter, typed errors, version selection, and stream compression policy.
- `src/pdf/serialize/tests.rs`: exact-byte determinism, independent parsing, compression, and adapter-error tests.
- `src/pdf/vf.rs`: bounded recursive detached virtual-font packet lowering from exact local TFM transports.
- `src/positioned.rs`: public driver-neutral positioned-page event model and lowering API.
- `src/positioned/traversal.rs`: Explicit-frame positioned sink for DVI-equivalent box, glue, rule, leader, special, and browser-shaped text-run events.
- `src/positioned/tests.rs`: line-anchor, baseline, box-shift, rule, ligature, and kern-boundary coordinate tests.
- `src/tests.rs`: Round-trip, deterministic byte/hash, and binary rejection tests for artifact serialization.

## Boundaries

- Do not depend on `tex-state` or `Universe`; artifact data must be detached from live stores.
- Do not add engine mutation, page-builder logic, or file effects here.
- PDF finalization accepts only `PdfFinalizationInput`; adapters must expand
  engine token lists, read referenced files/artifacts, and acquire validated
  font/image resources before crossing into this crate.
- Keep binary format changes explicit, versioned, and covered by round-trip tests.
- Canonical artifact bytes remain the sole serialized page authority. A fresh
  shipout may retain one operation-local `DviPagePlan` sidecar co-emitted while
  the live node root is borrowed, but the sidecar must never serialize into the
  artifact, retain live engine handles, or introduce a second semantic node
  store.
- Use `tex-arith::Scaled` raw values consistently for serialized dimensions.

## Validation

Run `cargo test --tests -p tex-out` after model, hash, or binary-format changes. For shipout integration, also run the focused `tex-exec` or `umber` tests that create artifacts.
