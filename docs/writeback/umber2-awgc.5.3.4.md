# umber2-awgc.5.3.4: Sealed Packed-Macro Arena

## Authority

The implementation is commit `10e3d8f07`. It was measured after the accepted
expansion-owner series `2cb505d76..ad685ccb0` and direct hot-apply patch
`454ac10cc` were integrated on main. The optimized profiling binary has
SHA-256
`c7aa9cc6de2ead4ca1a6991faf16d7ac052fc7452bf2d0dec26c04749a8c00c0`.

The exact pinned 6M and 12M rows reuse the source, schema-11 format, schema-3
distribution, ordered 105-key closure, offline cache, 120-second timeout, and
1,536-MiB RSS guard from [`umber2-awgc.5.2`](umber2-awgc.5.2.md). Both stopped
at the requested typed fuel boundary with empty stdout and exact work vectors:

The historical command left the TeX-visible job clock live. The exact vectors
in this table are reproducible with the corrected
`SOURCE_DATE_EPOCH=1787080434` authority recorded by
[`umber2-awgc.5.3.8`](umber2-awgc.5.3.8.md); a different clock is a different
semantic workload even when every content hash is unchanged.

| Boundary | Fuel charges | Token-frame steps | Expanded deliveries | Meaning lookups | Scanner tokens | Write expansions |
| -------- | -----------: | ----------------: | ------------------: | --------------: | -------------: | ---------------: |
| 6M       |    6,000,000 |         5,999,815 |             507,410 |       1,718,333 |      5,352,087 |              588 |
| 12M      |   12,000,000 |        11,999,815 |           1,177,349 |       3,506,292 |     10,599,869 |            1,182 |

## Ownership correction

Definition installation previously called `Arc::make_mut` on a published
logical 64-record chunk. A later definition therefore copied every admitted
record, parameter and replacement word, and token-list coordinate even though
only one recyclable definition slot changed.

Packed macro storage now separates logical definition chunks from immutable
physical arena segments. Installation writes only an unshared physical tail.
Command admission or a store fork seals that segment; later installation
appends to a fresh or recycled delta segment. A dense generation-bearing
slot-to-segment coordinate keeps current lookup constant-time, while command
state retains exact older generations. An inverse coordinate journal makes a
mark constant-size and rollback proportional to installed definitions; it
never walks or copies published payloads.

Focused controls cover a published segment followed by another definition,
10,000 alternating publish/redefine cycles plateauing at two reusable
segments, all-live growth, coordinate rollback, loaded formats, provenance,
and exact-generation replay.

## Fixed-boundary results

| Boundary |   Wall |   User | System |    Peak RSS | Semantic-apply calls | Semantic-apply bytes |
| -------- | -----: | -----: | -----: | ----------: | -------------------: | -------------------: |
| 6M       |  7.53s |  7.95s |  0.86s | 324,220 KiB |              517,028 |          140,727,570 |
| 12M      | 16.83s | 15.61s |  1.58s | 453,164 KiB |              696,253 |          199,391,421 |

Against the integrated macro-owner result, 6M wall time falls from 9.76 to
7.53 seconds and peak RSS from 551,948 to 324,220 KiB: 22.8% less wall time
and 41.3% less RSS. The directly measured semantic-apply bytes fall 68.3%
from the hot-apply receipt's 443,235,216 bytes. At 12M, wall time falls from
21.15 to 16.83 seconds and peak RSS from 874,616 to 453,164 KiB: 20.4% less
wall time and 48.2% less RSS.

The 6M stderr and time receipts have SHA-256
`2bc975017908cc20417b04390826d1eafadd93ca0e2a6d5dcb403578153ba5c5`
and
`41eac65b38a3acf59684e0e521fa4e7d14d37f22ac1713c3f125d6ee840f14cf`.
The corresponding 12M hashes are
`c67d0e50811ff1825367be6720f097edcd9f324db0637fef571058359ccb19b7`
and
`ad47bad2fde70fb9de8ad4eb4f29f76e60b13d11473d4910bdf1d58fa43b65f6`.
Raw local evidence is under `target/umber2-awgc.5.3.4`.

## Semantic validation

The optimized exhaustive `tex-command-stream` compared every registered
fixture through Plain, Story, and Gentle to exhaustion. It reported
`VERDICT: CLEAN`, zero ordered gating divergences, and zero advisory geometry
differences at 426,060 KiB peak RSS. The report has SHA-256
`748869e78b5621b5af53f401b5447a39d91fdaf517b906988e013310ec3eb864`.

The focused `tex-state` and `tex-command` suites pass. The full workspace and
repository quality gates are recorded on the Beads issue after their final
run.
