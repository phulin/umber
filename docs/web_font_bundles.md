# TeX Web-Font Bundle Generation

Status: long-term implementation plan. Current HTML schema 1 accepts explicit
caller-provided `WebFont` bindings as documented in
[html_output.md](html_output.md). This document defines the reproducible
publisher and catalog needed to generate those bindings for TeX packages and
records a future render/semantic mapping refinement that requires an explicit
HTML schema change.

## Goals and non-goals

Every TeX font used by HTML output needs a verified browser face and an
explicit interpretation of its font-local 8-bit codes. Package ingestion, not
the WASM runtime, is responsible for producing those resources whenever the
source and license permit it.

The bundle pipeline must:

- bind resources to exact TFM content identity rather than a basename;
- preserve or deliberately reproduce the physical glyph selected by TeX;
- keep glyph selection separate from semantic Unicode text;
- generate WOFF2 deterministically with a pinned toolchain;
- verify maps, glyph coverage, digests, provenance, and embedding permission;
- share identical faces and encodings across compatible TFM bindings;
- support native and browser catalogs with identical bytes; and
- fail explicitly for fonts that cannot be converted or redistributed.

The runtime does not convert arbitrary fonts, infer encodings from names,
search the host operating system, or treat a visually similar fallback as the
requested TeX font.

## Why the TFM and WOFF2 remain separate

Classic TeX identifies a typeset character as a font plus an 8-bit code. A TFM
provides TeX dimensions, font parameters, ligature/kern programs,
next-larger links, and extensible recipes. It contains no required outline or
Unicode identity.

WOFF and WOFF2 package OpenType/SFNT font tables for browsers. They provide
outlines and character-to-glyph mappings but do not uniquely determine the
original TFM. An OpenType font can generate many valid TFMs depending on the
chosen 8-bit encoding, features, TeX font parameters, ligature policy, and
rounding. Therefore:

- existing classic packages treat the distributed TFM as authoritative and
  bind a generated or curated WOFF2 to its exact identity;
- modern packages controlled by Umber may generate TFM, encoding, and WOFF2
  together from one pinned source; and
- a WOFF2 must not be used to synthesize a replacement TFM in the browser.

If reducing requests is important, the catalog may group the separate objects
as one logical package while retaining their individual content identities.

## Bundle identities

The catalog distinguishes:

- `TfmIdentity`: name, domain-separated TFM content hash, TFM checksum, and
  design size;
- `HtmlFontKey`: the complete artifact request, adding selected size;
- `EncodingIdentity`: the hash of the complete versioned slot mapping;
- `WebFaceIdentity`: SHA-256 of the exact WOFF2 bytes; and
- `BundleIdentity`: the hash of the binding metadata joining those resources.

Selected size affects the resolved `HtmlFontKey` and CSS projection but does
not ordinarily require a different WOFF2. A catalog binding may therefore
serve many selected sizes while each resolver response repeats and validates
the complete requested key.

No identity is derived solely from a TeX filename. The catalog may list
accepted logical names, but the TFM hash is mandatory.

## Two mapping domains

The principled mapping separates exact rendering from semantic text:

```text
(TFM identity, TeX slot) -> exact render glyph
(encoding identity, source slot sequence) -> semantic Unicode text
```

These agree for ordinary letters but diverge for ligatures, glyph variants,
math extension pieces, ornaments, and private package characters.

The planned mapping record is:

```rust
pub struct WebGlyphMapping {
    pub glyph_name: Option<String>,
    pub render_text: String,
    pub semantic_text: Option<String>,
    pub provenance: MappingProvenance,
}
```

`render_text` is the scalar sequence placed in the visible font-controlled
layer. `semantic_text` is used by copy and accessibility output when the
mapping is known. An absent semantic value makes no Unicode claim.

