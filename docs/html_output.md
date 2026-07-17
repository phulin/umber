# Coordinate-Identical HTML Output

Status: implementation contract for artifact schema 23 and HTML schema 1,
plus the linear OpenType completion contract below.

HTML is a downstream view of committed `PageArtifact` values. It is not a page
builder and never observes `Universe`, node handles, or mutable font state. DVI
remains the glyph-position conformance driver. HTML preserves the TeX page,
box, rule, leader, special-anchor, and text-container coordinates described
below. Rustybuzz owns layout shaping and line breaking; the browser rasterizes
the identical retained font instance inside fixed positioned runs.

Artifact schema 23 keeps this fixed-page model, makes
`OpenTypePreferred` the modern font authority, and adds engine-positioned
OpenType math. It does not delegate formula layout to MathML.

## Coordinate model

All canonical coordinates are signed TeX scaled-point (`sp`) integers. Event
coordinates have an origin at the unshifted upper-left of the shipped root box,
with positive x rightward and positive y downward. The page records a separate
physical-media origin. Native plain-TeX/DVI pages use TeX's conventional
one-inch origin; pdfTeX pages use the captured `\pdfhorigin` and `\pdfvorigin`.

The page media rectangle uses positive `\pdfpagewidth` and `\pdfpageheight`
values when configured. When either dimension is unset, that axis surrounds
the shipped root with equal media-origin-plus-offset space on both sides. This
gives plain TeX a physical page box without requiring pdfTeX primitives. Page
dimensions and media origins are emitted as exact `sp` metadata. Negative
child coordinates are allowed and do not change the origin.

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

The supported matrix is zoom 100%, 125%, and 200% in pinned Chromium 149 and
Firefox 152, with device-pixel ratios 1 and 2 exercised in Chromium. Firefox
exercises every zoom at the CI host DPR; DPR does not alter CSS-pixel geometry.
The package test installs serializer-generated HTML, waits for its embedded
faces, rejects fallback for every emitted character, and measures page,
negative-rule, run-anchor, and baseline metadata. Comparisons tolerate 1/30
CSS px after scaling; screenshots are diagnostic only.

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
explicit encoding map supplied with the web font. Artifact schema 12 records
the complete source sequence for a ligature, including every character in
`ffi` and `ffl`, instead of retaining only its endpoints. Discretionary replacement nodes are already the shipped
choice and are traversed as such. Math uses its actual font/code mapping and
splits at math boundaries. Mixed text/rules, shifted boxes, manual boxes, and
direction changes therefore retain exact anchors without pretending that a
whole TeX line has one semantic string.

Artifact schema 22 emits the selected integer-valued
`font-feature-settings`, signed 16.16 `font-variation-settings`, direction,
script metadata, and BCP-47 language on each OpenType run. The browser policy
also keeps `font-synthesis: none` and `font-optical-sizing: none`.
The resolved face name is content-derived and is the only member of the CSS
font-family list. A load failure is fatal in parity mode; platform fallback is
never named. Artifact text is already in shipped visual order, so schema 1
forces LTR visual ordering with `unicode-bidi: isolate-override`; semantic
bidirectional reconstruction is outside this schema.

## Implemented OpenType-preferred text and positioned math

The modern session policy maps TFM-style text syntax to an exact WOFF2
bundle keyed by TFM content identity. The bundle's code-to-Unicode map feeds
the existing rustybuzz shape/break/reshape path, so OpenType cluster advances
rather than TFM widths locate line breaks and later events. The chosen policy,
mapping version, TFM identity, OpenType program/instance identity, and
fontdimen-synthesis version are committed in the artifact. `ClassicTfmExact`
retains schema 1 behavior for parity documents, virtual fonts, and explicit
legacy output.

OpenType math extends the positioned output as a detached overlay rather than
inserting a reflowing subtree into the legacy page tree. Artifact schema 20
records each fixed math container and its ordered glyph/rule events as:

```text
MathStart { id, x_sp, baseline_sp, width_sp, height_sp, depth_sp }
MathGlyph { font_instance, glyph_id, cmap-or-outline, ssty, x_sp, baseline_sp,
            width_sp, height_sp, depth_sp }
MathRule  { x_sp, y_sp, width_sp, height_sp }
MathEnd
```

`tex-typeset` derives these coordinates from validated MATH constants, italic
corrections, math kern, top-accent attachments, variants, and assemblies.
The binary schema, validation, hashing, and native/WASM round trips preserve
this stream; HTML rendering is the next output stage. HTML serializes a fixed
zero-layout SVG at the recorded anchor. When the
selected glyph is reproducible through cmap, it emits positioned `<text>`
using the retained WOFF2. Script-style substitutions use the recorded `ssty`
feature. When a selected MATH variant or assembly part is addressable only by
glyph id, the serializer emits its validated outline as a positioned SVG
`<path>`. Fraction and radical rules are explicit rectangles or paths.

