# Sharded Distribution Manifest

The relationship between semantic request identity and catalogue transport
identity is fixed by [Canonical resource identity and lifecycle](resource_lifecycle.md).
This document remains authoritative for implemented catalogue schemas and
publication.

Status: schema-2 browser/native resolution, schema-3 format-closure publishing,
and schema-4 HTML font-profile resolution implemented.

The separate immutable HTML font profile and its mapping/license record
migration are specified by the normative
[cross-output font system contract](cross_output_fonts.md). Existing schema-2
and schema-3 TeX Live roots retain their exact meanings and broad runtime
scope.

## HTML font profile schemas

Root schema 4 retains the authenticated root layout and shard partition
algorithm of schemas 2/3, but pairs only with index-shard schema 2. Existing
root schemas 2/3 continue to require index-shard schema 1. This explicit pair
prevents an old parser from treating font-profile records as file-only shards
and leaves every published TeX Live root and format closure unchanged.

Index-shard schema 2 has the existing `files` map plus independent `fonts` and
`legacyMappings` maps. An explicit font key is the canonical version-1
encoding of the complete `FontRequestKey`: UTF-8 logical name, face index,
variation instance and sorted coordinates, sorted feature policy, direction,
script, and language. Variable text and four-byte OpenType tags are lowercase
hex encoded, so delimiters cannot alias request fields. A legacy key contains
mapping schema 1, the lowercase SHA-256 of the exact TFM bytes, layout-policy
version, purpose, and optional encoding-catalog identity. Neither key uses a
basename, family fallback, or platform font name.

Font record schema 1 binds a content-addressed WOFF2 object, optional declared
program identity, feature-policy version, bounded conversion provenance, and
a content-addressed license record. Legacy-mapping record schema 1 additionally
repeats and validates its TFM digest, selects a complete canonical font key,
carries exactly 256 nullable nonempty Unicode strings, fixes mapping and
fontdimen versions, and records either `classic-tfm-exact` or `error` fallback.
Every hosted record requires affirmative embedding and redistribution flags;
missing, false, oversized, or malformed license metadata rejects the shard
before font bytes are fetched.

Rust and authored JavaScript consume the shared closed case under
`tests/corpus/distribution/cross-frontend-v1/`. Its `case.inventory` is
validated against the runtime checkout's exact Git and filesystem inventories
before Rust reads any payload. Validation inspects every directory component
from the selected checkout through the case root without following symlinks,
so `target`, generated/scratch, alternate-checkout, or builder-checkout bytes
cannot become fixture authority. Both frontends reject unsupported record or
policy versions, noncanonical or duplicate request components, TFM/key drift,
malformed Unicode maps, conflicting digest lengths, missing licenses, and
non-embeddable records. A verified canonical shard miss remains authoritative
profile absence. Root/shard HTTP, authentication, JSON, digest, or partition
failure remains an actionable resolver error and never becomes an unavailable
font response.

The extension-seam tests derive a synthetic mixed catalog from that shared
schema-1 fixture. Two exact TFM identities and two distinct encoding-catalog
identities select complete OpenType instance keys while reusing one declared
program and content-addressed WOFF2 object; an additional explicit family
record uses the same unchanged font-record grammar. Unique fetch/cache work is
deduplicated by object digest, not by collapsing the semantic record keys.
An exact unmapped TFM digest stays a `LegacyMapping` miss and is never retried
by TeX basename. No new production record version or catalog entry is implied.

After acquisition, the session parses each selected object once into the
canonical `OpenTypeFont` and retains that program with its authenticated
catalog response. HTML output consumes the same realized program and decoded
SFNT allocation; it does not reinterpret the catalog or create a second
distribution/cache identity. This is an ownership change only: schema-4 keys,
object digests, program declarations, cold/warm/offline selection, and cache
invalidation remain byte-for-byte unchanged.

## Trust root

The deployed release pin names `manifest-v2.json` and its SHA-256 digest. The file is a
compact, canonical JSON object with schema 2. It contains the distribution and
object-base identities, inline format entries, `shardBits`, `shardCount`, and
an ordered `shards` array of lowercase SHA-256 digests.

