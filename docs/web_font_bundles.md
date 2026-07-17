# OpenType Font Resources for Native and Web Rendering

Status: implemented foundation and linear completion plan. This document
defines the font-resource architecture shared by native and WebAssembly
execution. OpenType font data is the modern source of truth for layout and
rendering. Native sessions accept OpenType or TrueType SFNT containers;
browser sessions accept WOFF2 and decode the same OpenType tables for engine
use. Exact TFM behavior remains available as an explicit legacy policy.

## Decision summary

Font acquisition follows the same host-neutral, batched resource protocol as
other external inputs:

```text
session.advance()
  -> NeedResources { required, prefetch_hints }
  -> client acquires resources concurrently
  -> session.provide_resources(responses)
  -> session.advance()
  -> Complete | NeedResources | Error
```

The engine never invokes an asynchronous resolver and never constructs a URL.
The authored JavaScript facade may accept a resolver as a convenience, but it
only drives this explicit state machine.

One acquired font resource serves both layout and output:

```text
native: OTF/TTF -> validate OpenType -> derive metrics -> render/output asset
WASM:   WOFF2   -> decode OpenType   -> derive metrics -> browser @font-face
```

The font is requested before layout. The engine retains its validated identity
and bytes, records the selected font identity in committed artifacts, and
reuses the already supplied resource when generating HTML. There is no second
HTML-font lookup, no separate web-face binding, and no font catalog inside the
engine or core WASM package.

The client application owns font selection and distribution. It chooses how a
logical request maps to a licensed font, where the bytes are hosted, and how
they are fetched, authenticated, prefetched, cached, and installed. Umber owns
the request contract, structural validation, resource limits, OpenType
interpretation, deterministic identity, artifact binding, and output reuse.

New-document sessions use `OpenTypePreferred`: an exact client mapping may
upgrade a TFM-style text-font selection to a WOFF2/OpenType program without
changing document syntax. `ClassicTfmExact` preserves historical metrics,
lig/kern programs, virtual fonts, math parameters, and byte-oriented output.
Transparency is a user-facing selection rule, never an identity shortcut: the
chosen policy and every TFM/OpenType/mapping version enter committed identity.

## Goals and non-goals

The completed architecture must:

- derive font metrics and supported shaping data directly from validated
  OpenType tables;
- accept OTF and TTF on native hosts and WOFF2 in WebAssembly;
- request all currently knowable missing fonts in one deterministic batch;
- let the client resolve logical font requests without exposing URLs to Rust;
- compute immutable identities after decoding and validation;
- use the same selected font for layout, artifacts, and HTML;
- preserve exact page, box, rule, and text-run anchor coordinates while
  allowing bounded browser rasterization differences inside a text run;
- derive modern text and math layout from the same OpenType program that HTML
  installs, including engine-positioned math from the MATH table;
- support embedded and content-addressed manifest HTML assets;
- make duplicate provisioning idempotent only for identical resources;
- fail explicitly on malformed, unsupported, conflicting, or unavailable
  fonts; and
- behave identically under the low-level session API and the high-level
  JavaScript resolver facade.

This architecture does not:

- require pixel equality for browser-rasterized prose glyphs;
- define a universal font CDN, package catalog, or name-to-URL convention;
- convert OTF or TTF to WOFF2 during a browser session;
- search operating-system fonts implicitly;
- permit a TeX or document font name to become a URL;
- silently substitute a visually similar platform font;
- make licensing decisions for the application; or
- require a separately generated metrics file or browser-font binding.

## OpenType font model

After container decoding, native and WASM paths produce the same validated
logical model:

```rust
pub struct OpenTypeFont {
    pub identity: FontProgramIdentity,
    pub face_index: u32,
    pub cmap: CharacterMap,
    pub metrics: FontMetrics,
    pub shaping: ShapingTables,
    pub math: Option<MathTables>,
    pub metadata: FontMetadata,
}
```

The parser bounds table counts, offsets, glyph counts, outlines, collection
faces, variation axes, mappings, substitution and positioning programs, and
decoded allocation before publishing the value. Unknown optional tables may
be retained or ignored by versioned policy; malformed required tables fail.

The initial metric projection includes:

- units per em;
- horizontal advances and side bearings;
- outline bounds needed by native rendering and diagnostics;
- ascender, descender, and line-gap policy;
- underline, strikeout, cap-height, and x-height metadata when present;
- character-to-glyph mappings;
- GDEF, GSUB, and GPOS data used by the supported shaping policy; and
- MATH constants, italic corrections, variants, and assemblies when present.

The engine applies one documented rounding policy when projecting font units
into scaled points. The same parser, projection rules, feature selection, and
test vectors compile for native and WASM. Browser shaping inside an HTML run is
allowed to differ slightly from engine positioning; those differences do not
move the run anchor or any later TeX-positioned object.

## Containers and identity

The resource has two distinct identities:

- `FontObjectIdentity` is SHA-256 of the exact supplied OTF, TTF, or WOFF2
  bytes. It verifies transport, cache entries, and duplicate provisioning.
- `FontProgramIdentity` is a versioned canonical identity over the validated
  OpenType face and selected variation-independent tables after container
  decoding. It binds layout and artifacts to the logical font program.

The canonical program identity includes the face index and table tags,
lengths, and canonical decoded bytes for every table that can affect metrics,
mapping, shaping, outlines, or math. It excludes transport-only representation
differences such as WOFF2 compression and explicitly ignored metadata. The
identity version changes if the included-table or canonicalization policy
changes.

This separation permits an OTF or TTF native object and its WOFF2 browser
representation to identify the same logical font program while retaining
different transport digests. A distribution that claims such equivalence must
publish the expected program identity, and Umber verifies it after decoding.

Font instances add the selected size, face index, variation coordinates,
feature policy, synthetic-style prohibition, and writing direction to the
program identity. Artifacts record the complete instance identity required to
reproduce their font selection.

## Resource protocol

The host-neutral session uses an extensible batch:

```rust
pub enum ResourceRequest {
    File(FileRequest),
    Font(FontRequest),
}

pub struct FontRequest {
    pub key: FontRequestKey,
    pub accepted_containers: AcceptedFontContainers,
    pub purposes: FontPurposes,
}

pub struct FontRequestKey {
    pub logical_name: String,
    pub face_index: u32,
    pub variation: VariationSelection,
    pub feature_policy: FontFeaturePolicy,
}

pub enum ResourceResponse {
    File(ResolvedFile),
    Font(ResolvedFont),
}

pub struct ResolvedFont {
    pub request: FontRequestKey,
    pub container: FontContainer,
    pub bytes: Vec<u8>,
    pub declared_object_sha256: Option<[u8; 32]>,
    pub declared_program_identity: Option<FontProgramIdentity>,
    pub provenance: Option<String>,
}
```

Native requests initially accept OTF and TTF. WASM requests accept WOFF2. A
future native host may accept WOFF2 as an optimization if it uses the identical
decoder and limits. Container acceptance is an execution capability, not a
font-name convention.

Requests are sorted and deduplicated by complete typed key. They contain no
URLs. Responses repeat the request key and are accepted in any order. The
session validates the container, digest, program identity, face selection,
variations, tables, and limits before the font becomes visible. Registering
the same request and identical bytes is a no-op; a different response for an
already selected request is a typed conflict.

`NeedResources` separates correctness requirements from optional latency
hints:

```rust
pub struct NeedResources {
    pub required: Vec<ResourceRequest>,
    pub prefetch_hints: Vec<ResourceRequest>,
}
```

The client may ignore every hint. A hinted resource never becomes live until
the session actually requires and validates it. Calling `advance` again
without satisfying any required request is a typed no-progress error.

## Font selection and artifact binding

A logical name is a request to the host, not a stable font identity. The
client resolver is the authority that selects a font for that name under its
application and licensing policy. The first accepted response fixes the font
program and instance identity for the session.

Committed artifacts never rely on the logical name alone. They record the
validated font program and instance identities plus the information required
to associate text runs with the retained resource. Re-rendering an artifact
therefore requires the same program identity; another face with the same
family or filename is a conflict, not a fallback.

Font bytes are immutable after registration and shared by identity across
font instances and output pages. Selected sizes do not duplicate the font
object. Session and long-lived render ownership use explicit retain/release
accounting so cached font bytes cannot leak across revisions indefinitely.

