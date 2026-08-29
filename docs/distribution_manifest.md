# Packed Distribution Shards

This document is the current distribution catalogue, publication, cache, and
trust-boundary contract. Canonical request semantics are defined by
[Canonical resource identity and lifecycle](resource_lifecycle.md).

## Identity contract

Repository-owned distribution content uses `ahash64-v1`: 16 lowercase
hexadecimal digits produced by `umber-hash` with fixed seeds, algorithm version
1, and explicit domains. The implementation is portable Rust rather than
`RandomState` or upstream `AHasher`, whose output is not a persisted-format
contract.

An aHash64 identity is a deterministic content selector and accidental
corruption check. It is not collision resistant and does not authenticate a
hostile publisher. HTTPS and the release channel establish publisher
provenance. Exact key bytes, lengths, paths, and object metadata are still
validated after every digest check.

The SHA-era hosted distribution is deliberately incompatible. Native and
browser defaults report `default-distribution-unpublished` until
`umber2-66p0.27` republishes the objects, packed shards, roots, formats, HTML
catalogue, and root pins. There is no SHA or JSON-shard fallback.

SHA-256 remains only at external compatibility boundaries: licensed source and
archive locks, reference-oracle and corpus evidence, and output parity receipts.
Those values are not distribution object identities or cache keys.

## Persisted schemas

- Monolithic publisher input schema 2 uses aHash64 object entries.
- Full root schema 8 and HTML root schema 9 name packed shard payloads.
- Every newly produced shard is packed schema 2 with magic `UMBRPKS2`.
  Readers retain explicit schema-1/`UMBRPKS1` compatibility for authenticated
  roots published before the canonical-table cutover; producers never emit it.
- Font and legacy-mapping records are schema 2 inside the packed payload.
- Producer format metadata is schema 3 without an input closure and schema 4
  with a schema-1 input closure.
- Umber format images are schema 12 and page artifacts are schema 24.
- Native cache envelopes are schema 2 under `blobs-v2`.

The small root remains canonical JSON because it is fetched and parsed once.
It contains `distribution`, `objectsBaseUrl`, `shardBits`, `shardCount`, the
ordered bare-digest `shards` array, and optional inline formats. `shardCount`
equals both `2^shardBits` and the array length. Old root schemas and JSON shard
payloads are rejected rather than upgraded implicitly.

Objects are named `ahash64-v1-<16 lowercase hex digits>`. Root JSON maps are
sorted by raw UTF-8 key order and the document ends in one newline. Shard bytes
are canonical little-endian binary and contain no host `usize`, enum layout, or
pointer representation.

## Shard selection and lookup

Publisher scan order remains configured root order followed by normalized path
order. The first canonical request winner is frozen before packing, so runtime
lookup never reconstructs publisher precedence or walks candidates.

The canonical request key is hashed once in `DistributionShardKey` domain 2.
Its leading `shardBits` select exactly one physical shard; the low bits seed
that shard's open-addressed table. Buckets use linear probing. A matching
64-bit hash is only a candidate: lookup compares the stored key length and
exact UTF-8 key bytes before returning a record. A verified empty probe-chain
slot is authoritative absence. Root, digest, schema, identity, partition,
offset, or table failure remains corruption and never becomes absence.

The table size is a power of two and the validator rejects load above 80%.
Each fixed 80-byte header records the distribution and shard identity, counts,
and exact section offsets. Sections are contiguous in this order:

| Section      | Row bytes | Contents                                                            |
| ------------ | --------: | ------------------------------------------------------------------- |
| buckets      |        16 | aHash64, record index or `u32::MAX`, reserved zero                  |
| records      |        32 | key span, kind, object/path indexes, dependency span, metadata span |
| objects      |        16 | numeric aHash64 and declared byte length                            |
| paths        |         8 | offset/length into the shared string blob                           |
| dependencies |        16 | key span plus object and path indexes                               |
| keys         |  variable | deduplicated canonical request-key bytes                            |
| strings      |  variable | distribution name, deduplicated paths, and catalogue metadata       |

Object, path, and dependency records are referenced by compact indexes. In
schema 2, object rows are strictly ordered by numeric aHash64 digest and path
rows are strictly ordered by raw UTF-8 bytes. The producer builds those final
canonical tables first and assigns record and dependency references from their
renumbered indexes. File
dependencies are strictly key-sorted spans and carry their already-resolved
object/path hint, so prefetch never loads another index shard. Font and legacy
mapping metadata use bounded explicit encodings in the same string section.
Primary records are strictly sorted by raw UTF-8 key bytes. The packed key
blob is valid UTF-8 as a whole, so validation scans those bytes once and then
checks record and dependency spans at character boundaries without decoding
the same key again.

