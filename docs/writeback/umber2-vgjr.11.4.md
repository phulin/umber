# umber2-vgjr.11.4 — canonical font runtime forecast reconciliation

Audit base: `08036ceba9bbc3090dcb5d7bf29c1a677a212a80`.

Program 11 has one runtime authority for each promised font fact, but the
surviving format and phase boundaries are intentionally not one Rust struct.
The original 800--1,200-line production forecast is retired at the measured
result. It inferred net deletion from gross predecessor sizes and understated
the strict validation, projection, detached-output, and compatibility code
that had to survive.

## Retained source inventory

- Classic metrics: `crates/tex-fonts/src/tfm/parse.rs` keeps private encoded
  character, lig/kern, kern, and extensible records only through validation.
  `crates/tex-fonts/src/metrics.rs::FontMetrics` is the only retained runtime
  metric graph. `crates/tex-fonts/src/tfm/types.rs::TfmFont` is the public
  successful-parser value containing canonical metrics plus TFM header,
  selected size, parameter, and `font_info_words` metadata. The executor and
  VF paths consume its one `into_loaded_font` constructor. Fixturegen also
  reads the header and parameter metadata for live `tftopl` comparison, so it
  is neither a second metric graph nor dead production API.
- Frozen and detached classic projections:
  `crates/tex-state/src/stores/format.rs::FormatFont` is the schema-bound
  format DTO, not live metric authority. `tex-out::FontResource` is the
  schema-23 committed artifact binding. `PdfFontMetricsInput` and
  `PdfVirtualFontInput` in `crates/tex-out/src/pdf/finalization.rs` are public
  detached-finalization inputs. The in-tree finalizer no longer reads the
  latter two compatibility payloads after Umber performs VF lowering and
  derives final metrics, but removing their public fields or constructors is
  an unapproved Rust API contraction. They are not credited as safe deletion.
- OpenType MATH: `crates/tex-fonts/src/opentype/math.rs` owns only the strict
  bounded validation walk. `OpenTypeFont` retains canonical decoded SFNT bytes
  and a validated-presence bit. `OpenTypeMathMetrics` in `metrics.rs` borrows
  `ttf_parser::math::Table` lazily and returns only consumer-sized projected
  values. `tex-out::MathOutputEvent` is the detached positioned artifact
  chosen by typesetting; HTML's retained SFNT view reproduces glyphs and
  outlines from that artifact. None is a second retained MATH graph.
- OpenType program: `crates/tex-fonts/src/opentype/parse.rs::OpenTypeFont`
  owns the canonical validated cmap, metrics, shaping tables and face,
  metadata, decoded SFNT, and exact transport. `VirtualCompileSession` retains
  those values by request key and keeps only response metadata beside them.
  The HTML resolver borrows the same program. The audit found one real byte
  duplication: native SFNT transport and decoded data were identical but used
  separate allocations. Commit `a5df75c73` shares one `Arc` for OTF, TTF, TTC,
  and OTC while preserving separate WOFF2 transport and decoded allocations.
- Identity domains: `FontObjectIdentity`, `FontProgramIdentity`, and
  `FontInstanceIdentity` in `opentype/contract.rs` respectively bind supplied
  bytes, a canonical decoded face, and the complete selected instance.
  `RealizedFontIdentity` in `metrics.rs` additionally binds TFM metrics, size,
  layout and fallback policy, mapping, and generated ancestry.
  `PdfFontResourceIdentity` is intentionally narrower so pdfTeX can reuse an
  equal TFM/program subset across sizes. State's font hash fragments and exact
  immutable identities bind rollback slots and handles, not distribution or
  output resources. These domains cannot be merged without changing cache,
  rollback, subset-reuse, or artifact semantics.
- Output projections: schema-23 `FontResource` and `OpenTypeFontResource`
  repeat identity digests with their reconstructible inputs so detached
  validation can reject inconsistent artifacts. DVI's private `FontKey`
  combines the realized identity with the fields emitted by `fnt_def`.
  `PdfFontResourceRecord` maps live aliases to the narrower PDF object key and
  preserves object allocation order. `HtmlFontKey` is the public exact asset
  lookup key. These are format-specific keys, not competing font authorities.

## Compatibility decisions

`FontSourceIdentity` and `LoadedFont::source_identity` remain exact public
compatibility spellings for `RealizedFontIdentity`. Artifact schema 23 and its
binary codec continue to use that spelling and retain all identity inputs.
The default `HtmlFontAssets::realized_opentype` method remains so external
implementations compile unchanged; only those implementations use the
single-decode fallback. Public `TfmFont` header/parameter queries and the
detached PDF input structs likewise remain. Deleting or renaming any of these
surfaces requires an explicit API/schema decision and is not a
behavior-preserving repository cleanup.

## Accounting and forecast

The three completed authority migrations contributed production Rust
+183/-242 for canonical TFM, +272/-456 for lazy MATH, and +248/-146 for
realized font/output identity. The storage audit contributes +11/-11
production Rust and +8/-0 proof tests. Exact Program 11 production accounting
is therefore +714/-855, net -141. No moved code, generated source,
documentation, compatibility surface, or binary asset is credited.

The original 800--1,200-line production reduction is revised to the measured
141 lines, a 659--1,059-line shortfall. No further production reduction is
scheduled under Program 11. The retained boundaries above have independent
semantic or compatibility obligations; future retirement must be authorized
and accounted as a new change.

## Verification

The exact implementation built with `CARGO_BUILD_JOBS=6`: the focused
`tex-fonts` build and complete native `cargo test -q --tests --no-run` build
passed uncapped. All 79 focused font tests passed under `MemoryMax=512M`, and
the complete native routine suite passed under `MemoryMax=1G`.

Under `MemoryMax=1G`, the wasm32 check, Biome, all 89 authored Node tests, the
built-package Node project consumer, release package construction through
`wasm-opt`, and `npm pack --dry-run` passed. The first cold package build
exceeded the cap, so the exact package was built once uncapped and the capped
gate was repeated successfully through package construction. The wasm-bindgen
browser test was blocked by absent Firefox; the browser smoke reached the
built package and then failed only because `/usr/bin/google-chrome` is absent.
The final exact-tree `scripts/check.sh` run passed all four gates with six Cargo
jobs under `MemoryMax=1G`.
