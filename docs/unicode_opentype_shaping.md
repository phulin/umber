# Native Unicode and OpenType/TrueType Shaping

Status: proposed design, not yet implemented. This document amends the
shaping-ownership decision in `docs/web_font_bundles.md` for OpenType-selected
fonts and defines the engine-side shaping architecture that supersedes its
Stage 7.

## Relationship to the existing OpenType resource architecture

`docs/web_font_bundles.md` (Beads epic `umber2-y2ei`) already defines font
*acquisition*: `FontRequest`/`ResolvedFont`, content-addressed identity, and a
validated `OpenTypeFont { cmap, metrics, shaping: ShapingTables, math,
metadata }` produced by `crates/tex-fonts/src/opentype/`. That work is done.
`ShapingTables` retains raw `gdef`/`gsub`/`gpos` table bytes but nothing
applies them; no shaping engine exists.

That document states its model explicitly: *"browser owns glyph selection,
advances, kerning, ligatures... inside a run"* — it deliberately avoids an
engine-side shaper and accepts that HTML output cannot guarantee
glyph-coordinate equality inside a text run. This document amends that
decision for OpenType-selected fonts: the engine shapes text itself, using
`rustybuzz`, so that line-breaking gets real widths, kerning, and ligatures
instead of an approximation, and so that any future glyph-exact output
backend has an authoritative shaped-glyph stream to consume. TFM-selected
fonts are entirely unaffected by this document; they keep the existing
byte-indexed lig/kern automaton.

This is tracked as a continuation of `umber2-y2ei` (its Stage 7 description —
"add advanced OpenType text support... mark positioning, and the supported
shaping boundary" — is superseded by the design below) rather than a new
epic.

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
    font: &OpenTypeFont,
    text: &str,
    features: &FontFeaturePolicy,
    direction: Direction,
) -> ShapedRun;
```

Font-unit-to-scaled-point conversion routes through `tex-arith`, not a local
rounding routine — today `OpenTypeMetrics::units_to_sp` in `tex-fonts`
performs its own rounding independent of `tex-arith`; unifying that is part
of this work. A `rustybuzz::Face` is built from the already-validated SFNT
bytes and cached alongside `LoadedFont`, so no second untrusted parse pass is
introduced.

## Font-metrics abstraction split

Today `LoadedFont` requires TFM metrics unconditionally and treats OpenType
data as an identity-only sidecar (`LoadedFont::with_opentype`). Every
consumer gates character lookups on `u8::try_from(ch)` before touching
metrics — in `tex-exec`'s `append_hchar` and in `tex-typeset`'s
`linebreak/widths.rs`. That cast is the single choke point enforcing 8-bit
semantics everywhere downstream, and it means any codepoint above U+00FF is
unconditionally treated as "missing" even when an attached OpenType font has
a real cmap entry for it.

Introduce a small enum so callers stop assuming TFM:

```rust
pub enum FontMetricsSource {
    Tfm(FontMetrics),             // existing, unchanged, u8-indexed
    OpenType(OpenTypeFontShaped), // new: metrics + cached rustybuzz face
}
```

- TFM-selected fonts keep their exact current behavior: 256-character cap,
  existing lig/kern automaton, no DVI/TRIP/e-TRIP fixture risk.
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

This plan synthesizes a documented, versioned mapping from OpenType metrics
to the classic fontdimen slots when a font is selected without a TFM:
interword space from the space glyph's advance, stretch/shrink as a
documented fraction of it, quad and x-height from `hhea`/OS/2 fields already
extracted into `FontMetadata`. Math-font parameter synthesis (the MATH table)
is out of scope here and left as a seam for the existing Stage 8 (OpenType
math support) work.

`execute_font_definition` (`tex-exec/src/assignments/fonts.rs`) currently
requires TFM bytes unconditionally. This plan extends `FontSource` with a
variant carrying only a `ResolvedFont`/`OpenTypeFont` and no TFM, selected
through new `\font` syntax that distinguishes a TFM font from an
OpenType-only one, rather than continuing to bolt OpenType on as a sidecar to
a TFM-required path.

## Shaping pipeline integration into horizontal-mode construction

This is the hardest part of the design.

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

HTML rendering (`tex-out/src/html.rs`) keeps its current contract: Unicode
text plus `@font-face`, browser shapes the run. There is no way to address a
glyph by glyph ID from plain HTML/CSS, so engine-side shaping cannot, by
itself, make HTML glyph-exact. Its value in this phase is entirely upstream
of HTML: correct Unicode character-existence and width lookups, and
shaping-informed line-breaking for OpenType-only fonts that have no TFM to
approximate widths from.

The workspace has since gained a real PDF backend
(`docs/pdf_backend.md`, `crates/tex-out/src/pdf.rs`), but it is scoped to
pdfTeX parity: TFM/PK-derived Type3 bitmap glyph streams, not OpenType font
embedding with glyph-ID-addressed text showing. Embedding validated OpenType
programs as Type0/CID-keyed fonts and emitting glyph-show operators driven
directly by `tex-shape`'s `ShapedRun` output is the natural next home for
glyph-exact rendering, since PDF (unlike HTML) can address glyphs by ID. That
work is deliberately out of scope for this plan and left as a documented
extension point: `tex-shape`'s output type is designed so a PDF (or future
XDV-style) consumer can use it directly without redesigning the shaping
kernel.

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
  using a non-Latin-1 Unicode OpenType-only document, verifying no TFM is
  involved and the browser renders it, using DOM-text assertions rather than
  pixel comparison, consistent with the existing "no coordinate equality
  inside a run" HTML contract.

## Staged rollout

1. Character-existence and width dispatch fix (font-metrics abstraction
   split above) — smallest, highest-value, fixes a real bug independent of
   shaping.
2. `tex-shape` crate and rustybuzz integration: single-run shaping API, no
   line-break integration yet.
3. OpenType-only `\font` path and fontdimen synthesis.
4. Two-pass shape/linebreak/reshape integration into `tex-exec` and
   `tex-typeset` — the largest chunk of this plan.
5. Complex-script and bidi follow-on (separate stage).
6. Glyph-exact PDF output via Type0/CID font embedding driven by
   `tex-shape` (separate stage, depends on the PDF backend's font-embedding
   support, not yet started).

Each stage should land as its own coherent change with fixtures, per the
project's usual practice, rather than being split into smaller partial
fragments.
