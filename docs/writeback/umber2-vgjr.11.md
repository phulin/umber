# umber2-vgjr.11 — canonical font runtime closeout

Closeout base: `357d997cc3f4e79397233c8651e3a85786b5875a`.

## Surviving authorities

`tex-fonts::FontMetrics` is the sole retained classic runtime metric graph.
The TFM parser keeps raw character, lig/kern, kern, and extensible records only
through validation, then publishes canonical metrics plus the required header,
selected-size, padded-parameter, and `font_info_words` metadata through
`TfmFont`. Executor and virtual-font loading share its one consuming
`into_loaded_font` constructor. The public raw TFM graph and later conversion
boundary are deleted.

OpenType MATH has one strict eager validation walk and no retained owned graph.
`OpenTypeFont` retains canonical decoded SFNT bytes and validated MATH
presence; `OpenTypeMathMetrics` borrows `ttf_parser::math::Table` for lazy,
consumer-sized scaled queries. The former twelve public owned MATH projection
types are deleted. The detached positioned `MathOutputEvent` is chosen layout,
not another font-table representation.

`LoadedFont::realized_identity` is the sole host-neutral selected-font digest.
It binds metrics, size, layout and fallback policy, mapping, program, instance,
and generated ancestry. `OpenTypeFont::instance_identity` owns the complete
OpenType instance projection. `PdfFontResourceIdentity` remains intentionally
narrower so pdfTeX can reuse an equal TFM/program subset across sizes. Schema
DTOs and DVI, PDF, and HTML keys retain their independent serialization,
validation, allocation-order, or lookup obligations.

The session retains the same validated `OpenTypeFont` used for layout and HTML
painting. Native OTF, TTF, TTC, and OTC transport and decoded views share one
`Arc`; WOFF2 correctly keeps distinct compressed transport and decoded SFNT
allocations. Identity is checked before ownership moves, and reuse clones the
validated transport and decoded allocations without decoding again.

## Compatibility boundary

`FontSourceIdentity` and `LoadedFont::source_identity` remain exact public
compatibility spellings for `RealizedFontIdentity`. Schema 23 font resources,
the public `HtmlFontAssets::realized_opentype` default, public `TfmFont`
metadata, and detached PDF metric and VF inputs also remain. Removing or
renaming them would be a Rust API or artifact-schema decision, not a safe
behavior-preserving cleanup. The audit found no further duplicate retained
model or repeated active-path decode that can be removed within Program 11.

## Exact accounting

Production Rust accounting by authority migration is:

| Category                                   | Additions | Deletions |      Net |
| ------------------------------------------ | --------: | --------: | -------: |
| Canonical TFM runtime                      |       183 |       242 |      -59 |
| Strict validation and lazy MATH queries    |       272 |       456 |     -184 |
| Realized identity and selected-face repair |       248 |       146 |     +102 |
| Shared native SFNT storage                 |        11 |        11 |        0 |
| **Program 11 production total**            |   **714** |   **855** | **-141** |

The storage audit additionally adds eight proof-test lines and deletes none.
No moved code, generated source, documentation, compatibility surface, or
binary asset is credited. The 800--1,200-line forecast is reconciled to the
measured 141-line reduction, a 659--1,059-line shortfall. No further reduction
is scheduled or silently transferred to another program.

## Verification

Fresh closeout verification used `CARGO_BUILD_JOBS=6`, finite timeouts, and no
overlap within slot-1. Focused `tex-fonts` and complete native
`cargo test -q --tests --no-run` builds passed uncapped. All 79 `tex-fonts`
tests passed under `MemoryMax=512M`; the complete routine suite passed under
`MemoryMax=1G`. A directly built TFM fuzz binary completed 10,000 inputs under
`MemoryMax=512M`; the earlier coverage-instrumented child gate also completed
10,000 inputs.

Under `MemoryMax=1G`, the wasm32 check, Biome over 40 files, all 89 authored
Node tests, release package construction through `wasm-opt`, the built-package
Node project consumer, and `npm pack --dry-run` passed. Firefox and Chrome were
absent, so wasm-bindgen and browser smoke were unavailable and are not reported
as passes. The final exact-tree `scripts/check.sh` run passed all four gates
with six Cargo jobs under `MemoryMax=1G`.
