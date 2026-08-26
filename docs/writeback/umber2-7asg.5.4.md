# `umber2-7asg.5.4`: Destination-directed raw delivery

## Boundary and implementation

Commit `68889cf2b` makes canonical input delivery write into one 88-byte call-local
`RawDeliverySlot`. `TokenCursor::deliver_into` uses the
`PackedTokenSpanHandle` storage-lifetime variant selected when the level was
created, borrows that backing domain, writes the raw resolution inputs into the
slot, and advances the fixed `PackedInputFrame` in place. Physical source
tokenization writes the same destination after its source cursor advances.

The value-returning `take_input_token`, its `ActiveInput` projection, and the
104-byte `DeliveredToken` envelope are absent. A parameter candidate invokes
the input owner's existing `param_start` replay rule and restarts before
`CurrentCommand::resolve_into`; a literal parameter level continues through
ordinary resolution. The raw slot owns no backing handle, frame position,
source cursor, replay-completion frontier, or rollback coordinate and never
crosses suspension.

Outer validity, delivery stamps, source provenance, backed-up `\noexpand`
treatment, alignment classification and delimiter interception, raw
observation order, source retirement, and one-shot replay completion retain
their prior owners and order. The change adds no meaning cache, recent-token
cache, inferred classifier, corpus path, or consumer-specific delivery mode.

## Exact 20M measurement

The optimized frame-pointer binary has SHA-256
`7e74261ad567cd9a41243084cbf183e2277268217bc2eb07243644b4dfea0f83`.
It reused the immutable source, authenticated packed distribution, schema-12
format, 123-key closure, offline policy, fixed clock, 20M fuel, and guards from
[`umber2-7asg.5.3`](umber2-7asg.5.3.md). Every host row was serialized with
`flock /tmp/umber-perf-host.lock` and used issue-private cache and output
directories under `target/umber2-7asg.5.4/`.

Both controls and the perf row intentionally exited 1 at exact fuel exhaustion
and reproduced `(20000000,19913119,2218327,6020965,16785710,4011)`. The cold
and warmed controls were 9.66 and 8.25 seconds wall, 9.51 and 9.04 seconds user,
and 320,204 and 327,308 KiB peak RSS. The perf row was 8.79 seconds wall and
captured 1,612 samples with zero lost samples. Its exact period sum is
19,021,546,858 weighted user cycles.

| Measure                                    | `.5.3` borrowed-row baseline | Destination delivery | Absolute change | Relative change |
| ------------------------------------------ | ---------------------------: | -------------------: | --------------: | --------------: |
| `get_next_canonical` self cycles           |                2,275,681,240 |        1,659,371,328 |    -616,309,912 |         -27.08% |
| `get_next_canonical` inclusive cycles      |                5,020,724,989 |        4,933,173,540 |     -87,551,449 |          -1.74% |
| `CurrentCommand::resolve_into` self cycles |                1,013,455,502 |        1,147,951,035 |    +134,495,533 |         +13.27% |
| `CurrentCommand::resolve_into` inclusive   |                1,172,956,898 |        1,336,658,709 |    +163,701,811 |         +13.96% |
| canonical self cycles per completed frame  |                       114.28 |                83.33 |          -30.95 |         -27.08% |
| canonical inclusive cycles per frame       |                       252.13 |               247.73 |           -4.40 |          -1.74% |

The canonical inclusive row already contains resolution descendants, so it is
the honest combined current-main result and must not be added to the resolver
row. The resolver regression is recorded separately and no resolution saving
is claimed by this delivery change. The zero-loss capture shows that removing
the returned envelope materially reduces canonical self work, while separately
owned command construction offsets most of that reduction in the inclusive
result.

## Allocation and validation evidence

The warmed packed cutover gate reports zero allocations and zero requested
bytes for ordinary source delivery, packed backup/replay, mixed stored spans,
stored control-sequence delivery, and macro matching/replay/expansion. It ends
with `packed token/macro cutover gate: PASS`; its receipt has SHA-256
`ab936af0fbeb8fa5a4a7f1655bcd2e7bd3703e0d92f56de62dc8510b61e6427a`.

The issue-private `perf.data`, raw event stream, and self-period report have
SHA-256 values
`ecbce3aa9deea0a20ad76d0fc55ea9d0a23fa947e75611834f77cd3ad5fbb628`,
`7bfea2b2020f0a611780ed2fd17bdea734279e99c05bd83cb5563a897e8ccf4b`,
and `f0c004a1bfb7fa602eb267931982f21309c76994b50299076cd0694d03aaa8e0`.
The `tex-command` focused suite passes. Complete routine and repository gate
results are recorded at issue close.
