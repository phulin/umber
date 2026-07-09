# Compact Source Spans and Token Provenance

Status: proposed design for the source-location and provenance refactor that
follows mandatory packed token provenance.

## 1. Purpose

Umber currently allocates one 32-byte `OriginRecord::Source` for nearly every
token emitted from source. The record repeats `SourceId`, byte offset, line,
and column even though the input content and source cursor already determine
those values. It also computes byte offsets by rescanning the current line
prefix, so the existing provenance benchmark does not cleanly isolate origin
allocation cost.

This design adopts the source-location pattern used by production compilers:
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
- Ordinary source locations and arena-backed origins are indistinguishable to
  downstream callers. Only the aggregate state facade and resolver decode the
  representation.
- A source position is meaningful only while its source-map region is live on
  the current timeline. An arena origin is meaningful only while its record is
  live. Rolled-back or absent data resolves to unknown rather than causing a
  second diagnostic failure.
- Source-map and provenance rollback are watermark truncations. Snapshot
  capture remains O(1).
- Source position allocation and origin-record allocation are infallible from
  the engine's perspective. Representation exhaustion degrades through the
  fallback path described below and ultimately to unknown.
- All mutation and liveness validation remain behind `Universe`, `Stores`, or
  capability-appropriate aggregate facades. Downstream crates do not receive
  raw source-map or provenance-store mutation access.

## 4. Source map

### 4.1 Global byte-position space

Each live input buffer receives a disjoint region in a logical global byte
space:

```text
global byte space

0          1200                 4096
| prelude  | chapter.tex        | macro package ...
           ^
           global position 1342 = chapter.tex byte 142
```

A source-map entry records at least:

```rust
struct SourceRegion {
    start: SourcePos,
    len: u64,
    source: SourceId,
}
```

The source map does not own file I/O or duplicate input bytes. `World` remains
the authority for input records and content-addressed bytes. The map only
connects compact positions to those records.

Regions are append-only within a timeline. A source-map watermark joins the
aggregate store snapshot, and rollback truncates regions and restores the next
global position. Reusing discarded positions after rollback is safe for the
same reason that reusing discarded arena ids is safe: values from the discarded
timeline are no longer live.

### 4.2 Incremental lexer cursor

`SourceFrame` tracks both its character cursor and its UTF-8 byte cursor.
Reading a character increments the byte cursor by `ch.len_utf8()`. Any lexer
lookahead or rewind saves and restores the complete cursor, including control
sequence scanning and TeX `^^` notation.

The source-frame summary contains all cursor state required for exact replay.
Obtaining the current source position becomes O(1); it never rescans
`line[..offset]`.

### 4.3 Lazy lines and columns

Tokens never store line or column. When a diagnostic is rendered, the resolver
uses the source region and original input bytes to find the containing file and
byte offset. A per-input line-start index maps the byte offset to a line by
binary search. Column and caret padding are computed from the selected line at
render time.

The line-start index may be built when content is registered or lazily on the
first diagnostic. It is display-only cache data and does not participate in
semantic state or snapshot hashes.

## 5. Packed origin representation

`OriginId` remains an opaque 32-bit value carried in the low half of
`TracedTokenWord`. Internally, it has two nonzero forms:

```text
0x00000000                         unknown/bootstrap
0ppppppp pppppppp pppppppp pppppppp  direct SourcePos payload
1iiiiiii iiiiiiii iiiiiiii iiiiiiii  provenance-arena index
```

The exact bit constants must be private and covered by capacity tests. The
conceptual split reserves one tag bit, leaving approximately two billion
direct source positions and two billion arena entries. No downstream crate may
branch on the tag.

The direct payload stores `SourcePos + 1` so raw zero remains unknown. Checked
construction rejects positions that cannot be shifted into the payload and
uses the wide arena fallback instead.

### 5.1 Direct source form

The common case encodes the token's starting `SourcePos` directly. Emitting an
ordinary source character therefore performs no provenance-store mutation.

The resolver obtains a minimal display range by decoding the Unicode scalar at
that byte position. This is sufficient for ordinary characters and spaces.

### 5.2 Arena form

The arena form addresses structured records:

```rust
enum OriginRecord {
    UnknownBootstrap,
    SourceRange(SourceSpan),
    WideSourceRange(WideSourceSpan),
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

`hi` is exclusive. `SourceRange` is used when a token covers multiple source
characters or when exact spelling extent matters, including control sequences
and transformed `^^` input. `WideSourceRange` stores `SourceId` plus wide local
offsets when a buffer cannot fit in the direct global position space.

If the direct source space is exhausted, new source tokens use arena-backed
wide ranges. If the arena is also exhausted, allocation returns
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

- A one-scalar source token receives the direct start position.
- A multi-character control sequence receives an arena-backed source range.
- A token produced through `^^` notation receives the complete spelling range.
- End-line and paragraph tokens remain inserted origins whose parent is the
  relevant direct source position or range.
- Ignored characters and comments allocate no origins unless an error refers
  to them.

The semantic `Token` remains unchanged.

### 6.2 Token lists and macro definitions

Frozen token lists continue to use parallel `OriginListId` spans. Those lists
store opaque `OriginId` values and therefore support direct and arena forms
without a format change.

Macro replacement tokens reuse their definition-time origin list. Macro
arguments reuse their call-site origin lists. A replay frame carries one shared
`MacroInvocation` origin linking the invocation location and definition
location. Expansion traces continue to walk live replay frames rather than
allocating a wrapper for every delivered body token.

### 6.3 Scanners and execution

Consumers should retain the narrowest provenance needed by an actual
diagnostic:

- An error about one offending token uses that token's origin.
- A scanner that needs to label the complete consumed spelling may ask the
  aggregate facade to join compatible first and last source origins into a
  `SourceRange` record.
- Joining succeeds only when both endpoints resolve to the same live source
  and compatible expansion context. Otherwise the diagnostic keeps a primary
  origin and optional related origins rather than inventing one contiguous
  range.
- Execution nodes retain origins only where later diagnostics consume them.

Errors must carry explicit primary and related origins. They must not depend on
a mutable global "current location." Diagnostic rendering converts those
origins into primary ranges, secondary labels, source excerpts, and expansion
notes.

## 7. Resolver behavior

The resolver handles both forms of `OriginId`:

1. A direct source value is checked against the live source map and converted
   to a minimal `SourceSpan`.
2. An arena value is checked against the live provenance watermark and its
   record is read.
3. Inserted and synthesized records follow their parent.
4. Macro invocation records expose invocation and definition locations.
5. Missing, rolled-back, or exhausted data resolves to unknown.

Line text, line number, display column, caret width, source label, and bounded
macro traces are produced only at this boundary.

## 8. Rollback and replay

An aggregate snapshot records:

- the source-region watermark and next direct position;
- the provenance-record watermark;
- existing origin-list span and entry watermarks.

Rollback truncates all three stores as one tuple. Direct positions in retained
regions stay live. Direct positions in discarded regions fail liveness checks.
Arena records and origin-list entries follow the existing truncation policy.

Input summaries compare and hash decoded semantic tokens, not origin bits.
Source cursor fields needed for replay are operational input state, but direct
positions and range identities remain excluded from semantic convergence.

## 9. Capacity and format constraints

Before adopting the tagged representation, implementation must establish and
test:

- the exact number of direct source bytes and arena records available;
- reserved encodings for unknown and overflow;
- checked arithmetic when assigning source regions;
- fallback behavior for a single oversized source and cumulative source-map
  exhaustion;
- origin-list compatibility with both packed forms;
- whether snapshot serialization or debug tooling exposes raw origin values.

No raw `OriginId` encoding is a stable artifact format. If provenance is ever
serialized, it must use an explicit versioned logical representation rather
than dumping packed ids.

## 10. Rejected alternatives

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
possible internal fallback for wide ranges only if measurements later justify
it.

### Full span beside every token

This is conventional in parser-oriented compilers, but it would enlarge
Umber's packed runtime token and add bandwidth to every macro movement. It may
be reconsidered only if precise ranges become more valuable than the measured
hot-path cost.

## 11. Phased implementation

Each phase is independently reviewable and must preserve the existing semantic
and output parity suites. Later phases do not begin until the preceding phase's
measurements and invariants are documented.

### Phase 1: Incremental coordinates and trustworthy baselines

- Add the UTF-8 byte cursor to source frames and resumable summaries.
- Make lookahead and rewind restore the complete cursor.
- Remove line-prefix rescanning from coordinate production.
- Extend lexer tests for ASCII, mixed-width UTF-8, control sequences, comments,
  normalized end lines, `^^` notation, and snapshot resume.
- Re-run the source provenance benchmarks and update
  `docs/provenance_performance.md`.

Exit gate: coordinates are exact, long-line tokenization is linear, and the
readonly/traced comparison isolates provenance work.

### Phase 2: Source-map substrate

- Add opaque `SourcePos`, `SourceSpan`, source regions, and rollback marks in
  `tex-state` behind the aggregate facade.
- Register input content without moving file I/O or bytes out of `World`.
- Implement live position lookup and lazy line-start resolution.
- Convert `ProvenanceResolver` to render source records through the source map
  while retaining the existing flat source-origin representation.

Exit gate: all existing diagnostics render identically or improve, source-map
rollback/reuse is tested, and no downstream crate can mutate raw map state.

### Phase 3: Tagged direct-source origins

- Make `OriginId` encoding opaque and add private direct/arena constructors and
  decoders.
- Add capacity, saturation, liveness, packing, rollback, and state-hash tests.
- Emit direct positions for ordinary one-scalar source tokens.
- Keep existing arena source records as the fallback during migration.
- Extend provenance statistics to distinguish direct source deliveries from
  arena records without exposing mutation internals.

Exit gate: the common source-character path performs zero provenance-record
appends, token packing remains 64 bits, and all semantic hashes and parity
artifacts are unchanged.

### Phase 4: Precise source ranges

- Add `SourceRange` and wide-range records.
- Emit exact spelling ranges for control sequences, transformed input, and
  other multi-character source tokens.
- Add range-joining support for scanners with a demonstrated diagnostic use.
- Upgrade structured errors and rendering to support a primary range and
  related origins.

Exit gate: focused diagnostics underline exact token spellings across ASCII,
UTF-8, control sequences, and `^^` transformations, with graceful degradation
for incompatible or unavailable ranges.

### Phase 5: Derived provenance and replay audit

- Audit inserted, synthesized, macro-definition, macro-argument, and
  macro-invocation paths against the mixed direct/arena representation.
- Verify origin lists, replay frames, snapshot rollback, memo reconstruction,
  and stale-side-table degradation.
- Preserve the rule that macro-body delivery performs no per-token provenance
  writes.

Exit gate: every token-delivery path carries valid best-effort provenance,
discarded timelines retain no live arena or source-map growth, and expansion
traces remain bounded and lazy.

### Phase 6: Measurement, cleanup, and adoption decision

- Benchmark source-heavy ASCII, mixed UTF-8, control-sequence-heavy,
  scanner-heavy, macro-heavy, and rollback workloads.
- Report time, arena growth, source-map growth, and total estimated bytes.
- Remove the legacy flat source-origin path only after the fallback and
  capacity tests pass.
- Update `docs/architecture.md`, `docs/core_state.md`, and
  `docs/provenance_performance.md` to describe the adopted representation and
  measurements rather than the proposal.

Exit gate: the design is adopted only if measurements show a material memory
improvement without an unacceptable token-throughput regression; otherwise
the source-map and incremental-coordinate improvements remain and the tagged
encoding is revised or reverted cleanly.

## 12. Validation matrix

Every implementation phase runs the affected crate tests and the repository
gate. The final phase additionally covers:

| Area | Required cases |
| --- | --- |
| Lexer coordinates | ASCII, UTF-8, normalized lines, comments, spaces, control sequences, `^^` notation |
| Source map | multiple inputs, empty input, oversized input fallback, rollback and position reuse |
| Packing | direct source, arena record, unknown, both capacity boundaries |
| Ranges | one scalar, multi-character token, incompatible endpoints, missing source bytes |
| Expansion | macro body, macro argument, nested invocation, inserted and synthesized tokens |
| Semantics | token equality, `\ifx`, token-list interning, input-summary equality, state hashes |
| Rollback | discarded direct regions, arena records, origin lists, diagnostic rendered before rollback |
| Output | existing fixture and DVI parity corpuses remain byte-identical |

Use `cargo test --workspace --tests` through `scripts/check.sh` for the local
gate. Keep long-running parity corpuses in their existing scripts rather than
moving them into ordinary unit tests.