Engine coordinates and glyph choice are authoritative. Browser font
rasterization may differ in antialiasing and subpixel ink, but no glyph can
move another glyph, resize the formula box, or reflow the page. A separate
artifact-order accessibility representation remains geometry-free; it does
not participate in layout.

## Font and asset contract

Artifact schema 23 records the selected layout policy, explicit mapping
fallback result, encoding-map version and identity, fontdimen-synthesis
version, selected OpenType program, transport-object, collection face,
default/named/explicit resolved variation, feature policy, direction, script,
language, and instance identities beside the classic TeX metric identity. A downstream
`HtmlFontResolver` is only an asset-access adapter: host-neutral sessions bind
it to the already validated and retained resource rather than performing a
second acquisition. Legacy native TFM-only artifacts may still use an explicit
driver binding during migration. The resulting `WebFont` contains:

- the TeX font name, TFM content hash/checksum, design and selected sizes;
- WOFF2 bytes and their SHA-256 content identity;
- one total mapping from every used 8-bit TeX code to Unicode text;
- a redistribution/provenance string and an affirmative embed license; and
- fixed OpenType feature, variation, direction, script, and language settings.

Bindings are keyed by the complete TeX and OpenType identities, not by
basename. Duplicate, missing, corrupt, unlicensed, or incomplete bindings are
typed failures. The content-derived CSS family uses program identity, while
manifest paths use exact object identity.
Before serialization, the driver bounds and fully decodes each WOFF2, parses
its SFNT tables, and requires a cmap glyph for every scalar in every declared
code mapping; a matching caller-supplied digest alone is not accepted.
Embedded mode writes WOFF2 as a canonical base64 data URL. Manifest mode writes
content-addressed asset names and a sorted manifest; it never derives a URL
from TeX input. Deterministic subsetting is allowed only when the subsetter,
version, glyph closure, tables, and output hash are pinned. Schema 1 embeds
whole supplied faces to keep native and WASM bytes identical.

The initial WASM convenience bundle provisions a redistributable CM Unicode
Roman face and an explicit OT1-like text map. Math fonts and other encodings
must currently be supplied as exact resolver bindings; absent mappings fail
serialization instead of falling back. A future bundle can add pinned
OML/OMS/OMX faces and maps without changing engine state or the artifact.
The repository does not silently convert a host TeX installation or infer an
encoding from a font name. Native callers may load a verified bundle from a
configured path; WASM callers provide the same content-addressed bundle through
the session cache. Client distribution policy remains outside engine state,
while validated selection identities enter the immutable loaded-font record
and committed artifact at `\font` load and shipout respectively.

## Page, accessibility, and printing

Each page is an isolated fixed-size `section` with `position: relative`,
`contain: strict`, and `overflow: hidden`. Events are absolute children in
artifact traversal order. Its `data-umber-page` ordinal is immediately paired
with `data-umber-output`, the producing session's collision-resistant 128-bit
identity, and `data-umber-revision`, the accepted editor revision whose
deterministic page and event ordinals the HTML describes. Each run is a zero-layout SVG whose
`text` element receives the exact projected `x` and `y` baseline. The baseline
is retained in metadata and marked by a transparent one-CSS-pixel SVG geometry
probe; the probe is `aria-hidden`, does not paint, and works around Firefox
returning no rectangle for zero-area SVG elements. Rules use exact projected
rectangles. Printing uses the same fixed page boxes and does not reflow lines.

Visible runs are selectable and carry `aria-hidden="true"`. A separate
artifact-order accessibility layer contains escaped semantic text in normal
reading order but is visually clipped and cannot affect page geometry. The
document language and page labels are explicit serializer options with stable
defaults. Copy/paste fidelity is best-effort for mapped source text and is not
a coordinate claim.

The authored WASM package's `source-map.js` helper is the canonical bridge from
a browser point to this metadata. It uses `caretPositionFromPoint`,
`data-umber-codes`, and the selected font encoding to compute a text-unit
ordinal, then includes the page's stamped output identity and revision in the query. The helper
counts DOM UTF-16 offsets from the actual encoding entries, so a TeX code that
maps to multiple Unicode scalars still names exactly one rendered unit.

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
hex, canonical decimal/base64 encodings, and no timestamps or host paths. The
session/output identity is intentionally nondeterministic across independent
sessions; repeated serialization of one accepted session remains byte-identical.
Page and event identifiers derive from page/event ordinals, never addresses or
hash map iteration. Equal artifact/resource bytes and an equal output identity
produce identical HTML and asset bytes across native and WASM.

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
`leaders` documents. The Firefox wasm-bindgen gate and optimized Chrome package
fixture both compile and install generated HTML before checking its page,
negative-rule, run-anchor, and baseline projection at the supported zoom
policy; Chrome also exercises DPR 1 and 2 and verifies the embedded face and
mapped glyph coverage. Story, Gentle, TRIP, and e-TRIP remain conditional on
the external inputs installed by `scripts/setup-conformance-tests.sh`. When
present, their in-process runs lower every committed artifact through the
positioned HTML stream and compare it with the DVI coordinate oracle before
the existing byte-level DVI comparison.

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
