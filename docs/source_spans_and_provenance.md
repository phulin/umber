# Compact Source Spans and Token Provenance

Status: authoritative contract for the adopted compact source-map, source-span,
and derived-provenance representation.

The ownership and lifetime of these values is governed by
[Structural and demand-driven provenance](structural_provenance.md). Where this
document describes append-only arena retention or convergence-time graph
import, the structural ownership contract supersedes that representation while
preserving the packed coordinate and resolver behavior specified here.

## 1. Purpose

Umber follows the source-location pattern used by production compilers:
hot paths carry compact byte locations or handles, a central source map owns
buffer-to-file mapping, and diagnostic formatting resolves line, column,
source excerpts, and macro traces lazily.

The design has four goals:

1. Eliminate provenance-arena writes for ordinary source character tokens.
2. Preserve `TracedTokenWord(u64)` and its 32-bit provenance field.
3. Support precise source ranges and shared macro-expansion context without
   making provenance part of TeX semantics.
4. Preserve snapshot rollback, replay, overflow degradation, and the aggregate
   live-state boundary.

## 2. Non-goals

- Provenance does not participate in `Token` equality, token-list interning,
  `\ifx`, semantic hashes, memo keys, page output, or DVI artifacts.
- The refactor does not pre-tokenize source lines. Catcode changes between
  delivered tokens must remain observable.
- The refactor does not put rendered paths, line text, columns, or diagnostic
  strings in tokens or provenance records.
- The refactor does not add a per-expanded-token macro wrapper.
- The refactor does not require every stored TeX node to retain a full range.
  Long-lived state keeps diagnostic provenance only where a later diagnostic
  has a concrete consumer.

## 3. Required invariants

- Every token delivered to expansion or execution carries an opaque
  `OriginId`; `OriginId::UNKNOWN` remains the graceful fallback.
- Source byte coordinates always address immutable physical backing bytes.
  Line normalization never makes removed terminators, stripped trailing
  spaces, or a synthetic `\endlinechar` look like bytes that existed in that
  backing.
- Every traced source is registered before its first token is delivered.
  World inputs refer to their live `InputRecordId`; generated and in-memory
  inputs retain shared immutable backing through a state-owned registration.
- Ordinary source locations and arena-backed origins are indistinguishable to
  downstream callers. Only the aggregate state facade and resolver decode the
  representation.
- An engine source position is meaningful only while its source-map region is
  live on the current timeline. An editor-fragment position remains meaningful
  for the session and resolves through the current layout to either a current
  document offset or typed `Deleted`. An arena origin is meaningful only while
  its record is live. Rolled-back or absent data resolves to unknown rather
  than causing a second diagnostic failure.
- Source-map and provenance rollback are watermark truncations. Snapshot
  capture remains O(1).
- Source position allocation and origin-record allocation are infallible from
  the engine's perspective. Representation exhaustion degrades through the
  fallback path described below and ultimately to unknown.
- Required measurement must not add production hot-path counter writes.
  Direct-delivery counts come from benchmark-only instrumentation or
  crate-private inspection of returned ids, while production statistics remain
  derived from arena/region lengths and capacities.
- Origin-list absence is whole-list state. A transformation that extends a
  token list either extends its existing parallel origin list by the same
  number of entries or preserves `OriginListId::EMPTY`; it never creates a
  partial origin list for an otherwise untraced token list.
- `SourcePos` is a logical `u64` coordinate. The smaller direct payload in
  `OriginId` is an encoding optimization, not the definition of the source
  coordinate domain.
- A diagnostic captures its primary origin, labeled related origins, and the
  head of a persistent parent-linked macro-invocation chain. Paths, lines,
  excerpts, display widths, trace depth, and strings remain lazy.
- All mutation and liveness validation remain behind `Universe`, `Stores`, or
  capability-appropriate aggregate facades. Downstream crates do not receive
  raw source-map or provenance-store mutation access.

## 4. Source map

### 4.1 Logical position space

Each live input backing receives a disjoint region in a logical global
`u64` position space:

```text
logical source position space

0          1201                 4098
| prelude· | chapter.tex·       | macro package ...
           ^
           global position 1343 = chapter.tex byte 142

· = one reserved end-of-backing anchor position
```

A source-map entry records at least:

```rust
struct SourceRegion {
    start: SourcePos,
    byte_len: u64,
    source: SourceId,
    backing: SourceBacking,
}

enum SourceBacking {
    World(InputRecordId),
    Generated(GeneratedSourceId),
}
```

