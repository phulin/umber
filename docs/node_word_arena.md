# Compact node-word arena

Status: authoritative design; all six phases are implemented and the compact
representation is adopted by the Phase 6 measurements below.

This document combines the Phase 1 layout/frequency baseline with the layout
contract for replacing the epoch arena's `Vec<Node>` by a compact word stream.
There is deliberately no separate `node_word_layout.md`: arena ownership,
encoding, rollback, migration, and adoption are one design and must not drift.

## 1. Decision and scope

The measured premise is useful but weaker than the original proposal. Char,
glue, kern, and penalty nodes are 66.52% of all appends, not an overwhelming
majority. The DVI-heavy workload is 94.30% common-four, while math workloads
contain many boxes and intermediate noads. We will therefore implement a
general compact stream with sidecars, but treat vectorized widths and final
adoption as measured decisions rather than assumptions.

The representation preserves these hard invariants:

- frozen lists are immutable and are minted only by the aggregate state API;
- every mutable semantic or ownership field is in the `Universe`/`Stores`
  aggregate boundary, including all sidecar lengths and survivor refcounts;
- epoch rollback truncates one aggregate watermark tuple, never a subset;
- survivor promotion copies a bottom-up graph into one self-contained root,
  and recycling never reuses a root identity;
- downstream crates receive decoded read-only views and builders, not raw
  words, sidecar indexes, stores, constructors, or mutable columns;
- semantic hashes traverse decoded logical nodes and content, never tags,
  indexes, vector capacities, addresses, or allocation order.

This is primarily a memory-bandwidth design. It is adopted only if final
typesetting-kernel benchmarks show a material improvement without a material
end-to-end regression. Otherwise Phase 6 revises the inline/sidecar split or
reverts the representation cleanly while retaining independently useful
`NodeListId` packing.

## 2. Measurement baseline

On `aarch64-apple-darwin` with Rust 1.93.0, the current layouts are:

| Type | Size |
| --- | ---: |
| `Node` | 72 bytes |
| `BoxNode` | 44 bytes |
| `UnsetNode` | 44 bytes |
| `Whatsit` | 48 bytes |
| `NodeListId` | 16 bytes |

The measurement build used `cargo build --release -p umber --features
node-stats` at commit `217878e1`. Relaxed process-local counters at
`NodeArena::append` add no `NodeArena` or `Universe` fields and are absent from
normal snapshots, rollback, replay, and hashes. Each node-producing
`benchmarks/plain-tex` input ran in a fresh process with committed Computer
Modern metrics and DVI output. `expand.tex` did not finish its 100,000
expansion-only iterations in five minutes, so its exact 56-node producing
suffix was measured separately after the same preamble.

| Workload | Appended nodes |
| --- | ---: |
| `dvi.tex` | 4,331,406 |
| `expand.tex` node-producing suffix | 56 |
| `paragraph-wide.tex` | 1,275,891 |
| `paragraph-narrow.tex` | 655,891 |
| `math.tex` | 6,040,698 |
| `math-nested.tex` | 4,161,034 |
| `pages.tex` | 1,611,063 |
| **Total** | **18,076,039** |

| Node kind | Count | Share | Proposed storage |
| --- | ---: | ---: | --- |
| char | 8,224,274 | 45.50% | inline |
| ligature | 9,502 | 0.05% | inline |
| kern | 2,130,236 | 11.78% | inline |
| glue | 1,608,403 | 8.90% | inline unless leader-bearing |
| penalty | 61,117 | 0.34% | inline |
| rule | 374,702 | 2.07% | sidecar |
| hlist | 2,857,357 | 15.81% | box sidecar |
| vlist | 561,392 | 3.11% | box sidecar |
| whatsit | 1,001 | 0.01% | sidecar |
| mark | 8,043 | 0.04% | sidecar |
| math on/off | 60,012 | 0.33% | inline |
| math noad | 1,960,000 | 10.84% | sidecar |
| fraction noad | 190,000 | 1.05% | sidecar |
| math style | 30,000 | 0.17% | inline |

No unset, discretionary, insertion, math-choice, math-list, nonscript, or
adjust nodes occurred in these fixed workloads; they remain fully supported.

### 2.1 Conservative storage model

