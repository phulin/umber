# Cross-output font system contract

Status: normative architecture contract for `umber2-nobk` and all of its
implementation children.

Contract version: 1

This document fixes the font boundary shared by TeX82, e-TeX, pdfTeX, LaTeX,
and pdfLaTeX sessions and by DVI, PDF, and HTML output. It supersedes placement
or authority implications in older font documents when they conflict. The
HTML catalog described here is deliberately small; the DVI and PDF resource
surface is not.

## 1. Decisions and terminology

An **engine contract** determines primitives, format compatibility, token and
assignment semantics, node construction, and the meaning of `\font`. An
**output capability** asks a downstream driver to publish DVI, PDF, or HTML.
They are independent axes. Selecting HTML never changes the engine identity,
and selecting `PdfTex` does not by itself request PDF output.

A session fixes these values before execution:

```rust
pub struct CompileContract {
    pub engine: EngineMode,
    pub outputs: OutputCapabilitySet,
    pub font_layout: FontLayoutContract,
}

pub struct FontLayoutContract {
    pub version: u8, // version 1
    pub policy: FontLayoutPolicy,
    pub missing_mapping: FontMappingFallbackPolicy,
}

pub enum FontLayoutPolicy {
    ClassicTfmExact,
    OpenTypePreferred,
}
```

`OutputCapabilitySet` is a nonempty set of `Dvi`, `Pdf`, and `Html`. PDF is
valid only for an engine contract that owns the pdfTeX PDF semantics. A format
image may constrain the selected layout policy, but it does not add an output
capability. Output planning occurs from the explicit set, never by inspecting
the engine name or by treating HTML as an addition to a default DVI run.

The **engine resource closure** is everything required to execute and lay out
the document. A **driver resource closure** is everything additionally needed
to finalize one requested output. The session acquires the deduplicated union
of the engine closure and only the requested driver closures. A resource
required by two closures is registered and validated once.

## 2. Layout authority

The chosen authority is compilation-wide and is recorded per loaded font and
in every artifact font record.

| Selection                         | `ClassicTfmExact` authority                                                                                   | `OpenTypePreferred` authority                                                                                                                                                |
| --------------------------------- | ------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Unprefixed TFM-style real font    | Exact TFM widths, heights, depths, italic correction, lig/kern program, fontdimens, checksum, and design size | An exact mapping keyed by TFM content identity selects Unicode mapping plus an OpenType program/instance; OpenType shaping and synthesized text fontdimens are authoritative |
| Unprefixed TFM-style virtual font | Exact TFM and VF composition semantics                                                                        | Explicit recorded `ClassicTfmExact` fallback, or a typed failure; version 1 never mixes mapped OpenType advances with VF packets                                             |
| `opentype:<logical-name>`         | Explicit OpenType is authoritative for this selection; the compilation policy remains recorded                | Explicit OpenType is authoritative                                                                                                                                           |
| Classic TeX math families         | Appendix G and classic TFM parameters                                                                         | Classic authority unless an explicit OpenType MATH selection is made                                                                                                         |
| Explicit OpenType MATH            | Validated MATH program, instance, glyph selection, and positioned geometry                                    | Same                                                                                                                                                                         |

`ClassicTfmExact` is not a request for an old renderer. It says that TFM/VF
semantics determine layout. HTML may paint that layout with a licensed mapped
WOFF2 only when the mapping is exact and total for every used code; the WOFF2
does not retroactively change advances or line breaks.

Under `OpenTypePreferred`, an unprefixed selection first loads the exact TFM
object, because the TFM digest—not a name or basename—is the mapping key. An
accepted mapping fixes its encoding-map version and identity, program and
instance identities, feature policy, and fontdimen-synthesis version before
font-dependent layout. `FontMappingFallbackPolicy::ClassicTfmExact` records a
classic fallback in loaded state and artifacts. `Error` returns typed absence.
Neither policy searches an OS font, guesses an encoding, or substitutes by
family, PostScript name, filename, or visual similarity.

One accepted run has one geometry. DVI, PDF, and HTML generated together use
the same loaded-font authority and committed coordinates. A driver may encode
or rasterize the chosen glyph differently, but it may not reshape, re-break,
or reinterpret the artifact. A requested driver that cannot represent the
chosen authority returns a capability error for the whole output transaction;
it never silently changes the authority.

