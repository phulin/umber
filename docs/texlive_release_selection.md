# TeX Live Distribution Acquisition and Release Selection

Status: proposed implementation contract tracked by Beads decision
`umber2-ve7f`. The currently implemented self-hosted packed-distribution path
remains documented in [Automatic CTAN Resource Fetch](ctan_resource_fetch.md)
and [Packed Distribution Shards](distribution_manifest.md) until this design is
implemented.

## Decision summary

An installed Umber binary does not contain a TeX Live tree and does not require
a separately installed TeX Live. It resolves one explicit TeX Live release,
downloads the upstream `texlive.tlpdb` and platform-independent package
archives directly from TeX Live mirrors, and admits package runfiles into the
existing verified platform cache only as the engine requests them.

Umber does not publish a second per-file copy of TeX Live and does not run a
central preprocessing pipeline. The only derived local state is a rebuildable
lookup index, safe extraction of upstream package archives, the small set of
configuration files that a TeX Live installation normally generates, and
Umber-native format images. None of those derived files becomes upstream
authority.

The user-facing model is:

```text
umber run document.tex                 # compiled-in finalized default
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
and content-addressed per-file R2 objects with the upstream TLPDB and package
archives. It also changes the natural download granularity from a single file
to one package. That is an acceptable trade: package archives are the smallest
authenticated, durable objects TeX Live mirrors expose, and one package often
satisfies several successive TeX requests.

## Immutable releases and the rolling repository

Reproducibility requires more than pinning a TLPDB digest. A current `tlnet`
mirror mutates and may remove the revisioned package archives named by an old
TLPDB. A pin to unavailable bytes is not a usable snapshot.

Annual numeric selectors therefore use TeX Live's archived `tlnet-final`
repositories, for example:

```text
https://ftp.math.utah.edu/pub/tex/historic/systems/texlive/2025/tlnet-final/
```

The URLs in the release policy use a mirror-relative historic path rather than
the Utah host. The resolver tries configured TeX Live mirrors that carry the
historic archive and verifies all bytes independently of the chosen mirror.
The initial supported range is the same 2020 through 2026 range proposed by
the earlier multi-version design, but a year is enabled only after its
`tlnet-final` repository exists and its TLPDB pin has shipped in an Umber
release. Thus 2026 must not silently mean the mutable 2026 repository before
the 2026 final archive exists.

An optional `current` selector may target the live `tlnet` repository. It is a
rolling channel, not an alias for a numeric year:

```text
umber run --texlive current document.tex
```

The CLI must say that `current` is mutable, record the exact TLPDB identity in
the run receipt, and never use it as the compiled-in default. Once acquired,
one compile session remains bound to those exact metadata bytes. If a named
archive disappears during acquisition, the run fails instead of mixing two
TLPDB revisions. Offline reuse remains deterministic for all objects already
in the verified cache.

The default selector is the newest finalized annual release known to that
Umber build. A newer default arrives only with a new Umber release. There is no
remotely resolved `latest` alias.

## Typed release policy

`umber-distribution` owns a small closed annual release type and the production
frontend owns reviewed release policy:

```rust
struct TexliveReleaseSpec {
    year: TexliveYear,
    repository_path: &'static str,
    tlpdb_bytes: u64,
    tlpdb_sha512: [u8; 64],
    format_clock: TexClock,
}
```

The TLPDB length and SHA-512 are acquisition pins, not a second inventory.
They authenticate the downloaded database whose package records in turn name
and authenticate package archives. `format_clock` is the deterministic job
clock used when constructing the standard formats for that release; it feeds
the existing format-cache identity.

The table validates unique years, safe relative repository paths, complete
SHA-512 values, and one compiled-in default. The browser projection is
generated from or calls through the same Rust authority. Static product
policy stays in typed source code; it does not gain a JSON policy manifest.

The initial command-line surface remains compatible with the earlier design:

```text
umber run --texlive 2020 document.tex
umber run --texlive 2025 document.tex
```

`--texlive` conflicts with the existing low-level `--distribution` and
`--distribution-ahash64` pair while that packed-distribution escape hatch is
retained. `watch` accepts the same selection and keeps one resolved release
for the lifetime of the session.

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
| TLPDB payload      | SHA-512 and length from release policy or signed live head | authoritative downloaded metadata                            |
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
6. Otherwise fetch the revisioned `.tar.xz` from a configured TeX Live mirror,
   enforce the TLPDB length limit, verify SHA-512, and publish the archive
   atomically.
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

## LaTeX and pdfLaTeX format storage

Umber's existing `.fmt` versioning, validation, and cache remain the sole
format-image authority. This design does not introduce a per-release format
lock, another format manifest, or a parallel version scheme.

For a standard `latex` or `pdflatex` invocation, the prepared-format provider:

1. resolves the selected TeX Live identity;
2. constructs the existing `FormatCacheIdentity` with the engine mode, current
   format schema and ABI fingerprints, exact construction-input closure,
   source/build identity, selected release's deterministic clock, and a
   distribution identity derived from the authenticated TLPDB;
3. restores and fully decodes a matching cached format when present; or
4. acquires missing construction inputs through the same demand-driven package
   resolver, generates the format once, validates it, and atomically stores it
   through the existing format cache.

The current format key already prevents reuse across engine modes, schemas,
ABIs, construction closures, build identities, clocks, and distributions. The
implementation change is to feed it the selected TLPDB-based distribution
identity instead of a self-hosted root-manifest identity. A 2024 `pdflatex`
format can never satisfy a 2025 run, and an Umber upgrade reuses an image only
when the existing format compatibility contract permits it.

The TeX Live archives' Web2C `.fmt` files are neither fetched nor loaded. They
are not Umber format images and are not portable across the engine boundary.

`umber texlive cache YEAR` includes successful preparation and validation of
the standard `latex` and `pdflatex` images by default. The command is complete
only when an immediate `--offline` run of either standard format requires no
network. These images remain in the existing format-cache namespace rather
than being copied into a synthetic distribution tree.

## Explicit complete-distribution caching

`umber texlive cache YEAR` means "cache the complete platform-independent
runtime profile Umber can consume," not "install every TeX Live binary,
source, or documentation package." The profile is computed from the
authenticated TLPDB using typed package-category and runfile-area policy. It
includes:

- all eligible package runfiles needed for TeX, LaTeX, pdfTeX, fonts, maps,
  encodings, virtual fonts, and supported bibliography workflows;
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

`list` shows supported, default, cached, and rolling status. `status` performs
no network access. `verify` runs the separately explicit complete cache audit;
ordinary lookup never scans unrelated entries. `gc` removes only unreferenced
or user-selected derived and authority blobs under the exact cache root and
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

A project may pin a numeric year in its normal Umber configuration. It may
also pin an exact TLPDB digest for a rolling-channel experiment. The resolved
TLPDB identity belongs in run receipts, generated-format metadata, and accepted
input provenance. Neither a project nor an output artifact records the local
cache path or chosen mirror hostname.

## Native and browser transport

The shared Rust TLPDB parser, index semantics, resource keys, archive
validation, and format identities remain host-neutral. Native Umber downloads
directly from TeX Live mirrors.

A browser deployment may use a same-origin gateway or a byte-for-byte mirror
when upstream CORS or compressed-archive transport is unsuitable. Such a
gateway may cache the exact TLPDB and package archives but must not produce a
different per-file distribution format. The archive SHA-512 and TLPDB identity
remain the cross-frontend authority, so native and browser builds select the
same member bytes even when their transport origins differ.

Browser persistent storage may extract and cache members in IndexedDB rather
than using the native blob envelope. It must enforce the same archive and
member validation before passing responses to the existing WASM resource
protocol.

## Trust, limits, and mirror policy

Finalized release policy pins the TLPDB SHA-512 in the Umber binary. Package
archive SHA-512 values come from those authenticated bytes. HTTPS protects
mirror selection and the digest chain detects corrupt or substituted content.
If TeX Live's signed TLPDB verification is adopted for the rolling channel,
signature verification augments this chain; it does not replace the shipped
annual pins.

Mirrors are transport peers, not identities. Failover may change the hostname
but never the repository-relative path, declared length, archive digest,
selected member, or accepted bytes. Redirects must remain HTTPS and bounded.

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
generated configurations, offline completeness, format isolation, and exact
native/browser member parity.

## Migration from packed hosted snapshots

The migration is additive until direct acquisition proves the full runtime
surface:

1. Introduce typed final-release policy and a strict bounded parser for the
   required TLPDB fields.
2. Build the derived request index and package-archive resolver behind the
   existing resource host interface.
3. Reuse `BlobStore` for TLPDB, archive, and extracted-member namespaces.
4. Implement deterministic language and font-map configuration projections.
5. Feed the TLPDB-based distribution identity into the existing format cache
   and prepared-format provider.
6. Add `--texlive`, on-demand `run` and `watch`, and the `umber texlive`
   maintenance commands.
7. Prove 2020 and the newest finalized release through native, browser,
   offline, format, and parity gates.
8. Enable the intervening final releases and scheduled matrix.
9. Retire the self-hosted per-file production pin and publication pipeline
   only after no supported resource kind depends on it.

The existing `--distribution` local/hosted-root escape hatch may survive one
compatibility cycle. Old verified cache blobs are not reinterpreted as TLPDB
packages; normal garbage collection can remove them after the old resolver is
retired.

## Acceptance criteria

- Installing Umber installs no TeX Live tree and needs no system TeX Live.
- A numeric year selects one immutable archived TLPDB and package set.
- The default is a finalized release and changes only with an Umber update.
- Native compilation downloads only packages demanded by actual resource
  requests.
- `umber texlive cache YEAR` makes the supported runtime profile, generated
  configuration, and standard `latex`/`pdflatex` formats usable offline.
- Upstream runfile bytes come directly from verified TeX Live package archives;
  no project-hosted per-file distribution or central preprocessing is needed.
- Existing `.fmt` versioning and validation remain the only format authority,
  with reuse bound to the selected TLPDB identity.
- Equal archives and member bytes can be shared across releases without
  allowing request bindings or formats to cross release identities.
- Native and browser transports validate and expose the same upstream member
  bytes.
- Routine tests remain hermetic and the explicit annual matrix proves
  multi-version compatibility.