The current stream occupies about 1,301.5 MB (`18,076,039 * 72`). An 8-byte
word for every append occupies 144.6 MB. Even the deliberately pessimistic
model in which every non-common-four node (33.48%) retains a full 72-byte
sidecar payload adds only about 435.8 MB, for about 580.4 MB total: a 55.4%
live-byte reduction before tighter SoA payloads or additional inline kinds.
At an extreme two-times retained-capacity factor for every rare sidecar, the
model is about 1,016 MB, still 21.9% below the current tightly-sized stream.

This is an upper-bound model, not a benchmark result. Real accounting in
Phase 6 must include word capacity, every sidecar column's capacity, survivor
roots, recycled buffers, allocator overhead, and peak promotion scratch. It
must report hlist/vlist and math costs separately: boxes are 18.92% and math
noad/fraction payloads are 11.89% of measured appends. Shared immutable
`GlueId`, `TokenListId`, font data, and child lists are not charged twice.

The model makes implementation credible, but does not prove speed. If sidecar
indirection erases the expected scan benefit, the design must change even when
memory falls.

## 3. `NodeWord`: exact eight-byte encoding

`NodeWord` is a private transparent wrapper around `u64`. A compile-time
assertion requires `size_of::<NodeWord>() == 8`. Bits 63..59 are a five-bit
tag and bits 58..0 are a 59-bit payload. Unused payload bits must be zero on
construction and are rejected by debug validation; raw words are not a stable
serialization format.

```text
63             59 58                                      0
+----------------+------------------------------------------+
| tag (5 bits)   | payload (59 bits)                        |
+----------------+------------------------------------------+
```

| Tag | Kind | Payload, low bits first |
| ---: | --- | --- |
| 0 | char | USV 21, `FontId` 32 |
| 1 | ligature | ch 8, left original 8, right original 8, `FontId` 32 |
| 2 | kern | signed `Scaled` bits 32, `KernKind` 2 |
| 3 | glue | `GlueId` 32, `GlueKind` 6; leaderless only |
| 4 | penalty | signed `i32` bits 32 |
| 5 | math-on | signed `Scaled` bits 32 |
| 6 | math-off | signed `Scaled` bits 32 |
| 7 | math-style | `MathStyle` 2 |
| 8 | nonscript | zero |
| 9 | hlist | box sidecar index 32 |
| 10 | vlist | box sidecar index 32 |
| 11 | unset | unset sidecar index 32 |
| 12 | rule | rule sidecar index 32 |
| 13 | leader glue | leader-glue sidecar index 32 |
| 14 | discretionary | disc sidecar index 32 |
| 15 | mark | mark sidecar index 32 |
| 16 | insertion | insertion sidecar index 32 |
| 17 | whatsit | whatsit sidecar index 32 |
| 18 | math noad | noad sidecar index 32 |
| 19 | fraction noad | fraction sidecar index 32 |
| 20 | math choice | choice sidecar index 32 |
| 21 | math list | math-list sidecar index 32 |
| 22 | adjust | adjust sidecar index 32 |
| 23..31 | reserved | invalid until a versioned in-memory migration assigns one |

The 32-bit sidecar index is intentionally stricter than the available 59-bit
payload. It matches Rust vector indexing limits already enforced by the arena,
keeps marks compact, and permits at most `u32::MAX` entries per kind. Appends
check word and selected-sidecar capacity before changing any length; capacity
exhaustion follows the existing explicit arena-overflow failure rather than
silently changing TeX semantics.

Capacity details:

- char stores every Unicode scalar (`0..=0x10ffff`, excluding surrogate
  values by constructor validation) and every 32-bit `FontId`; six payload
  bits remain unused;
- ligatures are restricted to the TFM byte-character domain for `ch` and both
  originals. A future shaped glyph form gets a sidecar tag; it does not
  truncate glyph ids into this layout. Three payload bits remain unused;
- signed dimensions and penalties preserve their exact two's-complement
  32-bit representation;
- all current kern, glue, and style values fit their listed discriminant bits;
  constructors use exhaustive mapping rather than enum-layout casts;
- a glue with a leader cannot use tag 3. Its sidecar owns the glue spec/kind
  and the complete leader payload, so there is no hidden parallel leader map.

## 4. Generation-tagged `NodeListId`

`NodeListId` is a private two-word runtime identity with a compile-time
sixteen-byte size assertion. Epoch handles wrap the common state-layer
`(namespace, generation, slot)` identity. The owning `NodeArena` keeps a dense
parallel table from allocation slot to compact `(start, len)` span, so a read
performs one bounds/tag comparison and one indexed span load. Raw span
accessors no longer exist for live epoch handles: resolving storage without
the owning arena would bypass liveness validation. Empty epoch lists use the
universal immutable built-in identity and span `(0, 0)`.