For a region starting at `start`, physical byte offset `n` maps to
`start + n`. The position `start + byte_len` is a reserved anchor for EOF and
empty-input diagnostics; the next region starts one position later. Direct
origins are created only for backed byte positions, never for the anchor.
Half-open spans may end at the anchor, and an empty or EOF diagnostic may use
the zero-width span `[anchor, anchor)`.

`World` remains the authority for World input records and content-addressed
bytes. A World-backed region stores only its `InputRecordId`; it does not copy
the bytes. Generated and memory inputs instead share immutable bytes between
their input adapter and a rollback-coupled generated-source registry. This is
necessary because their input frame may disappear before a diagnostic is
rendered. Production traced-input paths may not rely on a transient
`InputSource` as the only owner of diagnostic source text.

An input adapter exposes a source descriptor containing its physical byte
length and either a World record or shared generated backing. Generated
descriptors may also carry the backing's logical path; the editor root uses
this form, while anonymous pseudo-files remain explicitly unnamed.
`tex-command`'s input state mints `SourceId` and idempotently registers
`(SourceId, descriptor)` through the aggregate state facade before the first
delivery from a source level. The command snapshot retains the source identity
allocator high-water mark so resumed input cannot reuse a still-live id.

Regions are append-only within a timeline. A source-map watermark joins the
aggregate store snapshot, and rollback truncates regions plus their allocation
identity tags. Logical positions are process-unique and are not rewound or
reused after rollback; fork descendants therefore cannot reinterpret one
another's direct-origin payloads. Registration uses checked `u64` arithmetic. If a
region and its anchor cannot be represented, that source remains executable
but its new source origins degrade to `OriginId::UNKNOWN`.

### 4.2 Physical lines and a byte-canonical lexer cursor

Unicode command-profile inputs are valid UTF-8. A malformed byte sequence is
rejected during complete-backing registration over its exact physical byte
range before source identity allocation or lossy conversion can destroy the
mapping. The exact-eight-bit command profiles instead preserve and deliver
every byte without decoding. Both modes use the same byte-canonical physical
line and range structure; they do not reinterpret one another's character
domain.

The physical-line reader retains:

- the raw start and content-end byte offsets;
- the raw line-terminator range, including the distinction between LF and
  CRLF;
- the byte offset after the retained prefix once TeX trailing-space stripping
  has run; and
- whether the normalized suffix contains a synthetic `\endlinechar`.

`SourceFrame` uses one byte cursor as its canonical indexing state. Unicode
delivery decodes at most four bytes and advances the cursor by `ch.len_utf8()`;
exact-eight-bit delivery advances it by one. A separately maintained scalar
delivery offset is retained only where operational state requires it; there is
no parallel mutable `Vec<char>` index. Any lexer lookahead or rewind saves and
restores the byte cursor and scalar state together, including control-sequence
scanning and every success or failure path in TeX `^^` notation.

For a cursor inside the retained physical prefix, the source offset is the
physical line start plus the normalized byte cursor. The synthetic end-line
character maps to a zero-width anchor after the retained prefix. A spelling
that mixes backed characters and the synthetic suffix spans the backed prefix
and ends at that anchor; it never claims that the synthetic character existed
in the original bytes.

The source-frame summary contains the normalized byte cursor, physical-line
metadata, column, lexer N/M/S state, pending traced tokens, and all other state
required for exact replay. `InputSummary` also preserves the source-id
allocator high-water mark and Unicode `^^` configuration. Obtaining the
current physical source position becomes O(1); it never rescans a line prefix.

For the editor root, a frozen `LayoutCursor` is installed beside the reopened
contiguous document buffer. Its immutable O(pieces) segment table maps each
physical line refill to one fragment `RegisteredSource` and a
fragment-relative line base. The mutable cursor advances only at refill, while
ordinary token delivery keeps the same single-add `direct_origin(start, end)`
path. Source summaries retain document offsets as before; resume reinstalls and
seeks the frozen cursor from those offsets without changing the root
`SourceId`.

### 4.3 Lazy lines and columns

Tokens never store line or column. When a diagnostic is rendered, the resolver
uses the source region and original input bytes to find the containing file and
byte offset. A per-input line-start index maps the byte offset to a line by
binary search. Column and caret padding are computed from the selected line at
render time.

