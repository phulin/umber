# `umber2-66p0.31`: packed-distribution hot-start validation

## Boundary and implementation

The packed shard validator now validates each immutable table in its owning
pass. It validates the complete key blob as UTF-8 once, requires primary
records to be strictly ordered by their raw key bytes, and checks subsequent
key spans through the already validated string. Object and path identity,
dependency rows, record policy, and bucket coverage remain authenticated.
Bucket validation unwraps the circular hash table after a guaranteed empty
bucket and proves every occupied bucket reachable by its linear-probe chain in
one scan instead of replaying a lookup for every record. Runtime accessors then
borrow the validated path, object, and dependency spans without repeating path
or reserved-field policy.

Native resolver ownership is unchanged: one `Arc<ValidatedPackedShard>` is
retained for every touched shard and reused throughout the process-local
resolver session. The change adds no selected-hit or selected-miss cache and
does not change aHash64. New telemetry attributes serialized manifest read
bytes, packed selection calls/keys/shard bytes, and packed structural
validation calls/bytes.

The remaining first-touch validation is irreducible at this trust boundary.
Each of the 163 independently authenticated touched shards must still prove its
layout, UTF-8 and canonical key spans, root partition and key hashes,
object/path uniqueness and policy, record and dependency metadata, and bucket
coverage and probe topology once before lookup can borrow its contents.

## Authority and correctness evidence

The before and after binaries have SHA-256
`ac2afb96d1744a26bffc8007f7f7b6b0b21f872cfd6fdaf6a03fba7754e9c241`
and
`56b0575654b3ed3fc8b2c78fa97e09af8400d49ebb553e99ef5eaf7d24b1aebc`.
Both used packed root aHash64 `721e833071d92bba`, root-manifest SHA-256
`4d3887c289078e2bc0f88e96f4e77989dc38522a59c230fa5be94dffa1c7cff9`,
schema-12 format object `ahash64-v1-2b924b5bba05d8a0` with SHA-256
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`,
and 123-key closure SHA-256
`e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`.
The source `ArXiv.tex` SHA-256 is
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`.

Cold acquisition and hot-cache startup were measured separately under
`flock /tmp/umber-perf-host.lock`. Both cold rows acquired exactly 124
resources and populated an identical 572-file, 48,775,587-byte cache. Every
hot row acquired zero resources. Before and after telemetry was identical:
164 reads and validations covering 46,856,310 serialized bytes, 163 shard
loads and packed validations covering 46,848,325 bytes, and 225 packed
selection calls for 245 keys and 65,121,544 selected shard-bytes. The retained
owner held exactly those 163 touched shards and 46,848,325 bytes.

All cold and hot rows intentionally exhausted the 20M action limit and
reproduced
`(20000000,19913119,2218327,6020965,16785710,4011)` exactly. The before and
after caches have the same file count and byte inventory, so the reduced
validation loses no resource closure, offline behavior, or authenticated
selection.

## Cycle evidence

The frame-pointer profiles sampled user cycles at 199 Hz with zero lost
events. Full-ancestry attribution records absolute inclusive cycles rather
than inferring from top-down percentages:

| Full ancestry                                       | Before cycles | Before share | After cycles | After share | Reduction |
| --------------------------------------------------- | ------------: | -----------: | -----------: | ----------: | --------: |
| `DistributionResolver::resolve_batch_with_prefetch` | 1,072,165,577 |       5.623% |  824,326,881 |      4.398% |    23.12% |
| `ValidatedPackedShard::new`                         |   913,551,272 |       4.791% |  589,260,926 |      3.144% |    35.50% |
| `manifest::validate_path`                           |   351,574,971 |       1.844% |  262,902,631 |      1.403% |    25.22% |
| `ValidatedPackedShard::lookup`                      |   100,094,063 |       0.525% |            0 |      0.000% |   100.00% |
| `AHash64Hasher::write`                              |   129,442,808 |       0.679% |  161,526,714 |      0.862% |   -24.79% |

The complete profiles cover 19,068,433,326 before and 18,743,030,285 after
cycles. Raw callchains, reports, process censuses, CPU-pressure receipts, and
cache inventories are retained under `target/umber2-66p0.31/quiet3-before/`
and `target/umber2-66p0.31/quiet3-final/`. The `perf.data` SHA-256 values are
`a02f4ca9bf4517b522344d635a600f0b37993fb8b7f8ca31d94f401e03360ac3` and
`8aaafa8860a6534d77ca3705674e6d3067185f604ed240a3ddecbbd37e2748d6`.
Performance evidence remains checkout-local and is not a portable fixture.

## Validation

Representative packed-distribution corruption and wrapped-cluster tests cover
probe gaps, duplicate and out-of-order keys, invalid paths, oversized objects,
and multiple table sizes. Native distribution ownership, offline closure,
browser catalogue tests, the complete routine suite, and the repository format
and clippy gates pass. The browser-only wasm-bindgen runner could not start
because this host has no Firefox; the other five WebAssembly gates pass.