Survivor handles retain the previous self-contained packed word inside a
reserved identity namespace:

```text
survivor: 1 | root:20 | start:21 | len:22
```

Epoch span-table entries support starts `0..=u32::MAX` and lengths
`0..=2^31-1`.
Survivor spans support roots `0..=2^20-2`, starts `0..=2^21-1`, and lengths
`0..=2^22-1`. The all-ones word is reserved and is the exact `None` encoding
in the Env box-register bank; every other stored box word is a survivor handle.
Epoch handles never enter raw Env words: box assignment promotes them first.
Constructors check `start + len` in the owning storage and reject overflow
before minting a handle. A survivor root is folded into every
child handle during promotion, making each span self-describing without a
second root pointer. Root slot identities remain monotonic and are never
reused; only their storage buffers recycle.

The identity-table watermark and compact-storage watermark are one
`NodeArenaMark`. Rollback validates and truncates the identity suffix, advances
the non-restored generation before a discarded allocation slot can be reused,
then truncates words and all sidecars. Handles below the mark remain live;
discarded handles cannot revive after equal or covering span reuse. Cloning an
arena preserves inherited allocation tags and selects a fresh namespace for
post-fork allocations. Survivor identity, refcounting, promotion, and buffer
recycling are unchanged.

The reserved final root id is never allocated, even for a list that will not
enter a box register. This makes null encoding canonical throughout the state
layer and avoids the current `id + 1` overflow edge. Reaching that root limit
is an explicit survivor-arena capacity failure; it never aliases `None` or
falls back to an epoch handle.

`NodeListId` encoding is an in-memory implementation detail. Live handles do
not implement successful serialization. Format capture first replaces every
node child with a detached DTO-local arena/span reference; format restore
validates/remaps those references and mints fresh epoch identities through the
aggregate arena. Artifacts and semantic hashes encode referenced logical node
content, not runtime identity or allocation order.

## 5. Sidecar storage and ownership

Each node storage instance owns one word vector and per-kind sidecars. Tables
are structure-of-arrays where fields are independently useful in hot scans;
columns advance in lockstep and share one logical row count.

- boxes: width, height, depth, shift, display, glue-set numerator/denominator,
  glue sign/order, children;
- unsets: kind, dimensions, span count, stretch/order, shrink/order, children;
- rules: three optional dimensions;
- leader glues: spec, glue kind, leader kind, leader box/rule fields;
- discretionaries: kind, pre, post, replace;
- marks: class, token list;
- insertions: class, size, split-top-skip, split-max-depth, floating penalty,
  content;
- whatsits: kind-specific detached payload columns (including owned bytes or
  strings where the current logical value owns them);
- noads: noad kind plus nucleus/subscript/superscript field columns;
- fractions: numerator, denominator, thickness, delimiters;
- choices: four lists; math lists: display and content; adjusts: content.

Small nested sum types such as math fields may remain packed value columns if
splitting them would increase bytes or branch count. “SoA” is not permission
to create a global side table: every field is owned by the same `NodeStorage`
and its row is addressed only through a validated word.

### 5.1 Epoch storage and rollback

`NodeArenaMark` is one opaque aggregate value containing the word length and
every sidecar column length. Taking it is O(1), with a constant number of
integers independent of arena contents. `Stores::checkpoint` captures that
mark with Env, content, World, page, and input state. Rollback validates all
lengths first, then truncates every column and the word stream as one private
operation. No public or downstream method can mark, truncate, append raw
words, append a sidecar row, or restore a subset.

Builder finish is transactional with respect to logical state: it validates
all child handles and capacities, reserves required columns, encodes sidecar
rows, then publishes words. An allocation failure may abort the process as it
does today, but no recoverable error can leave a word naming an unpublished
row. Bottom-up validation resolves child handles through the aggregate arena
and requires epoch children to end before the new parent span.

Vector capacity retained after truncation is allocator state only. It cannot
affect decoded nodes, identities, hashes, or replay and is reported separately
in memory benchmarks. Process-local measurement counters remain feature-gated
and outside `Universe` exactly as in Phase 1.

### 5.2 Survivor roots and recycling

