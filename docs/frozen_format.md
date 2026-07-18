# Portable Frozen Format Images

Status: schema-10 container contract; frozen-store payloads are phased work.

This document is the durable ABI contract for Umber format images. The outer
container is implemented in `tex-state::format_container`. Schema 10 replaces
the schema-9 envelope and temporarily carries the existing validated semantic
DTO as section 1. Later phases replace that transitional section with the
fixed-width store sections specified here; they do not serialize live Rust
objects.

## Goals and trust boundary

A format image is immutable, deterministic input. It may be memory-mapped or
copied, but neither choice changes its interpretation. The decoder treats all
bytes as untrusted and publishes no live state until checksum, compatibility,
directory, section, and cross-reference validation completes.

The file never contains a pointer, `usize`, Rust enum/layout bytes, allocator
capacity, `Vec`/`Arc`/`Box` representation, native `HashMap` representation,
or process-local handle. Integers in frozen sections have an explicit `u8`,
`u16`, `u32`, `i32`, `u64`, or `i64` wire width and are little-endian. A file
schema change is required to change the meaning or width of any existing
field.

## Schema-10 container

The header is exactly 80 bytes:

| Offset | Width | Field                            |
| -----: | ----: | -------------------------------- |
|      0 |     8 | magic `UMBRFMT\0`                |
|      8 |     4 | schema version, currently `10`   |
|     12 |     4 | header size, `80`                |
|     16 |     4 | directory-record size, `40`      |
|     20 |     4 | section count, `1..=64`          |
|     24 |     8 | directory offset, `80`           |
|     32 |     8 | exact file length                |
|     40 |     8 | container ABI fingerprint        |
|     48 |     8 | lookup-configuration fingerprint |
|     56 |     8 | image checksum                   |
|     64 |     4 | flags, zero in schema 10         |
|     68 |    12 | reserved, all zero               |

Every integer is little-endian. The ABI fingerprint is FNV-1a-64 of the
literal contract string in `format_container.rs`; the lookup fingerprint is
the same operation over the literal lookup-configuration string. A decoder
requires exact values. The strings, not a compiler's struct layout, define the
values.

The directory immediately follows the header. Each record is exactly 40
bytes:

| Offset | Width | Field                                                    |
| -----: | ----: | -------------------------------------------------------- |
|      0 |     4 | nonzero section kind                                     |
|      4 |     4 | flags, zero in schema 10                                 |
|      8 |     8 | file-relative payload offset                             |
|     16 |     8 | stored byte length                                       |
|     24 |     8 | logical byte length; equal to stored length in schema 10 |
|     32 |     4 | alignment                                                |
|     36 |     4 | reserved, zero                                           |

Records are strictly increasing by section kind. Kinds are unique. Alignment
is a power of two from 8 through 4096. Each payload begins at the first
possible aligned offset after the preceding directory or section; alignment
bytes are zero, and no bytes follow the last section. These rules make one
canonical byte layout and rule out aliases, overlaps, hidden data, and
platform-dependent padding.

The checksum is FNV-1a-64 over the exact complete file with header bytes
56..64 treated as zero. It therefore covers header fields, compatibility
fingerprints, the directory, alignment padding, and every payload byte. It is
an accidental-corruption checksum, not an authenticity mechanism.

Section kind 1 is `TransitionalSemanticV9`. It contains the preexisting
detached semantic DTO and is the only section accepted by the schema-10
runtime today. It exists to land and exercise the container independently of
the frozen-store rollout. The following kinds are allocated for that rollout:

| Kind | Intended contents                        |
| ---: | ---------------------------------------- |
|   16 | frozen-format manifest and root indices  |
|  256 | canonical names and string bytes         |
|  257 | control-sequence/name lookup table       |
|  272 | token-list records and token words       |
|  288 | macro records                            |
|  304 | glue records and lookup table            |
|  320 | font records and immutable metric tables |
|  336 | code tables                              |
|  352 | hyphenation patterns and exceptions      |
|  512 | reachable frozen node and math graph     |
|  528 | frozen environment cells and roots       |

Adding one of these payload schemas changes the format schema version until
that section's exact record vocabulary is documented. Unknown kinds are not
silently ignored by a version that does not define them.

## References and structural validation