Where a glyph has an unambiguous standard Unicode character, the generated
WOFF2 may map that character directly. Otherwise the publisher assigns a
deterministic bundle-local Private Use Area scalar and adds a `cmap` entry that
selects the exact glyph. A PUA scalar is a rendering protocol between the HTML
and its embedded font; it must never leak into the semantic accessibility
layer as meaningful text.

Mapping sources have strict precedence:

1. an explicit package-provided mapping;
2. a versioned Umber registry entry for a recognized encoding such as OT1,
   T1, OML, OMS, or OMX;
3. an authoritative Unicode `cmap` in the source font;
4. a pinned Adobe Glyph List mapping for a recognized glyph name;
5. an explicit catalog override; and
6. deterministic PUA rendering with no semantic mapping.

The publisher never infers meaning from a slot number, TFM basename, visual
similarity, or another font's encoding.

Current HTML schema 1 uses one source-code-to-text map and delegates shaping
inside a run to the browser. Splitting render and semantic values, or carrying
the final TeX-selected glyph independently from its source sequence, changes
that contract and requires a new artifact/HTML schema. It must not be slipped
into schema 1 as an implementation detail.

## Physical source handling

Package ingestion handles source formats as follows:

- **WOFF2:** validate and retain exact bytes when they meet catalog policy.
- **WOFF1:** decode to SFNT and generate canonical WOFF2.
- **OpenType or TrueType:** generate WOFF2 with pinned fontTools and Brotli
  versions and canonical options.
- **Type 1 PFB/PFA:** convert or wrap the outlines as OpenType/CFF, normalize
  the encoding and tables, then generate WOFF2.
- **METAFONT source:** prefer a curated licensed vector counterpart. A pinned
  MF-to-outline conversion is permitted only with documented quality and
  reproducibility checks.
- **PK/GF bitmap only:** report HTML unsupported unless a separate,
  deliberately designed bitmap-font path exists. Do not silently trace or
  substitute it.
- **Proprietary or system font:** record an external binding requirement and
  require an application- or user-supplied licensed face.

Already compressed input is never trusted merely because a browser can load
it. The publisher and serializer still enforce SFNT structure, table and glyph
limits, `cmap` coverage, and embedding policy.

## Ligatures, kerning, and glyph identity

Conversion is more than wrapping outlines. A legacy Type 1 font may lack
OpenType GSUB/GPOS behavior corresponding to its TFM. The publisher must choose
and record one of these versioned strategies:

- construct browser shaping tables that reproduce the supported TFM
  ligature/kern behavior;
- map final TeX-selected glyphs directly and disable conflicting browser
  substitutions; or
- declare the binding unsupported for the current HTML schema.

The artifact currently retains source sequences for ligatures so schema 1 can
reconstruct text and request browser shaping. A future exact-glyph schema
should retain both the final TeX-selected glyph code and its semantic source
sequence. That permits a generated PUA `cmap` to select the exact visible glyph
while the accessibility layer emits `fi`, `ffi`, or other source text.

The publisher must not assume every TFM ligature program can be reproduced by
turning on generic `liga` and `kern`. Boundary programs, retain/pass-over
operations, package overrides, and non-text glyphs require explicit testing.

## Virtual fonts

A virtual font is a glyph program that may select, move, and compose glyphs
from several physical fonts. It is not equivalent to one WOFF2 face.

The preferred long-term boundary lowers VF programs during artifact creation
into positioned physical-font operations. HTML requirements then name the
underlying physical fonts. Until the engine and artifact model implement that
boundary, a package whose HTML path depends on an unresolved VF receives a
typed unsupported-font error. The bundle publisher must not flatten a VF by
guessing at one constituent face.

## Catalog schema

The canonical catalog stores binding metadata separately from content-addressed
objects. An illustrative shape is:

```json
{
  "schema": 1,
  "distribution": "texlive-2026",
  "objectsBaseUrl": "https://cdn.example/texlive/2026/objects/",
  "texFonts": {
    "tfm-content-hash": {
      "names": ["cmr10"],
      "tfmChecksum": 0,
      "designSizeRaw": 655360,
      "encoding": "sha256-encoding",
      "webFace": "sha256-woff2",
      "binding": "sha256-binding"
    }
  },
  "encodings": {
    "sha256-encoding": {
      "object": "sha256-object",
      "bytes": 4096
    }
  },
  "webFaces": {
    "sha256-woff2": {
      "object": "sha256-object",
      "bytes": 222840,
      "license": "OFL-1.1",
      "provenance": "source and pinned conversion description"
    }
  }
}
```

The final schema must use canonical field order and encoding, reject duplicate
or case-fold-colliding identities, and keep URLs at the trusted catalog layer.
Font names and TeX input never become URLs directly.

Small encoding and license records may be inline when measurement shows that
doing so reduces latency without making the catalog unbounded. WOFF2 and other
large objects remain content-addressed. Common-family aggregate packs are an
optional transport optimization, not a new identity layer.

## Offline publisher

A standalone `tools/font-pack` publisher performs package ingestion. It is not
a root-workspace dependency and is invoked explicitly by distribution build
scripts.

For each candidate font, it:

1. resolves the TFM, VF, encoding, map, physical font, and license files using
   the pinned distribution's canonical TEXMF winner rules;
2. computes Umber's domain-separated TFM identity and validates the TFM;
3. parses the explicit 256-slot encoding and applies versioned mapping sources
   and overrides;
4. verifies embedding and redistribution permission before conversion;
5. converts or normalizes the physical font with pinned tool versions;
6. constructs the required `cmap` and, when selected, shaping tables;
7. emits deterministic WOFF2 and computes its SHA-256 and length;
8. fully decodes the result and verifies every declared render scalar resolves
   to the intended glyph;
9. verifies every used or promised semantic mapping and records unknowns
   explicitly;
10. emits canonical binding metadata and content-addressed objects; and
11. repeats the clean build and requires byte-identical output.

Temporary paths, timestamps, host font discovery, hash-map iteration order,
and unpinned converter defaults must not affect published bytes.

## Licensing and provenance

Package presence in TeX Live or another distribution does not by itself prove
that browser embedding and redistribution under Umber's delivery model are
allowed. Each binding records:

- source distribution and package version;
- upstream font name and version;
- source object digests;
- conversion tools, versions, and options;
- license identifier and retained license text;
- whether modification, WOFF2 conversion, embedding, and redistribution are
  permitted; and
- any reserved-font-name or attribution obligations.

Unknown or incompatible permission prevents publication. Applications may
still provide a private binding under their own authority, but Umber must not
cache or redistribute it as a public catalog object without permission.

## Runtime integration

At runtime, the committed artifact supplies a complete `HtmlFontKey`. The
session looks up its TFM identity in the catalog and requests one logical HTML
font binding through the protocol in
[wasm_resource_acquisition.md](wasm_resource_acquisition.md). The JavaScript
resolver may fetch the encoding and WOFF2 concurrently or satisfy them from
cache, then supplies one atomic response to Rust.

Rust validates the complete binding before serialization. A missing catalog
entry is an actionable resource miss, not permission to fall back. DVI output
may still complete when HTML is unavailable, subject to the caller's requested
aggregate-output policy.

## Initial package coverage

The first curated catalog target is the Computer Modern family required by
Plain TeX:

- OT1 Roman, slanted/italic, bold, and typewriter faces;
- OML math italic;
- OMS math symbols; and
- OMX math extensions.

This target exercises ordinary Unicode mappings, ligatures, kerning, math
symbols, glyph variants, extension pieces, shared objects, and multiple
selected sizes. Additional package coverage is added only with reproducible
fixtures and explicit license review.

## Staged implementation plan