The line-start index records physical byte starts and understands LF, CRLF,
and a missing final terminator. It may be built when content is registered or
lazily on the first diagnostic. A cache that survives rollback is keyed by
stable content identity such as `ContentHash`, never solely by a reusable
`InputRecordId`; otherwise it is truncated with the source registry. Cache
data is display-only, does not participate in semantic state or snapshot
hashes, and is measured separately as retained diagnostic memory.

### 4.4 Edit-stable fragment overlay

The editor root uses a session-scoped `FragmentStore` alongside the
rollback-coupled source map. Each immutable fragment reserves one disjoint
logical range plus its end anchor from the same non-rewinding allocator used
by engine sources. The session retains metadata for current and
checkpoint-protected fragments plus a fixed 64-row retired-coordinate window.
Accepted transitions move newly unprotected rows into that FIFO and evict its
oldest row when full; engine generations share the bounded window and receive
only the installed layout's current metadata and backing. Mutable fragment
bytes and pruning state stay in the session owner. Every writable clone
receives a fresh append lineage, and `FragmentId` carries that lineage plus its
dense slot, so sibling appends cannot alias even when they occupy the same
slot. Discarded forks and retired rows cannot cause either fragment ids or
logical ranges to be handed out again.

A raw coordinate in the retired window resolves typed `Deleted`; one evicted
from the window resolves `Unknown`. Detached artifact and diagnostic
`RootSpanId` recipes instead own their registration, mint revision, range, and
content identity, so they retain exact `Current`/`Deleted` behavior without a
historical store row.

`EditorLayout` is an immutable piece table for one `LayoutGeneration`. It
validates generation-tagged fragment identities and relative piece ranges and
stores document prefix sums. `Universe::install_editor_fragments` requires the
paired layout and revalidates that pairing before publishing fragment metadata.
Layout-aware resolution checks fragments before the engine source map: a live
piece yields current document offsets and lazy line/column data, a fragment
range absent from the layout yields typed `Deleted`, an engine source yields
`Foreign`, and invalid or absent provenance yields `Unknown`. Its line-start
cache is display-only and belongs to the exact layout generation, preventing a
retained origin from observing a stale line index after an edit.

## 5. Packed origin representation

`OriginId` remains an opaque 32-bit value carried in the low half of
`TracedTokenWord`. Internally, its exact private layout is:

```text
0x00000000                              unknown/bootstrap
0x00000001..=0x7fffffff                 direct SourcePos = raw - 1
0x80000000..=0xffffffff                 provenance arena index = raw & 0x7fffffff
```

This provides exactly 2,147,483,647 directly encodable logical positions
(`0..=0x7fff_fffe`) and 2,147,483,648 arena indexes
(`0..=0x7fff_ffff`). The raw constants, raw accessors, constructors, and
decoders are crate-private. No downstream crate may inspect or branch on the
tag. Unknown is a logical value and does not need to occupy arena index zero.

Checked construction rejects positions whose `SourcePos + 1` does not fit the
clear-tag payload and uses an arena-backed `SourceSpan` instead. The logical
source map continues above that boundary until `u64` registration arithmetic
itself is exhausted.

### 5.1 Direct source form

The common case encodes the token's starting `SourcePos` directly. Emitting an
ordinary source character therefore performs no provenance-store mutation.

The resolver obtains a minimal display range by decoding the Unicode scalar at
that physical byte position. Direct origins are valid only for backed UTF-8
scalar starts, so this is sufficient for ordinary characters and spaces.

### 5.2 Arena form

The arena form addresses structured records:

```rust
enum OriginRecord {
    SourceSpan(SourceSpan),
    MacroInvocation(MacroInvocationOrigin),
    Inserted(InsertedOrigin),
    Synthesized(SynthesizedOrigin),
    Synthetic(SyntheticOrigin),
}

struct SourceSpan {
    lo: SourcePos,
    hi: SourcePos,
}
```

`hi` is exclusive. Construction resolves `lo` to one live source region and
requires `lo <= hi <= region.anchor`; it never resolves `hi` independently,
because an exclusive high endpoint may equal the region anchor. A zero-width
span is valid, including `[anchor, anchor)` for empty input or EOF.

`SourceSpan` is used when a token covers multiple source characters, contains
a synthetic normalization anchor, or lies outside the direct payload. There
is deliberately no separate `WideSourceSpan`: a logical `u64` position has the
same meaning whether its origin is direct or arena-backed.