`shards` is ordered by numeric shard index. It contains bare digests exactly
once, rather than repeating derivable object names or byte lengths. Shard
object names are `sha256-<digest>`. `shardCount` must equal both `2^shardBits`
and the array length. Formats remain inline because they are selected before
ordinary `kind:name` lookup.

Canonical JSON has no insignificant whitespace, preserves schema field order,
sorts every map by raw UTF-8 key order, and ends with one newline. For the
256-shard production layout the root is 17,743 bytes; roughly 16 KiB is the
irreducible payload of 256 SHA-256 hex digests.

Schema 3 preserves the shard and object contract while adding an optional
`inputClosure` to inline format entries. The old schema-2 key remains immutable:
schema-2 parsing explicitly rejects closures, and new closure-bearing snapshots
publish as `manifest-v3.json`. Each closure is independently versioned with
schema 1 and contains 1 through 256 unique canonical file request keys in raw
UTF-8 sort order. Keys use the same `kind:name` vocabulary as shard lookup and
are limited to 1,024 UTF-8 bytes. Unknown closure versions, invalid or duplicate
keys, unsorted arrays, and oversized closures fail parsing.

Format metadata schema 1 remains the legacy no-closure form. Publisher metadata
schema 2 requires a schema-1 input closure; it validates and canonicalizes the
keys, then requires every key to resolve to the authoritative published file
map. The staged verifier repeats the bounds, order, syntax, and existence checks
against the complete authenticated shard set. After a compatible pinned format
is selected, browser hosts may translate its closure into typed prefetch
requests. The native compiler keeps the closure as authenticated publication
metadata and opens no closure shard or object until the engine requests that
logical input. Schema-2 roots continue to carry no closure.

Hints remain optional browser transport advice. Browser responses are still
validated and installed atomically, and retry progress still comes from
required requests only. Native `--prefetch-input` is a separate explicit user
request; ordinary native compilation does not follow format closures or inline
dependency hints speculatively.

## Partition and shard schema

The canonical lookup key is the case-sensitive `kind:name` string, for example
`tex:article.cls`, `tfm:cmr10.tfm`, `bib-aux:main.aux`,
`classic-bib:refs.bib`, or `bst:plain.bst`. The classic keys map one-to-one to
the VFS wire kinds `bib-aux`, `classic-bib-data`, and `bib-style`; the shorter
manifest spellings are immutable distribution vocabulary, not a browser-only
translation. Hash its UTF-8 bytes with SHA-256 and interpret the first
`shardBits` bits in network bit order as the numeric shard index. `shardBits`
may be 0 through 16, so every supported shard count is a configurable power of
two.

The host protocol has finer PDF-font semantic kinds than the immutable
manifest vocabulary. `vf`, `font-map`, `font-encoding`, and `font-program`
requests all select the existing `tex:<name>` entry. Native and browser
resolvers retain the original semantic request key in positive and negative
responses; only shard selection uses the transport key. This translation adds
no alias object or mutable identity layer.

Every digest-addressed shard is compact canonical JSON with schema 1,
distribution identity, its numeric `index` (which also makes empty shard
objects distinct), and a `files` map sorted by canonical lookup key. File
values retain the existing `virtualPath`, `object`, `sha256`, and `bytes`
entry fields. Dependency hints are sorted and embed the target `key` plus its
complete `virtualPath`, `object`, `sha256`, and `bytes` fetch metadata.

Inlining makes a hinted fetch independent of the dependency's own shard. The
publisher verifies that every inline record exactly matches its authoritative
entry. Hints remain transport optimization only and do not change engine
resource semantics.

The TeX Live package database supplies same-package peer hints and direct
package dependency representatives. For packages with more than 16 preferred
runfiles, each file receives the next 16 peers in canonical key order, wrapping
at the end of the package. These deterministic rotating windows collectively
cover the package as files are discovered while keeping peer metadata linear
in package size. Cross-package representatives share the existing total budget
of 32 sorted hints per owner.

After the pinned root and selected shard digest validate, absence of a key
from its canonical shard is authoritative distribution absence. No other
shard may contain the key. The staged verifier rejects noncanonical JSON,
wrong partition membership, duplicate keys, missing or stale dependencies,
and any shard, file, or format whose bytes differ from its declared digest.
Thus the root digest transitively pins every shard and every fetchable object.

### Native authenticated-state ownership

