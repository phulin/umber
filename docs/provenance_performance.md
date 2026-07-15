# Provenance Performance Notes

Status: Phase 6 adoption measurements for compact tagged source provenance,
2026-07-10.

## Reproduction and comparison method

Run the current matrix with Criterion's 100-sample default:

```bash
cargo bench --manifest-path benchmarks/tex-state/Cargo.toml --bench state_budgets -- provenance
```

Set `UMBER_PROVENANCE_REPORT=1` and select `provenance_memory` to print the
deterministic counters summarized below. Timed source rows do not sample
statistics per token; memory is measured in a separate untimed pass. The Phase
1 comparator is commit `6022b9fa`, rebuilt with the same Rust toolchain and
dependency resolution as the current tree. Retained historical executables are
not valid comparators: rebuilding Phase 1 changed the macro row from about
0.74 ms to about 1.17 ms on this machine.

Each time interval is Criterion's 95% confidence interval from 100 samples.
The reported delta compares medians. The adoption ceiling is a statistically
significant regression greater than 5% in any primary row.

## Throughput adoption matrix

| Workload | Phase 1 time | Adopted time | Median delta | Adopted throughput |
| --- | ---: | ---: | ---: | ---: |
| ASCII source, 23,552 tokens | 350.39-354.88 us | 332.89-333.55 us | -5.43% | 70.68 Mtok/s |
| Mixed UTF-8, 16,384 tokens | 251.03-253.65 us | 245.12-246.49 us | -2.56% | 66.65 Mtok/s |
| One 65,536-scalar line, 65,537 tokens | 710.73-719.20 us | 599.38-607.30 us | -15.63% | 108.66 Mtok/s |
| Control-sequence-heavy, 4,096 tokens | 535.13-537.56 us | 547.23-563.62 us | +3.03% | 7.41 Mtok/s |
| Macro body replay, 32,768 delivered tokens / 2,048 calls | 1.1618-1.1750 ms | 1.1462-1.1691 ms | -1.03% | 28.34 Mtok/s delivered |
| Scanner-heavy `\number`, 5,120 outputs / 1,024 runs | 553.10-592.60 us | 504.21-510.97 us | improvement | 10.09 Mtok/s output |
| Generated `\romannumeral`, 15,360 outputs / 1,024 runs | 840.10-867.10 us | 774.01-811.14 us | improvement | 19.47 Mtok/s output |

Control-sequence-heavy input is the only slower median. Its confidence interval
overlaps the Phase 1 neighborhood and its 3.03% median cost is below the 5%
ceiling. No primary row blocks adoption. Ordinary-source throughput was recovered
by carrying an opaque `RegisteredSource` capability in the live input frame:
direct origins are encoded without repeated source-map lookup, while wide and
exhausted cases still use the aggregate validated fallback. Scanner range proofs
are reconstructed only when a scanner asks for one instead of being written on
every source delivery.

### Registered-span follow-up

On 2026-07-12, the control-sequence row was remeasured before and after routing
registered input frames directly from `RegisteredSource::span` to provenance
allocation. Both executables used the same checkout dependencies and Rust
toolchain, and each measurement used Criterion's 100-sample default. The
baseline executable was built from commit `e278af20` in a detached worktree;
the updated executable was measured immediately afterward on the same host.

| Workload | Before time | After time | Median delta | After throughput |
| --- | ---: | ---: | ---: | ---: |
| Control-sequence-heavy, 4,096 tokens | 831.45-843.65 us | 813.44-839.44 us | -1.41% | 4.97 Mtok/s |

Inspection confirms that the registered-frame control-sequence path performs
zero `SourceMap` region lookups: it validates offsets through the frame's
`RegisteredSource` capability and appends the resulting `SourceSpan` directly.
Unregistered or invalid ranges retain the aggregate-validated fallback. The
remaining cost includes one exact-range arena record per control sequence.

### Edit-stable layout cursor contract