The implementation is tracked by Beads epic `umber2-y2ei`. Stages are ordered
by their dependencies, and each stage ends in a usable or independently
verifiable artifact. Stages 1 through 4 are the first delivery milestone: they
produce a Computer Modern Roman bundle that an application can fetch on
demand or prefetch while it acquires the corresponding TFM, then use beside
manifest-mode Umber HTML. Stages 5 through 9 expand fidelity and package
coverage without delaying that vertical slice.

The first bundle has this transport shape:

```text
cm-web-fonts/
  catalog.json
  objects/
    sha256-<encoding digest>
    sha256-<woff2 digest>
    sha256-<license digest>
    sha256-<binding/provenance digest>
```

`catalog.json` is the only trusted URL-bearing object. Its entries bind exact
TFM content identities to the other immutable objects. An application may
fetch the catalog and required objects before compilation, fetch them after an
HTML font requirement is known, or start them speculatively from a trusted TFM
or format dependency hint. All three paths assemble the same `WebFont` bytes.
The HTML references only the validated content-addressed face path; it never
contains a URL derived from a TeX font name.

### Stage 1: freeze the bundle contract

Tracked by `umber2-y2ei.2`.

Define the canonical schema-1 catalog, TFM binding, complete 256-slot encoding,
provenance/license, and content-object records. Fix canonical JSON field order,
integer and string rules, object naming, identity domains, size limits, URL
resolution, and duplicate/case-collision rejection. Add shared golden fixtures
and identity vectors for Rust and authored JavaScript consumers.

This stage does not change HTML schema 1, perform font conversion, or add
network access to Rust. It is complete when valid fixtures reserialize to
byte-identical bytes and both consumers reject the same malformed, ambiguous,
oversized, and unsupported-version fixtures.

### Stage 2: publish the first exact-identity CM bundle

Tracked by `umber2-y2ei.3`; depends on stage 1.

Create standalone `tools/font-pack`, kept outside the root workspace. Its first
input profile retains the already pinned CM Unicode Roman WOFF2, verifies the
TeX Live 2025 `cmr` TFM inputs, publishes their Umber font-metric identities,
emits the explicit OT1-like schema-1 map, and retains the complete OFL text and
source/tool provenance. Compatible selected sizes share the one WOFF2 object.

The publisher must parse and validate each TFM checksum and design size, fully
decode the WOFF2, verify every declared render scalar against its `cmap`, and
write only canonical content-addressed output. Two builds in clean temporary
directories must be byte-identical. Host font discovery, basename-only
binding, and substitution are forbidden. The committed catalog and objects
are copied into the WASM package by the ordinary package build.

### Stage 3: consume identical bundle bytes natively and in JavaScript

Tracked by `umber2-y2ei.4`; depends on stage 2.

Add strict catalog readers that resolve an exact `HtmlFontKey`/TFM identity,
load and verify its encoding, face, license, and binding metadata, and assemble
the existing `WebFont` or `SessionWebFont` value. Rust receives bytes from a
configured path or caller-owned cache; authored JavaScript receives bytes from
an injected fetch/cache implementation. Both use the same fixtures, limits,
and validation outcomes.

The existing directory resolver remains a documented development adapter.
Production catalog lookup never falls back to it and never accepts a matching
basename with a different TFM identity. Embedded schema-1 HTML remains
byte-stable when supplied the same binding bytes.

### Stage 4: fetch or prefetch the sidecar with HTML

Tracked by `umber2-y2ei.5`; depends on stage 3. Completion of this stage is the
first shippable bundle milestone.

Expose package APIs to:

- resolve exact HTML font requirements on demand;
- prefetch the associated immutable objects from trusted TFM and format
  dependency hints;
- join duplicate in-flight object requests and verified persistent-cache hits;
  and
- request manifest-mode WASM HTML whose returned asset paths use the same
  content-addressed WOFF2 objects.