Native multi-session hosts retain one parsed root plus compact shard-selection
evidence in an explicit `NativeDistributionOwner`. The owner is bound at
construction to one exact distribution source, optional root digest, and
offline policy; a session whose three fields differ is rejected before
resolution. Its lifetime is the reuse bound: dropping it drops all
authenticated catalogue state. Watch-mode replacement sessions share this
owner, while ordinary one-shot CLI runs create an owner scoped to that run.

For an unseen key, the owner reads and digest-authenticates the complete
canonical shard, strictly parses every record, validates root/shard identity,
and checks every file key's partition before selection. It then retains only
the selected key, virtual path, object name, object digest and length, or an
authoritative negative key. The serialized bytes, complete `ManifestShard`
maps, unselected records, font records, legacy-mapping records, and inline
dependency arrays are dropped together after selection. A later unseen key in
the same shard repeats complete authentication and validation from the verified
persistent manifest cache or immutable source before extending the compact
snapshot. An already selected positive or negative key performs no read,
authentication, or parse.

These retained values are immutable authenticated snapshots. Mutation of a
local root cannot alter a published snapshot, and a fresh owner re-reads and
re-authenticates the pinned root, so a mutation is detected rather than
silently adopted. The root continues to retain format/source identity and every
shard digest; selected records retain deterministic lookup, object
verification, offline reuse, and error evidence. Object payloads are
deliberately not retained by this owner: every fresh engine session still loads
them through the content-addressed blob store, which rechecks cache bytes and
preserves the existing local/cache/remote and offline ladders.

The live native resolver reads and authenticates only the pinned root, the
canonical shards for unresolved required or explicitly prefetched keys, and
the selected objects. Inline dependency records and format closures do not
cause background shard or object access. Consequently `shard_loads`,
`object_requests`, and `object_hashes` are proportional to selected resources,
apart from the single root and deduplicated shard lookups.

Native resolver telemetry separately counts root/shard reads, strict parses,
digest authentications, shard loads, authenticated-owner hits, persistent
manifest cache hits, object payload hashes, and object cache hits. It also
reports the largest authenticated serialized payload passed to one strict parse
separately from the current compact record count, miss count, and exact retained
requested heap bytes. The last number includes the sorted evidence vector's
spare capacity and every owned string capacity; fixed scalar fields reside in
the vector allocation, and allocator bookkeeping remains outside requested
bytes. A failed bounded run emits these fields in
`DISTRIBUTION_MANIFEST_TELEMETRY`, so a command-fuel endpoint does not lose its
final owner measurement. The hermetic
`distribution-startup-benchmark` compares real cold child processes with fresh
sessions under one owner and fails unless manifest work decreases, all DVI
bytes remain identical, and the complete cache byte inventory remains
unchanged. Its valid unrequested dependency control must remain outside the
cache, and each measured compile must report exactly one requested object hash.

Complete immutable-graph and cache auditing is explicit:

```bash
CARGO_BUILD_JOBS=1 cargo run-dev -p umber --bin distribution-verify -- \
  --distribution target/texlive-snapshot \
  --distribution-sha256 <pinned-root-sha256> \
  --cache <umber-cache-root>
```

The command requires a root pin, strictly parses canonical root and shard
bytes, assembles the complete cross-shard graph, streams every referenced
object digest, and authenticates every current `blobs-v1` cache envelope. It
is read-only and reports exact root, shard, object, blob, and hashed-byte work.
Mutation of a root, shard, referenced object, envelope, or payload fails the
command. Neither `umber run` nor `umber watch` invokes this exhaustive walk.

## Publisher and release workflow

`tools/texlive-wasm-publish` emits schema-3 roots directly. Root values, shard
values, canonical JSON serialization, partitioning, omission rules, and
complete cross-shard graph validation are constructed by
`umber-distribution`; the publisher owns only source scanning, dependency-hint
derivation, object hashing, and filesystem publication. The production
builder accepts `--shard-bits` (default 8), performs two clean builds, and
requires byte-identical directory trees. `--shard-existing STAGING
--shard-bits BITS` converts a verified schema-1 staging bundle without
re-reading TeX Live, while `--verify-sharded STAGING` performs the complete
offline integrity check used by the R2 publication script.