Phase 2 of edit-stable source coordinates preserves the measured construction
path above. `LayoutCursor` work occurs once while freezing an editor layout and
once per physical-line refill; it installs a `RegisteredSource` plus one
fragment-relative line base on the source frame. The ordinary scalar path still
calls `registration.direct_origin(start, end)` directly, with no layout lookup,
allocation, provenance-store write, or new conditional per token. Exact
control-sequence and transformed-input spans likewise validate through the
already-selected registration.

Lexer coverage exercises direct ASCII/UTF-8 delivery, `^^` lookahead and rewind,
piece transitions, synthetic endline anchors, and summary restoration with a
cursor reinstalled. The existing throughput matrix remains the adoption gate;
the 2026-07-14 construction-parity rerun completed the ASCII, mixed UTF-8,
long-line, control-sequence, macro replay, scanner, and generated-value rows.
The ordinary source rows remained in the same sub-millisecond class and the
cursor design adds no work inside their measured per-token loop. Criterion's
stored-baseline comparisons were noisy in both directions, so this is recorded
as structural parity plus a complete-matrix execution, not as a claimed
throughput improvement.

### Edit-stable retention and read-path follow-up

Phase 4 makes the session fragment table the only owner of editor source bytes.
Accepted engine generations retain metadata-only fragment snapshots, so a
checkpoint can still classify an old coordinate without pinning its backing
text. Once a fragment is absent from the current layout and no retained
checkpoint predates its removal revision, its bytes are released permanently;
its reserved range, byte length, mint revision, and removal revision remain.
Resolution therefore returns typed `Deleted` after pruning rather than aliasing
the position or degrading merely because its text allocation was reclaimed.

Retention metrics now charge the fragment table, retained fragment bytes,
piece table, document-start table, editor path, and lazily built line index as
`diagnostic_bytes`. Metadata-only fragment tables installed in engine roots are
charged with those roots. The checkpoint budget uses the sum, while output
bytes remain reported separately.

Accepted-output metrics are the point-in-time values captured during accept,
before a cold source query can allocate the line index. Native session
telemetry and the WASM `retentionMetrics` getter are live views: after a query
they refresh `diagnostic_bytes` and the checkpoint budget overage from the
accepted layout, so the lazy allocation is charged without making acceptance
or semantic snapshots cache-dependent.

The long-session tests exercise 64 alternating leading insert/deletes, 128
successive edits to one line, and 32 separated line replacements. Fully
replaced fragment bytes are reclaimed after both convergent and nonconvergent
advances. Alternating edits finish with only the current document bytes live;
the separated replacements grow the piece table by no more than two pieces per
edit. The direct-position capacity projection remains about 10.1 million
positions for 100,000 edits to a typical 100-byte line, 0.47% of the 2^31
boundary. Boundary tests prove the degradation chain is direct position, exact
arena `SourceSpan`, then `OriginId::UNKNOWN` for an invalid span.

The dedicated Criterion fixture keeps the 4,096-piece `LayoutCursor` and cold
resolution rows, then scales warm current and typed-deleted resolution across
64, 256, 1,024, 4,096, and 16,384 repeated views of one fragment. The deleted
origin lies in a gap in that same fragment, so it exercises indexed offset
rejection rather than the absent-fragment fast path. The cold row includes
lazy line-index construction; warm current reuses it. Run it with:

```bash
cargo bench --manifest-path benchmarks/tex-state/Cargo.toml --bench state_budgets -- edit_stable_source_coordinates
```

`EditorLayout` now builds a static two-dimensional index per fragment. A
start-offset binary search selects a persistent prefix root; an end-offset
range-min query returns the earliest document-order covering piece. Index
construction and retained storage are O(Σ `v_f log v_f`) for per-fragment
view counts `v_f`; current/deleted lookup is O(log fragments + log `v_f`).
The minimum document-order value preserves repeated/overlapping-view and
zero-width-anchor semantics. Accepted engine snapshots remain O(1): the
immutable layout and any operational line index already present are shared
rather than rebuilt by snapshot capture, while constructing the index later
does not alter semantic snapshot state.

The scaling table below is a 20-sample calibration run on the adoption host
(one-second warmup and two-second measurement); the reproduction command
above retains Criterion's 100-sample default.

