# TeX Live Distribution Acquisition and Release Selection

Status: proposed implementation contract tracked by Beads decision
`umber2-ve7f`. The currently implemented self-hosted packed-distribution path
remains documented in [Automatic CTAN Resource Fetch](ctan_resource_fetch.md)
and [Packed Distribution Shards](distribution_manifest.md) until this design is
implemented.

## Decision summary

An installed Umber binary does not contain a TeX Live tree and does not require
a separately installed TeX Live. It resolves one explicit TeX Live release,
downloads its upstream `texlive.tlpdb` and platform-independent package
archives from a minimal immutable mirror on `assets.umber.ink`, and admits
package runfiles into the existing verified platform cache only as the engine
requests them.

The hosted mirror preserves upstream TLPDB and package archive bytes and
filenames. It is not the current per-file object distribution: there are no
packed lookup shards, individually republished TEXMF files, or server-side
rewrites. The only derived local state is a rebuildable lookup index, safe
extraction of upstream package archives, the small set of configuration files
that a TeX Live installation normally generates, and Umber-native format
images. None of those derived files becomes upstream authority.

The user-facing model is:

```text
umber run document.tex                 # compiled-in published default
umber run --texlive 2024 document.tex  # one immutable annual release
umber texlive cache 2024               # make that release fully usable offline
umber texlive status
```

Ordinary compilation downloads only demanded packages. `umber texlive cache`
is the explicit opt-in operation that acquires the complete Umber runtime
profile for one release and prepares its standard `latex` and `pdflatex`
formats.

## Why the upstream package repository is the right boundary

Raw CTAN is not directly usable as a runtime filesystem. It is mutable, its
layout is not TeX runtime layout, and some packages contain `.dtx` and `.ins`
sources rather than generated runfiles. TeX Live's `tlnet` repository has
already performed that package-author step and publishes the resulting
platform-independent runfiles in package `.tar.xz` archives.

The TeX Live package database supplies the information Umber needs without a
custom hosted catalogue:

- package name and revision;
- exact runfile paths;
- package dependencies;
- archive byte length; and
- archive SHA-512.

The archive, not an individual extracted file, is the upstream authenticated
unit. Umber verifies its declared length and SHA-512 before inspecting or
publishing any member. Safe local extraction does not transform the member
bytes.

This replaces the project's current root, thousands of packed index shards,
and content-addressed per-file R2 objects with a minimal byte-for-byte mirror
of the upstream TLPDB and package archives. It also changes the natural
download granularity from a single file to one package. That is an acceptable
trade: package archives are the smallest authenticated, durable objects TeX
Live publishes, and one package often satisfies several successive TeX
requests.

## Immutable annual mirrors

Reproducibility requires more than pinning a TLPDB digest. A current `tlnet`
mirror mutates and may remove the revisioned package archives named by an old
TLPDB. A pin to unavailable bytes is not a usable snapshot.

Each numeric year therefore selects one project-hosted immutable package-level
mirror, for example:

```text
https://assets.umber.ink/texlive/texlive-20250308/
```

The concrete snapshot name contains its upstream date. Following the earlier
multi-version design, the initial snapshot for each year is the initial/DVD
release rather than an arbitrary end-of-cycle tree. A later urgent corrected
snapshot gets a new concrete name; an Umber release may deliberately move the
year selector to it, but an existing prefix is never mutated.

The initial supported range is 2020 through 2026. Each row is enabled only
after its complete minimal mirror has been published and verified through the
public origin. The selected TLPDB identity is recorded in every run receipt,
so a compile never mixes releases even when two Umber versions map the same
year to different corrected snapshots.

The default selector is the newest published immutable annual snapshot known
to that Umber build. A newer default arrives only with a new Umber release.
There is no remotely resolved `latest` or mutable `current` alias.

## Typed release policy

`umber-distribution` owns a small closed annual release type and the production
frontend owns reviewed release policy:

```rust
struct TexliveReleaseSpec {
    year: TexliveYear,
    distribution: &'static str,
    base_url: &'static str,
    tlpdb_bytes: u64,
    tlpdb_sha512: [u8; 64],
    font_catalog: Option<FontCatalogSpec>,
    format_clock: TexClock,
}
```

`FontCatalogSpec` carries the existing catalogue schema, URL, byte length, and
cryptographic digest. Its records carry the existing object, program, mapping,
provenance, and license identities rather than defining replacements for them.