`ValidatedPackedShard` owns the authenticated bytes and checks the complete
layout in owning table passes: object and path identity, dependency rows,
strict record order and policy, and bucket coverage and probe reachability.
Before any table pass or table-sized scratch allocation, it computes every
fixed-section end without narrowing and proves that the end fits the packed
`u32` address space, the declared total length, and the authenticated byte
slice. Validation scratch uses fallible reserves after those bounds are
established, so malformed counts return a packed-shard error rather than
requesting an attacker-sized allocation.
Schema-2 object and path admission compares adjacent borrowed rows directly in
one linear pass, rejecting disorder, duplicates, conflicting object lengths,
invalid spans, and invalid paths without copying or sorting either table.
Schema-1 compatibility retains its legacy encounter-order duplicate proof.
The probe proof unwraps the circular table after one guaranteed empty bucket
and scans every bucket once; it does not replay a live lookup for every record.
Successful lookup thereafter borrows already validated key, path, object, and
dependency spans directly without repeating path or reserved-field policy. It
does not parse JSON, build a `BTreeMap`, materialize every record, or allocate
while probing.

## Native and browser acquisition

Native and WebAssembly use the same `umber-distribution` packed validator and
lookup view. Native `DistributionResolver` retains each touched
`Arc<ValidatedPackedShard>` for the resolver session. There is no selected-hit
vector, selected-miss cache, or reparsing path: later keys in a touched shard
reuse its validated bytes, and an exact packed miss remains authoritative.
Resolver telemetry separately attributes serialized manifest reads, packed
selection calls/keys/shard bytes, and packed structural-validation calls/bytes.

The WebAssembly catalogue boundary owns a persistent `CatalogSession`. Authored
JavaScript parses only the small root response shape needed for transport,
fetches the requested digest-addressed shard as `Uint8Array`, and supplies those
bytes to Rust once. It stores immutable bytes in HTTP/IndexedDB caches and does
not use `TextDecoder` or materialize catalogue objects. `prepareBatch`,
`provideShard`, `planBatch`, and `selectFormat` all operate on the retained Rust
session.

Both frontends preserve required request order, first-key deduplication,
dependency-hint order, typed misses, corruption errors, bounded acquisition,
and offline object behavior. JavaScript owns asynchronous transport and native
Rust owns blocking transport; neither owns a second catalogue schema.

The explicit native verifier exhaustively authenticates all packed shards and
referenced objects. Ordinary sessions validate only root/shards/cache entries
they touch.

## Publication and provisioning

`tools/texlive-wasm-publish` owns scanning, winner selection, tree identity,
object emission, deterministic packing, successor verification, and the
`--file-ahash64` release-tool boundary. Publisher configuration schemas are 8
for full and 9 for HTML publication, with `treeAhash64` roots. The typed
`ManifestShard` and its JSON parser remain publisher/test construction APIs;
production shard payloads and runtime selection are packed only.

`scripts/publish-texlive-r2.sh` validates all staged bytes first, uploads
objects immutably, verifies remote inventory, and publishes `manifest-v8.json`
or `manifest-v9.json` last. Python provisioning validates the complete packed
layout when it materializes a local execution mirror, but native and browser
runtime authority remains the shared safe Rust view.

External source files continue to be checked against their licensed SHA-256
locks before their aHash64 distribution identity is accepted. Format images
remain independent schema-12 objects: repacking the catalogue changes their
root reference, not their engine contents.

## Exact production inventory

The 2026-03-01 full inventory contains 322,537 canonical requests: 164,643 TeX
keys and 157,894 TFM keys. The former 256 JSON shards occupied 93,525,476
bytes. The schema-2 8-bit packed baseline occupies 73,283,781 bytes, a
reduction of 20,241,695 bytes (21.64%). The canonical publication policy is now
12 bits: 4,096 self-contained shards occupy 79,302,720 bytes, and the root is
80,947 bytes. The finer partition intentionally spends 6,018,939 inventory
bytes and 72,962 root bytes to bound exact runtime admission.

The 8-bit packed total contains 579,584 buckets, 322,537 primary records,
212,109 dependency rows, 450,131 shard-local deduplicated object rows, and
452,941 shard-local deduplicated path rows. Key sections total 15,107,186 bytes
and string sections total 24,342,219 bytes. Counts for objects and paths are
summed across independent physical shards; cross-shard duplication is
intentional so one selected shard remains self-contained.