Every survivor root owns a complete `NodeStorage`: words and all sidecars.
Promotion iteratively decodes the mixed epoch/survivor DAG, memoizes exact
source spans, appends logical nodes into the destination storage, and rewrites
all child handles in destination sidecar rows to the new monotonic root id.
There are no cross-root sidecar indexes. This keeps a promoted root
self-contained and makes recursive ownership inspection independent of the
source arenas.

Root slots and refcounts remain in `SurvivorArena` under aggregate Env-journal
ownership. Live box registers and retained undo records own references;
replacement, group exit, rollback, and shipout release them through the same
barriered paths as today. At refcount zero, all destination vectors are
cleared and move together into a recycled `NodeStorage` pool. Recycling may
reuse capacity but never the root slot or a packed handle. The pool and its
reuse counters are derived allocator state: cloning may copy them and rollback
need not restore their exact capacity/order, because they cannot affect
meaning, liveness, ids, hashes, or output. Tests must prove that claim by
replay/hash equality with different recycling histories.

## 6. Read and mutation boundaries

`Universe` remains the only public live-state owner. The node API exposes:

- a builder accepting logical `Node` values during migration and eventually
  typed `push_char`, `push_kern`, `push_box`, and equivalent methods;
- `NodeList<'a>`/`NodeIter<'a>` read-only views over a live opaque handle;
- a `NodeRef<'a>` decoded view with `kind()` and typed accessors such as
  `as_char`, `as_box`, `kern`, `glue`, and `child_lists`;
- narrow immutable traits used by pure `tex-typeset` kernels.

No API returns `&[NodeWord]`, a sidecar slice/index, `&mut` storage, an
unchecked decoder, or a raw handle constructor. A compatibility iterator may
yield owned/borrowed logical `Node` views while consumers migrate, but it is
not a second mutable representation. Debugging and `\showlists` use the same
accessors as production.

All node mutation is builder-then-freeze. Algorithms that currently rewrite a
cloned `Node` list build a new list; they never mutate a frozen word or sidecar
row. Pure typesetting receives immutable views and plain copied parameters.
Execution performs stateful list publication, survivor transfer, and box
register writes only through `Universe`. Shipout lowers through views into
detached `tex-out` artifacts and cannot retain a live sidecar reference.

## 7. Semantic hashing and replay

Hashing dispatches through `NodeRef` and hashes the same logical discriminant
and fields as the current `Node` implementation. Every content handle is
followed to semantic content: child node lists, glue specs, token lists,
fonts, and whatsit payloads. Sidecar indexes, `NodeListId` raw bits, root ids,
capacities, recycled-buffer order, and column addresses are excluded.

The aggregate node hash cursor uses the word-stream watermark only to locate
the newly appended logical slice; it does not hash raw words. Rollback clears
or rebuilds any derived fingerprint cache exactly as today. Tests compare
checkpoint hashes across append/rollback/reappend, different sidecar allocation
orders, promotion, release, and recycled-capacity reuse. Shadow mode must use
the public/aggregate logical view and may not enable raw production mutation.

## 8. Consumer migration

Migration is coherent by boundary, not a long-lived dual representation:

1. Pack `NodeListId`, update Env codecs, and prove liveness/capacity without
   changing node storage.
2. Introduce private `NodeWord`/sidecars, aggregate watermarking, builders,
   logical views, hashing, survivor promotion, and recycling in `tex-state`.
   The old `Vec<Node>` storage is removed in this phase.
3. Migrate pure `tex-typeset` scans first (packing, vertical breaking,
   line-width accumulation, line breaking), then execution construction and
   list surgery, diagnostics, page building, survivor flows, and shipout. A
   temporary logical compatibility iterator is removed after the last
   exhaustive `Node` match outside the state layer disappears.
4. Add same-font run width accumulation only after scalar accessor scans are
   correct and benchmarked.

Every phase preserves exact fixture and DVI output. No consumer may cache a
`NodeRef` or sidecar reference across a mutable `Universe` call.

## 9. Width-array and vectorization plan

Loaded immutable font metrics gain a dense width array indexed by TFM byte
character. A typeset scan identifies a contiguous run of inline char words
with the same `FontId`, validates the font once, and gathers widths while
accumulating in TeX's exact `Scaled` integer order. Scalar unrolled and
portable-vector/SIMD implementations must produce the identical sum and
overflow behavior; runtime selection is a pure cache keyed by target features,
not timeline state.

The first benchmark compares ordinary accessor iteration, scalar same-font
runs, and vectorized runs. SIMD is retained only where it beats the scalar run
on representative short and long hlists. Ligatures, missing-character
diagnostics, non-byte modern glyphs, font switches, and non-char nodes end a
run and use the ordinary accessor path. Glue, box dimensions, italic
corrections, and TeX glue-set rounding keep their existing exact order.