| Repeated pieces | Warm final-piece current | Indexed in-fragment deleted |
| ---: | ---: | ---: |
| 64 | 59.630-59.681 ns | 25.463-25.484 ns |
| 256 | 64.253-64.768 ns | 37.339-37.446 ns |
| 1,024 | 71.481-71.842 ns | 76.218-76.759 ns |
| 4,096 | 86.222-87.710 ns | 195.29-196.15 ns |
| 16,384 | 124.71-130.00 ns | 657.38-664.20 ns |

The final-piece row grows by about 2.1x while the repeated-view count grows
256x. Deleted lookup remains sub-microsecond at 16,384 views and follows the
same bounded binary-search/range-min path; neither operation visits preceding
document-order pieces.

The one-time 4,096-piece rows from the same calibration are recorded below.

| Operation | Time | Effective piece rate |
| --- | ---: | ---: |
| `LayoutCursor` construction, 4,096 pieces | 56.700-58.581 us | 69.92-72.24 Mpiece/s |
| Resolve final-piece origin, cold line index | 60.215-72.068 us | 56.84-68.02 Mpiece/s |

An epoch rebase is not adopted. Bytes already remain bounded independently of
metadata, observed piece growth meets the ≤2-per-edit design bound, 100,000
typical edits leave more than 200x direct-position headroom, and the measured
construction/read costs do not justify remapping every retained coordinate.
The reserved rebase design remains available if future million-edit sessions
or measured resolver latency cross a host-facing budget.

### Accepted-edit fragment metadata scaling

The post-review accepted-edit fixture measures one fragment append plus the
immutable engine-generation metadata snapshot after geometrically increasing
inherited fragment counts. Teardown and fixture construction are outside the
timed interval. Set `UMBER_ACCEPTED_EDIT_REPORT=1` to emit allocation counts:

```bash
UMBER_ACCEPTED_EDIT_REPORT=1 cargo bench \
  --manifest-path benchmarks/tex-state/Cargo.toml \
  --bench accepted_edit_scaling
```

| Inherited fragments | Append + snapshot time | Allocations |
| ---: | ---: | ---: |
| 128 | 456.22-473.24 ns | 13 |
| 512 | 531.83-579.12 ns | 15 |
| 2,048 | 584.74-626.62 ns | 17 |
| 8,192 | 650.77-707.65 ns | 19 |

Each 4x increase adds two persistent-tree levels and exactly two allocations;
time grows from about 0.46 to 0.68 microseconds across 64x more metadata.
`FragmentStore` now separates the mutable session byte owner from a persistent
indexed metadata tree. Appends path-copy O(log fragments) nodes, metadata-only
generation snapshots clone one root in O(1), and pruning marks retained byte
entries by layout generation without cloning metadata or allocating a
fragment-count bitmap. A pointer-identity regression test also proves a
snapshot retains its old length and no source bytes after later owner appends.

This measurement deliberately excludes the `EditorLayout` rebuild, which now
costs O(Σ `v_f log v_f`) for the fragment/offset index in addition to the flat
O(pieces) arrays. It remains a separate layout cost rather than being hidden in
the fragment-table claim.

## Incremental memory

Logical bytes include live origin records, origin-list spans and entries, source
regions, and generated-backing metadata. Retained bytes use vector capacities.
World bytes and the `Arc<[u8]>` already shared with a memory input adapter are
not charged twice; a duplicate allocation would be charged in full. There is no
persistent line-index cache, so cache growth is zero and repeated rendering
recomputes physical line starts.

| Workload | Tokens / direct | Records / list spans / entries | Regions / backing metadata | Logical bytes | Retained / peak bytes | Phase 1 logical bytes | Reduction |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| ASCII | 23,552 / 23,040 | 1,024 / 0 / 0 | 1 / 1 | 32,848 | 33,088 | 770,048 | 95.73% |
| Mixed UTF-8 | 16,384 / 15,872 | 1,024 / 0 / 0 | 1 / 1 | 32,848 | 33,088 | 540,672 | 93.93% |
| One long line | 65,537 / 65,536 | 2 / 0 / 0 | 1 / 1 | 144 | 448 | 2,097,184 | 99.99% |
| Control sequences | 4,096 / 0 | 4,608 / 0 / 0 | 1 / 1 | 147,536 | 262,464 | 147,456 | -0.05% |

