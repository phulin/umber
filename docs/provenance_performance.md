# Provenance Performance Notes

Status: measured after mandatory packed-token provenance implementation.

The benchmark entry point is:

```bash
cargo bench --manifest-path benchmarks/tex-state/Cargo.toml --bench state_budgets -- provenance
```

The 2026-07-09 sample run used Criterion sample size 10 on the local
development machine.

## Speed

| Workload | Benchmark | Observed time |
| --- | --- | --- |
| Source-heavy lexing, semantic-only readonly | `provenance_source_lexing/semantic_only_readonly` | 334.86-345.20 us |
| Source-heavy lexing with source origins | `provenance_source_lexing/traced_source_origins` | 575.03-580.16 us |
| Macro-heavy expansion with invocation origins | `provenance_expansion/macro_body_replay_invocation_origins` | 749.01-813.96 us |
| Scanner-heavy `\number` runs | `provenance_expansion/scanner_number_runs` | 553.10-592.60 us |
| Generated `\romannumeral` token runs | `provenance_expansion/generated_value_origin_sharing` | 840.10-867.10 us |

The source-origin path is about 1.7x the semantic-only readonly lexer path on
the synthetic source-heavy benchmark. Macro-body replay does not allocate
origin records per delivered body token; it pays one macro-invocation origin
record per call.

## Memory

`Universe::provenance_stats()` reports live arena lengths for origin records,
origin-list spans, and packed origin-list entries. On this target
`OriginRecord` is 32 bytes, an origin-list span is 8 bytes, and `OriginId` is
4 bytes.

| Workload | Expanded/source tokens | Record growth | Span growth | Entry growth | Estimated bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| Source-heavy traced lexing | 23,040 source tokens | 23,040 | 0 | 0 | 737,280 |
| Macro-heavy long run | 32,768 expanded body tokens from 2,048 calls | 2,048 | 0 | 0 | 65,536 |
| Scanner-heavy `\number` runs | 5,120 generated digit tokens | 1,024 | 1,024 | 5,120 | 61,440 |
| Generated `\romannumeral` runs | 15,360 generated roman tokens | 1,024 | 1,024 | 15,360 | 102,400 |
| Discarded generated-token fork after rollback | discarded run | 0 | 0 | 0 | 0 |

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
