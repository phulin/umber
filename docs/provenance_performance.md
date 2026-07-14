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
explicit query. `scripts/check-snapshot-budgets.sh` continued to meet every
snapshot and retained-allocation budget after this change, and the affected
native, Firefox WASM, and optimized Chrome suites remained green. The existing
source-token throughput matrix is unchanged because token delivery and
provenance-arena allocation are unchanged.

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
