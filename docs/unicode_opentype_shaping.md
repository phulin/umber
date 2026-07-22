# Native Unicode and OpenType/TrueType Shaping

Status: text shaping and mapped TFM text implemented; remaining work is tracked in the
single linear `umber2-y2ei` plan. This document defines the engine-side shaping
architecture used by OpenType-only fonts and by TFM-style text selections that
the modern layout policy maps to OpenType resources.

**Scope.** The implemented shaping work targets modern HTML layout. The
shaping kernel remains backend-neutral—glyph IDs and positions in scaled
points, nothing HTML- or DOM-specific—but PDF glyph embedding is a separate
future concern and is not part of the linear HTML epic.

## Relationship to the existing OpenType resource architecture

`docs/web_font_bundles.md` (Beads epic `umber2-y2ei`) already defines font
_acquisition_: `FontRequest`/`ResolvedFont`, content-addressed identity, and a
validated `OpenTypeFont { cmap, metrics, shaping: ShapingTables, math,
metadata }` produced by `crates/tex-fonts/src/opentype/`. That work is done.
`tex-shape` now applies the validated face through rustybuzz and the
shape/break/reshape pipeline consumes its cluster advances.

OpenType math uses a separate direct path. `LoadedFont::math_metrics_source`
returns validated, size-bound MATH data when present and the explicit
`ClassicTfmExact` fallback otherwise. The math converter consumes native MATH
constants and glyph records (including `ssty`, italic correction, four-corner
math kern, and top-accent attachment) without synthesizing TeX's 22 math
fontdimens. Exact selected glyph ids remain in the backend-neutral math layout
arena. Variant selection and deterministic horizontal/vertical assembly are
now part of that layout; positioned HTML lowering is the remaining stage of
`umber2-y2ei.9`.

That document states its model explicitly: _"browser owns glyph selection,
advances, kerning, ligatures... inside a run"_ — it deliberately avoids an
engine-side shaper and accepts that HTML output cannot guarantee
glyph-coordinate equality inside a text run. This document amends that
decision for OpenType-backed fonts: the engine shapes text itself, using
`rustybuzz`, so line-breaking gets real widths, kerning, and ligatures instead
of an approximation. `ClassicTfmExact` keeps the existing byte-indexed
lig/kern automaton. Under `OpenTypePreferred`, an exact client-supplied mapping
may transparently route TFM-style text syntax through the same Unicode and
OpenType shaping path; that policy is specified in
`docs/web_font_bundles.md` and tracked by `umber2-y2ei.12`.

All work is tracked directly under `umber2-y2ei`. The former nested epic
`umber2-y2ei.11` is retained only as historical issue provenance.

## Shaping engine choice: rustybuzz, not harfbuzz-rs

The workspace forbids `unsafe_code` at the lint level, has no C toolchain
dependency today, and treats WebAssembly as a first-class target
(`crates/umber-wasm`, the HTML backend). Its only existing font dependencies,
`ttf-parser` and `woff2-patched`, are both pure Rust and already exercised in
the wasm32 build.

`rustybuzz` is a pure-Rust reimplementation of HarfBuzz's shaping algorithm
(used in production by Typst and resvg). It cross-compiles to wasm32 with no
extra toolchain and introduces no FFI boundary. Real `harfbuzz-rs` wraps the
C library: higher fidelity on rare edge cases, but it would be the first C
build dependency in the repository and would need a separate emscripten story
to keep working under WASM. Given this project's existing posture, rustybuzz
is the correct default. If a specific shaping divergence ever matters enough
to justify it, real HarfBuzz could be reintroduced later as an optional
native-only feature behind a trait boundary — but nothing in this plan
requires that up front.

## New crate: `crates/tex-shape`

A pure, backend-neutral shaping kernel, following the same shape as
`tex-typeset` (pure list-in/list-out, no `Universe`, no direct `tex-state`
dependency).

Depends on `tex-arith` (unit conversion), `tex-fonts` (validated OpenType
tables, cmap, metrics), `rustybuzz`, and Unicode itemization crates
(`unicode-bidi`, `unicode-script` or equivalent) for run segmentation. Does
not depend on `tex-state` or `tex-exec`.