If the direct source space is exhausted, new source tokens use arena-backed
`SourceSpan` records. If the arena is also exhausted, allocation returns
`OriginId::UNKNOWN`. Diagnostic resource exhaustion never aborts TeX
execution.

### 5.3 Why a hybrid representation

An eight-byte span on every token would enlarge the hot token carrier and
increase memory traffic throughout macro expansion. A point-only
representation cannot underline a multi-character control sequence exactly.
The hybrid keeps the current packed carrier, makes ordinary characters
allocation-free, and pays for a range record only when the source spelling is
nontrivial or a consumer explicitly needs a composed range.

## 6. Threading source ranges

### 6.1 Lexer

The lexer records the start position before consuming a token and the end
position after consuming it:

- A one-scalar token backed by one physical UTF-8 scalar receives the direct
  start position when it fits the payload; otherwise it receives an
  arena-backed `SourceSpan`.
- A multi-character control sequence receives an arena-backed source range.
- `tex-command` registers each immutable command source through its
  `CommandContext` source-map boundary before its first raw delivery. The raw
  observation retains the exact registered range and opaque origin; expansion
  and replay retain the traced origin, while a backed-up direct-source command
  also carries its committed range as backup-level metadata. Neither value
  consults or changes fixture data.
- A token produced through `^^` notation receives the complete spelling range.
- End-line and paragraph tokens remain inserted origins whose parent is the
  zero-width normalized-line-end anchor or the relevant backed range.
- Ignored characters and comments allocate no origins unless an error refers
  to them.
- Invalid UTF-8 and other failures that occur before a valid TeX token exists
  carry a structured diagnostic site over the offending physical bytes.

The semantic `Token` remains unchanged.

### 6.2 Token lists and macro definitions

Frozen token lists continue to use parallel `OriginListId` spans. Those lists
store opaque `OriginId` values and therefore support direct and arena forms
without a format change.

Macro replacement tokens reuse their definition-time origin list. Macro
arguments reuse their call-site origin lists. A replay frame carries one shared
`MacroInvocation` origin linking the invocation location, definition location,
and parent invocation. Delivering body or argument tokens never allocates
per-token wrapper records.

Invocation records use fixed-width archived `(packed key, record)` slots in
1,024-record immutable chunks. Each timeline leases process-global packed keys
in 256-key ranges, so concurrent timelines do not fragment one another's
index one record at a time. Unused keys are never reassigned. A sparse affine
run index replaces a per-record hash-table entry: a straight timeline needs at
most one run per lease, while rollback and forks add runs where key ranges
diverge. The index accepts imported fork ranges independently of their global
key order while record slots remain append-only. This storage is charged
conservatively at 64 retained bytes per invocation, including chunk and
run-index metadata. The production arena admits at most 1,048,576 such charges
(64 MiB); after that diagnostic-only allocations degrade to unknown without
aborting TeX execution. The on-demand macro memory report scans the fixed-width
records so measurement adds no expansion-path counter writes.

The input stack maintains the active invocation head in O(1). When nested
frames retire before one delivery attempt completes, it retains only the
innermost retired head; its parent links preserve the complete outer chain.
An error captures that head in its diagnostic site. Resolving invocation and
definition locations, choosing trace depth, and formatting remain lazy.

### 6.3 Scanners and execution

Consumers should retain the narrowest provenance needed by an actual
diagnostic:

- An error about one offending token uses that token's origin.
- Expansion-control classification inspects arena-backed inserted origins
  without resolving direct source positions. This keeps editor-fragment
  positions on their session-owned layout timeline while stale arena origins
  still fail liveness validation.
- When scanner recovery backs up expanded tokens, later diagnostics use the
  first token at that replay frontier; they do not replace it with the mutable
  source cursor's current position.
- A scanner that needs to label the complete consumed spelling may ask the
  aggregate facade to join compatible first and last source origins into a
  `SourceSpan` record.
- Two `OriginId`s do not encode expansion context and therefore cannot prove
  join compatibility. The initial join API accepts an unforgeable proof that
  both endpoints were delivered directly from the same live source frame.
  Expanded or replayed endpoints are not joined in the first implementation;
  they remain primary and related locations. A later context-aware scanner API
  may return an explicit `DeliveryContextKey` beside tokens that opt into
  composition, without enlarging `TracedTokenWord` or every delivery path.
- Joining also requires ordered endpoints in the same source region. If any
  proof or liveness check fails, the diagnostic keeps separate locations
  rather than inventing a contiguous range.
