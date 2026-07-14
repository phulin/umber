# Coordinate-Identical HTML Output

Status: implementation contract for artifact schema 11 and HTML schema 1.

HTML is a downstream view of committed `PageArtifact` values. It is not a page
builder and never observes `Universe`, node handles, or mutable font state. DVI
remains the glyph-position conformance driver. HTML preserves the TeX page,
box, rule, leader, special-anchor, and text-container coordinates described
below while delegating glyph advances, shaping, ligatures, and kerning inside a
text run to the browser.

## Coordinate model

All canonical coordinates are signed TeX scaled-point (`sp`) integers. Page
coordinates have an origin at the unshifted upper-left of the shipped root box,
with positive x rightward and positive y downward. The page media rectangle is
the smallest origin-anchored rectangle containing the root box after applying
the artifact's `\hoffset` and `\voffset`; its width and height are emitted as
exact `sp` metadata. Negative child coordinates are allowed and do not change
the origin.

Each ordered positioned event is one of:

```text
PageStart { page, width_sp, height_sp, mag, counts[10] }
TextRun   { id, x_sp, baseline_sp, font, source_codes, text }
Box       { id, kind, x_sp, y_sp, width_sp, height_sp, baseline_sp }
Rule      { id, x_sp, y_sp, width_sp, height_sp }
Special   { id, x_sp, y_sp, class, inert_payload }
PageEnd   { page }
```

`TextRun.x_sp` and `baseline_sp` are exact. Run width, glyph origins, advances,
kerning, ligature selection, and ink bounds are deliberately absent from the
parity record. Event order is significant, including events with equal
coordinates. Rules include ordinary rules and each concrete leader instance.
Empty or nonpositive rule rectangles are not emitted, matching DVI.

The serializer records every compared integer as a decimal `data-umber-*-sp`
attribute. CSS projection uses one rational conversion, independently for each
coordinate (never by adding CSS values):

```text
css_px = sp * mag * 48 / (65536 * 5 * 7227)
```

This is exactly `sp / 65536 * mag / 1000 * 96 / 72.27`. The serializer uses
checked integer arithmetic and a canonical, round-half-away-from-zero decimal
with eight fractional digits. It rejects an intermediate or coordinate that
cannot be represented. Exact parity reads the integer metadata, not computed
CSS. Browser projection tests compare `getBoundingClientRect()` against the
same rational after browser quantization and never accumulate a previous
element's rectangle.

The supported matrix is zoom 100%, 125%, and 200% at device-pixel ratios 1 and
2 in the repository's pinned CI browser builds. The initial validated builds
are Chromium 149 and Firefox 152. A major-version change requires rerunning the
projection self-tests and updating this paragraph. Layout comparisons tolerate
only the containing engine's observed layout-unit quantization (1/64 CSS px in
Chromium 149 and 1/60 CSS px in Firefox 152), plus conversion to device pixels;
screenshots are diagnostic only.

## Text grouping and shaping

Every horizontal box that contains emitting text owns browser-shaped runs.
Nested horizontal boxes start new runs at their exact DVI traversal anchor.
A run contains the longest adjacent sequence that has one font identity and
direction and contains only characters, ligatures, font kerns, and ordinary
inter-character glue. It ends at:

- a font or direction change;
- an explicit/accent kern, rule, leader, special, or nested box;
- math on/off boundaries;
- discretionary material that survived line breaking; or
- any unsupported node whose placement could be ambiguous.

TeX glue and font-kern widths are consumed only to locate the next run or
non-text event. They are not reproduced inside a run as browser geometry.
Consequently a run's browser-shaped width may differ from its TeX width and may
overlap following material; it is clipped only by the page, never wrapped or
used to move another event. `white-space: pre`, `text-wrap: nowrap`, fixed
positioned containers, and zero layout participation preserve TeX's line set.

Character codes are retained in `source_codes`; `text` is obtained from an
explicit encoding map supplied with the web font. A ligature node contributes
its retained `left` and `right` source codes, not a guessed Unicode expansion
of the ligature glyph. Discretionary replacement nodes are already the shipped
choice and are traversed as such. Math uses its actual font/code mapping and
splits at math boundaries. Mixed text/rules, shifted boxes, manual boxes, and
direction changes therefore retain exact anchors without pretending that a
whole TeX line has one semantic string.

The browser policy is `font-kerning: normal`, `font-variant-ligatures:
common-ligatures`, `font-synthesis: none`, and `font-optical-sizing: none`.
The resolved face name is content-derived and is the only member of the CSS
font-family list. A load failure is fatal in parity mode; platform fallback is
never named. Bidirectional inference is disabled: each run has an explicit
`dir` and `unicode-bidi: isolate`.

## Font and asset contract

A `FontResource` from the artifact is only TeX metric identity. A downstream
`HtmlFontResolver` must bind it to exactly one `WebFont` containing:

- the TeX font name, TFM content hash/checksum, design and selected sizes;
- WOFF2 bytes and their SHA-256 content identity;
- one total mapping from every used 8-bit TeX code to Unicode text;
- a redistribution/provenance string and an affirmative embed license; and
- fixed OpenType feature and variation settings.

Bindings are keyed by the complete TeX identity, not by basename. Duplicate,
missing, corrupt, unlicensed, or incomplete bindings are typed failures.
Embedded mode writes WOFF2 as a canonical base64 data URL. Manifest mode writes
content-addressed asset names and a sorted manifest; it never derives a URL
from TeX input. Deterministic subsetting is allowed only when the subsetter,
version, glyph closure, tables, and output hash are pinned. Schema 1 embeds
whole supplied faces to keep native and WASM bytes identical.