The TLPDB length and SHA-512 are acquisition pins, not a second inventory.
They authenticate the downloaded database whose package records in turn name
and authenticate package archives. `format_clock` is the deterministic job
clock used when constructing the standard formats for that release; it feeds
the existing format-cache identity.

The table validates unique years, concrete distribution names, HTTPS URLs
under the production origin, complete SHA-512 values, optional authenticated
font-catalogue specifications, and one compiled-in default. The browser
projection is generated from or calls through the same Rust authority. Static
product policy stays in typed source code; it does not gain a JSON policy
manifest.

The initial command-line surface remains compatible with the earlier design:

```text
umber run --texlive 2020 document.tex
umber run --texlive 2025 document.tex
```

`--texlive` conflicts with the existing low-level `--distribution` and
`--distribution-ahash64` pair while that packed-distribution escape hatch is
retained. `watch` accepts the same selection and keeps one resolved release
for the lifetime of the session.

## Minimal mirror contents and publication

Each annual prefix preserves the relevant part of an upstream TeX Live package
repository:

```text
texlive/texlive-20250308/
  tlpkg/texlive.tlpdb.xz
  tlpkg/texlive.tlpdb.xz.sha512
  tlpkg/texlive.tlpdb.xz.sha512.asc
  archive/<package>.r<revision>.tar.xz
```

The TLPDB and every package archive are byte-for-byte upstream files. Archive
names retain their revision so URLs are immutable and different annual
prefixes can be audited without aliases. The unversioned package archive names
are not published or used.

"Minimal" means complete for Umber's platform-independent runtime surface.
The publisher selects every non-architecture package container with eligible
runfiles in the TeX, TFM, map, encoding, virtual-font, Type 1, OpenType,
TrueType, PK, AFM, and supported bibliography areas. Metadata-only collection
and scheme records require no archive. Documentation containers, source
containers, TeX Live executables, and packages with only unsupported runtime
areas are omitted.

The complete upstream TLPDB is retained even though it describes omitted
documentation, source, and platform packages. Client policy selects only the
supported runtime records; an omitted non-runtime archive is not a broken
mirror. Conversely, every selected runtime archive named by the authenticated
TLPDB must exist before publication can complete.

The TeX Live mirror contains no Web2C `.fmt` files. Official Umber builds may
publish their existing versioned `latex` and `pdflatex` format images under a
separate companion prefix on the same origin, as described below. Those images
are Umber release artifacts, not mirrored TeX Live bytes.

Likewise, the mirror contains upstream TFM, VF, map, ENC, Type 1, TrueType, and
OpenType files exactly where their package archives place them. A separately
pinned first-party font catalogue may add typed legacy mappings, WOFF2
transports, and license records for supported HTML selections. It does not
replace or rewrite the mirrored font files.

Publication is a bounded copy-and-verify operation, not distribution
preprocessing:

1. Acquire the pinned annual TLPDB and selected revisioned archives from
   authenticated TeX Live release media or historic mirrors.
2. Verify TLPDB and package lengths and SHA-512 values before staging them.
3. Copy the exact upstream bytes under a new immutable R2 prefix.
4. Upload package archives first and the TLPDB last.
5. Fetch the public TLPDB and every selected archive through
   `https://assets.umber.ink/`, verify identities, CORS, counts, and total
   bytes, and only then enable the release-policy row.

The publication command is resumable and uses copy semantics, never sync or
remote deletion. A conflicting existing key fails closed. Published prefixes
have immutable cache headers and no lifecycle deletion while any released
Umber version names them. Equal package archives may be server-side copied or
deduplicated by storage tooling, but every annual URL must continue to return
the exact upstream bytes independently of another prefix's lifetime.

No custom root manifest is required. The compiled release row authenticates
the TLPDB, and the TLPDB is the complete inventory and digest authority for the
mirrored packages. Publication may produce an uncommitted operational receipt,
but clients do not download or trust it.

## Installation and cache location

Package-manager installation places only the Umber binaries and their normal
small support files in the install prefix. Distribution data never goes beside
the executable, in Cargo's installation tree, or in the current project.

Native distribution and format state lives under the existing platform Umber
cache root:

- `${XDG_CACHE_HOME:-$HOME/.cache}/umber` on Unix;
- `~/Library/Caches/umber` on macOS; and
- the platform local application-data cache on Windows.