The publisher also accepts an explicit `html` profile (configuration schema
4). It verifies every configured source-tree pin, then publishes only the
union of declared runtime file keys and the authenticated closures of selected
schema-2 format metadata. Selected TEXMF objects must be under `tex/` or be
TFM metrics. The accompanying file-free schema-2 catalog supplies exact WOFF2
font and legacy-mapping records, and `objectSources` must exactly cover their
content-addressed WOFF2 and license objects. Mapping TFM digests must name a
selected TFM object. The build rejects VF, AFM, ENC, PDF/dvips maps, PK, Type
1, TrueType, and OpenType transport inputs even when they exist in a pinned
source root.

The contract-version-1 production catalogue is committed canonically at
`tools/texlive-wasm-publish/catalog/html-mvp-v1.json` and inventoried in
[HTML MVP Font Catalog Inventory](html_font_catalog.md). It is the reviewed
data authority rather than the output of an executable shadow catalogue. The
publisher strictly parses it and verifies the exact digest and length of every
declared WOFF2 and license object before staging. The native audit independently
validates decoded program identities, the complete 256-entry legacy map, cmap
coverage, MATH presence, shared CMU object reuse, and affirmative license
capabilities.

HTML staging uses independent ceilings for logical files, unique staged
objects, total staged object bytes, font records, mapping records, and unique
licenses. Verification rehashes every runtime, format, shard, WOFF2, and
license object and checks each mapping-to-font/license link. A successful
rebuild removes unreferenced staging objects. `scripts/publish-texlive-r2.sh
--profile html` requires a distinct `html/` immutable prefix and explicit root
SHA-256, audits the complete remote object inventory, verifies public HTTPS
digests and CORS, and writes `manifest-v4.json` only after those checks.
`scripts/build-html-r2.sh` is the production staging entry point for this
profile. It performs the two-clean-build comparison against the pinned source
tree and exact contract-version-1 catalog before the publication script may
address `html/umber-html-mvp-v1/manifest-v4.json`.

### 2026-07-22 HTML MVP publication receipt

The contract-version-1 HTML profile was built twice from the pinned source
tree and exact catalog; the two verified directory trees matched byte for
byte. The schema-4 root has 16 schema-2 shards and reaches exactly 86 unique
objects totaling 9,304,142 bytes. The immutable public application pin is
`https://assets.umber.ink/html/umber-html-mvp-v1/manifest-v4.json`, SHA-256
`42fdceeaecf0e80c072bb69cf3b77f0cb20e755f69110c04124474fadb1cd5fc`.

Publication to `umber-assets/html/umber-html-mvp-v1` used immutable object
writes. The final remote audit found zero differences across all 86 staged
objects and exact remote count and byte equality. Remote metadata places the
root manifest after every object, proving manifest-last ordering. The public
manifest is byte-identical to staging, reports schema 4, distribution
`umber-html-mvp-v1`, 16 shards, and the pinned content-addressed object base.
It and deterministic first, middle, and last objects passed HTTPS digest and
wildcard-CORS verification.

This hosted profile certifies only the three selections inventoried in
[html_font_catalog.md](html_font_catalog.md): exact `cmr10` mapping, explicit
CMU Serif text, and explicit STIX Two Math. Its runtime and format closures do
not claim arbitrary HTML legacy coverage. Full local/client TFM, VF, map, ENC,
PK, Type 1, TrueType, and OpenType scope remains a separate typed DVI/PDF
provider capability under [cross_output_fonts.md](cross_output_fonts.md).

The production `texlive-20260301` 8-bit output has 152,560 unique objects,
3,520,195,192 object bytes, and root digest
`43a31da364e4607957a38da10dabff227657d607d1845d502204adfd5d002e4b`.
The deployed immutable public key is `manifest-v3.json`. Publication remains
manifest-last: all content and shard objects are uploaded and checked before
that root key.

### 2026-07-21 successor publication receipt

The successor snapshot adds the generated `texmf-var` aggregate
`pdftex.map` as a deterministic third publisher root; this is a
hosting/resource correction and does not change core TeX, e-TeX, or pdfTeX
semantics. The clean schema-3 staging verifier accepted all 256 shards,
152,560 objects, and 3,520,195,192 object bytes. Publication to the new
immutable `texlive/texlive-20260301` prefix completed through the repository
publisher with zero object differences and exact remote inventory equality
before the first manifest write. The public manifest digest is
`43a31da364e4607957a38da10dabff227657d607d1845d502204adfd5d002e4b`;
the manifest and deterministic first, middle, and last object representatives
passed HTTPS digest and browser CORS checks.