Demand remains authoritative: a hint may warm the cache but cannot add an
unused face to output or satisfy a different TFM identity. Fetches are bounded,
concurrent, cancellable, length- and digest-verified, and injectable in Node
tests. Cold, warm, corrupt-cache, cancellation, and no-progress browser tests
must install the returned HTML and assets, wait for every face, reject platform
fallback, and demonstrate byte identity with the native catalog path. This
stage may adapt the resource-acquisition session described in
[wasm_resource_acquisition.md](wasm_resource_acquisition.md), but it must not
rerun completed TeX execution merely to obtain an HTML font.

### Stage 5: cover Computer Modern text families

Tracked by `umber2-y2ei.6`; depends on stage 4.

Add licensed, curated Roman, italic/slanted, bold, small-caps, sans, and
typewriter faces for the Plain TeX TFM set. Every binding records its shaping
strategy and fixed feature settings. Fixtures cover ordinary text, accents,
font changes, ligatures, kerning, and multiple selected sizes against the DVI
coordinate oracle and pinned browsers. A TFM program that schema 1 cannot
reproduce is reported as unsupported instead of being approximated silently.

### Stage 6: cover OML, OMS, and OMX math

Tracked by `umber2-y2ei.7`; depends on stage 5.

Publish licensed math faces and versioned OML, OMS, and OMX mappings. Assign
deterministic bundle-local PUA scalars to glyph variants and extension pieces
that lack an unambiguous Unicode scalar, and record absent semantic mappings
explicitly. Focused fixtures cover math symbols, variants, delimiters, and
extensible constructions. PUA values may select visible glyphs but must not
enter accessibility text as meaningful Unicode.

### Stage 7: generalize conversion and package ingestion

Tracked by `umber2-y2ei.8`; depends on stage 6.

Add pinned Type 1 to OpenType/CFF to WOFF2 conversion, pinned Adobe Glyph List
resolution, canonical TEXMF winner selection, package scanning, license gates,
coverage reports, and a machine-readable unsupported-font inventory. The
publisher records every source digest, tool version and option, license
decision, modification, and attribution obligation. Bitmap-only, proprietary,
ambiguous, unlicensed, and unresolved-VF inputs remain explicit failures.

### Stage 8: version exact render-glyph semantics

Tracked by `umber2-y2ei.9`; depends on stages 6 and 7.

Design and implement a new artifact/HTML schema that retains the final
TeX-selected glyph independently from its semantic source sequence. Use that
identity to select exact visible glyphs through generated `cmap` entries while
the accessibility layer emits semantic text such as `fi` or `ffi`. Disable
conflicting browser substitutions for direct-glyph runs. Schema 1 remains
readable and unchanged; this separation is never introduced as a schema-1
implementation detail.

### Stage 9: lower virtual fonts and complete package coverage

Tracked by `umber2-y2ei.10`; depends on stages 7 and 8.

Lower VF programs during artifact creation into bounded, positioned operations
over exact physical font bindings. Test composition, movement, nesting,
limits, failures, and DVI-coordinate parity. Only after this stage may the
catalog claim general supported-package coverage. Unresolved programs remain
typed unsupported-font failures, and package completion still requires every
exit criterion below.

## Exit criteria

Bundle generation is complete for a package only when:

- every published TFM binding is keyed by exact content identity and resolves
  independently of basename or host search order;
- two clean publisher runs produce byte-identical catalogs and objects;
- every declared render scalar selects the intended WOFF2 glyph;
- every semantic mapping has recorded provenance, and unknown semantics remain
  explicitly absent;
- license review permits the exact conversion, embedding, and distribution;
- corrupt, incomplete, conflicting, ambiguous, and unlicensed inputs fail with
  actionable diagnostics;
- native and WASM consume the same binding bytes and produce identical HTML;
- ordinary text, ligatures, math symbols, and extensible pieces have focused
  browser fixtures; and
- unsupported bitmap, proprietary, or unresolved virtual fonts fail without
  platform fallback or visual substitution.