## 10. Phases and exit gates

### Phase 1 — measurement (complete)

Record compile-time layout assertions and the fixed-corpus histogram above.
Exit: instrumentation is nonsemantic/process-local and the full distribution,
including math intermediates, is durable.

### Phase 2 — design (complete)

Exit: exact encodings/capacities, conservative sidecar cost, aggregate
ownership, mutation boundary, semantic hashing, migration, width plan, and
validation matrix are reviewed against `core_state.md`. The design must remain
conditional on measured performance.

### Phase 3 — packed list handles (complete)

Exit: `NodeListId` is compile-time eight bytes; all boundary/capacity and
optional-box encodings round-trip; stale epoch/survivor handles remain
unforgeable; normal/shadow replay and semantic hashes pass.

The implementation stores the packed word directly, reserves `u64::MAX` as
the canonical box-register `None`, and keeps construction crate-private. The
packing also reduces the measured layouts of `Node` from 72 to 64 bytes and
`BoxNode`/`UnsetNode` from 44 to 40 bytes before the Phase 4 word-stream work.
Dense and sparse Env banks use the codec's semantic default word, so void box
registers remain allocation-free while a live all-zero epoch handle is a
distinct `Some` value. Arena lookup, rollback, promotion, and survivor
recycling continue to validate logical ownership rather than raw bits.

### Phase 4 — words and sidecars

Exit: `NodeWord` is compile-time eight bytes; every logical variant
round-trips; aggregate rollback truncates every column; promotion produces a
self-contained root; release/recycling cannot revive stale handles; hashing is
allocation-independent; no old `Vec<Node>` store remains.

Implementation status: the compact `NodeStorage` word stream and per-kind
sidecars are canonical for both epoch and survivor roots. The temporary
decoded `Node` mirror has been removed. Epoch and survivor reads now return
opaque `NodeList`/`NodeIter`/`NodeRef` logical views, while owned `Node` values
remain construction and test/debug values only and are never retained as a
second arena representation.

### Phase 5 — consumer migration

Exit: typeset, exec, page builder, diagnostics, survivor transfer, and shipout
use logical accessors; downstream raw/exhaustive storage matches are gone;
temporary compatibility APIs are removed; fixture and DVI corpuses are
byte-identical.

Implementation status: compact logical views are the state and typesetting
read boundary, the compatibility mirror is gone, and packing, line-width,
page, alignment, and execution scans consume those views. Algorithms that
genuinely produce rewritten lists may materialize owned builder scratch; this
scratch is never checkpoint state and is frozen back through the aggregate
arena API.

### Phase 6 — widths, measurement, adoption

Measure scalar accessors before adding SIMD, then same-font scalar and SIMD
variants. Report per-workload medians and intervals, instructions, cache
misses where available, logical/retained/peak bytes, sidecar distribution,
promotion/recycling cost, and full end-to-end time. Compare to the pinned
Phase 1 commit with identical toolchain, inputs, fonts, output, and warmup.

Exit: a material typesetting-kernel improvement on typesetting-heavy Plain TeX
workloads, no material end-to-end regression, exact output parity, and a
credible memory reduction after all sidecars/capacities are charged. There is
no fixed percentage gate: noise and workload shape are documented, and the
agent must judge whether further optimization is realistic. A large slowdown
is never accepted merely for memory reduction. If evidence is weak or
negative, revise inline tags/accessors/sidecars or revert cleanly and record
the decision.

### Phase 6 adoption results

Phase 6 is **adopted**. `FontMetrics` derives one immutable 256-entry `Scaled`
width table at load time. An opaque lazy iterator walks contiguous same-font
byte-character words without exposing words or sidecars; hpack validates the
font once per run and reads both width and character tables directly. Unicode
outside the TFM byte domain, ligatures, font changes, and non-character nodes
end a run. Saturating additions remain in source order. The tables are derived
immutable font content, with no rollback mark, mutable cache, semantic hash
input, or hidden incremental state.

The controlled comparison used pre-epic commit `217878e1` and final Phase 6
code on the same aarch64 Apple host with Rust 1.93.0, clean release builds,
identical synthetic metrics, and Criterion warmup plus 100 samples. Listed
intervals are Criterion 95% confidence intervals and are disjoint.