Authenticated public shard lookup and payload fetch additionally verified the
default map and representative arXiv 1204.5690 font closure:

| key                    | SHA-256                                                            |     bytes |
| ---------------------- | ------------------------------------------------------------------ | --------: |
| `tex:pdftex.map`       | `622cafc1a370ada71b298ee0396620bc49decb82c7472a5daa75124612f57f0b` | 5,541,360 |
| `tex:cm-super-ts1.enc` | `558da5de87db45ed719dda9c679e6b164d520b21d9100357dcf17124291ed97c` |     2,900 |
| `tex:cmbx10.pfb`       | `ca41102968b817bf6e8b22fd6de205ca23bf5088218511cce0c8129e1577cb70` |    34,811 |
| `tex:cmr10.pfb`        | `fdcede8794018df5f2b58f0905fb20a2b418ed8f67b73ee12445855dfbe5b1be` |    35,752 |
| `tex:cmsy10.pfb`       | `62ee8cef552017551cd3e026a483e700730103eceaad959c87b7730017f59cff` |    32,569 |
| `tex:sfrm0800.pfb`     | `5882372155f6b14414ffd9572947fb1415dc81ff4bc12b673219594e927bb44a` |   164,227 |

The native CLI was rebuilt with that URL and digest as its defaults, then run
from a clean cache on the pristine arXiv 1204.5690 archive with `TEXFONTS`
explicitly unset and no paper edits. The cold pass authenticated and cached the
hosted inputs; its immediate warm continuation accepted the document and
finalized an eight-page, 1,044,673-byte PDF with 26 embedded fonts. This proves
that the default map, encoding, and font-program path no longer depends on a
host font tree. The authored browser resolver exports the identical production
URL and digest.

### 2026-08-11 LaTeX coherence successor staging receipt

An authenticated sparse successor was derived from production root
`43a31da364e4607957a38da10dabff227657d607d1845d502204adfd5d002e4b`
without copying or weakening its unchanged content-addressed payloads. The
publisher authenticated all 256 base shards, overlaid the complete locked
format-construction root, and emitted every successor shard plus each changed
payload. Two clean publications were byte-identical. The schema-3 successor
root is `61b8d665e492662b18c8beb70ab8cd8a8f73d9bd7e4d9aeb2f958ea8613f8883`;
its sparse staging set contains 322 objects totaling 102,653,889 bytes.

The successor binds source-manifest identity
`ba49b8698d222b16afc26811c03d52125b632545c963ad8b506a6939e91925db`
to LaTeX format
`9ea2783f3000423606a274974145f7b07580753bbd53682a10c3a55b3f4b9fd9`
and pdfLaTeX format
`1dce7159b8c974ebfd896a74edff4c3f6870497ccb28fc3979b2cdf5ef773a6f`.
The intended unique hosted key is
`manifest-v3-latex-dev-20260601.json`. Hosted defaults must not move to this
identity until manifest-last publication and public digest/CORS verification
complete; the prior default remains unambiguous in the meantime.

`python3 scripts/provision.py snapshot` performs verified builds for both
`latex.fmt` and `pdflatex.fmt`. It derives their 61-key common and 64-key PDF
closures from `tests/latex-source.lock`, stages the complete authenticated
construction closure as the highest-precedence TEXMF root, and publishes both
format closures in the schema-3 root. This prevents a basename winner from a
different LaTeX release line from drifting away from the frozen format's input
receipt. Two clean publications must still be byte-identical.
Format construction requires an already materialized local authority selected
with `--format-distribution` and `--format-distribution-sha256`; their
authenticated checkout-local defaults are the existing
`target/texlive-snapshot` root and the digest pinned by
`tests/latex-source.lock`.
`scripts/publish-texlive-r2.sh` retains manifest-last upload order for full
snapshots and authenticated sparse successors; a successor must use a unique
schema-3 root key and may reuse only the base's immutable content-addressed
object namespace.

### 2026-08-12 Babel language successor staging receipt

