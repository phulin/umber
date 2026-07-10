# Provenance Performance Notes

Status: Phase 3 tagged-direct-source measurement, compared with the Phase 1
byte-canonical coordinate baseline after mandatory packed-token provenance.

The benchmark entry point is:

```bash
cargo bench --manifest-path benchmarks/tex-state/Cargo.toml --bench state_budgets -- provenance
```

The source-coordinate rows below were rerun on 2026-07-10 with Criterion's
100-sample default on the local development machine. The expansion rows are
the 2026-07-09 mandatory-provenance baseline; Phase 3 does not change those
paths.

## Speed

| Workload | Benchmark | Observed time |
| --- | --- | --- |
| Source-heavy lexing, semantic-only readonly | `provenance_source_lexing/semantic_only_readonly` | 260.53-261.73 us |
| Source-heavy lexing with tagged source origins | `provenance_source_lexing/traced_source_origins` | 383.35-388.82 us |
| Macro-heavy expansion with invocation origins | `provenance_expansion/macro_body_replay_invocation_origins` | 749.01-813.96 us |
| Scanner-heavy `\number` runs | `provenance_expansion/scanner_number_runs` | 553.10-592.60 us |
| Generated `\romannumeral` token runs | `provenance_expansion/generated_value_origin_sharing` | 840.10-867.10 us |

Criterion reports the tagged source-origin path 1.38% faster than its stored
Phase 1 baseline, within the configured noise threshold, so the representation
change does not regress throughput. The byte-canonical cursor remains shared
by both paths. Macro-body replay does not allocate
origin records per delivered body token; it pays one macro-invocation origin
record per call.

## Memory

`Universe::provenance_stats()` reports live arena lengths for origin records,
origin-list spans, and packed origin-list entries. On this target
`OriginRecord` is 32 bytes, an origin-list span is 8 bytes, and `OriginId` is
4 bytes.

| Workload | Expanded/source tokens | Record growth | Span growth | Entry growth | Logical live bytes | Retained capacity bytes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Source-heavy traced lexing | 23,552 delivered tokens (23,040 direct) | 1,024 | 0 | 0 | 32,768 arena bytes plus one source region/backing | retained arena and source-map capacity reported separately |
| Macro-heavy long run | 32,768 expanded body tokens from 2,048 calls | 2,048 | 0 | 0 | 65,536 | — |
| Scanner-heavy `\number` runs | 5,120 generated digit tokens | 1,024 | 1,024 | 5,120 | 61,440 | — |
| Generated `\romannumeral` runs | 15,360 generated roman tokens | 1,024 | 1,024 | 15,360 | 102,400 | — |
| Discarded generated-token fork after rollback | discarded run | 0 | 0 | 0 | 0 | retained capacity reported separately |

The source-heavy input contains 45 ordinary backed scalars and one synthetic
end-line delivery per line. All 23,040 ordinary deliveries are direct and add
no origin records. Each of the 512 synthetic end lines retains one flat source
parent and one inserted-origin record during migration, producing the 1,024
arena records. Logical bytes use live lengths plus source-map structural bytes;
retained bytes use vector capacities. `Universe::provenance_stats()` exposes
arena/source-map lengths and capacities without production hot-path counter
writes; the direct count comes from benchmark-only id inspection. Rollback
truncates logical lengths but intentionally may retain capacity for reuse.

The macro-heavy result is O(source tokens + definitions + calls), not
O(tokens expanded). `OriginId` is a u32; these workloads top out at tens of
thousands of live ids, leaving roughly 4.29 billion raw id slots of headroom.

## Hot-Path Conclusions

- Macro-body delivery performs zero provenance-store writes once the replay
  frame has been pushed. The body uses definition-time origin lists, arguments
  use frozen call-site origin lists, and expansion traces use the frame-carried
  macro invocation origin.
- Generated token runs now allocate one synthesized origin record and a packed
  repeated-origin span directly. This removes the temporary origin-list builder
  allocation for shared generated origins.
- Rollback truncates provenance arenas to the snapshot mark; discarded forks
  do not retain provenance growth after rollback.
- Ordinary source-heavy lexing performs zero provenance-record appends per
  backed scalar. Only synthetic end-line parent/inserted records and nontrivial
  migration paths allocate arena records.
- Physical byte offsets are now produced in O(1) from the canonical UTF-8 byte
  cursor and retained line start. CRLF bytes, stripped spaces, and synthetic
  `\endlinechar` bytes are never charged as normalized backing bytes.
