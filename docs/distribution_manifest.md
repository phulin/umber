# Sharded Distribution Manifest

This document is the current distribution catalogue, publication, cache, and
trust-boundary contract. Canonical request semantics are defined by
[Canonical resource identity and lifecycle](resource_lifecycle.md).

## Identity contract

Repository-owned distribution content uses `ahash64-v1`: 16 lowercase
hexadecimal digits produced by `umber-hash` with fixed seeds, algorithm version
1, and explicit domains. The implementation is portable Rust rather than
`RandomState` or upstream `AHasher`, whose output is not a persisted-format
contract. Authored JavaScript implements the same operations and stable test
vectors.

An aHash64 identity is a deterministic content selector and accidental
corruption check. It is not collision resistant, does not authenticate a
hostile publisher, and must not be described as a cryptographic trust root.
HTTPS and the release channel establish publisher provenance.

The SHA-era hosted distribution is deliberately incompatible. Native and
browser defaults report `default-distribution-unpublished` until
`umber2-66p0.27` republishes the payloads, shards, roots, formats, HTML catalog,
and root pins. There is no SHA fallback. An explicit migrated root remains
usable with `--distribution` and `--distribution-ahash64`.

SHA-256 remains only at external compatibility boundaries: licensed source and
archive locks, reference-oracle and corpus evidence, and output parity receipts.
Those values are not distribution object identities or cache keys.

## Persisted schemas

- Monolithic manifest schema 2 uses aHash64 object entries.
- Full sharded root schema 6 pairs only with index shard schema 3.
- HTML sharded root schema 7 pairs only with index shard schema 4.
- Font and legacy-mapping records are schema 2.
- Producer format metadata is schema 3 without an input closure and schema 4
  with a schema-1 input closure.
- Umber format images are schema 12 and page artifacts are schema 24.
- Native cache envelopes are schema 2 under `blobs-v2`.

Old schemas and digest widths are rejected rather than upgraded implicitly.
Objects are named `ahash64-v1-<16 lowercase hex digits>`. Root and shard JSON
are canonical, maps are sorted by raw UTF-8 key order, and documents end in one
newline.

## Root and shard selection

A sharded root contains `distribution`, `objectsBaseUrl`, `shardBits`,
`shardCount`, the ordered bare-digest `shards` array, and optional inline
formats. `shardCount` equals both `2^shardBits` and the array length. Full roots
may carry format input closures; HTML roots additionally select font and legacy
mapping records.

The canonical request key is hashed in `DistributionShardKey` domain 2. The
leading `shardBits` of the numeric 64-bit result select exactly one shard. A
verified miss in that shard is authoritative. Root, shard, partition, schema,
length, or digest failures remain errors and never become resource absence.

Within each shard, file records are sorted by request key and bind canonical
virtual path, object, digest, byte length, and dependencies. Publisher winner
precedence is unchanged: roots are scanned in configuration order, the first
canonical request winner is retained, and dependencies preserve their declared
order. Replacing the JSON tree walk with a direct table may preserve semantics
only if it stores that already-resolved winner map and the same canonical
request-key encoding.

## HTML records

Index shard schema 4 adds independent `fonts` and `legacyMappings` maps. Font
record schema 2 binds a WOFF2 object, optional program identity, feature-policy
version, bounded provenance, and a content-addressed redistributable license.
Legacy mapping schema 2 additionally binds the exact TFM aHash64, a complete
font request key, a 256-entry Unicode map, mapping/fontdimen policy versions,
and explicit fallback policy. TFM, font-object, program, license, and rendered
resource identities use separate domains where their semantics differ.

## Native and browser acquisition

Native `DistributionResolver` and JavaScript `HttpManifestResolver` verify the
same root, shard, object, format, and dependency graph. Both deduplicate object
work by aHash64 and preserve request order at admission. The native cache hashes
its envelope in domain 3 and its path key separately; browser IndexedDB uses a
versioned aHash64 database name. Neither frontend uses runtime-random hashing or
Web Crypto for repository-owned resource identity.

The explicit native verifier walks every `blobs-v2` entry, validates the cache
envelope and caller-owned payload identity, and reports object/manifest counts.
Ordinary loads validate only entries they touch.

## Publication and provisioning

`tools/texlive-wasm-publish` owns scanning, winner selection, tree identity,
object emission, sharding, canonical JSON, successor verification, and a
`--file-ahash64` release-tool boundary. Configuration schemas are 6 for full
and 7 for HTML publication, with `treeAhash64` roots.

`scripts/publish-texlive-r2.sh` validates all staged data first, uploads objects
immutably, verifies remote inventory, and publishes the manifest last. It
requires an explicit 16-digit root aHash64 while the default pin is
unpublished. Python provisioning contains a byte-identical implementation of
algorithm version 1 and fails explicitly when no migrated root pin is supplied.
External source files continue to be checked against their licensed SHA-256
locks before their aHash64 distribution identity is accepted.

## Inventory and table sizing evidence

The retained production manifest contains 322,537 canonical request entries:
164,643 TeX keys and 157,894 TFM keys. Their raw canonical keys occupy
12,614,820 bytes. They reference 152,302 unique objects, with 170,235 duplicate
mappings, and the sharded JSON occupies 93,525,476 bytes.

For a future flat lookup table, 80% load requires 403,172 buckets: 9,676,128
bytes with 24-byte rows or 12,901,504 bytes with 32-byte rows. At 85% load,
379,456 buckets require 9,106,944 or 12,142,592 bytes respectively. Adding the
12,614,820-byte key blob and a 16-byte-per-object table (2,436,832 bytes) gives
totals of about 23.58/26.66 MiB at 80% or 23.04/25.93 MiB at 85%. This is sizing
evidence only; the current runtime still walks root to selected shard to its
typed sorted map.

## Performance evidence

`distribution-startup-benchmark` measures the exact cold/shared resolver work
without mutating the shared production cache. The aHash64 migration removes SHA
compression from root, shard, and payload verification. Until the external
republication makes the 20M workload runnable, the issue-namespaced focused
measurement records the cold and shared startup cost and validation counts; the
blocking asset inventory is tracked in `umber2-66p0.27`.
