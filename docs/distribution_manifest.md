# Sharded Distribution Manifest

Status: schema-2 browser/native resolution and schema-3 format-closure publishing and runtime prefetch consumption implemented.

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
is selected, native and browser hosts translate its closure into typed file
requests. The compile session emits the deduplicated closure once in
`NeedResources.prefetch_hints`, alongside the first actual format-input miss.
Schema-2 roots simply contribute no hints.

Hints remain optional transport advice. Resolvers fetch the authenticated
closure in one bounded speculative batch and may return positive responses for
the exact file hints emitted by the session. The session authorizes only that
one-shot set, validates and installs the complete response batch atomically,
and still measures retry progress from required requests only. User files are
removed before hint emission, native local search has first refusal, and stale,
absent, failed, or over-budget hints produce neither responses nor unavailable
bindings. Transitive dependency prefetches remain cache-only transport work.

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

After the pinned root and selected shard digest validate, absence of a key
from its canonical shard is authoritative distribution absence. No other
shard may contain the key. The staged verifier rejects noncanonical JSON,
wrong partition membership, duplicate keys, missing or stale dependencies,
and any shard, file, or format whose bytes differ from its declared digest.
Thus the root digest transitively pins every shard and every fetchable object.

## Publisher and release workflow

`tools/texlive-wasm-publish` emits schema-3 roots directly. The production
builder accepts `--shard-bits` (default 8), performs two clean builds, and
requires byte-identical directory trees. `--shard-existing STAGING
--shard-bits BITS` converts a verified schema-1 staging bundle without
re-reading TeX Live, while `--verify-sharded STAGING` performs the complete
offline integrity check used by the R2 publication script.

The production `texlive-2026-r79639` 8-bit output has 154,153 unique objects,
3,672,643,852 object bytes, and root digest
`7c2784bca891844d37465083b93466b78429c7282d7ba915f40a08d150651fd0`.
The deployed immutable public key is `manifest-v2.json`; the already cached
schema-1 `manifest.json` is not overwritten. Publication remains manifest-last:
all content and shard objects are uploaded and checked before that root key.

`scripts/build-texlive-snapshot.sh` now performs verified builds for both
`latex.fmt` and `pdflatex.fmt`. It derives their 57-key common and 60-key PDF
closures from `tests/latex-source.lock`, stages the two repository-local
configuration inputs as a pinned auxiliary TEXMF root, and publishes both
format closures in the schema-3 root. Two clean publications must still be
byte-identical. `scripts/publish-texlive-r2.sh` reserves `manifest-v3.json` for
this new immutable contract and retains the manifest-last upload order.

### Format-closure retry verification receipt

The focused
`format_closure_batch_is_installed_for_an_exactly_two_attempt_retry` native
host test constructs canonical schema-3 distributions with runtime-created
schema-10 LaTeX and pdfLaTeX formats and nested closures at the production
cardinalities. Run it with:

```bash
cargo test -q -p umber \
  format_closure_batch_is_installed_for_an_exactly_two_attempt_retry \
  -- --nocapture
```

Both cases fetch and authenticate the full 57- or 60-key closure in the first
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
schema-10 determinism, source-versus-format, corpus, and live WASM gates from
reaching their comparison phases and is not hidden by weakening those gates.

### Production shard selection and publication evidence

The 2026 snapshot uses 256 shards (`shardBits = 8`). Candidate layouts were
regenerated from the same verified schema-1 staging manifest with
`--shard-existing` and passed `--verify-sharded`. The sizes below use the
canonical uncompressed JSON and the sum of independently compressed roots or
shards from `gzip -n -c`; the shard totals include package-complete inline
dependency metadata.

| Shards | Root bytes | Root gzip bytes | All shard bytes | All shard gzip bytes | 100 independent cold runs: requests / gzip bytes |
| -----: | ---------: | --------------: | --------------: | -------------------: | :----------------------------------------------- |
|     64 |      4,878 |           2,840 |     164,926,880 |           30,453,157 | 4,970 / 2,318,727,611                            |
|    256 |     17,743 |          10,276 |     164,940,668 |           33,415,396 | 9,464 / 1,223,882,079                            |
|  1,024 |     69,201 |          39,809 |     164,995,988 |           36,025,682 | 12,260 / 434,397,963                             |