Within a section, a reference is either a fixed-width record index or an
unsigned byte offset relative to the beginning of that section. Cross-section
references are the pair `(section_kind: u32, record_index: u32)`. Optional
indices use `0xffff_ffff` as the null sentinel. No reference is an address or
an offset from a mapped base pointer.

Validation is complete, ordered, and bounded:

1. validate the fixed header, declared file length, and checksum;
2. require exact ABI and lookup-configuration fingerprints;
3. validate sorted directory records, alignment, zero padding, and ranges;
4. validate every section header, count, multiplication, and local range;
5. validate every index, offset, tag, canonical order, uniqueness rule, and
   cross-section reference;
6. validate graph topology and required roots; and
7. publish immutable stores, then create fresh job-local overlays.

Counts and offsets are widened before checked arithmetic and converted to
host `usize` only after proving they fit the actual byte slice. Validation
rejects cycles where a section requires dependency order, duplicate canonical
keys, unreachable required records, noncanonical padding, and unsupported
flags. A checksum-valid image can still be structurally invalid.

## Literal deterministic lookup tables

Frozen lookup indexes use literal bucket arrays, never serialized
`HashMap` state. A lookup-table header consists of:

| Field         | Type  | Schema-10 configuration              |
| ------------- | ----- | ------------------------------------ |
| algorithm     | `u32` | `1`, FNV-1a-64                       |
| table version | `u32` | `1`                                  |
| seed          | `u64` | `0xcbf29ce484222325`                 |
| bucket count  | `u32` | power of two, minimum 8              |
| entry count   | `u32` | number of canonical entries          |
| empty value   | `u32` | `0xffff_ffff`                        |
| maximum probe | `u32` | exact maximum emitted probe distance |

The header is followed by `bucket_count` little-endian `u32` entry indices,
then section-specific fixed-width entry records and canonical key bytes.
Entries are sorted by complete canonical key bytes. Bucket count is the
smallest allowed power of two satisfying `entry_count * 4 <= bucket_count *
3`. Emission inserts entries in canonical order. The initial bucket is
`fnv1a64(seed, key) & (bucket_count - 1)` and collisions use forward linear
probing with wraparound.

The lookup-configuration fingerprint covers the algorithm, algorithm version,
seed, capacity/load policy, empty sentinel, and probe strategy. Exact
configuration compatibility plus full structural validation are authoritative:
the decoder verifies bucket bounds, entry uniqueness, one bucket per entry,
canonical insertion/probe placement, key equality, and the declared maximum
probe. Deterministic checksum-derived spot checks may additionally exercise
the runtime lookup implementation after validation. Those checks are
supplementary diagnostics and can never make an incompatible fingerprint or
invalid structure acceptable.

## Immutable and job-local state

Frozen sections contain only state TeX deliberately preserves at `\dump`:
names and current meanings, immutable tokens/macros/glue/fonts, code tables,
hyphenation data, reachable box/node graphs, format-visible environment cells,
interaction mode, and permitted format-level PDF configuration.

The image excludes group journals, rollback epochs, allocation identities,
input frames/cursors, page-builder and mode-nest material, output transactions,
open streams, `World` effects/resources, clocks, random state, diagnostics and
provenance caches, incremental checkpoints, profiling counters, pending job
flags, and document-local PDF objects/pages/resources. Loading constructs
fresh job-local state, installs the current job clock, and schedules
`\everyjob`. Later mutable entries live in an overlay; the mapped/frozen bytes
are never mutated and group rollback applies only to job-local state.

## Migration from schema 9

Schema 9 was a deterministic semantic reconstruction format, but its outer
envelope had one opaque payload rather than an extensible fixed-width section
directory and carried no compatibility fingerprints. Schema 10 is a clean
boundary: the loader rejects schema 9 with `UnsupportedVersion(9)`. Users
regenerate format images from their source under the schema-10 engine; Umber
does not reinterpret an old image heuristically.

During the transition, schema 10 writes section 1 and restores fresh dense
stores exactly as schema 9 did. Epic phases replace semantic reconstruction
with the allocated frozen sections, literal lookup arrays, immutable graph
stores, and mutable overlays. Once those sections are integrated across all
drivers, section 1 is removed under another explicit schema bump.