## 3. Session decision table

This is the single normative output decision table. “Classic closure” means
the complete closure reached by the document, not the HTML MVP subset.

| Requested outputs | Engine/layout work                                                                                                          | Additional driver closure                                                                                                                                                                                                                        | Publication and failure rule                                                                                                                                                                           |
| ----------------- | --------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| HTML only         | Execute with the selected policy; retain mapped or explicit OpenType resources used by layout; do not build a DVI page plan | Licensed WOFF2 transport for every painted OpenType instance; exact legacy mapping for every painted classic byte font; mapping/license/provenance records                                                                                       | Publish HTML and its embedded or manifest assets atomically. No VF/map/ENC/PK/Type 1 discovery merely because the engine is pdfTeX. Unsupported HTML mapping or VF is a typed HTML capability failure. |
| DVI only          | Execute with the selected policy and build byte-valid DVI page plans                                                        | No embedding closure. The DVI file names fonts but does not acquire consumer raster programs. Unicode or font semantics not representable by the byte DVI contract are typed capability failures.                                                | Publish DVI atomically. Do not acquire HTML WOFF2/license records or PDF VF/map/ENC/program/PK resources.                                                                                              |
| PDF only          | Execute with the selected policy; do not build DVI plans or HTML assets                                                     | For reached classic fonts: recursive VF and local TFM closure, effective maps, ENC, Type 1/TrueType/OpenType programs, or exact PK fallback. For explicit/mapped OpenType: the same program/instance selected by layout plus PDF embedding data. | Publish PDF atomically after complete detached validation. Do not acquire HTML-only WOFF2 transport unless it is also the accepted program container needed by this host.                              |
| Any mixed set     | Execute and lay out once with one authority                                                                                 | Exact set union of the selected DVI, PDF, and HTML rows, deduplicated by typed request and immutable object/program identity                                                                                                                     | Stage every requested output and publish all or none. One driver's incompatibility does not trigger a layout fallback; it fails the mixed request and identifies the driver.                           |

A DVI consumer outside Umber may of course need its own font installation.
That deployment concern is not a compile-session driver closure and must not
cause a DVI-only Umber session to download raster or outline programs.

## 4. Typed request and response identity

All acquisition crosses one output-neutral protocol. Version 1 retains the
existing required/probe/prefetch batch semantics and adds typed mapping and
legacy leaf requests instead of output-specific callbacks:

```rust
pub enum ResourceRequest {
    File(FileRequest),
    Font(FontRequest),
    LegacyFontMapping(LegacyFontMappingRequest),
    PkFont(PkFontRequest),
}

pub enum ResourceResponse {
    File(ResolvedFile),
    Font(ResolvedFont),
    LegacyFontMapping(ResolvedLegacyFontMapping),
    PkFont(ResolvedPkFont),
    Unavailable(ResourceRequestKey),
}
```

These variants and their distinct wire tags are the migration target; legacy
files remain `File` variants with distinct `FileKind` values:

- `FileRequestKey` is `(ResourceDomain, FileKind, normalized relative name)`.
  VF, AFM, ENC, map, outline program, and license records never alias ordinary
  TeX input merely because a manifest transports both under `tex:<name>`.
- `FontRequestKey` is logical name, face index, variation selection, feature
  policy, direction, script, and language. Accepted containers and purposes
  are request capabilities, not part of selection identity.
- `LegacyFontMappingRequestKey` is mapping-schema version, exact TFM SHA-256,
  requested layout-policy version, purpose, and (when constrained) encoding
  catalog identifier. It is never keyed only by `cmr10` or `OT1`.
- `PkFontRequestKey` is TeX name bytes, resolved DPI, and frozen mode. Its
  resolved object identity is the digest of the exact PK bytes.
- Responses repeat the complete request key. A provider does not rewrite a
  request from one semantic kind to another, even when both select the same
  immutable distribution object.