Core shape of the API:

```rust
pub struct ShapedGlyph {
    pub glyph_id: u32,
    pub cluster: u32,       // offset into the source run; maps back to TeX chars
    pub x_advance: Scaled,
    pub y_advance: Scaled,
    pub x_offset: Scaled,
    pub y_offset: Scaled,
}

pub struct ShapedRun {
    pub glyphs: Vec<ShapedGlyph>,
    pub direction: Direction,
    pub script: Script,
}

pub fn shape_run(
    font: ShapingFont<'_>,
    text: &str,
    features: &FontFeaturePolicy,
    direction: Direction,
) -> ShapedRun;
```

`ShapingFont` is the validated OpenType program plus the size of its enclosing
`LoadedFont`; classic TFM records cannot produce one. Font-unit-to-scaled-point
conversion routes through `tex-arith`. A `rustybuzz::Face` is built only after
the bounded SFNT validation pass and cached with its owned decoded SFNT bytes
inside the OpenType record shared by `LoadedFont`, so shaping never decodes or
parses an untrusted transport object. Stage 2 also provides deterministic
script detection and first-strong base-direction detection for callers
preparing one run; full bidi run reordering remains Stage 5.

## Font-metrics abstraction split

`LoadedFont` now selects character metrics through `FontMetricsSource`.
Before Stage 1, OpenType data was only an identity sidecar and consumers
gated lookups on `u8::try_from(ch)`, so every codepoint above U+00FF was
reported missing even when the selected OpenType program had a cmap entry.

Introduce a small enum so callers stop assuming TFM:

```rust
pub enum FontMetricsSource {
    Tfm(FontMetrics),             // existing, unchanged, u8-indexed
    OpenType(OpenTypeFontShaped), // validated cmap/metrics; cached face follows in Stage 2
}
```

During Stage 1, `OpenTypeFontShaped` also retains the accompanying TFM tables
for classic-only lig/kern, math, and font-parameter enquiries. Character
existence, dimensions, packing, line-breaking, and artifact emission dispatch
to the OpenType cmap and advances. Stage 3 removes the need for that
compatibility TFM when it adds OpenType-only font selection and synthesized
fontdimens.

- TFM-selected fonts under `ClassicTfmExact` keep their exact current behavior:
  256-character cap, existing lig/kern automaton, and byte-identical parity
  fixtures. `OpenTypePreferred` may instead resolve a versioned mapping and
  route text through the OpenType variant.
- OpenType-selected fonts check the real `cmap` for character existence
  instead of `u8::try_from`, which alone fixes the false "missing character"
  behavior independent of shaping.
- `tex-out`'s `PageNode::Char { ch: u32, .. }` already carries a `u32`; only
  `validate_character()`'s `ch <= u8::MAX` clamp needs relaxing, and only for
  fonts whose `FontResource.opentype` is present. Classic-DVI output stays
  byte-clamped, correctly, since real DVI opcodes are bytes.

## `\font` semantics for OpenType-only fonts

Plain TeX and LaTeX macro packages depend on `\fontdimen` values that
classically come from a TFM's param section: interword space/stretch/shrink
(params 2-4), quad, extra space, and, for math fonts, the 22-parameter
symbol/extension arrays. A font selected from OpenType data alone has none of
this.

Explicit OpenType-only selection uses the syntax
`\font\name=opentype:<logical-name>`, followed by the ordinary optional
`at <dimension>` or `scaled <integer>` size clause. The prefix is part of the
engine syntax, not the logical resource name. An unprefixed `\font` remains
TFM-style document syntax. `ClassicTfmExact` resolves it only as TFM;
`OpenTypePreferred` may use an exact client mapping keyed by TFM identity and
recorded mapping policy. It never probes filenames or substitutes a platform
font by name.

The current mapping is **OpenType fontdimen synthesis version 1**, exposed as
`tex_fonts::OPENTYPE_FONTDIMEN_SYNTHESIS_VERSION`. At the selected size it is:

| Slot                       | Value                                                                |
| -------------------------- | -------------------------------------------------------------------- |
| `fontdimen1` (slant)       | zero                                                                 |
| `fontdimen2` (space)       | horizontal advance of the cmap-selected U+0020 glyph; zero if absent |
| `fontdimen3` (stretch)     | one half of `fontdimen2`, rounded to nearest scaled point            |
| `fontdimen4` (shrink)      | one third of `fontdimen2`, rounded to nearest scaled point           |
| `fontdimen5` (x-height)    | OS/2 `sxHeight`; zero when absent                                    |
| `fontdimen6` (quad)        | one em (the selected font size)                                      |
| `fontdimen7` (extra space) | zero                                                                 |

OpenType fonts have no intrinsic TeX design size, so an omitted size and
`scaled 1000` both use a 10pt nominal design size. The mapping is host-neutral
and computed only from validated cmap/metric metadata. A future mapping change
must increment the version constant and document the compatibility impact.

Math-font parameter synthesis is not part of text fontdimen mapping v1.
Assigning an OpenType-only font through `\textfont`, `\scriptfont`, or
`\scriptscriptfont` currently fails with an explicit capability error. The
first next stage, `umber2-y2ei.9`, adds a direct `MathMetricsSource` over the
OpenType MATH table rather than forcing its richer constants, kerns, variants,
and assemblies into TeX's 22 symbol/extension parameters.

`execute_font_definition` (`tex-exec/src/assignments/fonts.rs`) receives an
explicit `FontSource::OpenType` variant carrying the selected validated
`OpenTypeFont` and no TFM. The existing `FontSource::Tfm` variant may still
carry an OpenType program alongside TFM tables for compatibility selection.

## Shaping pipeline integration into horizontal-mode construction

This is the hardest part of the design.

This pipeline is now implemented. Horizontal construction retains Unicode
character nodes for output compatibility and places a font-kern adjustment at
the end of each shaped cluster. The character widths plus that adjustment are
exactly the cluster advance returned by `tex-shape`, so `tex-typeset` can keep
its zero-copy prefix-width traversal while consuming pass-1 shaped widths.
These adjustments are provisional: post-line-break processing removes and
rebuilds them by reshaping every materialized line independently.

**Run segmentation.** Maximal runs of the same font, direction, and script
are built while horizontal-mode list construction walks characters
(`tex-exec/src/assignments/hmode.rs`), analogous to the existing per-character
loop in `append_hchar` but batched per run before shaping.

**Two-pass width strategy for line-breaking.** Shaping is not concatenative:
kerning and ligation at a candidate break point depend on what sits on both
sides of it, but `tex-typeset`'s badness/line-break pass wants per-node
widths accumulated incrementally. The approach used in practice by XeTeX and
LuaTeX, and adopted here:

1. Shape each maximal run once, ignoring line breaks, to get provisional
   per-cluster advances. `tex-typeset`'s `widths.rs` consumes these directly
   for OpenType-backed fonts instead of TFM widths.
2. Once `tex-typeset` selects final breakpoints, reshape each output line's
   runs independently for final glyph output, so a ligature that would have
   spanned a chosen break point is correctly not formed.

In implementation terms, glue, explicit kerns, discretionaries, font changes,
and strong-script changes terminate a shaping run. Common and inherited
characters inherit the surrounding run script. The pass-2 materializer uses
the same segmentation rules, so list surgery cannot accidentally join text
across an explicit TeX boundary.

Badness estimates from pass 1 can differ very slightly from the pass-2
reshape at interior candidate breakpoints; this is a bounded, accepted
approximation, consistent with existing production engines, and is
documented here rather than silently claimed to be exact.

**Discretionary and hyphenation interaction.** `tex-state`'s hyphenation trie
is already `char`-based and Unicode-clean; no changes are needed there. The
new complication is that a hyphenation point can fall inside a shaped
cluster (for example, a ligature such as "ffi"). The resolution is to
suppress optional ligation across candidate hyphenation points before pass-1
shaping (a standard shaping feature-string toggle, not a custom hack), so
clusters never straddle a legal break, and to re-enable full ligation for
whichever side of a chosen break is not split during the pass-2 reshape.

