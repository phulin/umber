# Portable Frozen Format Images

Status: schema-11 container, authoritative core-store sections, portable
precomputed lookup indexes, runtime-ready frozen node arena, and groupable
environment base/overlay.

This document is the durable ABI contract for Umber format images. The outer
container is implemented in `tex-state::format_container`. Schema 11 replaces
schema 10 to make token-parameter cell presence part of the frozen environment
vocabulary. Section 1 contains only Universe-level interaction
mode and permitted PDF configuration. Format-visible environment entries are
authoritative in kind 528, reachable node graphs are authoritative in kind
512, and non-node semantic stores are authoritative in their fixed sections.
No section serializes live Rust objects.

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

## Schema-11 container

The header is exactly 80 bytes:

| Offset | Width | Field                            |
| -----: | ----: | -------------------------------- |
|      0 |     8 | magic `UMBRFMT\0`                |
|      8 |     4 | schema version, currently `11`   |
|     12 |     4 | header size, `80`                |
|     16 |     4 | directory-record size, `40`      |
|     20 |     4 | section count, `1..=64`          |
|     24 |     8 | directory offset, `80`           |
|     32 |     8 | exact file length                |
|     40 |     8 | container ABI fingerprint        |
|     48 |     8 | lookup-configuration fingerprint |
|     56 |     8 | image checksum                   |
|     64 |     4 | flags, zero in schema 11         |
|     68 |    12 | reserved, all zero               |

Every integer is little-endian. The ABI fingerprint is FNV-1a-64 of the
literal contract string in `format_container.rs`; the lookup fingerprint is
the same operation over the literal lookup-configuration string. A decoder
requires the current values, except that it recognizes the previous
uncompressed/literal-token-lookup pair for schema-11 migration. The strings,
not a compiler's struct layout, define the values.

The directory immediately follows the header. Each record is exactly 40
bytes:

| Offset | Width | Field                            |
| -----: | ----: | -------------------------------- |
|      0 |     4 | nonzero section kind             |
|      4 |     4 | flags: bit 0 means raw DEFLATE   |
|      8 |     8 | file-relative payload offset     |
|     16 |     8 | stored byte length               |
|     24 |     8 | uncompressed logical byte length |
|     32 |     4 | alignment                        |
|     36 |     4 | reserved, zero                   |

Records are strictly increasing by section kind. Kinds are unique. Alignment
is a power of two from 8 through 4096. Each payload begins at the first
possible aligned offset after the preceding directory or section; alignment
bytes are zero, and no bytes follow the last section. These rules make one
canonical byte layout and rule out aliases, overlaps, hidden data, and
platform-dependent padding. New images compress every section independently
with deterministic raw DEFLATE level 6. The decoder bounds each logical
section at 512 MiB, limits decompression to the declared logical length plus
one byte, and requires the result to have exactly that length. It retains the
container-v1 fingerprint and zero-flag decode path only to read already
published uncompressed schema-11 images.

The checksum is FNV-1a-64 over the exact complete file with header bytes
56..64 treated as zero. It therefore covers header fields, compatibility
fingerprints, the directory, alignment padding, and every payload byte. It is
an accidental-corruption checksum, not an authenticity mechanism.

Section kind 1 retains the historical directory name
`TransitionalSemanticV9`, but its schema-11 payload is restricted to
Universe-level interaction mode, TeX82 allocation-reporting metadata, and a
versioned pdfTeX INITEX resource DTO. The reporting record carries
`str_ptr`/`pool_ptr`, the format-relative `init_str_ptr`/`init_pool_ptr`, the
low/high main-memory allocator extents, and the profile's
`max_strings`/`pool_size`; typed
control-sequence, filename, hyphenation, and token-list owners retain their own
bytes and publish only their TeX-style accounting transitions to this record.
TeX82 §1334's main-memory statistic reports the allocator's low/high coordinate
extent under the pinned Web2C capacity profile (tex.web §§125--130), not typed
store occupancy. The low arena grows in 1000-word blocks. The one-word arena is
derived from the reachable semantic closure; §200 token-list reference heads
and §384 macro-definition `end_match` words are mapped back from Umber's split,
hash-consed representation. The format persists both INITEX extents, so loading
does not reinterpret immutable backing history as live TeX memory. The dense
canonical empty-list identity does not consume a TeX word.
Immutable glue-value interning is deliberately outside that projection: it is
not a third WEB allocator and remains ordinary store state across format
dump/load.
The PDF DTO retains allocation counters, raw objects, forms, external images,
and ToUnicode mappings; token lists and node graphs are embedded as validated
handle-free semantic envelopes. It contains no store or environment data.
The schema-11 runtime requires
exactly kinds 1, 256, 257, 272, 288, 304, 320, 336, 352, 512, and 528. The
following kinds are allocated for the complete rollout:

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

