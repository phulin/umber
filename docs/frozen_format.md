# Portable Frozen Format Images

Status: schema-10 container, authoritative non-node core-store sections, and
portable precomputed lookup indexes.

This document is the durable ABI contract for Umber format images. The outer
container is implemented in `tex-state::format_container`. Schema 10 replaces
the schema-9 envelope. Section 1 is now an isolated transitional overlay that
contains only reachable node-list DTOs and format-visible environment entries;
the non-node semantic stores are authoritative in the fixed sections specified
here. Later phases replace the remaining overlay; no section serializes live
Rust objects.

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

Section kind 1 retains the historical directory name
`TransitionalSemanticV9`, but its schema-10 payload is restricted to detached
reachable node-list records and environment entries. It contains no names,
token lists, macros, glue, fonts, code tables, hyphenation data, prepared
magnification, or last-font metadata. The schema-10 runtime requires exactly
kinds 1, 256, 257, 272, 288, 304, 320, 336, and 352. The following kinds are
allocated for the complete rollout:

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

Adding an undocumented payload or changing a documented record vocabulary
requires a schema change. Unknown kinds are not silently ignored by a version
that does not define them.

## Foundational store sections

Kinds 256, 272, 288, and 304 have section version 1. All offsets in these
sections are section-relative, all counts and offsets are `u32`, all semantic
identities are `u64`, and every reserved field is zero.

### Names (kind 256)

The 24-byte header contains `(version, count, records_offset, strings_offset,
strings_length, reserved)` as six `u32` values. `records_offset` is 24. Each
24-byte record is:

| Offset | Type  | Field                                    |
| -----: | ----- | ---------------------------------------- |
|      0 | `u8`  | namespace: 0 named, 1 active character   |
|      1 | 3 B   | reserved                                 |
|      4 | `u32` | offset in the section string byte region |
|      8 | `u32` | UTF-8 byte length                        |
|     12 | `u32` | reserved                                 |
|     16 | `u64` | canonical control-sequence semantic atom |

String spans are contiguous in record order with no unused bytes. Names are
valid UTF-8, active names contain exactly one Unicode scalar, namespace/name
pairs are unique, and semantic atoms are recomputed during validation. The
dense record index is the local interner slot used by other frozen sections.

### Token lists (kind 272)

The 24-byte header contains `(version, count, records_offset, words_offset,
word_count, reserved)` as `u32` values. `records_offset` is 24. Each 24-byte
list record contains `start: u32`, `length: u32`, `semantic_id: u64`, and a
reserved `u64`. Spans are contiguous and list 0 is the canonical empty list.
Duplicate lists are rejected.

Each token word is `u64`. Bits 63..56 are the tag and bits 55..0 are payload:

| Tag | Payload                                              |
| --: | ---------------------------------------------------- |
|   0 | Unicode scalar in bits 31..0, catcode in bits 39..32 |
|   1 | names-section record index in bits 31..0             |
|   2 | internal/parameter byte in bits 7..0                 |
|   3 | frozen sentinel 0 or 1                               |

Unused payload bits are zero. Character, catcode, name-index, and sentinel
domains are validated. The semantic identity is recomputed from the decoded
tokens and name semantic atoms before the arena is published.

### Macros (kind 288)

The 16-byte header is `(version, count, records_offset, reserved)` as `u32`;
`records_offset` is 16. Each 16-byte record contains `flags: u8`, three
reserved bytes, parameter-list index `u32`, replacement-list index `u32`, and
a reserved `u32`. Only the four defined meaning-flag bits are accepted. Both
indices must name token-list records. Parameter delimiter metadata is derived
directly while the validated macro column is installed; definitions are not
reinterned.

### Glue (kind 304)

The 16-byte header is `(version, count, records_offset, reserved)` as `u32`;
`records_offset` is 16. Each 24-byte record contains signed `i32` width,
stretch, and shrink values at offsets 0, 4, and 8; `u8` stretch and shrink
orders at offsets 12 and 13; and ten reserved zero bytes. Orders are 0..=3,
record 0 is canonical zero glue, and duplicate specs are rejected.