Exact transport bytes have a domain-separated object identity. TFM, VF, AFM,
ENC, map, PK, license, and mapping records each retain their exact content
identity. OpenType has both `FontObjectIdentity` (the supplied OTF/TTF/WOFF2
bytes) and `FontProgramIdentity` (the canonical validated face after container
decoding). `FontInstanceIdentity` adds size, face, resolved variations,
features, direction, script, language, and synthesis prohibitions. Equivalent
SFNT and WOFF2 transports may share a program identity but never an object
identity.

A mapping record has its own versioned canonical identity and contains the
TFM digest, exactly 256 optional Unicode strings for the legacy mapping used
by version 1, expected OpenType program identity, permitted request key or
catalog selection, encoding-map version, fontdimen-synthesis version, and
license-record identity. The selected font response and mapping record are
validated together before either becomes live. Later mapping schemas may add
larger code spaces without changing version-1 identity.

Required requests block correctness. Probes require a positive object or
authoritative absence. Prefetch hints may be ignored and may install positive
objects only. Batches are sorted and deduplicated by complete key. Partial and
permuted responses are accepted atomically; retry without a newly bound
required request or probe is `NoProgress`.

## 5. Resource placement matrix

“HTML R2” below means the new immutable HTML publication profile, not the
existing broad TeX Live snapshot. “Local/client” includes explicit native
directories, a client VFS, application/private catalogs, and authenticated
private storage. “Generated” means produced by an output driver and never
accepted as authoritative input to the same compilation.

| Resource                                                | Semantic owner/use                                                              | HTML R2 profile                                                                      | Local/client providers                                               | Output-generated                                                                                   |
| ------------------------------------------------------- | ------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ | -------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| TeX runtime inputs (`.tex`, `.sty`, `.cls`, config)     | Engine input closure                                                            | Yes, but only the pinned HTML fixture/runtime closure and authenticated dependencies | Yes; unrestricted within normal typed lookup and limits              | AUX/TOC and other declared transactional job outputs only                                          |
| Umber format images                                     | Validated engine initialization                                                 | Yes, only named schema-compatible HTML MVP formats and their closure metadata        | Yes; explicit compatible formats                                     | Format builders may publish immutable formats; document drivers do not                             |
| TFM                                                     | Classic metrics and exact key for mapped selection                              | Yes, exactly the MVP TFM objects listed in section 7                                 | Yes; unrestricted legacy DVI/PDF scope                               | No                                                                                                 |
| VF                                                      | Classic virtual composition; PDF leaf discovery                                 | No in MVP                                                                            | Yes; unrestricted bounded VF closure for PDF and client workflows    | No                                                                                                 |
| AFM                                                     | Tooling/map preparation or application metadata; never runtime layout authority | No                                                                                   | Yes when an explicit client/tool workflow requests it                | May be an offline distribution-build intermediate, never a document output                         |
| ENC                                                     | PDF/dvips code-to-glyph-name encoding                                           | No                                                                                   | Yes; complete reached PDF legacy closure                             | No                                                                                                 |
| Font maps and inline mapping records                    | PDF real-font selection and configuration                                       | No PDF/dvips maps. The distinct HTML legacy mapping record is allowed.               | Yes; complete ordered pdfTeX/dvips map semantics                     | Effective map snapshot may be retained in a PDF build receipt, not published as source input       |
| PK                                                      | PDF Type3 bitmap fallback keyed by name/DPI/mode                                | No                                                                                   | Yes; exact reached leaf programs                                     | PDF Type3 CharProcs and subsets                                                                    |
| Type 1 (`.pfb`/`.pfa`)                                  | PDF outline embedding after map/VF resolution                                   | No                                                                                   | Yes; complete reached legacy PDF closure                             | PDF embedded subset/font objects                                                                   |
| TrueType/OpenType SFNT (`.ttf`, `.otf`, `.ttc`, `.otc`) | Modern layout and native/PDF embedding                                          | No in the browser-oriented MVP profile                                               | Yes; explicit, mapped, native, PDF, and private resources            | PDF subsets only; Umber does not convert release inputs during a session                           |
| WOFF2                                                   | Browser transport for a canonical OpenType program                              | Yes, only the exact MVP objects in section 7                                         | Yes; application/private HTML resources                              | Whole retained face in HTML schema 1; a future pinned subsetter may generate deterministic subsets |
| HTML legacy mapping records                             | Exact TFM-to-Unicode/OpenType selection                                         | Yes, only section 7 records                                                          | Yes; versioned application/private catalogs                          | No                                                                                                 |
| License and provenance records                          | Publication/embedding authorization and audit                                   | Yes, mandatory and identity-linked for every hosted font/mapping                     | Yes; mandatory affirmative authority for embedding or redistribution | Sorted output asset manifests may cite them; provenance does not enter font program identity       |
| Project-private resources                               | User/application authority                                                      | Never implicitly copied or published                                                 | Yes, highest external-provider precedence                            | Embedded/subset output only when the client affirmatively authorizes it                            |
| DVI, PDF, HTML, CSS, SVG outlines, asset manifests      | Driver products                                                                 | No                                                                                   | Not resolver inputs                                                  | Yes                                                                                                |