Kinds 256, 288, and 304 have section version 1; newly emitted kind 272 has
section version 2. All offsets in these
sections are section-relative, all counts and offsets are `u32`, all semantic
identities are `u64`, and every reserved field is zero.

### Names (kind 256)

The 24-byte header contains `(version, count, records_offset, strings_offset,
strings_length, reserved)` as six `u32` values. `records_offset` is 24. Each
24-byte record is:

| Offset | Type  | Field                                    |
| -----: | ----- | ---------------------------------------- |
|      0 | `u8`  | namespace: 0 named, 1 active character   |
|      1 | `u8`  | occupied TeX82 §259 hash entry: 0 or 1   |
|      2 | 2 B   | reserved                                 |
|      4 | `u32` | offset in the section string byte region |
|      8 | `u32` | UTF-8 byte length                        |
|     12 | `u32` | reserved                                 |
|     16 | `u64` | canonical control-sequence semantic atom |

String spans are contiguous in record order with no unused bytes. Names are
valid UTF-8, active names contain exactly one Unicode scalar, namespace/name
pairs are unique, and semantic atoms are recomputed during validation. The
dense record index is the local interner slot used by other frozen sections.
The hash-occupancy bit preserves §259's monotonic allocator coordinate across
format load, including one-letter control words and names later used by fixed
internal aliases. Its addition is covered by the container ABI fingerprint,
so caches with the formerly reserved zero byte are not reused.

### Token lists (kind 272)

The 24-byte header contains `(version, count, records_offset, words_offset,
word_count, reserved)` as `u32` values. `records_offset` is 24. Each 16-byte
list record contains `start: u32`, `length: u32`, and `semantic_id: u64`.
Spans are contiguous and list 0 is the canonical empty list. Duplicate lists
are rejected.

Each token word is `u32`. Bits 31..30 are the tag and bits 29..0 are payload:

| Tag | Payload                                              |
| --: | ---------------------------------------------------- |
|   0 | Unicode scalar in bits 20..0, catcode in bits 24..21 |
|   1 | names-section record index in bits 29..0             |
|   2 | internal/parameter byte in bits 7..0                 |
|   3 | frozen sentinel 0 or 1                               |

Unused payload bits are zero. Character, catcode, name-index, and sentinel
domains are validated. The semantic identity is recomputed from the decoded
tokens and name semantic atoms before the arena is published. The decoder
also accepts the previous version-1, 24-byte-record, `u64`-word token section
when loading an older schema-11 image; new dumps never emit it.

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

Before these sections are encoded, capture computes the mandatory semantic
closure rooted in current environment meanings, token registers and token
parameters, live macro parameter/replacement lists, and token-bearing frozen
nodes. It retains the names referenced by the resulting token lists, meaning
cells, current-font cells, and font identifiers. Names, token-list IDs, and
macro-definition IDs are compacted in original relative order and every
cross-reference is remapped. Append-only INITEX history is not part of a
format's semantics and is omitted; a future warm-cache section would be an
explicit, separately budgeted optimization.

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
and a reserved `u32`. The payload is the canonical fixed-integer schema-11
encoding of detached font records: names and content hashes, immutable and
source parameters, TeX82's logical `font_info` word extent, character metrics,
lig/kern instructions, extensible recipes, derivation identity,
control-sequence identifier index, and pdfTeX expansion settings.

The decoder validates metric structure, derivation order, identifiers,
parameter-bank references from the environment overlay, and the last-font
index before any store is published. It then constructs the dense font prefix
in bulk, attaches fresh runtime identities, and rebuilds loaded-font lookup
keys and immutable/complete semantic hash fragments without calling the
ordinary font interning or mutable identifier/expansion paths.

The dense font bank is bounded to 32768 rows, including `nullfont`: each font
owns a 17-bit `fontdimen` subdomain inside the environment cell's 32-bit index.
Row zero retains TeX.web §§552--556's seven zero parameters, empty character
set, zero checksum and sizes, hyphen character 45, and skew character -1.
Dump/load preserves allocation order and every immutable loaded-TFM field plus
the mutable font banks; a decoded image outside the same bound is rejected.

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
`u32`. Its canonical fixed-integer schema-11 payload stores language-indexed
runtime tries, exception maps, and saved hyphen-code maps. Validation requires
one root per language, strictly sorted unique edges, live edge targets, exactly
one incoming edge for every non-root node, and nonempty exception words whose
positions do not exceed the character count. Endpoint and repeated exception
positions remain representable because TeX's exception scanner accepts leading,
trailing, and adjacent hyphens. The validated trie is installed as the immutable
format base; later job mutations retain the existing copy-on-write `Arc`
snapshot behavior.