The format-local language configuration retains Knuth's English patterns in
slot zero and now carries the `usenglish`, `USenglish`, and `american`
synonyms from the beginning of the pinned TeX Live `language.dat`. This
51-byte configuration is SHA-256
`0cf9b22368f29227d8ac86036bc65611b42ae5ef5f04b448a0b0609fd395c42d`.
It is both an authenticated format-construction input and the published
`tex:language.dat` winner, so Babel 26.3 creates every US English selector
that its paired runtime may request without changing the deterministic
English hyphenation slot.

Both formats were regenerated twice under the standard format guards and
passed source-loaded equivalence while exercising Babel's `USenglish` option.
LaTeX is
`45cd47858a905db3bce9ab333e664883fa530801eae09b81d86000dffd328727`
(1,988,314 bytes), and pdfLaTeX is
`32ae8a46f86ecc3520b48ff6739fa413170f7b34c2263560d7d589abe1466a7b`
(2,030,553 bytes). Both name source manifest
`6b00b710b60d4fcf21b792abfb5446963ee58a55d654574248b0c8d62027098b`.

Two authenticated sparse publications derived from root
`61b8d665e492662b18c8beb70ab8cd8a8f73d9bd7e4d9aeb2f958ea8613f8883`
were byte-identical. The successor root is
`560ab65f2a4933879b05e47554a9d94434ec1e94ff8f6caa163d26cde7fe35bd`;
its sparse staging contains 322 objects totaling 102,654,615 bytes. This is an
issue-local staging receipt only. No hosted root, application default, or R2
object was changed.

When the pinned source tree is unavailable, materialize an authenticated local
subset directly from the immutable hosted publication:

```sh
python3 scripts/provision.py materialize \
  --keys-from tests/latex-source.lock \
  --keys-from tests/latex/pdflatex-representative.lock
python3 scripts/provision.py materialize \
  --keys-from tests/latex-source.lock \
  --keys-from tests/latex/pdflatex-representative.lock \
  --offline
```

The command preserves the exact `manifest-v3.json` trust root, stages all
root-listed shards into a metadata-complete execution mirror, and downloads
only the payloads selected by requested keys and format closures. Every shard,
format, and selected file is stored under its verified digest name in
`target/texlive-snapshot/objects`; unselected payloads remain absent. The
command also constructs a sparse, verified
`target/texlive-snapshot/texmf-dist` view for tools that require paths. The
second command performs no network I/O and proves that the checkout-local root,
complete shard set, and selected payload set can be reauthenticated without any
seed. Add canonical `kind:name` records with `--key` or a one-key-per-line file
with `--keys-from`; repository lock records of the form
`source KIND PATH BYTES SHA256` are also accepted. The same option directly
accepts `tests/latex-source.lock`, including its common/local and PDF-specific
source rows, and derives their canonical TeX or TFM request kinds. Source-row
identities must equal the selected shard record. Repository-local rows select
the corresponding distribution prefetch key but retain their independent
on-disk identity because the local construction root deliberately shadows the
published basename winner.

`tests/latex/pdflatex-representative.lock` is the authenticated positive
runtime closure shared by the pdfLaTeX representative runs. The
source-initialized run combines its ten rows with the 64 construction keys from
`tests/latex-source.lock`; the loaded-format run combines those ten rows with
the named `pdflatex` format object instead. Local document, fixture, and AUX
files are intentionally outside the distribution closure. The lock records
the authoritative virtual path, byte length, and SHA-256 for every positive
runtime lookup, so the materializer verifies the selected shard identity and
can seed exact producer TEXMF bytes without weakening the pinned root.

An authenticated local producer can seed the identical selective workflow
without contacting the hosted origin:

```sh
python3 scripts/provision.py materialize \
  --root-path /path/to/pinned/manifest-v3.json \
  --root-sha256 <pinned-root-sha256> \
  --object-root /path/to/digest-named/objects \
  --texmf-root /path/to/authenticated/texmf-dist \
  --keys-from tests/latex-source.lock \
  --keys-from tests/latex/pdflatex-representative.lock \
  --output-dir target/texlive-snapshot \
  --offline
python3 scripts/provision.py materialize \
  --root-sha256 <pinned-root-sha256> \
  --keys-from tests/latex-source.lock \
  --keys-from tests/latex/pdflatex-representative.lock \
  --output-dir target/texlive-snapshot \
  --offline
```

