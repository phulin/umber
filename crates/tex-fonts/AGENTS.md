# tex-fonts Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns immutable font metric data and TFM parsing.

## Crate Role

`tex-fonts` parses classic TeX TFM files and exposes backend-neutral, immutable font metric records used by state, execution, and typesetting code. It owns TFM table structures, lig/kern programs, extensible recipes, font parameters, content hashes, loaded-font wrappers, and conversions from raw TFM data into scaled metrics.

Use this crate for font-domain parsing and metric representation that does not require live engine state. Keep the parsed data reusable by future output backends and layout code.

## Boundaries

- Do not depend on `tex-state`; state stores loaded font records, but font parsing must remain independent of the live engine.
- Keep host file I/O outside this crate's core parsing APIs; callers should provide bytes or already-loaded content.
- Put TeX arithmetic conversions through `tex-arith` so TFM scaling and scanner arithmetic stay consistent.
- Do not mix output-driver concerns into font metrics.

## File Map

- `AGENTS.md`: crate-specific guidance for future agents working on `tex-fonts`.
- `Cargo.toml`: crate manifest, dependencies, and package metadata for `tex-fonts`.
- `src/lib.rs`: public module wiring and re-exports for font metric and TFM APIs.
- `src/metrics.rs`: immutable loaded-font records, selected OpenType artifact bindings, and backend-neutral metric query types.
- `src/pdf_encoding.rs`: host-neutral parsing of named 256-entry PostScript encoding vectors.
- `src/pdf_map.rs`: host-neutral pdfTeX/dvips map directive and entry parsing; logical resource names only.
- `src/pdf_pk.rs`: bounded host-neutral PK bitmap font decoding, normalized glyph masks, and content identity.
- `src/pdf_truetype.rs`: validated SFNT bytes and PDF descriptor metrics normalized through `ttf-parser`.
- `src/pdf_vf.rs`: bounded host-neutral TeX VF parsing, typed packet programs, local-font declarations, and recursion metadata.
- `src/pdf_vf/tests.rs`: hermetic VF grammar, command, malformed-input, and configured-bound tests.
- `src/opentype/`: validated OpenType resource contracts, canonical identities, bounded SFNT/WOFF2 decoding, immutable metric/cmap/table projections, and cached rustybuzz faces.
- `src/opentype/math.rs`: strict bounded OpenType MATH parsing and immutable constants, glyph-info, kern, variant, construction, and assembly records.
- `src/opentype/math/tests.rs`: synthetic complete-table and malformed MATH validation tests.
- `src/tests.rs`: crate-internal test module declarations for TFM parsing and cross-checks.
- `src/tests/metrics_validation.rs`: Detached metric capacity/reference validation and runtime lig/kern cursor boundary tests.
- `src/tests/tfm_parse.rs`: unit tests and helpers for parsing fixtures, metrics conversion, and malformed TFM validation.
- `src/tfm/error.rs`: structured TFM parse error variants and display messages.
- `src/tfm/mod.rs`: TFM module boundary and public re-exports.
- `src/tfm/parse.rs`: binary TFM parser, table decoding, scaling, and validation logic.
- `src/tfm/types.rs`: parsed TFM data structures and conversions into backend-neutral metrics.
- `src/type1.rs`: bounded PFB segment decoding into identity-keyed PDF-ready Type-1 program bytes.
- `tests/fixtures/cm/cmex10.tfm`: Computer Modern extension font fixture with extensible recipes.
- `tests/fixtures/cm/cmmi10.tfm`: Computer Modern math italic font fixture.
- `tests/fixtures/cm/cmr10.tfm`: Computer Modern roman font fixture.
- `tests/fixtures/cm/cmsy10.tfm`: Computer Modern math symbol font fixture.
- `tests/fixtures/cm/cmtt10.tfm`: Computer Modern typewriter font fixture.
- `tests/fixtures/edge/boundary-char.tfm`: edge-case TFM fixture covering boundary-character lig/kern behavior.
- `tests/fixtures/edge/ptmr8g-longjump.tfm`: edge-case TFM fixture covering long lig/kern jump encodings.
- `tests/fixtures/README.md`: provenance and regeneration details for OpenType fixtures.
- `tests/fixtures/stix-two-math.woff2`: pinned SIL-OFL STIX Two Math container-equivalence fixture.
- `tests/fixtures/stix-two-math.LICENSE.txt`: upstream license for the STIX fixture.

## Validation

Run `cargo test --tests -p tex-fonts` after changes. Parser or metric-shape changes should keep the TFM fixture tests passing. Run `scripts/regen-fixtures.sh --area fonts` for the explicit live `tftopl` cross-check.