## Layout policy and mapped TFM selections

The session records one versioned layout policy:

```rust
pub enum FontLayoutPolicy {
    OpenTypePreferred,
    ClassicTfmExact,
}
```

`OpenTypePreferred` is the default for actively authored documents and HTML
preview. A TFM-style text selection may resolve a client-supplied mapping
bundle containing the exact TFM content identity, a complete used-code to
Unicode map, the selected OpenType program/instance identity, WOFF2 bytes,
and versioned feature and fontdimen policies. Umber converts legacy character
codes through that map, then uses OpenType cmap membership and rustybuzz
cluster advances for packing and line breaking. The retained WOFF2 is the
same object later installed by HTML. Per-character `hmtx` substitution is not
sufficient because kerning, ligatures, marks, and legal break boundaries are
run properties.

`ClassicTfmExact` preserves the existing byte-indexed metrics, lig/kern
automaton, fontdimens, virtual-font composition, DVI constraints, and parity
fixtures. A modern session may use a documented fallback to this policy only
when a mapping or required OpenType capability is absent; the fallback is
recorded in the font and artifact identity and is never a silent platform-font
substitution. Virtual fonts remain classic until their programs can be lowered
without losing composition semantics. Classic math families remain available
independently of modern text selection while OpenType MATH rolls out.

One compilation uses one recorded authority per font across every requested
output. HTML cannot line-break with OpenType metrics while a DVI or PDF from
the same accepted run claims TFM geometry. Cache and artifact identity include
the layout policy, TFM identity when present, OpenType program and instance,
encoding-map version, fontdimen-synthesis version, and fallback result.

## Native integration

Native applications resolve `FontRequest` values from explicit files,
application assets, or their own network/cache layer. Umber does not search
the operating system unless the application implements that policy in its
resolver.

The native path validates OTF or TTF, derives metrics, and retains the original
container. Native rendering uses the decoded outlines and shaping data. HTML
may embed or return the same OTF/TTF object with an appropriate validated MIME
and `@font-face` format declaration. Applications that prefer WOFF2 for native
HTML may provide a prebuilt equivalent through a future transport-variant
response; Umber never performs release conversion implicitly.

## WASM and browser integration

The low-level WASM API exposes `advance` and `provideResources`. The authored
JavaScript facade may accept:

```ts
interface ResourceResolver {
  resolve(
    requests: readonly ResourceRequest[],
    options?: { signal?: AbortSignal },
  ): Promise<readonly ResourceResponse[]>;
}
```

The facade canonicalizes each batch, invokes the client resolver, provides the
responses, enforces progress, and advances again. It does not prescribe how
the resolver maps names to objects or where those objects live.

For a font request, the client supplies WOFF2 once. Rust decodes and validates
the OpenType program, derives layout metrics, and retains the original WOFF2
object. HTML generation later uses those exact bytes in embedded mode or
returns them once under a content-addressed asset name in manifest mode. No
second fetch or post-layout font-finalization state is needed.

Worker wrappers transfer font bytes as `Uint8Array` values and include them in
the existing one-object, cached-resource, and aggregate-output budgets.
Cancellation, concurrency, in-flight joining, persistent storage, and eviction
belong to the client resolver or application resource coordinator.

## Client application responsibilities

The client application owns:

- mapping logical font requests to selected font objects;
- choosing a distribution, package, manifest, CDN, or private asset store;
- URL construction inside its trusted configuration;
- fetch, authentication, retry, cancellation, and offline policy;
- eager loading, dependency prefetch, and service-worker behavior;
- memory and persistent-cache budgets above Umber's hard safety limits;
- licensing authority for private and proprietary fonts;
- progress, missing-font, and recovery UX; and
- DOM installation, Content Security Policy, and asset lifetime.

The client does not parse OpenType for Umber, calculate layout metrics, mint
program identities, or modify committed artifact font identities. It may
verify transport digests early, but Rust repeats all correctness-critical
validation before using the bytes.

## Distribution patterns

The core package defines no catalog schema. Any of these client-owned patterns
are valid:

- statically import a WOFF2 URL from an application bundle;
- map logical names through an application JSON manifest;
- resolve content digests through a CDN or service worker;
- load user-provided fonts from local storage;
- fetch authenticated private fonts; or
- depend on a separate optional package containing Computer Modern assets.

A first-party Computer Modern convenience distribution, if maintained, is an
ordinary client resolver or asset package. It is versioned and licensed
separately from the core WASM runtime. The core compiler neither depends on it
nor contains special cases for its names.

Release pipelines convert OTF/TTF to WOFF2 with their chosen pinned toolchain.
They should publish object digests, program identities, provenance, and license
material, but those records are deployment metadata rather than an engine
protocol or mandatory Umber catalog.

## HTML behavior

HTML preserves exact engine page geometry and text-run anchors. For prose,
Umber shapes before line breaking and materialization; HTML emits Unicode runs
with the identical OpenType instance and fixed feature, variation, direction,
and synthesis policy. The browser rasterizes and may make bounded subpixel ink
choices inside the fixed run, but it cannot reflow the line or move any later
positioned event.

Math is not delegated to MathML layout. Umber parses and validates the selected
font's MATH table, selects glyphs and assemblies, and computes every script,
fraction, radical, accent, operator, delimiter, rule, and box coordinate. HTML
emits a fixed positioned math container. Ordinary cmap-addressable glyphs use
positioned SVG text with the retained WOFF2; `ssty` selections use the recorded
feature policy. MATH variants or assembly pieces that are addressable only by
glyph id use extracted SVG outlines. Rules are explicit rectangles or paths.
The engine geometry and glyph choice are authoritative; only browser text
rasterization and antialiasing may differ.

Visible text uses Unicode text and the acquired OpenType `cmap`. Accessibility
text remains a separate artifact-order layer. Unknown characters, missing
glyphs, unsupported variation coordinates, or incompatible shaping policy are
typed errors in parity mode; the serializer never adds a platform fallback
family.

Manifest mode returns content-addressed assets alongside HTML. Relative asset
paths derive only from verified object digests. The application selects the
installation base and owns object URLs or HTTP paths. Embedded mode uses the
same retained bytes without another resource lookup.

## Licensing and provenance

Umber validates structure and identity, not legal authority. Public font asset
packages must retain their license text, provenance, source version, conversion
tool versions, and redistribution obligations. Applications resolving private
fonts do so under their own authority.

The engine may preserve bounded provenance supplied with a resource for
diagnostics and output manifests. Provenance never changes font identity and
is never accepted as proof that embedding or redistribution is permitted.

## Security and limits

Required failures include:

- malformed SFNT, collection, or WOFF2 structure;
- unsupported or inconsistent table versions;
- offset, length, count, recursion, or decompression-limit violations;
- invalid mappings, outlines, variations, GSUB/GPOS, or MATH programs;
- declared object-digest or program-identity mismatch;
- response type or request-key mismatch;
- conflicting duplicate registration;
- unsupported face or variation selection;
- missing glyphs required by the document;
- unavailable client-selected resources; and
- retry without progress.

Error messages may contain logical request keys and content digests. They must
not interpret a document string as markup or a URL. No partially validated
font becomes visible to layout or output.

## Initial coverage

The first end-to-end fixture uses the existing licensed CMU Serif Roman WOFF2
as a normal OpenType font resource. It demonstrates client resolution, WOFF2
validation, metric derivation, ordinary Unicode text, browser ligatures and
kerning, embedded output, manifest assets, and native/WASM agreement at the
defined font-program boundary. It does not claim glyph-coordinate equality
inside a browser-shaped run.

Coverage then expands to italic, bold, sans, typewriter, variable fonts,
collections, and OpenType MATH fonts. Each addition uses the same resource
protocol; no family-specific engine binding is introduced.

## Staged implementation plan

The implementation is one linear chain under Beads epic `umber2-y2ei`.
The former nested shaping epic `umber2-y2ei.11` is historical only; its
children are direct stages in this chain.

### Completed foundation