`--object-root` names only a directory of raw `sha256-<digest>` objects; it
does not accept or copy a native cache namespace. `--texmf-root` supplies
preferred payload candidates by the manifest's authenticated virtual path;
digest-named object roots are the exact fallback. The
materializer stages and authenticates every shard named by the pinned root,
selects only requested payloads, verifies their exact declared lengths and
SHA-256 identities, and then constructs its sparse TEXMF view. This is an exact
digest-addressed traversal rather than a broad object-root or cache copy. The
seed-free second command is the offline completeness proof for that exact root,
catalogue, and payload selection.

An accepted native PDF run may write the engine-owned classic-font closure with
`--pdf-font-closure-out`. Its schema-1 TSV retains the semantic request kind,
canonical manifest key, virtual path, size, SHA-256, and authoritative
unavailable VF probes. `materialize --keys-from` consumes resolved rows
directly and pins their recorded identities against the authenticated shard.
Unavailable rows select no payload; they seed the canonical shard lookup and
must remain absent there. Consequently a closure receipt can be materialized
online and then repeated with `--offline` without transcribing or guessing font
names, while retaining its negative evidence.

The builder additionally verifies `tests/texlive-snapshot.lock` before it
publishes anything. That lock fixes the publisher-visible tree digest and the
exact 2026-03-01 LaTeX kernel, latex-dev `array.sty` v2.7a, and pdfTeX map
bytes, preventing a mutable year directory or a newer cached package from
being labeled as the pinned snapshot. Package metadata normally remains
required. `--without-package-database` exists only for a content-exact local
regeneration when the preserved snapshot lacks `texlive.tlpdb`; it omits
package dependency hints but cannot weaken tree, format-closure, inventory, or
object verification and is not suitable for production publication.

Native object and manifest cache namespaces are shared across authenticated
distributions; content addressing prevents byte confusion but does not make a
cache listing snapshot-exclusive. A default-hosted Umber run can therefore
repopulate hosted `texlive-20260301` manifest entries immediately after a
purge. For snapshot-sensitive corpus work, first stop concurrent Umber runs,
clear only the Umber `objects` and `manifests` namespaces, and warm with an
explicit `--distribution` path to the regenerated 2026-03-01 staging root.
Then rerun with `--offline` and the same explicit distribution and require
identical output. Do not infer provenance from cache recency or the year in a
mutable hosted distribution name.

### Format-closure retry verification receipt

The focused
`format_closure_batch_is_installed_for_an_exactly_two_attempt_retry` native
host test constructs canonical schema-3 distributions with runtime-created
schema-11 LaTeX and pdfLaTeX formats and nested closures at the production
cardinalities. Run it with:

```bash
cargo test -q -p umber \
  format_closure_batch_is_installed_for_an_exactly_two_attempt_retry \
  -- --nocapture
```

Both cases fetch and authenticate the full 61- or 64-key closure in the first
host batch, publish its validated positive file responses atomically, and
reach the synthetic bootstrap terminal state on compile attempt two. Separate
tests cover local and user precedence, stale hints without negative bindings,
resource budgets, and the equivalent browser resolver handoff.

The same verification exercised the repository-local pinned
`third_party/texlive-2026/texmf-dist`. The LaTeX builder produced no format or
terminal diagnostic before it was stopped after 689.50 seconds; the pdfLaTeX
builder likewise produced no format or diagnostic during a bounded 69.70-second
observation (including a 19.33-second release rebuild). This is the independent
early-completion/bootstrap path tracked by `umber2-pbxv.5.4.1`; it prevents the
schema-11 determinism, source-versus-format, corpus, and live WASM gates from
reaching their comparison phases and is not hidden by weakening those gates.

### Production shard selection and publication evidence

The 2026 snapshot uses 256 shards (`shardBits = 8`). Future corpus-based shard
measurements use `scripts/pdftex-arxiv-recent-sample-100.tsv`.
`scripts/measure-sharded-manifest.py`
reconstructs candidate roots and package-complete shards in memory from a
schema-1 staging manifest without changing or uploading content.