The source-heavy ASCII and UTF-8 rows exceed the 80% target. Control words need
exact arena ranges and therefore do not benefit from point encoding; the 80-byte
increase is the one source region plus shared-backing metadata, not duplicated
input bytes.

Macro-heavy growth remains 2,048 records (65,536 logical bytes), one invocation
record per call and no per-body-token write. Scanner-heavy growth is 1,024
records, 1,024 list spans, and 5,120 entries (61,440 bytes). Generated-value
growth is 1,024 records, 1,024 spans, and 15,360 entries (102,400 bytes).

Rollback/reuse reaches 102,480 logical and 155,808 retained bytes before
rollback. After rollback, logical growth is zero and retained capacity remains
155,808 bytes for reuse; a second run reuses that capacity. Source bytes remain
shared and are never included in those figures.

### Rendered-source retention follow-up

On 2026-07-14, rendered-source queries added a four-byte `OriginId` column
aligned with compact node words and one additional four-byte origin per source
character consumed by a ligature. Non-character rows contain the unknown id,
which keeps copying, truncation, promotion, and same-font character runs as
contiguous column operations rather than introducing a pointer-bearing map.
The owned `Node` layout remains 88 bytes. Origins are excluded from owned and
borrowed node equality, semantic hashes, format images, artifact bytes, and
artifact content identity.

Shipout retains a separate origin sidecar only for accepted in-process page
artifacts. Its logical payload is four bytes per renderable source character;
retention metrics additionally charge the outer `Arc` address table. The
source resolver and page positioning work run only when the host makes an
explicit query. The first query of a page retains only its compact event-prefix
and origin vectors plus the cache's page-slot table; live retention telemetry
charges those exact capacities to accepted `output_bytes`. A current-document
resolution can independently build the layout line-start index, whose retained
allocation remains in checkpoint-owned `diagnostic_bytes` and protected budget
overage. The accepted output keeps the point-in-time values captured before
either query cache exists.

The native retention regression checks the cold-query split exactly: the page
map's measured retained bytes equal the live `output_bytes` increase, while the
protected overage changes only by the line-index diagnostic allocation. On
2026-07-15, `scripts/check-snapshot-budgets.sh` continued to meet every snapshot
latency and retained-allocation budget. Query caches remain absent from snapshot
capture itself, so the gate stays a test of semantic root capture while the
session regression proves lazy output accounting. The existing source-token
throughput matrix is unchanged because token delivery and provenance-arena
allocation are unchanged.

## Resolver and capacity decision

Cold diagnostic rendering is 45.508-45.670 us. Repeated rendering over the same
live source is 42.962-43.789 us. Both paths retain zero cache bytes. This is an
intentional initial adoption choice: resolution is error-path work, content-keyed
cache ownership would add complexity, and measured repeated cost does not justify
checkpoint-coupled cache state.

The packed format remains exactly:

- raw zero: unknown;
- `0x00000001..=0x7fffffff`: direct `SourcePos 0..=0x7ffffffe`;
- `0x80000000..=0xffffffff`: arena indexes `0..=0x7fffffff`.

Boundary crossing, cumulative exhaustion, oversized sources, logical `u64`
overflow, origin-list packing, rollback liveness, resolver fallback, and
normal/shadow/replay tests pass. The tagged representation is adopted.

`OriginRecord::Source` remains only as degraded compatibility for explicitly
unregistered origins constructed by older APIs and tests. Production traced
World and memory inputs register before delivery and emit only direct positions,
validated `SourceSpan`s, or structured derived records. Removing the compatibility
form now would replace useful test construction with synthetic backing unrelated
to the behavior under test; it is not a production migration path and may be
deleted when those callers acquire real registered backing.