The existing explicit cache-root override remains available for hermetic,
portable, and test workflows. Project configuration records a selector or
TLPDB identity, never an absolute cache path.

The logical storage classes are:

| Class              | Identity                                                   | Retention and authority                                      |
| ------------------ | ---------------------------------------------------------- | ------------------------------------------------------------ |
| TLPDB payload      | SHA-512 and length from release policy                     | authoritative downloaded metadata                            |
| Package archive    | SHA-512 and length from its authenticated TLPDB record     | authoritative downloaded runfiles                            |
| Extracted runfile  | package identity, member path, and computed content digest | derived, verified against its containing archive receipt     |
| Distribution index | TLPDB identity and index schema                            | derived and freely rebuildable                               |
| Generated config   | TLPDB identity, exact inputs, and generator schema         | derived and validated like any other generated cache entry   |
| Umber `.fmt` image | existing `FormatCacheIdentity`                             | derived and validated by the existing format-image machinery |

Authority blobs should continue to use `umber-fetch::BlobStore` and its
anchored `blobs-v2` storage, locks, atomic no-clobber publication, quarantine,
and complete audit. New namespaces distinguish TLPDBs, upstream package
archives, and extracted members. Equal package archives and equal file bytes
can be shared across annual releases by content identity.

Verified package archives are the durable offline representation. Extracted
member blobs are a soft performance cache and may be evicted before their
containing archive; they can be reproduced locally without network access.
This keeps a complete cached release close to the upstream compressed size
instead of requiring a second fully expanded copy. Frequently used members may
temporarily occupy additional space until normal cache garbage collection.

The distribution index is not a mirror and is not trusted input. It maps the
canonical resource vocabulary and TeX search areas to TLPDB package/member
pairs, preserves deterministic precedence for duplicate basenames, and stores
the package dependency graph used for progress reporting and optional
transport advice. Its envelope names the index schema and TLPDB digest. A
missing, stale, or corrupt index is rebuilt from the authenticated TLPDB.

There is no eagerly expanded per-release TEXMF tree. That avoids multiplying
unchanged data across seven annual versions and makes the content cache useful
across versions.

## On-demand resolution

A cold `umber run` resolves resources as follows:

1. Resolve the requested annual release and acquire its pinned compressed
   TLPDB if it is not already cached.
2. Load or rebuild the local request-key-to-package index.
3. Let project-local inputs and generated job files retain their current
   precedence over distribution files.
4. For each unresolved distribution request, select the exact package and
   archive revision named by that TLPDB.
5. Reuse a verified extracted member or package archive when present.
6. Otherwise fetch the revisioned `.tar.xz` from the selected
   `assets.umber.ink` prefix, enforce the TLPDB length limit, verify SHA-512,
   and publish the archive atomically.
7. Safely extract the package's runfiles, rejecting absolute paths, parent
   traversal, links, duplicates, undeclared payload members, size violations,
   and a runfile list that differs from the TLPDB record. The standard package
   metadata member is validated separately and is not exposed as a runfile.
8. Admit only the resource responses needed by the compile attempt into the
   VFS. Other verified members remain cache entries, not live engine state.

Downloads are concurrent across independent demanded packages and
cancellation-aware. A batch is still atomic at the session boundary: verified
peer work may warm the cache after another package fails, but a partial
resource response does not reach the compile session.

Native compilation does not recursively download package dependencies merely
because they appear in the TLPDB. Dependencies and same-package members are
lookup and progress information; the engine's actual resource requests remain
the authority for network work. This preserves the desired demand-driven
behavior and avoids installing broad dependency closures for documents that
do not exercise them.

An authoritative absence in the authenticated index produces the existing
typed unavailable response so TeX probes preserve their ordinary missing-file
semantics. A missing archive, digest mismatch, malformed archive, or transport
failure is a distribution acquisition error, not an absent TeX file.

## Locally derived distribution configuration

Most upstream package runfiles can be consumed byte-for-byte. A normal TeX
Live installation also derives a few site-wide files, notably language
configuration and font maps such as `pdftex.map`. Those cannot be fetched as
stable package members for every annual repository.

Umber should implement only the deterministic configuration projections it
actually consumes. Each projection is keyed by:

- the authenticated TLPDB identity;
- the exact package-member input closure;
- a named generator and generator schema; and
- the output resource key.