The HTML publisher must reject a profile that accidentally includes VF, AFM,
ENC, PDF/dvips maps, PK, Type 1, or unlisted SFNT objects. Conversely, the
common resolver and non-HTML APIs must not encode a catalog allow-list: an
explicit local/client distribution remains eligible to answer every bounded
legacy request reached by DVI/PDF workflows.

## 6. Resolver precedence and authoritative absence

For each typed request, eligible providers are consulted in this order:

1. an immutable binding already accepted by the live session;
2. same-run generated input where TeX close-and-reopen semantics permit it;
3. user/project files and explicit per-session resources;
4. an application/private typed catalog;
5. an explicitly selected local or client distribution;
6. the selected authenticated hosted profile (HTML R2 for its eligible
   records, or the ordinary pinned TeX runtime distribution for eligible
   shared inputs); and
7. verified process/persistent object caches only as storage for an object
   selected by one of the preceding providers.

A cache never selects a resource by itself. Provider precedence selects a
typed record; its digest selects cache bytes. Local search preserves configured
`TEXINPUTS`, `TEXFONTS`, and relevant map/font-area candidate order within the
local-distribution provider.

A miss is provider-scoped. It advances to the next eligible provider. Only a
verified negative from every eligible provider becomes the session's immutable
`Unavailable` binding. Transport failure, offline cache miss, authentication
failure, corrupt manifest, digest mismatch, or an ineligible HTML-profile
record is not authoritative global absence. Project/private resources shadow
hosted records before either object becomes live. After acceptance, another
provider cannot rebind the request during that session.

No layer probes operating-system font registries or derives a URL/path from a
document font name. There is no basename, family-name, PostScript-name, or
platform fallback. Manifest transport aliases such as mapping semantic VF,
ENC, map, and program kinds through `tex:<name>` do not change semantic
identity or precedence.

## 7. Exact HTML MVP catalog

The version-1 hosted HTML catalog contains exactly these font selections:

| Catalog entry                   | Contents and purpose                                                                                                                                                                                                                                                                                                                                    |
| ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `classic-text/cmr10`            | The pinned `cmr10.tfm`; one version-1 256-entry OT1-like legacy mapping keyed by that exact TFM SHA-256; the mapping selects the CMU Serif Roman program below. This is the only mapped legacy family/face/size source in the MVP. TeX may instantiate the TFM at ordinary scaled sizes.                                                                |
| `opentype-text/cmu-serif-roman` | The repository-pinned 222,840-byte CMU Serif Roman WOFF2 used by existing conformance fixtures, its expected object and canonical program identities, provenance, upstream version, conversion-tool receipt, license text, and affirmative embedding/redistribution record. It serves both the `cmr10` mapping and explicit `opentype:` text selection. |
| `opentype-math/stix-two-math`   | The repository-pinned STIX Two Math WOFF2 used by positioned-MATH fixtures, its expected object/program identities, provenance, upstream version, conversion receipt, license text, and affirmative embedding/redistribution record. It is available only through explicit OpenType MATH selection in the MVP.                                          |

The profile also contains the exact pinned Plain/LaTeX fixture runtime inputs,
compatible format image(s), and TFM closure needed to exercise those three
entries. Those shared execution inputs are enumerated by the publisher's
authenticated closure and do not authorize more font catalog entries.