The follow-up measurement on 2026-07-16 acquired all 100 identifiers in the
committed, SHA-256 `bac78153b3d9fa4455020288511c1766e95dc9da551bd47f38e7e162ff09f11c`
sample. The pinned pdfTeX 1.40.27 build ran against the preserved TeX Live 2026
runtime used to build the snapshot. It completed 76 papers and retained the
deterministic pre-error accesses from 24 papers. Those partial traces make the
numbers a measurement of reached inputs, not an extrapolation of hypothetical
successful builds.

The recorder produced 100 raw `.fls` traces and 13,262 per-paper normalized
input-path occurrences. Replay matched 1,864 unique published lookup keys.
The only unmatched path was `/texlive/web2c/texmf.cnf`, which is configuration
rather than a publishable runtime object. Because `.fls` records resolved
physical paths rather than the original lookup spelling, replay projects each
path to the publisher's basename-precedence alias; this limitation is explicit
in the machine report.

The cold column above resets the root and shard cache for every paper, includes
one root request per paper, and sums the gzip bytes for each root and unique
shard requested by that paper. Package-complete dependency hints remain inline
in the owner shard, so their metadata contributes to compressed shard size but
does not add shard requests. The request/byte tradeoff has no single scalar
knee: 64 shards minimize requests, while 1,024 shards minimize cold bytes; 256
is the middle point selected before publication. No R2 object or manifest was
changed for this measurement.

For a persistent cache shared across the corpus, the corresponding union was
65 requests / 30,455,997 gzip bytes at 64 shards, 257 / 33,425,672 at 256,
and 872 / 30,733,086 at 1,024. Thus this broad corpus eventually touches every
64- and 256-way shard and 871 of 1,024 shards.

`scripts/profile-pdftex-arxiv.sh` now captures both primitive usage and file
access, while `scripts/measure-sharded-manifest.py` reconstructs candidate
roots and package-complete shards in memory from a schema-1 staging manifest.
It is read-only: it neither invokes the publisher's mutating conversion mode
nor uploads content. A reproduction using a pinned TeX Live runtime and an
unmodified schema-1 staging bundle is:

```bash
PATH=/path/to/texlive/bin/universal-darwin:$PATH \
  PDFTEX_PROFILE_ROOT=/tmp/umber-pdftex-file-trace \
  scripts/profile-pdftex-arxiv.sh all
scripts/measure-sharded-manifest.py \
  /path/to/texlive-2026-r79639/manifest.json \
  /tmp/umber-pdftex-file-trace/results \
  --output /tmp/umber-pdftex-file-trace/shard-knee.tsv
```

For audit, the captured summary SHA-256 was
`124bd670c24217d4150b5ef86e55bbda7374c9d6bc65617ac897fde6929b3537`
and the machine report SHA-256 was
`afe9ca14c36b87a7be1f0c5eb67af8704ecc304e03f4eacd0754aef892dbc077`.
The replay also regenerated the already-published 256-shard root digest
`7c2784bca891844d37465083b93466b78429c7282d7ba915f40a08d150651fd0`
and its 10,276 gzip bytes exactly.

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
SHA-256 pin. It verifies the bounded root bytes before parsing selection
metadata, hashes each canonical lookup key to its shard, and fetches the
digest-addressed shard through the same HTTP or IndexedDB verified-object cache
as content payloads. A verified shard miss becomes a typed unavailable answer;
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
shards without performing I/O. The native CLI verifies the root pin, hashes
each unresolved canonical lookup key to its one shard, and verifies that shard
through the digest-keyed manifest cache before treating absence as
authoritative. It fetches inline dependency records directly, so dependency
hints never require another index lookup. Root, shard, and ordinary object
cache entries are all reverified on read; an offline compile succeeds with a
fully warm cache without network access.