Generation happens on first demand, after acquiring only its input closure,
and the result is stored through the verified blob cache. The complete-cache
command prepares every standard projection. This is local installation work,
not a hosted snapshot pipeline: upstream package member bytes remain
unchanged, no external TeX Live executable or Perl installer is required, and
derived outputs can always be deleted and reproduced.

If faithfully implementing one projection would require executing arbitrary
package installation code, the release is not supported by this resolver until
that projection has a bounded typed implementation. Umber must not silently
invoke `tlmgr`, `fmtutil`, `updmap`, or package scripts from a downloaded
archive.

## Font acquisition and output closures

Fonts use the existing cross-output contract in
[Cross-output font system contract](cross_output_fonts.md). The TeX Live
package resolver is a first-party host adapter for that contract; it does not
add another font identity, layout policy, or engine lookup path.

### Classic TeX font resources

Classic font resources are ordinary members of the minimal annual mirror:

- TFM files provide execution and `ClassicTfmExact` layout metrics;
- VF files and their local TFM dependencies provide reached PDF virtual-font
  composition;
- generated effective map configuration selects map entries in deterministic
  order;
- ENC files provide PDF code-to-glyph-name encodings;
- Type 1, TrueType, and OpenType programs provide reached PDF embedding
  leaves; and
- AFM files remain tooling or map-generation inputs rather than runtime layout
  authority.

The derived distribution index maps each typed `FileRequestKey` to its exact
TLPDB package and member. TFM, VF, map, ENC, AFM, and outline-program kinds
remain distinct even if their transport adapter ultimately selects a package
archive. The archive is downloaded and verified once; extracted member bytes
retain their own typed content identities before VFS admission.

Acquisition follows the requested output closure:

| Output | Font acquisition from the annual mirror                                                                                                                                                                              |
| ------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| DVI    | Acquire reached TFMs needed for execution; do not fetch VF, outline, encoding, map, or PK programs solely for the external DVI consumer.                                                                             |
| PDF    | After execution, acquire the complete reached VF/local-TFM/map/ENC/program closure, preferring an exact mapped outline and using an already supplied exact PK leaf only where the existing PDF policy permits it.    |
| HTML   | Acquire the execution/layout authority, then use an exact first-party or client mapping and browser font object as described below; do not fetch the PDF embedding closure merely because the engine mode is pdfTeX. |
| Mixed  | Acquire the deduplicated union of the requested rows and retain one recorded layout authority for every font.                                                                                                        |

This means a DVI-only use of `cmr10` normally downloads its TFM-containing
package but not its Type 1 program. A PDF run downloads the Type 1 or SFNT
package only if the reached map and VF closure selects it. Package dependency
records remain transport advice and do not broaden these semantic closures.

TeX Live does not generally distribute every generated PK instance. Umber does
not run Metafont as an implicit network side effect. A missing outline with no
exact supplied PK object is a typed output-resource failure; an application
may satisfy the existing `PkFontRequestKey` through an explicit private or
local provider.

### OpenType selection and HTML companions

A TLPDB path is not an OpenType font selection policy. Umber must not infer a
`FontRequestKey` result from a filename, family name, PostScript name, or the
first vaguely matching SFNT in a package. Explicit OpenType and mapped legacy
HTML selections therefore come from an authenticated font catalogue.

The annual release policy may pin a first-party catalogue under the same
immutable distribution prefix:

```text
texlive/texlive-20250308/
  umber/font-catalog-v<schema>.bin
  umber/objects/sha256-<digest>
```

The catalogue reuses the existing canonical font and legacy-mapping request,
program, instance, mapping, provenance, and license identities. It maps
complete `FontRequestKey` and `LegacyFontMappingRequestKey` values to:

- the exact object identity and expected canonical `FontProgramIdentity`;
- a face index and allowed variations/features;
- for a legacy mapping, the exact annual TFM content identity and complete
  code-to-Unicode mapping;
- one or more explicit transport sources: a WOFF2 browser companion and,
  where supported, an eligible mirrored OTF/TTF package member identified by
  exact package and member;
- the layout and fontdimen-synthesis policy versions; and
- identity-linked provenance, license text, and affirmative redistribution and
  embedding permission.

The catalogue and its non-TeX-Live objects are generated Umber companion
artifacts, not part of the byte-for-byte package mirror. Each has a
cryptographic acquisition pin in the release policy. Existing object, program,
instance, mapping, and license identities remain authoritative after download.
The catalogue never claims that an OTF/TTF and WOFF2 are equivalent merely
because their names match; Rust decodes both and verifies the declared program
identity.

