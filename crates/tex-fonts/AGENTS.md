# tex-fonts Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns immutable validated font data, TFM parsing, and OpenType shaping.

## Crate Role

`tex-fonts` parses classic TeX TFM and modern OpenType resources and exposes backend-neutral, immutable font records used by state, execution, typesetting, and output code. Raw TFM tables exist only during structural and reference validation; successful parsing publishes the canonical `FontMetrics` representation and the metadata needed to construct `LoadedFont`. The crate also owns font parameters, content hashes, validated OpenType programs and instances, shaping contexts and operations, and conversions from font units into scaled metrics.

Use this crate for font-domain parsing, metric representation, and pure caller-delimited shaping that does not require live engine state. Keep the validated data reusable by output backends and layout code.

## Boundaries

- Do not depend on `tex-state`; state stores loaded font records, but font parsing must remain independent of the live engine.
- Keep host file I/O outside this crate's core parsing APIs; callers should provide bytes or already-loaded content.
- Put TeX arithmetic conversions through `tex-arith` so TFM scaling and scanner arithmetic stay consistent.
- Do not mix output-driver concerns into font metrics or shaping.
- Shape only caller-delimited runs here; line breaking and run integration belong to execution and typesetting.
- Preserve rustybuzz UTF-8 byte clusters so shaped glyphs map to source text.

## File Map

- `AGENTS.md`: crate-specific guidance for future agents working on `tex-fonts`.
- `Cargo.toml`: crate manifest, dependencies, and package metadata for `tex-fonts`.
- `src/lib.rs`: public module wiring and re-exports for font metric and TFM APIs.
- `src/metrics.rs`: immutable loaded-font records, canonical realized and PDF-resource identities, versioned classic/mapped layout policy and encoding identity, selected OpenType artifact bindings, and backend-neutral metric query types.
- `src/pdf_encoding.rs`: host-neutral parsing of named 256-entry PostScript encoding vectors.
- `src/pdf_map.rs`: host-neutral pdfTeX/dvips map directive and entry parsing; logical resource names only.
- `src/pdf_pk.rs`: bounded host-neutral PK bitmap font decoding, normalized glyph masks, and content identity.
- `src/pdf_truetype.rs`: validated SFNT bytes and PDF descriptor metrics normalized through `ttf-parser`.
- `src/pdf_vf.rs`: bounded host-neutral TeX VF parsing, typed packet programs, local-font declarations, and recursion metadata.
- `src/pdf_vf/tests.rs`: hermetic VF grammar, command, malformed-input, and configured-bound tests.
- `src/shaping.rs`: private rustybuzz adapter and public typed single-run shaping values and operation.
- `src/shaping/tests.rs`: deterministic fixture-based shaping tests.
- `src/opentype/`: validated OpenType resource contracts, canonical identities, bounded SFNT/WOFF2 decoding, immutable metric/cmap projections, strict eager MATH validation, lazy borrowed MATH queries, and cached rustybuzz faces.
- `src/opentype/variation.rs`: bounded `fvar` axis/named-instance parsing and canonical instance resolution.
- `src/opentype/math.rs`: strict bounded OpenType MATH validation plus the public constant selector used by the scaled metrics facade; `ttf-parser` remains the sole query-time graph.
- `src/opentype/math/tests.rs`: synthetic malformed-graph and budget validation tests.
- `src/tests.rs`: crate-internal test module declarations for TFM parsing and cross-checks.
- `src/tests/metrics_validation.rs`: Detached metric capacity/reference validation and runtime lig/kern cursor boundary tests.
- `src/tests/tfm_parse.rs`: unit tests and helpers for direct canonical TFM metric construction, loaded-font construction, fixtures, and malformed TFM validation.
- `src/tfm/error.rs`: structured TFM parse error variants and display messages.
- `src/tfm/mod.rs`: TFM module boundary and public re-exports.
- `src/tfm/parse.rs`: binary TFM parser, temporary raw-table decoding, scaling, reference validation, and direct canonical metric construction.
- `src/tfm/types.rs`: retained TFM metadata, parameters, canonical metrics, and the single parsed-TFM-to-loaded-font constructor.
- `src/type1.rs`: bounded PFB segment decoding and pdfTeX-compatible cleartext/private-dictionary/CharStrings subsetting into identity-keyed PDF-ready Type-1 program bytes.
- `tests/fixtures/cm/cmex10.tfm`: Computer Modern extension font fixture with extensible recipes.
- `tests/fixtures/cm/cmmi10.tfm`: Computer Modern math italic font fixture.
- `tests/fixtures/cm/cmr10.tfm`: Computer Modern roman font fixture.
- `tests/fixtures/cm/cmsy10.tfm`: Computer Modern math symbol font fixture.
- `tests/fixtures/cm/cmtt10.tfm`: Computer Modern typewriter font fixture.
- `tests/fixtures/edge/boundary-char.tfm`: edge-case TFM fixture covering boundary-character lig/kern behavior.
- `tests/fixtures/edge/ptmr8g-longjump.tfm`: edge-case TFM fixture covering long lig/kern jump encodings.
- `tests/fixtures/README.md`: provenance and regeneration details for OpenType fixtures.
- `tests/fixtures/shaping/`: pinned OFL fonts, licenses, provenance, and optional C HarfBuzz comparison receipt.
- `tests/fixtures/stix-two-math.woff2`: pinned SIL-OFL STIX Two Math container-equivalence fixture.
- `tests/fixtures/stix-two-math.LICENSE.txt`: upstream license for the STIX fixture.

## Validation

Run `cargo test --tests -p tex-fonts` after changes and verify the crate for `wasm32-unknown-unknown`. Parser or metric-shape changes should keep the TFM fixture tests passing. Run `scripts/regen-fixtures.sh --area fonts` for the explicit live `tftopl` cross-check. `scripts/check-hb-shape-fixtures.sh` compares the committed mark and conjunct fixtures with C HarfBuzz when that optional tool is available.