### Frozen node arena (kind 512)

The 32-byte header contains version, list count, records offset, payload
offset, payload length, and reserved zero words. Each 40-byte list record
contains its canonical detached key, payload offset and length, node count,
precomputed `u64` semantic identity, and reserved zero bytes. List keys are
allocation-independent dense dependency indices; payload spans are contiguous
and cover the payload exactly.

Node payloads use an explicitly selected little-endian fixed-integer DTO
vocabulary for every box, node, math field, whatsit, string, and byte vector.
They contain store record indices and canonical list keys, never runtime
handles or compact native node words. Lists occur bottom-up. Decoding rejects
forward or self references, cycles, invalid store indices, malformed enum
values, bad section geometry, reserved bytes, count mismatches, and semantic
identities that do not recompute from the validated graph.

Capture discovers every list edge that the DTO serializes, including detached
physical box children used to represent TeX82 §§115/162 replacement nodes for
§182 diagnostics. Those diagnostic-only edges do not enter semantic identity,
but they remain part of the self-contained frozen graph and of §638 shipout
memory observation; capture and decode therefore require their targets exactly
like semantic child targets.

After validation, all lists are installed into one immutable survivor-backed
arena root with their precomputed semantic spans. No legacy key map, graph
promotion, or semantic reseal runs on the load path. The job epoch arena starts
empty; job-local lists may safely refer to frozen lists and allocate normally.

### Frozen environment (kind 528)

The 16-byte header is `(version, count, records_offset, reserved)` as four
`u32` values; `records_offset` is 16. Each 24-byte record contains packed cell
id `u64`, value tag `u8`, seven reserved zero bytes, and payload `u64`.
Records are strictly ordered by the complete packed cell id, global-bit cells
and duplicate cells are rejected, and each bank's index and value domain is
validated against the decoded frozen stores.

Value tag 0 stores the raw fixed-width semantic cell word. Tag 1 is permitted
only for a nonvoid box cell and packs the frozen node-list record index in the
low `u32` and its validated node count in the high `u32`. No runtime handle is
serialized. After cross-store and node-reference validation, the loader maps
box records to the installed survivor arena and bulk-installs all cells as an
immutable format base without calling assignment APIs.

For token-parameter banks, record presence is semantic: an omitted cell is
null, while a present record whose payload is token-list record 0 is an
explicitly assigned empty token list. This distinction is required by e-TeX's
`\everyeof` test and is the environment-vocabulary change that introduced
schema 11.

The ordinary environment banks are the mutable job overlay seeded from that
base. Their existing write barrier owns all later local/global assignment,
save-stack journaling, grouping, snapshot, and rollback behavior. The retained
base cells are immutable and shared across environment clones; job mutation
changes only overlay storage.

The schema-11 frozen encoder and decoder are the only store format path.
Store-level round-trip tests call `encode_frozen_format` and
`decode_frozen_format` directly. Universe-level tests exercise `dump_format`
and `Universe::from_format`, including malformed-section rejection, immutable
base and mutable-overlay behavior, rollback, and byte-identical canonical
redumps. There is no separate DTO restorer or loader-work instrumentation.

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

| Field         | Type  | Schema-11 configuration              |
| ------------- | ----- | ------------------------------------ |
| algorithm     | `u32` | `1`, FNV-1a-64                       |
| table version | `u32` | `1`                                  |
| seed          | `u64` | `0xcbf29ce484222325`                 |
| bucket count  | `u32` | power of two, minimum 8              |
| entry count   | `u32` | number of canonical entries          |
| empty value   | `u32` | `0xffff_ffff`                        |
| maximum probe | `u32` | exact maximum emitted probe distance |

For names and glue, the header is followed by `bucket_count` little-endian
`u32` entry indices, then section-specific fixed-width entry records and
canonical key bytes.
Entries are sorted by complete canonical key bytes. Bucket count is the
smallest allowed power of two satisfying `entry_count * 4 <= bucket_count *
3`. Emission inserts entries in canonical order. The initial bucket is
`fnv1a64(seed, key) & (bucket_count - 1)` and collisions use forward linear
probing with wraparound.