The currently implemented HTML font records carry one WOFF2 transport. Adding
an SFNT package-member alternative is an additive catalogue-schema extension,
not a new font-selection identity: the complete request still selects one
program and instance, while host container capabilities choose among declared
transport objects that must decode to that program. Old WOFF2-only records
remain valid for browser use.

Publication uploads WOFF2, provenance, and license objects before the
catalogue, fetches all of them through the public origin, and runs the existing
font catalogue audit before enabling the pin. Updating a mapping or transport
creates a new catalogue identity and immutable object; published catalogue
bytes are never overwritten.

For `ClassicTfmExact`, HTML keeps TFM geometry and may paint with a catalogue
WOFF2 only when the exact TFM-keyed mapping covers every used code and permits
embedding. For `OpenTypePreferred`, the mapping selects the OpenType program
before layout and that same program determines shaping, metrics, and HTML
painting. A missing mapping follows the already selected fallback policy or
returns a typed capability failure; it never triggers an operating-system font
or visual substitution.

The initial first-party catalogue remains deliberately curated rather than
attempting to expose every TeX Live font in HTML. It should incorporate the
existing CMU Serif Roman and STIX Two Math records and grow additively through
reviewed exact mappings. Package-mirrored classic DVI and PDF coverage is not
limited by this HTML catalogue.

Native consumers may use the mirrored OTF/TTF member directly. Browser
consumers use a pinned WOFF2 companion when the existing WASM container policy
requires it; Umber does not convert OTF/TTF to WOFF2 during a compilation.

### Font caching and complete-cache behavior

Upstream font-containing package archives use the same durable package cache
as other TeX Live inputs. Extracted TFM, VF, ENC, map, and program members are
evictable derived blobs. First-party WOFF2, mapping, catalogue, provenance, and
license objects use separate verified blob namespaces keyed by their existing
identities. Decoded font programs and sized instances retain their existing
session and artifact lifetimes; the distribution cache does not persist
process-local parser state.

`umber texlive cache YEAR` downloads every eligible font-containing runtime
archive included by the complete Umber profile and every object in that
release's first-party font catalogue. Its completion receipt covers the font
catalogue identity and all referenced object identities. An offline DVI/PDF
run can therefore extract arbitrary mirrored classic font members, and an
offline HTML run can use every advertised first-party selection. Private fonts
and application-supplied mappings are never copied into the shared
distribution cache or completeness receipt.

## LaTeX and pdfLaTeX format storage

Umber's existing `.fmt` versioning, validation, and cache remain the sole
format-image authority. This design does not introduce a per-release format
lock, another format manifest, or a parallel version scheme.

An official Umber build may publish each standard image at a deterministic URL
derived from its existing format-cache key, for example:

```text
https://assets.umber.ink/formats/<umber-release>/<format-cache-key>.fmt
```

The binary's reviewed acquisition table associates that existing
`FormatCacheIdentity` with the URL, byte length, and cryptographic payload
digest. This is an acquisition pin for a downloaded artifact, not another
format identity or manifest. The published payload is the portable Umber
format image; the native filesystem cache envelope remains local-only.

For a standard `latex` or `pdflatex` invocation, the prepared-format provider:

1. resolves the selected TeX Live identity;
2. constructs the existing `FormatCacheIdentity` with the engine mode, current
   format schema and ABI fingerprints, exact construction-input closure,
   source/build identity, selected release's deterministic clock, and a
   distribution identity derived from the authenticated TLPDB;
3. restores and fully decodes a matching local cached format when present;
4. otherwise downloads the matching official image when that exact identity
   has a published acquisition pin, verifies its transport identity and full
   format decode, and stores it through the existing format cache; or
5. acquires missing construction inputs through the same demand-driven package
   resolver, generates the format once, validates it, and atomically stores it
   through that cache when no published image exists.

The current format key already prevents reuse across engine modes, schemas,
ABIs, construction closures, build identities, clocks, and distributions. The
implementation change is to feed it the selected TLPDB-based distribution
identity instead of a self-hosted root-manifest identity. A 2024 `pdflatex`
format can never satisfy a 2025 run, and an Umber upgrade reuses an image only
when the existing format compatibility contract permits it.

The TeX Live archives' Web2C `.fmt` files are neither fetched nor loaded. They
are not Umber format images and are not portable across the engine boundary.