MVP non-goals are: additional Computer Modern faces or sizes as distinct TFM
mapping keys; arbitrary TeX Live families; general OT1/T1/TS1/OML/OMS/OMX or
other legacy encoding catalogs; VF discovery or leaf lowering for HTML; Type 1
or PK browser conversion; OS-font discovery; arbitrary automatic OTF/TTF to
WOFF2 conversion; bidi/RTL expansion beyond the implemented shaping contract;
and automatic publication of project/private fonts. An exact miss returns
`UnsupportedHtmlLegacyMapping`, `UnsupportedHtmlVirtualFont`, or ordinary
typed resource absence as applicable. It never falls back to CMU Serif merely
because a requested font resembles Computer Modern.

## 8. Versioned additive seams

The following are reserved extensions, not MVP promises:

- new catalog families/faces are new records keyed by exact identities;
- new legacy encodings use a new mapping record or mapping schema version;
- math catalogs add explicit program/mapping records without changing text
  requests;
- one transport object may be referenced by multiple mapping records without
  changing its object/program identity;
- advanced faces and instances reuse the existing complete `FontRequestKey`;
- VF-to-HTML support may add a versioned VF leaf-lowering record containing
  VF and local-TFM identities plus exact leaf mappings; version 1 treats it as
  unsupported; and
- deterministic WOFF2 subsetting requires an explicit subset-policy version,
  glyph/table closure, tool identity, and output digest.

Unknown additive record kinds or versions are rejected by strict old parsers.
New parsers continue accepting version-1 records unchanged. Existing request,
mapping, artifact, object, program, instance, and cache identities are never
reinterpreted when a catalog grows.

## 9. Ownership, lifetime, and caches

The host/application owns provider configuration, credentials, I/O,
concurrency, cancellation, offline policy, and persistent cache eviction.
`umber`/`umber-wasm` owns session orchestration, precedence, immutable request
bindings, batching, and output-closure planning. `umber-vfs` owns file
registration, path/content identity, atomic generations, negatives, and file
budgets. `tex-fonts` owns bounded TFM/VF/PK/OpenType parsing, canonical font
identities, immutable decoded programs, mappings, and instances. The engine
owns loaded-font semantic state. `tex-out` and the PDF finalizer consume only
committed artifacts plus a read-only retained-resource view.

Transport bytes and decoded programs are shared by immutable identity. Sizes
and instances do not duplicate an object. A pending attempt privately retains
new resources; failure or cancellation releases that generation. Acceptance
moves references into the accepted session history and any staged output
closure. Incremental revisions retain only resources reachable from accepted
history and output. Disposing a session/output releases its references;
process or browser caches may keep unreferenced verified objects under their
own bounded eviction policy.

Persistent cache keys are provider namespace plus exact object digest (and
schema/program version where relevant), never logical name alone. Every read
is reverified before registration. A live reference cannot be evicted from
the Rust owner. Cache loss changes performance only. Cold, warm, and offline
runs supplied the same selected objects produce identical semantic identities
and outputs. Offline mode may use project files, explicit local distributions,
and verified cached objects; missing uncached hosted objects are actionable
offline misses, not fabricated global negatives.

## 10. Artifact and output identity

Every committed font resource records its TeX name and semantic source
identity, TFM content hash/checksum/design and selected sizes when present,
layout-policy version and value, explicit fallback result, mapping schema/
version/identity, fontdimen-synthesis version, OpenType object/program/instance
identities and complete instance inputs, and generated copied/letterspaced/
expanded construction provenance. VF, map, encoding, PK, and embedded-program
identities enter the PDF finalization identity when reached; they do not alter
the already committed layout authority.

Artifact identity excludes URLs, host paths, provider names, cache location,
fetch order, provenance prose, and license prose. Publication receipts bind
the license-record identity separately. Equal accepted resources and output
options yield equal DVI/PDF/HTML bytes under their existing deterministic
contracts. A transport change that preserves `FontProgramIdentity` may
preserve layout identity, but HTML asset bytes/manifest and any object-identity
field change. A mapping, policy, fallback, feature, variation, synthesis, or
VF-lowering version change necessarily changes the relevant semantic identity.

No output is published until all requested drivers validate and serialize
privately. Mixed output is an all-or-none transaction over engine effects,
accepted revision, generated files, and driver products.

## 11. Validation, security, licensing, and failures