These four sections are decoded into validated dense immutable prefixes with
their canonical record indices. Kind 257 holds the name index; the token-list
and glue indexes follow the canonical word and record regions inside kinds 272
and 304. Fresh generation-tagged runtime identities are attached in bulk.
Ordinary job-created values append after the prefix and use mutable overlay
indexes with the existing interning, snapshot, and rollback paths. The
process-wide compact symbol registry is resolved in one batch for names;
neither token lists, macro definitions, nor glue specs are replayed through
their semantic interning APIs.

### Fonts and font metadata (kind 320)

The 32-byte header contains version, font count, payload offset and length,
an optional-prepared-`mag` tag and signed value, the last-loaded font index,
and a reserved `u32`. The payload is the canonical fixed-integer schema-10
encoding of detached font records: names and content hashes, immutable and
source parameters, character metrics, lig/kern instructions, extensible
recipes, derivation identity, control-sequence identifier index, and pdfTeX
expansion settings.

The decoder validates metric structure, derivation order, identifiers,
parameter-bank references from the environment overlay, and the last-font
index before any store is published. It then constructs the dense font prefix
in bulk, attaches fresh runtime identities, and rebuilds loaded-font lookup
keys and immutable/complete semantic hash fragments without calling the
ordinary font interning or mutable identifier/expansion paths.

### Code tables (kind 336)

The 16-byte header is `(version, count, records_offset, reserved)` as `u32`;
`records_offset` is 16. Each 32-byte record contains code point `u32`, catcode
`u8`, three reserved bytes, lc- and uccode `u32`, sfcode `u16`, two reserved
bytes, mathcode `u32`, signed delcode `i32`, and four reserved bytes. Records
are strictly code-point ordered, contain valid Unicode scalars and catcodes,
and must differ from INITEX defaults in at least one column. Validated rows are
materialized directly as sparse radix roots with zero job-local generations
and no assignment or group history.

### Hyphenation (kind 352)

The 16-byte header contains version, payload offset and length, and a reserved
`u32`. Its canonical fixed-integer schema-10 payload stores language-indexed
runtime tries, exception maps, and saved hyphen-code maps. Validation requires
one root per language, strictly sorted unique edges, live edge targets, exactly
one incoming edge for every non-root node, and nonempty exception words whose
positions do not exceed the character count. Endpoint and repeated exception
positions remain representable because TeX's exception scanner accepts leading,
trailing, and adjacent hyphens. The validated trie is installed as the immutable
format base; later job mutations retain the existing copy-on-write `Arc`
snapshot behavior.

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

Each 16-byte entry record contains key offset `u32`, key length `u32`, target
record index `u32`, and a zero reserved `u32`. Key spans are contiguous. Name
keys are the namespace byte followed by UTF-8 name bytes, token-list keys are
the complete sequence of canonical little-endian token words, and glue keys
are the complete 24-byte glue record. Empty token lists therefore have an empty
key. The target is the dense record index in the corresponding foundational
store.

The lookup-configuration fingerprint covers the algorithm, algorithm version,
seed, capacity/load policy, empty sentinel, and probe strategy. Exact
configuration compatibility plus full structural validation are authoritative:
the decoder verifies bucket bounds, entry uniqueness, one bucket per entry,
canonical insertion/probe placement, key equality, and the declared maximum
probe. Deterministic checksum-derived spot checks additionally exercise the
runtime lookup implementation after validation. Schema 10 selects up to eight
entries per table from the container checksum using a fixed xorshift64*
sequence. Those checks are
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

During the transition, schema 10 writes section 1 only for node graphs and the
environment overlay. Names, token lists, macros, glue, fonts, code tables, and
hyphenation exist only in authoritative sections 256 through 352 and are never
reinterned during normal loading. The decoder validates overlay references
against those frozen stores before publication. Later phases replace the
remaining node/environment reconstruction with allocated frozen sections,
immutable graph stores, and mutable overlays. Once those sections are
integrated across all drivers, section 1 is removed under another explicit
schema bump.