| hpack kernel | Before | After | Change |
| --- | ---: | ---: | ---: |
| same font, 64 chars | 125.41 ns [123.73, 128.26] | 79.547 ns [79.449, 79.674] | -36.57% |
| same font, 4,096 chars | 7.8342 us [7.5418, 8.4064] | 4.5300 us [4.5130, 4.5517] | -42.18% |
| mixed/interrupted, 4,096 nodes | 8.5760 us [8.2778, 9.0159] | 6.8197 us [6.8017, 6.8390] | -20.47% |

End-to-end runs rebuilt each revision outside timing and used one warmup plus
ten runs with identical committed input, Computer Modern TFM files, and DVI
validation. These ranges are observed minima/maxima.

| Plain TeX workload | Before | After | Change |
| --- | ---: | ---: | ---: |
| paragraph-wide | 0.212 s [0.210, 0.218] | 0.199 s [0.197, 0.201] | -6.13% |
| paragraph-narrow | 0.112 s [0.109, 0.125] | 0.102 s [0.101, 0.104] | -8.93% |
| pages | 0.414 s [0.406, 0.448] | 0.425 s [0.403, 0.444] | +2.66% (ranges overlap) |
| dvi | 0.545 s [0.531, 0.565] | 0.530 s [0.526, 0.535] | -2.75% (ranges overlap) |

No end-to-end workload regressed by 5%. The expansion-only workload was
excluded after a trial took roughly three minutes per run: its separately
measured 56-node suffix cannot exercise this kernel meaningfully.

For peak process memory, `/usr/bin/time -l` around `paragraph-wide` reported
maximum RSS falling from 175,194,112 to 96,141,312 bytes (-45.12%) and peak
footprint from 163,202,368 to 88,212,800 bytes (-45.95%). This includes the
whole process and allocator-retained memory. The conservative corpus-wide
logical model in section 2.1 remains a 55.4% reduction even when every rare
node is charged a full old `Node`; actual SoA sidecars are smaller. Width
tables cost a fixed 1,024 bytes per loaded font and are included in RSS.

### 10.1 Actual arena and survivor accounting

The follow-up Phase 6 audit measures the canonical storage rather than
inferring it from process RSS. `node-stats` now computes an on-demand report
over the actual epoch storage, every live survivor root, and every cleared
recycled buffer. Each vector reports logical length and allocator capacity in
elements and bytes. Owned whatsit strings and byte payloads are separate
rows; shared glue, token, font, and child-list storage is not charged again.
The report excludes vector headers, allocator metadata, process code/stacks,
and shared stores, so it is intentionally distinct from RSS. Cleared recycled
vectors have zero logical bytes but retain capacity.

Process-local relaxed counters separately record fresh and recycled promotion
time, release-to-recycling time, largest canonical `NodeStorage`, and peak
promotion scratch. Scratch charges the owned `Vec<Node>`, pending-index vector,
and hash-map key/value payload capacity; hash-map control bytes and allocator
metadata are excluded. None of these counters or reports is a `Universe`
field, rollback mark, snapshot, hash input, or replay input. Normal builds do
not compile them. A feature test proves that reading the report leaves the
semantic state hash unchanged while stale-root, refcount, and recycled-buffer
tests exercise the same production paths.

The reproducible command was `MEASURE_CLEAN=1 MEASURE_RUNS=5
scripts/measure-node-arena.sh` at this commit on the same aarch64 Apple host
and Rust 1.93.0. It performs a clean release measurement build, stages the
committed inputs and Computer Modern metrics into a fresh directory for every
sample, verifies byte-identical DVI hashes across samples, and runs each input
in a fresh process. The byte totals were deterministic across all five runs.
RSS is the observed five-run range.

| Workload | End logical | End retained payload | Largest storage logical/retained | Promotion scratch logical/retained | RSS range |
| --- | ---: | ---: | ---: | ---: | ---: |
| paragraph-wide | 14,730,896 | 16,374,456 | 11,054,968 / 12,484,608 | 245,680 / 261,840 | 93,667,328–95,780,864 |
| pages | 29,651,391 | 35,589,920 | 17,583,783 / 22,347,776 | 466,856 / 569,536 | 100,319,232–103,350,272 |
| math | 24,058 | 40,272 | 14,453 / 25,408 | 16,232 / 20,464 | 110,673,920–113,229,824 |
| math-nested | 42,033 | 66,960 | 25,238 / 40,064 | 24,520 / 44,800 | 74,137,600–81,379,328 |