All supplied bytes are untrusted. Validate declared length and digest,
canonical request correspondence, path/domain/kind, manifest signature or
pinned root, decompression and allocation limits, container structure, face
and variation selection, cmap/mapping totality for used codes, shaping/MATH
tables, VF/PK/map/ENC/program grammar, recursive closure limits, and output
representability before publication. No parser performs filesystem/network
I/O or executes PostScript, shell commands, font bytecode, HTML, CSS, or URLs.
Recognized data is normalized to typed values; diagnostics escape untrusted
names and never treat them as markup.

Typed terminal outcomes distinguish at least:

- provider miss, final unavailable resource, and offline unavailable object;
- unsupported HTML legacy mapping and unsupported HTML VF lowering;
- output capability incompatibility (including DVI scalar limits and PDF
  engine mismatch);
- request/response mismatch, unexpected response, no progress, and conflicting
  positive/negative or positive/positive binding;
- digest, canonical program identity, mapping identity, or TFM identity
  mismatch;
- malformed, oversized, unsupported, cyclic, or over-budget resources;
- absent used glyph/code mapping or incompatible feature/variation/MATH data;
- missing or nonaffirmative embedding/redistribution authority; and
- mixed-output driver failure identifying the responsible capability.

Umber validates an affirmative license capability and identity link; it does
not decide whether a publisher or application legally owns that authority.
Public R2 records must include the complete license text, provenance, source
version, conversion tool/version, and redistribution obligations. Private
providers make the same affirmative embedding assertion when output will
contain font bytes or outlines. License/provenance text cannot authenticate a
font or change program identity. DVI-only publication, which embeds no font,
does not require an HTML embedding license.

## 12. Production API migration inventory

The following are all current production boundaries affected by this contract
and their mandatory migration targets. Tests and examples migrate with their
owning public boundary.