- Character and math-character nodes retain one compact origin id and ligature
  nodes retain one per consumed source character. These diagnostic-only
  columns survive ligaturing, hyphenation, math layout, packing, and line
  breaking so an accepted compile session can answer a lazy rendered-source
  query. The rendered DOM supplies the producing session's 128-bit output
  identity and accepted revision; both are checked before this sidecar is
  indexed. Synthetic characters use a
  related source origin where one is well-defined and otherwise degrade to
  `OriginId::UNKNOWN`.
- Character origins are excluded from node semantic identity, state hashes,
  format images, artifact bytes, and artifact content identity. Shipout may
  retain an in-process artifact-node sidecar for an explicit diagnostic
  consumer, and retained-output accounting includes that memory. A rendered
  source query may also lazily build a per-page event-prefix/origin map from
  that sidecar. The map is accepted-output-owned operational state when it fits
  the configured render-cache byte budget: live session telemetry adds its
  exact logical charge to `output_bytes`, while the point-in-time metrics copied
  into the accepted output remain unchanged. An over-budget map is used
  ephemerally and changes only later query work. The layout line-start index
  built while resolving a current origin is instead checkpoint-owned diagnostic
  state and is charged to `diagnostic_bytes` and the protected checkpoint
  budget.
- Detached artifact provenance recipes retain stable piece anchors plus compact
  relative ranges for rendered nodes. Expanded deliveries which produced no
  artifact node are absent from the rendered-source sidecar.

Errors use a structured payload conceptually equivalent to:

```rust
struct DiagnosticSite {
    primary: Option<OriginId>,
    related: InlineRelatedLocations,
    expansion_head: Option<OriginId>,
}

struct RelatedLocation {
    role: RelatedLocationRole,
    origin: OriginId,
}
```

The bounded related-location collection may be fixed inline or allocate only
on the error path; it is not stored beside ordinary tokens. Roles distinguish at
least invocation, definition, recovery frontier, and secondary consumed
spelling. Errors must not depend on a mutable global "current location" or on
replay frames remaining live. Diagnostic rendering converts the captured ids
into primary ranges, labeled secondary locations, source excerpts, and
expansion notes.

## 7. Resolver behavior

The resolver handles all logical forms of `OriginId`:

1. A direct source value is checked against the live source map and converted
   to a minimal `SourceSpan`.
2. An arena value is checked against the live provenance watermark and its
   record is read.
3. Inserted and synthesized records follow their parent.
4. Macro invocation records expose invocation and definition locations plus a
   parent invocation; the diagnostic site's captured head supplies the trace.
5. Missing, rolled-back, or exhausted data resolves to unknown.

Line text, line number, display column, caret width, source label, and
presentation-bounded macro traces are produced only at this boundary.
World-backed paths come from their input records, and generated paths come
only from their source descriptors. The resolver never accepts a caller-level
fallback path: an anonymous generated input remains `<generated>` and cannot
be mislabeled as the editor root.

Internal byte offsets and spans are zero-based. User-facing line and column
numbers are one-based. Display columns use Unicode display-cell width; tabs
advance to eight-column stops, combining marks have zero width, and an
otherwise zero-width underline still renders one caret cell. A single-line
span underlines at least one cell. A multi-line span renders the first and last
affected lines with an omission marker between them. These are presentation
rules only and never feed back into TeX state or source identity.

At the persistent compile boundary, the primary location is resolved while the
failed candidate universe is live and copied into one owned optional value.
That value contains path, the exact half-open UTF-8 byte range, line, and
column together; resolution failure omits the whole value. LaTeX project and
WASM/package layers preserve it without parsing diagnostic text or recomputing
coordinates. Related locations and expansion traces remain engine-internal
presentation data unless a future public contract explicitly exposes them.

## 8. Rollback and replay

An aggregate snapshot records:

- the source-region and generated-backing watermarks plus the next logical
  position;
- the provenance-record watermark;
- existing origin-list span and entry watermarks.

Rollback restores these together with the World input-record watermark as one
aggregate tuple. Direct positions in retained regions stay live. Direct
positions in discarded regions fail liveness checks and are never reassigned.
Packed arena-origin keys are likewise never reassigned; rollback removes their
lookup entries. Arena records,
generated backings, and origin-list entries follow the existing truncation
policy. A derived cache that survives rollback must be keyed by stable content
identity, so reuse of `SourceId`, `InputRecordId`, or logical positions cannot
make stale data appear live.

