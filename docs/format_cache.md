# Generated format cache

Status: schema-10 cache identity and validated native entry storage implemented;
generation and CLI selection are intentionally deferred.

## Identity contract

A generated format is reusable only when every input that can affect its bytes
is identical. `umber-fetch::FormatCacheIdentity` therefore keys an entry by:

- the composed engine mode (TeX82, e-TeX, pdfTeX, LaTeX, or pdfLaTeX);
- the format schema, container ABI fingerprint, and frozen-lookup configuration
  fingerprint exported by `tex-state::Universe`;
- SHA-256 identities for the pinned distribution root, exact format-input
  closure, generation source lock, and relevant build configuration; and
- all five fields of the pinned TeX job clock.

Callers hash canonical representations. The distribution identity is the
authenticated root-manifest bytes, the closure identity is its validated
ordered request-key encoding, the source-lock identity covers the complete
lock bytes, and build configuration covers feature, profile, and compiler
inputs declared relevant by the generator. Later generation integration owns
those source encodings; native storage does not infer them from paths,
environment variables, or modification times.

The key preimage is fixed-width after its domain prefix. It is, in order:
`umber.format-cache.key\0`, little-endian identity schema `1`, one engine byte
plus three zero bytes, little-endian format schema (`u32`), ABI (`u64`) and
lookup (`u64`) fingerprints, four 32-byte identities in the order above, then
the clock's time, second, day, month, and year as little-endian `i32` values.
The cache key is SHA-256 of that preimage. Any schema or ABI transition creates
a different namespace without probing or heuristically upgrading old images.
The public constructor always supplies the current build's schema and
fingerprints; callers cannot mint an identity that labels current format bytes
with stale compatibility metadata.

## Native entry and validation

`umber-fetch::FormatCacheStore` uses `formats-v1/sha256-<key>` below an
explicit root or the platform Umber cache directory. Each path is one atomic
binary entry containing an entry magic/schema, canonical key preimage, declared
payload length, payload SHA-256, and the schema-10 format bytes. A same-directory
temporary file is fully written and synchronized before no-clobber rename, so
readers see either the old complete entry or the new complete entry. Competing
publishers validate the winner before accepting it; if a corrupt entry won the
race, it is removed and publication is retried.

Every read independently checks file bounds, entry geometry and version, exact
key metadata, payload length and SHA-256, and finally calls
`Universe::from_format(World::memory(), bytes)`. Only the opaque
`ValidatedFormatImage` wrapper is returned. Store input passes the same
Universe validation before publication. A mismatched, truncated, corrupt, or
decoder-invalid entry is removed and reported as a cache miss; unrelated
temporary files are ignored. Cache deletion or corruption is therefore a
performance event, not a source of trusted engine state.

## Native/browser and portability boundaries

The schema-10 image and key preimage are host-neutral. Filesystem discovery,
temporary files, atomic rename, permissions, and recovery belong only to the
native `umber-fetch` boundary. Browser-packaged formats and HTTP/IndexedDB
caches must not refer to native paths or treat a native entry envelope as a
distribution artifact. They may reproduce the documented key encoding, but
must validate transport length and SHA-256 and pass the extracted image through
the same Rust `Universe` decoder before use.

Neither a release-manifest pin nor a cache key replaces format validation: the
manifest authenticates acquisition, the cache key identifies generation
inputs, and the schema-10 decoder establishes runtime compatibility and
structural validity. Formats are portable across native and browser hosts only
when those three independent checks agree. The cache contains no process-local
handles or job-local mutable state, consistent with
[the frozen-format contract](frozen_format.md).