`umber texlive cache YEAR` includes successful preparation and validation of
the standard `latex` and `pdflatex` images by default, preferring their pinned
official artifacts and using the normal local generator as fallback. The
command is complete only when an immediate `--offline` run of either standard
format requires no network. These images remain in the existing format-cache
namespace rather than being copied into a synthetic distribution tree or the
local package-archive namespace.

## Explicit complete-distribution caching

`umber texlive cache YEAR` means "cache the complete platform-independent
runtime profile Umber can consume," not "install every TeX Live binary,
source, or documentation package." The profile is computed from the
authenticated TLPDB using typed package-category and runfile-area policy. It
includes:

- all eligible package runfiles needed for TeX, LaTeX, pdfTeX, fonts, maps,
  encodings, virtual fonts, and supported bibliography workflows;
- every object advertised by the release's first-party font catalogue;
- every required locally generated configuration projection; and
- the standard Umber `latex` and `pdflatex` format images.

It excludes documentation, package source archives, platform executables, and
engines Umber does not implement. The command prints the TLPDB identity,
package count, total declared download bytes, already-cached bytes, and the
format work before starting. Work is bounded, resumable, independently
verified, and idempotent. An interrupted run leaves verified cache entries
that the next run reuses but does not mark the release complete.

Completeness requires every selected package archive, not a permanently
expanded copy of every member. A later offline compile may safely extract an
uncached member from its verified local archive.

A small completion receipt is derived state keyed by the release identity,
runtime-profile schema, configuration-generator schemas, and required standard
format identities. `--offline` verifies that receipt's referenced cache
entries; it does not trust a boolean marker.

Focused maintenance commands should include:

```text
umber texlive list
umber texlive status [YEAR]
umber texlive cache YEAR
umber texlive verify [YEAR]
umber texlive gc
```

`list` shows supported, default, and cached status. `status` performs no network
access. `verify` runs the separately explicit complete cache audit; ordinary
lookup never scans unrelated entries. `gc` removes only unreferenced or
user-selected derived and authority blobs under the exact cache root and
reports which offline-complete releases cease to be complete.

## Offline and project pinning behavior

`--offline` and `UMBER_OFFLINE=1` retain their current meaning: project files,
verified cached distribution data, generated configuration, and compatible
cached formats may be used, but no metadata or package request reaches the
network.

An offline failure reports the selected year and TLPDB identity, then the
missing package archives or generated artifacts. It does not collapse a cache
miss into TeX's optional-file absence and does not suggest installing a system
TeX Live.

A project may pin a numeric year in its normal Umber configuration. The
resolved concrete distribution name and TLPDB identity belong in run receipts,
generated-format metadata, and accepted input provenance. Neither a project
nor an output artifact records the local cache path.

## Native and browser transport

The shared Rust TLPDB parser, index semantics, resource keys, archive
validation, and format identities remain host-neutral. Native and browser
Umber both download the same package archives from `assets.umber.ink`. The
production origin supplies CORS and immutable cache headers, so the browser
does not need a per-file gateway or a second distribution representation.

Browser persistent storage may extract and cache members in IndexedDB rather
than using the native blob envelope. It must enforce the same archive and
member validation before passing responses to the existing WASM resource
protocol. First-party font-catalogue and WOFF2 companion objects use the same
origin and the existing typed font-response validation.

## Trust, limits, and mirror policy

Finalized release policy pins the TLPDB SHA-512 in the Umber binary. Package
archive SHA-512 values come from those authenticated bytes. HTTPS and the
compiled production-origin policy protect selection, while the digest chain
detects corrupt or substituted content. Retaining the upstream detached TLPDB
signature makes publication independently auditable, but runtime signature
verification does not replace the shipped annual pin.
Font-catalogue, WOFF2, mapping, provenance, license, and published-format
artifacts have independent cryptographic acquisition pins; their existing
semantic decoders and identities remain the final admission boundary.

Production acquisition does not fail over to mutable upstream mirrors. A
missing hosted archive is a publication defect and fails closed; it never
causes the client to select bytes outside the annual prefix. A low-level local
mirror override may change transport for air-gapped use, but never the TLPDB
identity, repository-relative path, declared length, archive digest, selected
member, or accepted bytes.

Existing per-resource and aggregate VFS limits remain authoritative. New
metadata, package archive, member-count, expansion-ratio, extracted-byte,
concurrency, timeout, and retry limits apply before allocation or
publication. Archive parsing rejects links and special files rather than
materializing them.

