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
bytes. Repartitioning and packing the same publisher-resolved inventory with
schema 2 produces 73,283,781 bytes, a reduction of 20,241,695 bytes (21.64%);
canonical table ordering changes identity but not section sizes.

The packed total contains 579,584 buckets, 322,537 primary records, 212,109
dependency rows, 450,131 shard-local deduplicated object rows, and 452,941
shard-local deduplicated path rows. Key sections total 15,107,186 bytes and
string sections total 24,342,219 bytes. Counts for objects and paths are summed
across independent physical shards; cross-shard duplication is intentional so
one selected shard remains self-contained.

The earlier schema-1 issue-namespaced repack root is schema 8 with aHash64
`721e833071d92bba`. It is historical measurement evidence, not a schema-2 or
hosted default pin.

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