The math inputs overwrite one survivor root 20,000 and 10,000 times, so their
small end state is expected rather than missing accounting: only the final
live root and one cleared recycled buffer remain. Their peak construction
shape is reported separately above. Against the Phase 1 append-stream model,
paragraph-wide falls from 91,864,152 modeled bytes to 14,730,896 measured
logical bytes (-83.96%), and pages from 115,996,536 to 29,651,391 (-74.44%).
This comparison is conservative because the measured totals also include
survivor copies omitted by the Phase 1 stream model. It corroborates, and is
better than, the prior conservative -55.4% estimate. The cumulative math
append model is not a live-memory comparator because rollback and survivor
replacement deliberately discard each iteration.

Five independent process samples contain many operations per sample. Fresh
promotion averaged 82.67–84.53 us/call for paragraph-wide (112 calls/sample)
and 37.58–38.23 us/call for pages (768 calls/sample). Recycled promotion was
12.61–12.68 us/call for math (19,999/sample) and 21.40–22.16 us/call for
math-nested (9,999/sample). Release to the recycle pool was 65.7–68.3 ns/call
and 68.5–73.0 ns/call respectively. These are instrumented wall-clock costs,
not semantic state. Together with the unchanged Phase 6 kernel/end-to-end
results and the new budget rerun, they leave the adoption decision unchanged.

The peak canonical-storage column distribution follows. Each cell is
`logical length / retained capacity`; zero-length math rows with capacity show
real buffers retained after epoch truncation.

| Column | Elem B | paragraph-wide | pages | math | math-nested |
| --- | ---: | ---: | ---: | ---: | ---: |
| `words` | 8 | 1,275,891/1,417,216 | 1,611,063/2,162,688 | 698/1,024 | 1,034/2,048 |
| `boxes.width` | 4 | 24,224/32,768 | 126,453/131,072 | 239/256 | 458/512 |
| `boxes.height` | 4 | 24,224/32,768 | 126,453/131,072 | 239/256 | 458/512 |
| `boxes.depth` | 4 | 24,224/32,768 | 126,453/131,072 | 239/256 | 458/512 |
| `boxes.shift` | 4 | 24,224/32,768 | 126,453/131,072 | 239/256 | 458/512 |
| `boxes.display` | 1 | 24,224/32,768 | 126,453/131,072 | 239/256 | 458/512 |
| `boxes.glue_set` | 8 | 24,224/32,768 | 126,453/131,072 | 239/256 | 458/512 |
| `boxes.glue_sign` | 1 | 24,224/32,768 | 126,453/131,072 | 239/256 | 458/512 |
| `boxes.glue_order` | 1 | 24,224/32,768 | 126,453/131,072 | 239/256 | 458/512 |
| `boxes.children` | 8 | 24,224/32,768 | 126,453/131,072 | 239/256 | 458/512 |
| `unsets.kind` | 1 | 0/0 | 0/0 | 0/0 | 0/0 |
| `unsets.width` | 4 | 0/0 | 0/0 | 0/0 | 0/0 |
| `unsets.height` | 4 | 0/0 | 0/0 | 0/0 | 0/0 |
| `unsets.depth` | 4 | 0/0 | 0/0 | 0/0 | 0/0 |
| `unsets.span_count` | 2 | 0/0 | 0/0 | 0/0 | 0/0 |
| `unsets.stretch` | 4 | 0/0 | 0/0 | 0/0 | 0/0 |
| `unsets.stretch_order` | 1 | 0/0 | 0/0 | 0/0 | 0/0 |
| `unsets.shrink` | 4 | 0/0 | 0/0 | 0/0 | 0/0 |
| `unsets.shrink_order` | 1 | 0/0 | 0/0 | 0/0 | 0/0 |
| `unsets.children` | 8 | 0/0 | 0/0 | 0/0 | 0/0 |
| `rules` | 24 | 0/0 | 8,545/16,384 | 21/32 | 39/64 |
| `leaders` | 56 | 0/0 | 0/0 | 0/0 | 0/0 |
| `discs` | 32 | 0/0 | 0/0 | 0/0 | 0/0 |
| `marks` | 8 | 0/0 | 8,043/8,192 | 0/0 | 0/0 |
| `insertions.class` | 2 | 0/0 | 0/0 | 0/0 | 0/0 |
| `insertions.size` | 4 | 0/0 | 0/0 | 0/0 | 0/0 |
| `insertions.split_top_skip` | 4 | 0/0 | 0/0 | 0/0 | 0/0 |
| `insertions.split_max_depth` | 4 | 0/0 | 0/0 | 0/0 | 0/0 |
| `insertions.floating_penalty` | 4 | 0/0 | 0/0 | 0/0 | 0/0 |
| `insertions.content` | 8 | 0/0 | 0/0 | 0/0 | 0/0 |
| `whatsits` | 48 | 0/0 | 0/0 | 0/0 | 0/0 |
| `noads.kind` | 8 | 0/0 | 0/0 | 0/128 | 0/64 |
| `noads.nucleus` | 16 | 0/0 | 0/0 | 0/128 | 0/64 |
| `noads.subscript` | 16 | 0/0 | 0/0 | 0/128 | 0/64 |
| `noads.superscript` | 16 | 0/0 | 0/0 | 0/128 | 0/64 |
| `fractions` | 40 | 0/0 | 0/0 | 0/8 | 0/16 |
| `choices` | 32 | 0/0 | 0/0 | 0/0 | 0/0 |
| `math_lists` | 16 | 0/0 | 0/0 | 0/0 | 0/0 |
| `adjusts` | 8 | 0/0 | 0/0 | 0/0 | 0/0 |
| `whatsits.owned_strings` | 1 | 0/0 | 0/0 | 0/0 | 0/0 |
| `whatsits.owned_payloads` | 1 | 0/0 | 0/0 | 0/0 | 0/0 |