The earlier schema-1 issue-namespaced repack root is schema 8 with aHash64
`721e833071d92bba`. It is historical measurement evidence, not a schema-2 or
hosted default pin.

## Exact partition choice

The immutable arXiv `2606.12566` control uses 123 positive closure keys plus
the same observed authoritative probes and fallbacks. Its positive keys occupy
103 8-bit prefixes and 121 12-bit prefixes. Every row below preserves the
ordered selections, authoritative misses, 124 acquired resources, and the 10M
work vector
`(10000000,9999815,926177,2917745,8911073,1094)`:

| Shard bits | Root bytes | Catalogue requests | Touched shard bytes | Validation scratch | Peak shard | Full shard inventory |
| ---------: | ---------: | -----------------: | ------------------: | -----------------: | ---------: | -------------------: |
|          8 |      7,985 |                141 |          40,708,643 |          1,612,962 |    569,064 |           73,283,781 |
|          9 |     12,849 |                165 |          24,312,712 |            947,295 |    354,063 |           74,800,866 |
|         10 |     22,579 |                181 |          13,884,746 |            531,360 |    181,827 |           76,407,827 |
|         11 |     42,035 |                186 |           7,384,154 |            276,903 |     95,047 |           77,935,152 |
|         12 |     80,947 |                190 |           3,859,523 |            142,515 |     52,697 |           79,302,720 |

Catalogue requests count one root plus every touched shard; payload requests
are unchanged. Native local/cache reads and browser HTTP transport therefore
pay 49 additional small catalogue requests at 12 bits, while touched bytes
fall 90.52%, validation scratch requested bytes fall 91.16%, and peak shard
size falls 90.74%. The same 10M controls recorded 91,836 KiB versus 58,620 KiB
peak RSS. The 12-bit root is aHash64 `218d7e6d43a798a6`, with SHA-256
`6b665f2bc2254286b55c88f6b81cde55d7de293cdf62a770afc56246dae07846`;
it remains issue-namespaced evidence rather than a hosted default pin.

Finer sharding overlaps the separate one-pass path/key-validator idea. The
8-bit control admitted 179,218 primary records, 251,665 path rows, and 118,340
dependency rows; 12 bits admits 15,835, 24,989, and 10,268 respectively, a
90.70% reduction before such a pass could run. In the matched high-sample
capture, complete packed validation fell from 5.84% to 0.70% of weighted
cycles. The disjoint path-table and file-key families together retain only
29,659,619 cycles, 0.415% of the run, so a combined validator change is not
currently justified.

## Performance evidence

The exact offline arXiv `2606.12566` 20M control against the materialized packed
root uses the same 123-key closure (SHA-256
`e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`) and
preserves the authoritative work vector
`(20000000,19913119,2218327,6020965,16785710,4011)`. The fresh zero-loss
profile records 2,021 samples and about 26.9 billion weighted cycles. Packed
distribution resolution is 4.77% inclusive, complete first-touch shard
validation is 3.70%, and exact lookup is 0.41% inclusive/0.13% self. The former
full JSON authority recorded distribution resolution at 18.07% and
`ManifestShard::parse` at 9.03%; current ancestry contains no
`ManifestShard::parse`, JSON shard parser, selected-record movement, or
distribution-owned `BTreeMap` construction. This comparison demonstrates the
removed work but does not attribute unrelated intervening engine changes to
the packed representation. Warmed lookup borrows the retained bytes and the
allocator-instrumented 20,000-probe gate records zero calls and zero requested
bytes.

The 12-bit 10M high-sample capture contains 13,370 samples, zero lost samples,
and 7,142,546,267 weighted cycles. Distribution resolution accounts for
140,511,466 cycles (1.97%), complete shard validation 50,190,454 (0.70%), and
cache-envelope shard loading 5,045,313 (0.07%); no retained-shard lookup leaf
was sampled. The 50M authority reaches the existing semantic stop before fuel
under every partition:

| Shard bits | Wall seconds | User seconds | Peak RSS KiB |
| ---------: | -----------: | -----------: | -----------: |
|          8 |         5.85 |         4.67 |      115,724 |
|          9 |         5.73 |         4.56 |       99,132 |
|         10 |         5.64 |         4.48 |       86,652 |
|         11 |         5.45 |         4.36 |       79,740 |
|         12 |         5.45 |         4.47 |       74,556 |

The stop is
`page construction contains only live page-arena children: ActiveBuilder` at
`crates/tex-state/src/command_context.rs:3287`.