On 2026-07-16 the 256 shard objects were uploaded through the configured R2
profile with immutable writes after an existing 548-byte object passed both
R2 and public-HTTPS digest checks. All 256 remote shard sizes matched staging,
and public HTTPS digest plus CORS checks passed for shards 0, 127, and 255.
Only then was `manifest-v2.json` published. Its public response is 17,743
bytes, has SHA-256
`7c2784bca891844d37465083b93466b78429c7282d7ba915f40a08d150651fd0`,
uses `application/json`, and permits cross-origin reads. The old
`manifest.json` and all older content-addressed objects remain intact.

Cross-frontend verification uses the shared distribution fixture to assert the
same canonical request keys, shard partitions, selected objects, dependency
hints, and typed misses in Rust and authored JavaScript. At the production pin,
native resolver tests cover clean shard selection, inline hints, warm-cache
offline reuse, authoritative absence, and corrupt-shard rejection; browser
tests cover the corresponding root-pin, shard, hint, persistent-cache,
absence, and tamper paths. Both frontends pin the URL above and the same root
digest, so a successful resolution supplies identical authenticated bytes to
the shared compile session and preserves engine-output parity.

The authored JavaScript resolver requires both the root URL and its lowercase
SHA-256 pin. It verifies the bounded root bytes before passing the exact text to
the synchronous `umber-wasm` adapter over `umber-distribution`. Rust returns one
prepared shard batch, reauthenticates every exact fetched shard byte string
against the root, and returns the canonical required-before-hint/miss plan with
complete selected records. JavaScript retains only HTTP, cache, concurrency,
cancellation, resource-budget, and response-materialization policy. A verified shard miss becomes a typed unavailable answer;
HTTP, CORS, cancellation, size, and integrity failures remain errors.

Dependency hints are consumed directly from their full inline fetch metadata,
without loading the dependency's shard. Shards and payloads remain immutable
and reusable across compiler sessions. The browser package exports the pinned
production `manifest-v2.json` URL and digest as
`TEXLIVE_2026_MANIFEST_URL` and `TEXLIVE_2026_MANIFEST_SHA256`.

For schema 3, format closure keys use the same canonical shard selection. The
resolver may load those shards concurrently after the first miss, but hinted
misses and transport failures are non-blocking; only the current required
selection can produce unavailable responses or actionable acquisition errors.

`umber-distribution` strictly parses the pinned root and individual index
shards without performing I/O. The native CLI verifies the root pin, maps TeX
requests to canonical distribution keys, consumes the shared partition and
selection plan, and verifies the selected shard
through the digest-keyed manifest cache before treating absence as
authoritative. It fetches inline dependency records directly, so dependency
hints never require another index lookup. Root, shard, and ordinary object
cache entries are all reverified on read; an offline compile succeeds with a
fully warm cache without network access.

## Legacy monolithic API disposition

The public schema-1 `Manifest` parser and associated monolithic record types
remain a compatibility and publication input surface.
`texlive-wasm-publish --shard-existing` consumes that parser to convert an
already verified monolithic staging bundle, and the publisher constructs the
same records before `umber-distribution` creates the sharded catalogue. They
are therefore retained until that documented conversion command is retired or
migrated; deleting them would remove a supported offline publication path.

The monolithic pretty writer and selection planner were retired after the
schema-1 DTO migration. A repository-wide caller audit found that neither was
used by `--shard-existing`, publisher assembly, native resolution, WebAssembly,
authored JavaScript, or the npm package; their only caller was a self-test and
its now-retired selection fixture. Schema 1 is an accepted publication input,
not an emitted catalogue or live selection authority. The retained reader
continues to enforce its complete strict validation contract, while all live
selection uses authenticated shards.

They are not a second browser wire authority. WebAssembly and authored
JavaScript expose only schema-1 DTOs for prepared shard batches,
authenticated selection plans, and named formats. Browser resolution never
parses or selects from the monolithic model.

The small authored-JavaScript request-key adapter remains intentionally. It
maps public resource DTO kinds onto the immutable manifest vocabulary, gives
the provider-composition facade stable request identities, and reconstructs
typed responses from authenticated plans. It does not parse catalogue JSON,
choose shards or records, authenticate bytes, or define selection order.
Moving this package-facing adaptation behind another synchronous WebAssembly
call would change the injectable `DistributionCatalogBindings` interface and
would not remove a second catalogue authority.