1. `umber2-y2ei.1`: rewrite the roadmap around OpenType resources.
2. `umber2-y2ei.2`: freeze resource and identity contracts.
3. `umber2-y2ei.3`: implement the shared validated OpenType core.
4. `umber2-y2ei.4`: acquire fonts through batched `NeedResources`.
5. `umber2-y2ei.5`: expose client-driven WASM orchestration.
6. `umber2-y2ei.6`: retain selected resources for HTML.
7. `umber2-y2ei.11.1`: split TFM and OpenType character metrics.
8. `umber2-y2ei.11.2`: add the pure rustybuzz shaping kernel.
9. `umber2-y2ei.11.3`: add OpenType-only font selection and text fontdimens.
10. `umber2-y2ei.11.4`: integrate two-pass shape, break, and reshape.

### Next 1: positioned OpenType math

Tracked by `umber2-y2ei.9`; this is the first new implementation stage.

Parse and validate MATH constants, glyph information, italic corrections,
math kern, accent attachments, variants, and assemblies. Replace lossy
projection into TeX's symbol/extension fontdimens with a `MathMetricsSource`
queried by the existing math-list conversion kernels. Emit fixed positioned
HTML math using ordinary WOFF2-backed SVG text where cmap can reproduce the
chosen glyph, and SVG outline fallback for glyph-id-only variants and assembly
parts. MathML does not own layout.

### Next 2: OpenType-preferred mappings for TFM-style text

Tracked by `umber2-y2ei.12`; depends on positioned math.

Make `OpenTypePreferred` the modern authoring/HTML default. Resolve exact
client mappings for TFM-style text selections, use the WOFF2's Unicode map,
fontdimens, and rustybuzz metrics for layout, and retain `ClassicTfmExact` for
old documents, virtual fonts, and explicit parity work.

### Next 3: advanced OpenType instances and features

Tracked by `umber2-y2ei.8`; depends on the mapped-text policy.

Add collections, variation axes, named/default instances, complete feature and
script/language identity, mark positioning coverage, resource sharing across
instances, and an optional local `hb-shape` fixture cross-check. The engine
continues to own layout shaping.

### Next 4: bidi and complex scripts

Tracked by `umber2-y2ei.11.7`; depends on advanced instances and features.

Add Unicode Bidi Algorithm level resolution, mixed-direction segmentation,
mirroring, run reordering, and pass-2 visual-order materialization for RTL and
reordering scripts without changing LTR output.

### Next 5: native, WASM, and browser vertical coverage

Tracked by `umber2-y2ei.7`; depends on bidi and complex scripts.

Exercise equivalent native and WOFF2 resources, mapped TFM-style text,
non-Latin Unicode, positioned math, variations, embedded and manifest assets,
workers, caching, Chromium, and Firefox. Tests wait for the exact
content-derived face, reject fallback, and compare engine coordinates while
allowing only the documented rasterization differences.

### Next 6: remove superseded delivery paths and release

Tracked by `umber2-y2ei.10`; depends on full vertical coverage.

Delete superseded web-font binding, preload, separate encoding-map, and
post-finalization acquisition APIs. Migrate supported callers, finish resource
lifetime/security/performance/licensing review, and ship the modern policy
with `ClassicTfmExact` as the explicit compatibility mode.

## Exit criteria

The architecture is complete when:

- native OTF/TTF and equivalent WASM WOFF2 yield the same validated font
  program identity and metric projection;
- every font is acquired through deterministic `NeedResources` batches before
  layout and conflicting responses fail atomically;
- artifacts bind exact font program and instance identities;
- HTML reuses retained font bytes without a second resolution phase;
- embedded and manifest modes install without platform fallback;
- modern mapped text uses one retained OpenType program for layout and HTML,
  while `ClassicTfmExact` remains byte-compatible;
- OpenType MATH controls formula geometry and fixed positioned HTML math
  without delegating layout to MathML;
- the core package contains no distribution catalog or name-to-URL policy;
- client resolvers can use static assets, manifests, CDNs, private stores, and
  persistent caches without changing the engine protocol;
- malformed, oversized, corrupt, unavailable, and unsupported fonts fail with
  actionable diagnostics;
- native, WASM, worker, Chromium, and Firefox gates pass for text and math
  coverage; and
- superseded font-delivery APIs and documentation are removed.