| Current production API/path                                                                                                  | Current role                                                            | Contract target                                                                                                                                                                                                       |
| ---------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `umber::SessionOptions::{engine,dvi,html,html_asset_mode,font_layout_policy,font_mapping_fallback,accepted_font_containers}` | Engine and output selection are partly coupled through booleans         | Keep `engine`; replace `dvi`/`html` and implicit CLI PDF finalization with `OutputCapabilitySet`; group the versioned layout fields as `FontLayoutContract`; derive accepted transports from host capabilities        |
| `umber::EngineMode` and format `install_after_format` paths                                                                  | Compatibility identity                                                  | Retain independently of outputs; validate only genuine engine/format constraints                                                                                                                                      |
| `umber::{ResourceRequest,ResourceResponse,NeedResources,CompileAttemptResult}`                                               | Shared batched acquisition                                              | Retain the state machine; extend the typed union for exact mapping and PK/legacy leaf resources plus provider-scoped resolution; do not add driver callbacks                                                          |
| `umber-vfs::{ResourceDomain,FileKind,FileRequestKey,FileRequest,ResolvedFile,FileProvisioner}`                               | Typed immutable files                                                   | Retain; add distinct missing legacy kinds such as AFM/license where requested and preserve semantic kinds through manifest transport aliases                                                                          |
| `tex_fonts::{FontLayoutPolicy,FontMappingFallbackPolicy,FONT_LAYOUT_POLICY_VERSION}`                                         | Layout authority                                                        | Retain version-1 meanings exactly and include the complete contract in artifact/cache identity                                                                                                                        |
| `tex_fonts::{FontRequestKey,FontRequest,ResolvedFont,LegacyFontMapping}` and wire `UFRQ\x02`/`UFRS\x03`                      | OpenType request plus response-embedded mapping                         | Retain font request/program selection; move response-embedded `legacy_mapping` to an authenticated typed mapping request/response keyed by TFM identity; introduce new wire versions without reinterpreting old bytes |
| `tex_fonts::{FontObjectIdentity,FontProgramIdentity,FontInstanceIdentity,FontSourceIdentity}`                                | Transport, canonical face, instance, and semantic construction identity | Retain their separate domains; never collapse them into manifest names or output-local IDs                                                                                                                            |
| `tex_out::{FontResource,OpenTypeFontResource,FontResourceConstruction}` in artifact schema 23                                | Committed font binding                                                  | Add any mapping/contract version fields required above through a new additive artifact schema; preserve schema-23 decoding and classic records                                                                        |
| `tex_out::html::{HtmlFontAssets,HtmlFontAsset,HtmlFontKey}` and `umber::{html_from_artifacts,html_from_committed_artifacts}` | Read-only post-layout HTML binding                                      | Replace the HTML-specific acquisition-shaped facade with a read-only `OutputResourceView` over the planned retained closure; HTML still cannot acquire or remap after layout                                          |
| `umber::AcceptedFinalization` and `PdfVirtualFontResources::{CachedVirtualFont,CachedLocalTfm}`                              | Post-engine PDF-only discovery state                                    | Replace with the generic planned/retained `OutputResourceClosure`; discovery runs only when `Pdf` is requested                                                                                                        |
| `tex_fonts::{PdfPkFontRequest,PdfPkFont}` plus `umber::pdf_output` fallback loader                                           | Typed PK identity but output-specific provisioning                      | Carry the same key/decoded resource through the common resource facade and PDF closure; eliminate post-acceptance filesystem-specific loading                                                                         |
| `umber::cli_resource` local resolver and native `TEXINPUTS`/`TEXFONTS` search                                                | Native layered resolution                                               | Implement the common composite provider precedence, retaining configured local candidate ordering and full legacy breadth                                                                                             |
| `umber_wasm` `SessionOptions`, `ResourceRequest`/`ResourceResponse`, `advance`/`provideResources` TypeScript unions          | Browser representation adapter                                          | Mirror `OutputCapabilitySet`, versioned mapping/legacy variants, and typed failures; remain acquisition-only                                                                                                          |
| Authored JS `ResourceResolver`, `HttpManifestResolver`, worker controller, and manifest schema                               | Async transport and hosted selection                                    | Implement the same composite typed provider interface; add the HTML profile records without making R2 the universal capability boundary                                                                               |
| `umber-distribution::{ManifestLogicalKey,JobRequirement,AcquisitionJob}` and sharded manifest schemas 2/3                    | Authenticated TeX file selection                                        | Add a new immutable HTML-profile schema for font/mapping/license records; preserve existing roots and semantic request keys                                                                                           |
| `tools/texlive-wasm-publish` and `scripts/publish-texlive-r2.sh`                                                             | Broad runtime snapshot publication                                      | Add a separate reproducible HTML-only profile and manifest-last publication; never mutate or prune the legacy snapshot                                                                                                |
| Native/WASM `MemoryRunOutput` and CLI output staging                                                                         | Returned DVI/HTML and downstream PDF products                           | Return the explicitly requested capability set and commit every selected product atomically under one output identity                                                                                                 |

Compatibility migration is additive until all production callers use the new
contract. Existing format schemas, artifact schema 23, font wire versions,
manifest schemas 2/3, and public methods are either decoded with their exact
old meaning or rejected explicitly; none are guessed. A compatibility adapter
may translate `dvi/html` booleans to a capability set for one release, but new
code cannot infer outputs from `EngineMode`. Existing `ClassicTfmExact` DVI
and PDF fixtures remain byte/structure/rendering gates. Existing explicit
local/client legacy closures remain valid even when the HTML catalog cannot
answer the same document.

## 13. Required verification

Implementation children must cite this file and jointly prove:

- all four rows of the decision table, including no irrelevant requests;
- exact union/deduplication and one immutable authority in mixed sessions;
- the three-entry HTML catalog from cold, warm, and offline caches;
- project/private override before R2, corrupt and conflicting object failure,
  license rejection, and deterministic publication;
- typed unsupported HTML mapping and VF outcomes without fallback;
- additive synthetic records for another family, encoding, shared object,
  instance, math entry, and VF leaf metadata without changing version-1 APIs
  or identities; and
- representative arbitrary local/client TFM/VF/map/ENC/PK/Type 1 and modern
  TrueType/OpenType PDF closures plus unchanged broad DVI behavior.

The complete native test suite, format/clippy gate, WASM build, worker and
browser gates, DVI byte parity, and PDF structural/rendering parity remain
release requirements. The HTML MVP is complete only when those non-HTML gates
continue to pass; small hosted scope is not permission to narrow the font
system.