Each 16-byte entry record contains key offset `u32`, key length `u32`, target
record index `u32`, and a zero reserved `u32`. Key spans are contiguous. Name
keys are the namespace byte followed by UTF-8 name bytes and glue keys are the
complete 24-byte glue record. The target is the dense record index in the
corresponding foundational store.

Token lists use direct-target lookup algorithm 2. Its header has the same
geometry and capacity policy, but each occupied bucket contains the token-list
record index directly; no entry table or copied key arena follows the buckets.
The hash consumes each canonical `u32` token word incrementally in
little-endian byte order. Lookup follows the linear-probe sequence, compares
each candidate exactly against the authoritative runtime token arena, and
stops at an empty bucket. Hash collisions therefore cannot return a false
match. Older version-1 token sections retain their literal-key lookup only on
the compatibility load path.

The lookup-configuration fingerprint covers the algorithm, algorithm version,
seed, capacity/load policy, empty sentinel, and probe strategy. Exact
configuration compatibility plus full structural validation are authoritative:
the decoder verifies bucket bounds, entry uniqueness, one bucket per entry,
canonical insertion/probe placement, key equality, and the declared maximum
probe. Deterministic checksum-derived spot checks additionally exercise the
runtime lookup implementation after validation. Schema 11 selects up to eight
entries per table from the container checksum using a fixed xorshift64*
sequence. Those checks are
supplementary diagnostics and can never make an incompatible fingerprint or
invalid structure acceptable.

## Immutable and job-local state

Frozen sections contain only state TeX deliberately preserves at `\dump`:
reachable names and current meanings, reachable tokens/macros, glue/fonts, code tables,
hyphenation data, reachable box/node graphs, format-visible environment cells,
interaction mode, and pdfTeX's INITEX resource closure.

The image excludes group journals, rollback epochs, allocation identities,
input frames/cursors, page-builder and mode-nest material, output transactions,
open streams, `World` effects/resources, clocks, random state, diagnostics and
provenance caches, incremental checkpoints, profiling counters, pending job
flags, and PDF pages or other job-only document state. Loading constructs
fresh job-local state, installs the current job clock, and schedules
`\everyjob`. Later mutable entries live in an overlay; the mapped/frozen bytes
are never mutated and group rollback applies only to job-local state.

Primitive identity tables are also driver-owned process state rather than
format payload. After `Universe::from_format` validates and installs the frozen
stores, the selected TeX82, e-TeX, pdfTeX, LaTeX-DVI, or pdfLaTeX driver
reconstructs its complete original-primitive registry without replacing the
live meanings restored by the format. This preserves deliberately shadowed
primitives while making primitive-enquiry and frozen-primitive tokens behave
the same as source initialization, without replaying store construction.

## Migration from schemas 9 and 10

Schema 9 was a deterministic semantic reconstruction format whose outer
envelope had one opaque payload rather than an extensible fixed-width section
directory and carried no compatibility fingerprints. Schema 10 introduced the
sectioned frozen-store representation, but it could not distinguish an absent
token-parameter cell from a present cell containing token-list record 0.
Schema 11 is therefore a clean boundary: the loader rejects schemas 9 and 10
with `UnsupportedVersion(9)` and `UnsupportedVersion(10)`. Users regenerate
format images from source under the schema-11 engine; Umber does not
reinterpret an old image heuristically.

Schema 11 writes environment cells only to kind 528 and node graphs only to
kind 512. Names, token lists, macros, glue, fonts, code tables, and hyphenation
exist only in authoritative sections 256 through 352 and are never reinterned
during normal loading. The decoder validates environment references and token
cell presence against those frozen stores before publication. Section 1
remains only for Universe-level format metadata until a later explicit schema
migration.

## Compatibility failures

The native CLI and host-neutral session report decoder failures as `format
image rejected: ...`; WASM returns the same message in its compile diagnostic.
Failures are deterministic and identify the rejected boundary:

- wrong magic means the input is not an Umber format image;
- any schema other than 11, including schemas 9 and 10, reports the unsupported
  version and must be regenerated rather than upgraded in place;
- ABI or lookup fingerprint mismatch means the image and runtime implement
  different schema-11 contracts;
- checksum mismatch means the bytes changed after publication; and
- directory, section, canonical-order, or cross-reference errors identify a
  structurally invalid image even when its checksum was recomputed.

Browser manifests reject an incompatible `engineVersion` or `formatSchema`
before downloading the object. Length and SHA-256 validate transport, then the
Rust decoder applies the complete schema-11 validation above. There is no
compatibility flag or fallback loader for TeX Live-native `.fmt`, schema 9,
schema 10, or partially migrated images.