Input summaries compare and hash decoded semantic tokens, not origin bits.
Source cursor fields, the source-id allocator high-water mark, and lexer
configuration needed for replay are operational input state, but direct
positions, range identities, diagnostic sites, and source-map cache identities
remain excluded from semantic convergence. Restoring the high-water mark is
nevertheless mandatory so future source ids are deterministic and do not
alias retained regions.

Capturing invocation ids in a diagnostic site preserves the trace across
replay-frame pop, not across provenance rollback. Any diagnostic that must
outlive rollback past its origin/source-map watermark is rendered to owned text
before rollback.

Main-control operations pair their `CommandState` snapshot with this
aggregate watermark in one private rollback value. Rollback removes all
invocation records created by a failed macro scan or a resource-suspended
nested expansion at the same time that it restores macro activations and
shared argument ranges. Ordinary rollback retires keys for removed arena
records. A local atomic-step retry leases the discarded source registrations
and origin records in allocation order, reusing an identity only while the
replayed registration or record is exactly equal; the first divergence
abandons the lease and returns to fresh process-global keys. Thus identical
command redelivery has stable diagnostic identity without letting a detached
stale `OriginId` alias different provenance. A retained aggregate checkpoint,
by contrast, restores the identical committed origin prefix and records.

## 9. Capacity and format constraints

The tagged representation preserves and tests:

- the exact direct range `0..=0x7fff_fffe` and arena-index range
  `0..=0x7fff_ffff`;
- raw zero as the only reserved unknown/overflow encoding;
- checked arithmetic when assigning source regions;
- fallback behavior for a source region crossing the direct boundary, a
  single source larger than the direct range, cumulative direct-space
  exhaustion, and logical `u64` registration exhaustion;
- origin-list compatibility with both packed forms;
- whether snapshot serialization or debug tooling exposes raw origin values.

The explicit `state_budgets` performance tier enforces at most 64 retained
bytes per macro invocation over its specified 2,048-call,
32,768-output-token long run. The value is an empirical regression threshold
for that sample and is also the conservative production admission charge; it
is not a per-document invariant. Story and Gentle report the observed ratio,
but enforce the underlying layout invariants: an archived slot fits the
64-byte admission charge, and retained archive, tail, chunk-pointer, and
affine-run capacity fits the bound derived from the 1,024-slot chunk geometry
and the containers' geometric growth. Origin-list and source-map storage remain
separately visible in total provenance bytes.

The 1,024-slot chunk and 256-key lease are explicit layout contracts, not
inputs that a test silently learns from whatever production constants happen
to be current. The chunk bounds the mutable snapshot tail copied at a fork;
the lease amortizes the process-global allocator while keeping a fresh,
unimported job to at most one affine run per lease. Conformance gates pin both
widths and reject excess fresh-job runs independently of the capacity formula.

This distinction matters for short loaded-format jobs. The source-bootstrap
and persistent-loaded Story A/B at migration base `8a5726a8` used the same
engine, staged Story source, clock, runtime configuration, and finite guards:

| component                        | source bootstrap | loaded Plain |
| -------------------------------- | ---------------: | -----------: |
| origin records                   |            9,261 |           54 |
| archived record capacity         |            9,280 |           64 |
| archive chunk-pointer bytes      |              256 |            0 |
| affine key runs / capacity       |          50 / 64 |        1 / 4 |
| macro invocations                |              222 |           23 |
| attributed macro retained bytes  |           12,483 |        1,547 |
| observed bytes per invocation    |               57 |           68 |
| origin-list spans / entries      |      512 / 5,019 |        1 / 0 |
| source-map live / retained bytes |  46,096 / 46,160 |  392 / 1,109 |

The loaded run retains 87.6% fewer attributed macro bytes. Its apparent
68-byte ratio is the expected `Vec` tail rounding from 54 live records to 64
slots plus four key-run slots, proportionally attributed across 23 macro
records; it is not construction leakage. Loaded formats reconstruct empty
diagnostic arenas, and cold-entry versus cache-hit runs of the same fresh job
must have identical complete retained layouts. These controls replace the
stale practice of applying the long-run 64-byte ratio to every document while
still detecting shared-cache leakage, repeated-job accumulation, slot growth,
or excess container capacity.

