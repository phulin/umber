# Provenance Performance Notes

Status: Phase 1 byte-canonical coordinate baseline measured after mandatory
packed-token provenance implementation.

The benchmark entry point is:

```bash
cargo bench --manifest-path benchmarks/tex-state/Cargo.toml --bench state_budgets -- provenance
```

The source-coordinate rows below were rerun on 2026-07-10 with Criterion's
100-sample default on the local development machine. The expansion rows are
the 2026-07-09 mandatory-provenance baseline; Phase 1 does not change those
paths.

## Speed

| Workload | Benchmark | Observed time |
| --- | --- | --- |
| Source-heavy lexing, semantic-only readonly | `provenance_source_lexing/semantic_only_readonly` | 303.48-305.58 us |
| Source-heavy lexing with source origins | `provenance_source_lexing/traced_source_origins` | 388.02-393.75 us |
| Macro-heavy expansion with invocation origins | `provenance_expansion/macro_body_replay_invocation_origins` | 749.01-813.96 us |
| Scanner-heavy `\number` runs | `provenance_expansion/scanner_number_runs` | 553.10-592.60 us |
| Generated `\romannumeral` token runs | `provenance_expansion/generated_value_origin_sharing` | 840.10-867.10 us |

The source-origin path is now about 1.28x the semantic-only readonly lexer
path on the synthetic source-heavy benchmark, down from about 1.7x. The
byte-canonical cursor removes prefix rescanning from both paths; traced source
lexing improved by roughly 32-33% relative to the recorded 2026-07-09 range
and did not regress throughput. Macro-body replay does not allocate
origin records per delivered body token; it pays one macro-invocation origin
record per call.

## Memory

`Universe::provenance_stats()` reports live arena lengths for origin records,
origin-list spans, and packed origin-list entries. On this target
`OriginRecord` is 32 bytes, an origin-list span is 8 bytes, and `OriginId` is
4 bytes.

| Workload | Expanded/source tokens | Record growth | Span growth | Entry growth | Logical live bytes | Retained capacity bytes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Source-heavy traced lexing | 23,552 delivered tokens | 24,064 | 0 | 0 | 770,048 | 1,048,544 |
| Macro-heavy long run | 32,768 expanded body tokens from 2,048 calls | 2,048 | 0 | 0 | 65,536 | — |
| Scanner-heavy `\number` runs | 5,120 generated digit tokens | 1,024 | 1,024 | 5,120 | 61,440 | — |
| Generated `\romannumeral` runs | 15,360 generated roman tokens | 1,024 | 1,024 | 15,360 | 102,400 | — |
| Discarded generated-token fork after rollback | discarded run | 0 | 0 | 0 | 0 | retained capacity reported separately |

The source-heavy record count exceeds delivered tokens by 512 because each
synthetic end line has both a flat source parent at its physical zero-width
anchor and an inserted-origin record. Logical bytes use live lengths; retained
bytes use vector capacities after subtracting the empty-Universe baseline.
`Universe::provenance_stats()` now exposes both without production hot-path
counter writes. Rollback truncates logical lengths but intentionally may
retain vector capacity for reuse.

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
- Source-heavy lexing still allocates one source-origin record per emitted
  token. That is the dominant measured overhead and is tracked as follow-up
  work rather than broadened here.
- Physical byte offsets are now produced in O(1) from the canonical UTF-8 byte
  cursor and retained line start. CRLF bytes, stripped spaces, and synthetic
  `\endlinechar` bytes are never charged as normalized backing bytes.
