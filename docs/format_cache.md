# Generated format cache

Status: schema-10 cache identity, validated native entry storage, and pinned
LaTeX/pdfLaTeX generation integration implemented.

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

## Pinned generation integration

`umber format-cache restore|store` is the narrow native adapter used by
`scripts/build-latex-format.sh`. For this local pinned-source workflow, the
builder hashes a domain-separated distribution release name authenticated by
the source lock, the sorted canonical request-key closure, the complete
`tests/latex-source.lock` bytes, and a canonical build receipt containing the
Umber version, release profile, feature set, and `rustc -Vv` identity. It takes
the five clock fields from the same `SOURCE_DATE_EPOCH`-initialized `World` as
the engine. Both actions accept an explicit cache root for hermetic workflows;
otherwise they use the platform Umber cache directory. Published distribution
workflows continue to identify snapshots from authenticated root-manifest
bytes rather than from a mutable path.

On a hit, `restore` revalidates the entry envelope, SHA-256, and full Universe
decode, then atomically materializes the requested `.fmt` output. The builder
does not require or open the TeX Live source tree on this path. A mismatch or
corrupt entry is a diagnosed miss. On a miss, the builder verifies every file
in the 57-key LaTeX or 60-key pdfLaTeX closure before source initialization,
performs two clean byte-identical generations and the representative
source-versus-loaded equivalence gate, then publishes the validated image with
the cache store's no-clobber atomic protocol. The ordinary output format and
metadata paths remain `target/<engine>-format/<engine>.fmt` and
`<engine>-format.json`, or the caller's explicit output directory.

`--force` ignores reuse for execution purposes, regenerates, and requires the
result to equal any valid entry already stored under the same semantic key.
`--check` requires a valid entry, regenerates twice, compares both the cache and
published output/metadata, and changes neither. Thus neither mode can silently
replace different bytes under an existing identity. Cache diagnostics include
the canonical key and distinguish hit, miss, and publication.

The full native TeX Live snapshot and WASM bundle builders invoke this same
pinned builder, so they share cache selection and deterministic publication.
Runtime distribution acquisition remains a separate object-cache workflow: a
published schema-3 format closure is still offered as prefetch hints and is
installed with the required input in the established two-attempt compile path.
The cache does not mask the live-bootstrap semantic blocker tracked by
`umber2-pbxv.5.4.1`: misses, `--force`, and `--check` all exercise source
initialization and report that failure normally.

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