**Complex scripts and bidi.** True right-to-left and reordering-script
support needs Unicode Bidi Algorithm-driven run reordering ahead of shaping.
This plan explicitly scopes that out as a later, separate stage: get
left-to-right Latin/Cyrillic/Greek/CJK working end-to-end through the
shape/break/reshape pipeline above first, then add bidi once that plumbing is
proven.

## Output side

HTML prose remains Unicode text plus the retained `@font-face`. Umber owns
shaping-informed line breaking and fixed run anchors; the browser rasterizes
the same font instance and may differ only in bounded subpixel ink placement
inside a run. It cannot reflow the line or move later events.

OpenType math uses a stronger positioned contract. Umber consumes MATH
constants, glyph information, italic corrections, math kern, accent
attachments, variants, and assemblies, then emits every math glyph and rule at
an engine-computed coordinate. Cmap-addressable selections use SVG text with
the original WOFF2. Glyph-id-only variants and assembly pieces use extracted
SVG outlines because HTML text APIs cannot request an arbitrary glyph id.
MathML does not own layout. This yields authoritative formula geometry and
glyph choice while accepting browser antialiasing differences for ordinary
font-rendered glyphs.

`ShapedGlyph` and the planned positioned-math glyph record remain
backend-neutral. OpenType PDF embedding may consume them later, but PDF work is
outside the linear HTML epic and does not block its release.

## Testing strategy

- `tex-shape` gets fixture-based unit tests: known input strings against a
  small set of pinned OpenType test fonts (reusing the existing CMU Serif
  WOFF2 fixture plus a couple of permissively licensed fonts exercising
  ligatures and mark attachment) produce known glyph-ID and advance
  sequences, generated once and committed. rustybuzz's output is
  deterministic and pinned by lockfile version, so no live shaping oracle is
  needed at test time.
- An optional, non-default script, following the existing
  `scripts/setup-*-tests.sh` pattern, can cross-check samples against real
  `hb-shape` when a C HarfBuzz happens to be available locally, to catch
  rustybuzz/HarfBuzz behavioral drift during development. This is never a
  CI dependency and does not compromise the no-C-toolchain default build.
- The existing WASM/browser HTML gate (`scripts/check-wasm.sh`) gains a case
  using non-Latin Unicode in explicit OpenType-only and mapped TFM-style text,
  verifying the same WOFF2 drives engine shaping and browser installation.
  DOM, font-load, artifact-identity, and coordinate assertions are normative;
  screenshots remain diagnostic.
- OpenType MATH fixtures cover scripts, fractions, radicals, accents,
  operators, delimiters, variants, assemblies, cmap-addressable positioned
  text, glyph-id-only outline fallback, and corrupt/cyclic assembly rejection
  across native, WASM, and browser output.

## Staged rollout

1. **Implemented.** Character-existence and width dispatch fix (font-metrics
   abstraction split above) — fixes the Unicode cmap/advance bug independent
   of shaping and keeps DVI's byte opcode boundary intact.
2. **Implemented.** `tex-shape` crate and rustybuzz integration: single-run
   shaping API, no line-break integration yet.
3. **Implemented.** OpenType-only `\font` path and fontdimen synthesis.
4. **Implemented.** Two-pass shape/linebreak/reshape integration into
   `tex-exec` and `tex-typeset` — the largest chunk of this plan.
5. **In progress.** Positioned OpenType MATH layout and HTML rendering
   (`umber2-y2ei.9`): formula geometry, variants, and assemblies are
   implemented; fixed positioned artifact/HTML lowering remains.
6. **Implemented.** OpenType-preferred mappings for TFM-style text
   (`umber2-y2ei.12`): exact TFM-identity bundle selection, explicit Unicode
   mapping, cluster-advance layout, synthesized text fontdimens, retained
   WOFF2 HTML reuse, and identity-bearing classic fallback.
7. Advanced instances, variations, and feature policy (`umber2-y2ei.8`).
8. Complex-script and bidi reordering (`umber2-y2ei.11.7`).
9. Full native/WASM/browser coverage (`umber2-y2ei.7`).
10. Superseded-path removal and release review (`umber2-y2ei.10`).

Each stage should land as its own coherent change with fixtures, per the
project's usual practice, rather than being split into smaller partial
fragments.