Append histograms provide the representative kind distribution absent from
the peak-after-truncation math rows: paragraph-wide has 24,000 hlists and 224
vlists; pages has 124,681 hlists, 1,772 vlists, 8,545 rules, and 8,043 marks;
math has 1,340,201 hlists, 240,038 vlists, 1,320,000 noads, 100,000 fractions,
and 20,000 styles; math-nested has 1,250,375 hlists, 270,083 vlists, 640,000
noads, 90,000 fractions, and 10,000 styles. These counts are cumulative work,
while the table is retained storage; keeping the two concepts separate avoids
double-counting discarded iterations.

`benchmarks/tex-exec/benches/widths.rs` is the kernel suite.
`scripts/check-node-width-budget.sh`, available through
`CHECK_BENCH=1 scripts/check.sh`, enforces committed means with the 10%
cross-run tolerance specified by `umber2-93q`; adoption used the stricter 5%
end-to-end regression ceiling.

## 11. Validation matrix

| Area | Required cases |
| --- | --- |
| Layout | compile-time 16-byte handle and 8-byte node-word assertions; every tag; reserved tags rejected; signed extrema; Unicode scalar validation; TFM ligature bounds |
| Handle identity | epoch generation/namespace/slot validation; equal and covering reuse; retained prefix; fork ancestry; survivor zero/maxima; start+len overflow; empty lists; max root; optional box-register null; raw constructors inaccessible downstream |
| Sidecars | every kind; zero/max indexes; leader glue; owned whatsit payloads; no word published without a row; column lengths agree |
| Bottom-up graph | epoch children, mixed survivor children, shared spans, deep graphs, cycles/forward references rejected |
| Rollback | atomic identity/storage mark; truncate all columns; arbitrary rollback/reappend never revives stale ids; retained capacities distinguished from live bytes; shipout release |
| Survivors | promotion, root folding, refcounts, journal-held owners, group exit, root non-reuse, buffer recycling, nested boxes/math/leader payloads |
| Access boundary | compile-fail probes for raw words, sidecars, constructors, partial marks, mutable views; shadow remains production-like |
| Hash/replay | equal logical graphs with different sidecar/root/recycling histories hash equally; changed fields differ; rollback convergence; deep iterative traversal |
| Kernels | hpack/vpack/vtop, vertical breaking, line breaking, diagnostics, page builder, insertion/mark handling, math lowering |
| Width runs | empty/short/long runs, font switch, missing char, ligature, overflow behavior, scalar/SIMD exact equality |
| Output | workspace fixtures plus Story/Gentle and fixed Plain TeX DVI corpuses remain byte-identical |
| Performance | each fixed workload; typesetting kernel and end-to-end time; logical/retained/peak bytes; box/math sidecars; promotion/recycling |

Use affected crate tests during each phase and `scripts/check-and-test.sh` for
the full workspace test, format, and clippy gate. Long-running parity corpuses
remain in their existing scripts rather than ordinary unit tests.
