# OpenType Font Resources for Native and Web Rendering

Status: long-term implementation plan. This document defines the font-resource
architecture shared by native and WebAssembly execution. OpenType font data is
the source of truth for layout and rendering. Native sessions accept OpenType
or TrueType SFNT containers; browser sessions accept WOFF2 and decode the same
OpenType tables for engine use.

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
  allowing the browser to shape within a run;
- support embedded and content-addressed manifest HTML assets;
- make duplicate provisioning idempotent only for identical resources;
- fail explicitly on malformed, unsupported, conflicting, or unavailable
  fonts; and
- behave identically under the low-level session API and the high-level
  JavaScript resolver facade.

This architecture does not:

- require coordinate equality for individual glyphs inside a browser-shaped
  text run;
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

HTML preserves exact TeX page geometry and text-run anchors. The browser owns
glyph selection, advances, kerning, ligatures, and ink placement inside a run
under the fixed feature, variation, direction, and synthesis policy recorded
by the font instance. A browser-shaped run may differ slightly in width from
the engine's line construction without moving any later positioned event.

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

The implementation is tracked by Beads epic `umber2-y2ei`.

### Stage 1: freeze resource and identity contracts

Tracked by `umber2-y2ei.2`.

Define `FontRequest`, `ResolvedFont`, container capabilities, object identity,
canonical program identity, instance identity, duplicate semantics, limits,
and shared native/WASM test vectors. The contract contains no URLs, catalog
records, or asynchronous callbacks.

### Stage 2: implement the shared OpenType core

Tracked by `umber2-y2ei.3`; depends on stage 1.

Implement bounded OTF, TTF, collection, and WOFF2 decoding; canonical program
identity; metrics and `cmap` projection; supported GSUB/GPOS extraction; and
immutable font-program storage. Prove equivalent OTF/TTF and WOFF2 fixtures
produce the same program identity and projected metrics.

### Stage 3: integrate batched font acquisition

Tracked by `umber2-y2ei.4`; depends on stage 2.

Generalize the host-neutral session to return fonts in `NeedResources`, accept
typed responses, detect conflicts and no progress, and retain selected font
resources before layout. Collect all currently knowable font misses in one
deterministic batch.

### Stage 4: expose client-driven WASM orchestration

Tracked by `umber2-y2ei.5`; depends on stage 3.

Expose the low-level resource state machine and a high-level authored
JavaScript facade that accepts an application resolver. Test concurrency,
cancellation, workers, transfer, client caching, hints, partial responses, and
progress without embedding any distribution policy in the package.

### Stage 5: reuse selected fonts in HTML

Tracked by `umber2-y2ei.6`; depends on stage 4.

Record font program and instance identities in artifacts and generate embedded
or manifest HTML from the already retained font objects. Remove any post-layout
font acquisition path. Verify one WOFF2 fetch serves WASM layout and browser
installation.

### Stage 6: complete native/WASM and CM vertical coverage

Tracked by `umber2-y2ei.7`; depends on stage 5.

Exercise OTF/TTF native loading and WOFF2 WASM loading with equivalent CMU
fixtures. Verify metrics, font identity, artifact selection, HTML text, asset
digests, browser installation, and coordinate anchors across native, WASM,
Chromium, and Firefox.

### Stage 7: add advanced OpenType text support

Tracked by `umber2-y2ei.8`; depends on stage 6.

Add collections, variation axes, feature policies, script/language selection,
mark positioning, and the supported shaping boundary. Keep browser ownership
inside runs explicit and test deterministic engine layout inputs across native
and WASM.

### Stage 8: add OpenType math support

Tracked by `umber2-y2ei.9`; depends on stage 7.

Parse and validate MATH constants, italic corrections, variants, assemblies,
and math glyph information. Integrate them with math layout without inventing
family-specific mappings or auxiliary metrics files.

### Stage 9: remove superseded font-delivery paths and release

Tracked by `umber2-y2ei.10`; depends on stage 8.

Delete superseded web-font binding, preload, and post-finalization APIs; migrate
native, WASM, worker, examples, and documentation to `NeedResources`; complete
resource-limit, corruption, cache-lifetime, browser, and licensing review; and
ship the OpenType resource path as the supported architecture.

## Exit criteria

The architecture is complete when:

- native OTF/TTF and equivalent WASM WOFF2 yield the same validated font
  program identity and metric projection;
- every font is acquired through deterministic `NeedResources` batches before
  layout and conflicting responses fail atomically;
- artifacts bind exact font program and instance identities;
- HTML reuses retained font bytes without a second resolution phase;
- embedded and manifest modes install without platform fallback;
- the core package contains no distribution catalog or name-to-URL policy;
- client resolvers can use static assets, manifests, CDNs, private stores, and
  persistent caches without changing the engine protocol;
- malformed, oversized, corrupt, unavailable, and unsupported fonts fail with
  actionable diagnostics;
- native, WASM, worker, Chromium, and Firefox gates pass for text and math
  coverage; and
- superseded font-delivery APIs and documentation are removed.