These arenas own diagnostic history, not semantic engine material. Repeated
macro invocations may thus make provenance the dominant RSS owner while the
node arena stays flat. Such growth must be diagnosed against source/scanner
progress: saturation to unknown bounds diagnostic retention, but it does not
make a replay loop correct. Integer-`\dimexpr` stress coverage enforces at most
512 retained provenance bytes per completed scan over 4,096 scans, alongside a
finite expansion-work bound.

No raw `OriginId` encoding is a stable artifact format. If provenance is ever
serialized, it must use an explicit versioned logical representation rather
than dumping packed ids.

## 10. Adoption decision

The compact representation is adopted. Against the original traced baseline,
logical provenance/source-map storage fell by 95.73% for ASCII input, 93.93%
for mixed UTF-8, and 99.99% for a single long line. No required throughput row
crossed the 5% regression ceiling; ordinary ASCII, mixed UTF-8, long-line,
macro-replay, scanner, and generated-value workloads were neutral or faster.

Cold and repeated diagnostic resolution intentionally retain no shared cache.
Resolution is error-path work, and measured repeated cost did not justify
checkpoint-coupled cache ownership. Accepted rendered-source maps remain lazy
session output and are charged to accepted-output retention rather than to
snapshot capture.

`OriginRecord::Source` remains only as degraded compatibility for explicitly
unregistered origins created by older APIs and focused tests. Production World
and memory inputs register before delivery and emit direct positions, validated
`SourceSpan`s, or structured derived records.

## 11. Rejected alternatives

### Flat source records

Keeping one full record per source token is simple but repeats source identity,
line, and column, and requires a store append on the dominant lexer path.

### Hash-consing source coordinates

Source coordinates are normally unique, so deduplication saves little and
complicates rollback liveness.

### Pre-tokenizing or batch-reserving a line

TeX can change catcodes between tokens from the same physical line. Allocation
optimization must not change token-at-a-time lexer semantics.

### Source-run segments as the primary representation

Affine source runs compress ordinary input, but every lookup requires segment
resolution and irregular spelling breaks runs. Direct source positions remove
the common-case arena write and lookup entirely. Run compression remains a
possible arena-storage optimization only if measurements later justify it.

### A separate wide local-span representation

Encoding fallback spans as `(SourceId, local_lo, local_hi)` creates a second
coordinate domain and duplicates resolver, validation, rollback, and testing
paths. One logical `u64 SourcePos` space keeps direct and arena-backed origins
semantically identical; only their `OriginId` encoding differs.

### Full span beside every token

This is conventional in parser-oriented compilers, but it would enlarge
Umber's packed runtime token and add bandwidth to every macro movement. It may
be reconsidered only if precise ranges become more valuable than the measured
hot-path cost.

## 12. Validation matrix

The focused suites and repository gates cover:

| Area              | Required cases                                                                                                                                                           |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Lexer coordinates | ASCII, valid UTF-8, invalid UTF-8 error, LF, CRLF, trailing-space stripping, missing final newline, comments, spaces, control sequences, successful/failed `^^` notation |
| Input resume      | byte cursor, column, physical-line metadata, next source id, Unicode `^^` mode, nested-source exhaustion                                                                 |
| Source backing    | World input, generated/memory input, multiple/empty inputs, missing content, frame pop                                                                                   |
| Source map        | anchor positions, direct-boundary crossing, oversized input, logical overflow, rollback and SourceId/InputRecordId/position reuse, cache alias prevention                |
| Packing           | first/last direct source, first/last arena index, unknown, fallback and saturation                                                                                       |
| Ranges            | one scalar, multi-character token, zero-width, multi-line, synthetic suffix, incompatible context, missing source bytes                                                  |
| Diagnostics       | primary and labeled related locations, Unicode/tabs, captured trace after frame pop, cold/warm resolution                                                                |
| Expansion         | macro body, macro argument, nested invocation, inserted and synthesized tokens, zero body-token writes                                                                   |
| Semantics         | token equality, `\ifx`, token-list interning, input-summary equality, state hashes                                                                                       |
| Rollback          | discarded direct regions, generated backings, arena records, origin lists, stale caches, diagnostic rendered before provenance rollback                                  |
| Output            | existing fixture and DVI parity corpuses remain byte-identical                                                                                                           |

Use `scripts/check-and-test.sh` for the workspace tests plus the format and
clippy gate. Keep long-running parity corpuses in their existing scripts
rather than moving them into ordinary unit tests.