## Multi-release conformance strategy

Distribution compatibility and engine conformance remain separate axes, as in
the earlier design.

The canonical engine tier keeps pdfTeX 1.40.29 as the sole semantic and
byte-level oracle for primitive behavior. Existing command-semantic, PDF,
TRIP, e-TRIP, and focused DVI fixtures do not multiply across TeX Live years
unless they intentionally read distribution files.

The annual distribution tier runs the same current Umber and reference pdfTeX
against each selected final TLPDB:

1. Populate or reuse that release through the new cache resolver.
2. Build a fresh reference format from the same upstream runfile bytes; never
   load a historical binary format.
3. Build or restore the Umber format through the existing format cache.
4. Run the stable representative document corpus through both engines.
5. Compare DVI byte-for-byte after only established normalization and compare
   PDF through the existing normalized structure and raster evidence.
6. Check recorder/input receipts so every distribution input maps to the
   selected TLPDB or a declared local input.

The explicit commands remain:

```text
scripts/check-texlive-parity.sh --years 2020,2025
scripts/check-texlive-parity.sh --all
```

The historical executable canary also remains: for each annual release, a
small engine-sensitive set compares that year's upstream pdfTeX executable
with pdfTeX 1.40.29 against the same runtime bytes. Missing old host binaries
are reported as unavailable and never treated as a pass.

Routine `cargo test --tests` uses synthetic TLPDBs and archives and performs no
network access. It covers year selection, TLPDB and archive validation,
duplicate-path precedence, safe extraction, cache separation and sharing,
generated configurations, classic output-specific font closures, exact
TFM-keyed HTML mapping, WOFF2/program equivalence, license rejection, offline
completeness, format isolation, and exact native/browser member parity.

## Migration from packed hosted snapshots

The migration is additive until package-mirror acquisition proves the full
runtime surface:

1. Introduce typed annual release policy and a strict bounded parser for the
   required TLPDB fields.
2. Build the derived request index and package-archive resolver behind the
   existing resource host interface.
3. Reuse `BlobStore` for TLPDB, archive, and extracted-member namespaces.
4. Build and verify one minimal annual package mirror on `assets.umber.ink`.
5. Implement deterministic language and font-map configuration projections.
6. Project the existing curated font and mapping records into one pinned annual
   companion catalogue and integrate it with the package resolver.
7. Feed the TLPDB-based distribution identity into the existing format cache
   and prepared-format provider.
8. Publish pinned standard formats through the existing format identity and
   validate download-to-cache admission.
9. Add `--texlive`, on-demand `run` and `watch`, and the `umber texlive`
   maintenance commands.
10. Prove 2020 and the newest published release through native, browser,
    offline, format, and parity gates.
11. Publish and enable the intervening annual mirrors and scheduled matrix.
12. Retire the self-hosted per-file production pin and publication pipeline
    only after no supported resource kind depends on it.

The existing `--distribution` local/hosted-root escape hatch may survive one
compatibility cycle. Old verified cache blobs are not reinterpreted as TLPDB
packages; normal garbage collection can remove them after the old resolver is
retired.

## Acceptance criteria

- Installing Umber installs no TeX Live tree and needs no system TeX Live.
- A numeric year selects one immutable package-level mirror on
  `assets.umber.ink`.
- The default is a published immutable snapshot and changes only with an Umber
  update.
- Native compilation downloads only packages demanded by actual resource
  requests.
- `umber texlive cache YEAR` makes the supported runtime profile, generated
  configuration, and standard `latex`/`pdflatex` formats usable offline.
- Hosted TLPDB and package archives are byte-for-byte upstream files; no
  project-hosted per-file distribution or central preprocessing is needed.
- Classic font resources resolve lazily from exact mirrored package members,
  and each requested output acquires only its defined font closure.
- HTML mappings and WOFF2 companions use the existing typed font identities,
  exact TFM binding, and affirmative license records without limiting broader
  classic DVI/PDF coverage.
- Existing `.fmt` versioning and validation remain the only format authority,
  with reuse bound to the selected TLPDB identity.
- Equal archives and member bytes can be shared across releases without
  allowing request bindings or formats to cross release identities.
- Native and browser transports validate and expose the same upstream member
  bytes.
- Routine tests remain hermetic and the explicit annual matrix proves
  multi-version compatibility.
