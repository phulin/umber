# Sharded Distribution Manifest

Status: publisher contract plus browser and native shard resolution implemented.

## Trust root

The release pin names `manifest-v2.json` and its SHA-256 digest. The file is a
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

## Partition and shard schema

The canonical lookup key is the existing case-sensitive `kind:name` string,
for example `tex:article.cls` or `tfm:cmr10.tfm`. Hash its UTF-8 bytes with
SHA-256 and interpret the first `shardBits` bits in network bit order as the
numeric shard index. `shardBits` may be 0 through 16, so every supported shard
count is a configurable power of two.

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

`tools/texlive-wasm-publish` emits schema-2 roots directly. The production
builder accepts `--shard-bits` (default 8), performs two clean builds, and
requires byte-identical directory trees. `--shard-existing STAGING
--shard-bits BITS` converts a verified schema-1 staging bundle without
re-reading TeX Live, while `--verify-sharded STAGING` performs the complete
offline integrity check used by the R2 publication script.

The production `texlive-2026-r79639` 8-bit output has 154,153 unique objects,
3,672,643,852 object bytes, and root digest
`7c2784bca891844d37465083b93466b78429c7282d7ba915f40a08d150651fd0`.
The new immutable public key is `manifest-v2.json`; the already cached
schema-1 `manifest.json` is not overwritten. Publication remains manifest-last:
all content and shard objects are uploaded and checked before that root key.

### Production shard selection and publication evidence

The 2026 snapshot uses 256 shards (`shardBits = 8`). Candidate layouts were
regenerated from the same verified schema-1 staging manifest with
`--shard-existing` and passed `--verify-sharded`. The sizes below use the
canonical uncompressed JSON and the sum of independently compressed roots or
shards from `gzip -n -c`; the shard totals include package-complete inline
dependency metadata.

| Shards | Root bytes | Root gzip bytes | All shard bytes | All shard gzip bytes | Cold requests / bytes |
| ---: | ---: | ---: | ---: | ---: | :--- |
| 64 | 4,878 | 2,840 | 164,926,880 | 30,453,157 | unavailable: no file-access trace corpus was present |
| 256 | 17,743 | 10,276 | 164,940,668 | 33,415,396 | unavailable: no file-access trace corpus was present |
| 1,024 | 69,201 | 39,809 | 164,995,988 | 36,025,682 | unavailable: no file-access trace corpus was present |

These whole-index sizes are reproducible layout evidence, not a substitute
for replaying the cold working set. The checked-in
`scripts/profile-pdftex-arxiv.sh` records primitive use rather than file
access, and neither its 100-paper sources nor separate file-access traces were
available during the production run. Consequently the release retained the
requested 256-shard production prior; it does not claim an empirical knee
between the three candidates. Follow-up `umber2-mbwq.6.6` owns deterministic
file-access capture and replay. Core-pack warming was not added because there
was no trace evidence to justify expanding the existing dependency-hint
working set.

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

`umber-distribution` strictly parses the pinned root and individual index
shards without performing I/O. The native CLI verifies the root pin, hashes
each unresolved canonical lookup key to its one shard, and verifies that shard
through the digest-keyed manifest cache before treating absence as
authoritative. It fetches inline dependency records directly, so dependency
hints never require another index lookup. Root, shard, and ordinary object
cache entries are all reverified on read; an offline compile succeeds with a
fully warm cache without network access.