Computer Modern support uses explicitly provisioned, redistributable WOFF2
faces and the canonical OT1/OML/OMS/OMX maps selected by the driver bundle.
The repository does not silently convert a host TeX installation or infer an
encoding from a font name. Native callers may load a verified bundle from a
configured path; WASM callers provide the same content-addressed bundle through
the session cache. Font delivery remains driver input and never enters engine
state or artifact identity.

## Page, accessibility, and printing

Each page is an isolated fixed-size `section` with `position: relative`,
`contain: strict`, and `overflow: hidden`. Events are absolute children in
artifact traversal order. Each run is a zero-layout SVG whose `text` element
receives the exact projected `x` and `y` baseline. The baseline is retained in
metadata and marked by a transparent one-CSS-pixel SVG geometry probe; the
probe is `aria-hidden`, does not paint, and works around Firefox returning no
rectangle for zero-area SVG elements. Rules use exact projected rectangles.
Printing uses the same fixed page boxes and does not reflow lines.

Visible runs are selectable and carry `aria-hidden="true"`. A separate
artifact-order accessibility layer contains escaped semantic text in normal
reading order but is visually clipped and cannot affect page geometry. The
document language and page labels are explicit serializer options with stable
defaults. Copy/paste fidelity is best-effort for mapped source text and is not
a coordinate claim.

## Specials and security

Unknown specials are inert; malformed values in a recognized family are typed
errors. Their bytes are retained only as
bounded hexadecimal diagnostic metadata; they are never parsed as markup, CSS,
script, or a URL. Schema 1 interprets only specials whose artifact class is
`html`, with these payloads:

- `color push <named-or-rgb>` and `color pop`, with bounded nesting;
- `dest <identifier>`; and
- `link <https-or-fragment>` and `endlink`.

All values are parsed into typed values and reserialized. Only `https` and
same-document fragment links are accepted. There are no event-handler
attributes, style payloads, remote fonts, images, imports, forms, or executable
scripts. Unknown payloads cannot trigger network access. Standalone output is
compatible with a CSP of `default-src 'none'; font-src data:; style-src
'unsafe-inline'; img-src data:`; hosted manifest mode replaces `data:` with a
caller-selected content-addressed same-origin prefix.

## Determinism, limits, and failures

Serialization is UTF-8 with LF endings, fixed attribute/style order, lowercase
hex, canonical decimal/base64 encodings, and no timestamps or host paths. Page
and event identifiers derive from page/event ordinals, never addresses or hash
map iteration. Repeated native and WASM runs over equal artifact and resource
bytes must produce identical HTML and asset bytes.

Default limits are 16,384 pages, 1,000,000 events per page, 16,384 run
codes, 4,096 nesting depth, 64 MiB per asset, 256 MiB total assets, 256 MiB
HTML, 4 KiB per special, and 256 nested color/link scopes. Callers may lower
them. Limits are checked before growth where possible and report the resource,
limit, and required minimum. Unsupported direction changes, ambiguous text
mappings, malformed allowed specials, coordinate overflow, and unavailable
fonts are actionable errors in parity mode; no partial HTML is published.

Native finalization stages HTML/assets alongside DVI and commits engine effects
before atomically publishing driver files. The host-neutral compile session can
request DVI, HTML, or both from the same committed receipts. WASM returns HTML
and assets as `Uint8Array` fields under the existing aggregate output budget;
JavaScript owns asynchronous acquisition, caching, cancellation, and safe DOM
installation. Rust owns lowering, exact coordinates, font validation, and
serialization.

## Conformance oracle

Every artifact-to-HTML conversion runs an exact comparator between the
driver-neutral events and an independently instrumented canonical DVI
traversal before serialization. It rejects a one-sp change in page
origin, run anchor/baseline, rule edge, leader instance, event order, or special
anchor. It explicitly accepts changed child glyph positions, advances,
ligature selection, ink bounds, and run width. Browser tests inspect page,
rule, and baseline-probe rectangles only; they never inspect glyph children or
use screenshot thresholds.

The hermetic gate covers all 61 committed `dvi`, `page`, `math`, `align`, and
`leaders` documents. The repository's Firefox wasm-bindgen gate and optimized
Chrome package fixture check the SVG baseline and rule projection; the Chrome
fixture additionally proves that enabled kerning/ligatures change the measured
run width without changing the baseline. Story, Gentle, TRIP, and e-TRIP remain
conditional on the external inputs installed by
`scripts/setup-conformance-tests.sh`, matching the existing DVI gate policy.

## Packaging and operational budgets

The packaged CM Unicode face is 222,840 bytes and is shared by content hash in
manifest mode. The optimized browser fixture's one-page embedded-font result is
299,553 HTML bytes; embedded mode intentionally pays base64 expansion per
document, while manifest mode emits the face once. HTML/font inputs and outputs
use `Uint8Array`, worker transfer lists, the 64 MiB one-file ceiling, 256 MiB
cache ceiling, and 256 MiB aggregate output ceiling. The driver lowers one page
at a time, but schema 1 retains positioned pages until deterministic
serialization; the 256 MiB HTML limit is therefore also the practical
peak-memory guard.

The accessibility layer preserves artifact event order, escaped mapped text,
document language, and page labels. It does not claim paragraph semantics,
math accessibility, or perfect copy/paste reconstruction. High zoom, print,
and longer browser-shaped runs may overflow or clip at the fixed TeX page edge;
they never participate in layout or move another exact event.
