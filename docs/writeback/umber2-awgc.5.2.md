# umber2-awgc.5.2: Hot Opcode and Materialization Census

## Authority

The profiling-only census was frozen at commit
`7c7679e4111f6884007add9857046cf942d23197`, after the schema-2 counters and
the persistent interpreter had both landed. The optimized profiling binary
has SHA-256
`f5d71e8f0ac76b1741ad29c6176c57e8917b3112e51cd0c8a4fcf4414264898d`.
It was built with one Cargo job; the redirected build log has SHA-256
`c2dbb3d5dfd8976975caa33d89cf2dba3179c2e89deededd2aea3bb39e9ad392`.

The rows reuse the immutable source, schema-11 format, schema-3 distribution,
ordered 105-key closure, offline cache, 120-second timeout, and 1,536-MiB RSS
guard from [`umber2-awgc.1.3`](umber2-awgc.1.3.md). No source, format,
distribution, prefetch closure, cache, affinity, guard, or integrity policy
changed. Only the fuel boundary and receipt output paths differ. The 6M and
12M command vectors have SHA-256
`d29b4c919b45455341e27b24901e719aa7e9580cb1df122a5778e4997f419600`
and
`b4eaffa7cc79bc9c5ea0666a70d807d95c5e6d2fccaf34ca57896ded651ac008`.

Correction: the original command receipt did not record a fixed job clock.
The vectors below are therefore tied to the run's live 2026-08-18 19:13 UTC
minute. [`umber2-awgc.5.3.8`](umber2-awgc.5.3.8.md) recovers that input as
`SOURCE_DATE_EPOCH=1787080434`, proves the first TeX-visible divergence, and
promotes the fixed value into the performance authority.

Both rows returned typed status 1 at exact fuel exhaustion, emitted empty
stdout with SHA-256
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`,
and published no PDF or input-record artifact. The 12M primary boundary is
exactly 12,000,000 fuel charges and 11,999,815 raw token-frame steps. Its
secondary work vector is 1,177,349 expanded deliveries, 3,506,292 meaning
lookups, 10,599,869 scanner-status tokens, and 1,182 deferred-write
expansions. This matches the immediately preceding journaled-core receipt
exactly, including every replay-sensitive counter under the versioned
[`umber2-awgc.12`](umber2-awgc.12.md) contract.

## Fixed-boundary results

These times include the Python guard and profiling counters and are diagnostic,
not production latency claims.

| Boundary |   Wall |   User | System |    Peak RSS | Named allocation calls | Requested bytes |
| -------- | -----: | -----: | -----: | ----------: | ---------------------: | --------------: |
| 6M       | 18.69s | 20.74s |  1.99s | 551,096 KiB |              1,925,251 |     705,739,106 |
| 12M      | 47.19s | 52.93s |  4.48s | 874,604 KiB |              3,472,885 |   1,199,877,316 |

The complete schema-2 values are committed as
[`umber2-awgc.5.2-census-6m.json`](umber2-awgc.5.2-census-6m.json) and
[`umber2-awgc.5.2-census-12m.json`](umber2-awgc.5.2-census-12m.json), with
SHA-256 values
`53bd37678cab9cbfb66657aee067036f8f05814db0be9491aa68fcb69fca4968`
and
`ac7cb93f300851e5370bd6324c3e0ae3ee7e2c91b9d3c8bc4f8c7f4322538d1c`.
The local stderr receipts have SHA-256
`ca4e8b3553d6f31a8639750b52cbdfd2d03e9cff833e6a10acc2c2a0e76da791`
and
`0924bc89d5a20b56ec175b235df99613105e383f908d556f9d8a54f404ed3092`;
their `/usr/bin/time` records have SHA-256
`0d2e34da21f0d0be74f024388eaf7e00dbb066272ce5f483218e1df10c4a49cd`
and
`3a5bd9b500de050549fb3f326df73eb7d06039df23e1a67343b5d9b4b2f87f31`.

At 12M, delivery/scanning owns 1,585,869 allocation requests for 356,478,202
bytes. Semantic apply owns another 1,350,455 requests for 797,632,968 bytes.
Together they are 84.6% of named calls and 96.2% of named requested bytes.
The fixed-size command-state and step-snapshot clone slots remain exactly zero.

## Materialization and interpreter census

| Boundary | Expansion commands | ScannedStep values | Prepared operations | Apply clones | Interpreter constructions | Processor entries |
| -------- | -----------------: | -----------------: | ------------------: | -----------: | ------------------------: | ----------------: |
| 6M       |            445,714 |             62,739 |              62,735 |       62,735 |                         2 |           190,107 |
| 12M      |            930,475 |            129,816 |             129,812 |      129,812 |                         2 |           392,501 |

The two interpreter constructions are the loaded-format session owner and the
fresh job owner, independent of fuel. Together they issue four allocation
requests for 144 bytes. Processor facade entries scale with real work and
average 3.02 per prepared operation at 12M, while all 392,501 borrow-facade
constructions allocate zero bytes. Every successfully prepared operation still
materializes and clones one universal `ScannedStep`. The four extra scanned
steps are typed prepare failures and never reach apply.

This establishes the structural fusion target: keep the two session-owned
interpreters, lengthen their borrow scopes, and eliminate the 129,812
prepared/apply DTO pairs rather than trying to optimize facade construction in
isolation. Crucially, the nested apply-clone scope records only eight allocation
requests for 1,060 bytes across all 129,812 clones. The remaining 1.35 million
requests and 797.6 MB belong to semantic apply itself. Deleting or moving the
DTO clone alone is therefore not the allocation win: fused handlers must also
replace allocation-heavy apply internals with direct dense-state and arena
writes.

## Ranked finite fusion target

Expansion executed 403,772 macros and 526,703 expandable primitives at 12M.
The following finite set covers 907,420 of 930,475 expansion dispatches
(97.52%):

| Rank | Expansion opcode |   Count |
| ---: | ---------------- | ------: |
|    1 | macro call       | 403,772 |
|    2 | `ExpandAfter`    | 170,470 |
|    3 | `Fi`             |  58,683 |
|    4 | `IfX`            |  57,978 |
|    5 | `IfNum`          |  32,958 |
|    6 | `If`             |  29,630 |
|    7 | `CsName`         |  23,758 |
|    8 | `NoExpand`       |  21,359 |
|    9 | `Detokenize`     |  18,591 |
|   10 | `String`         |  18,056 |
|   11 | `IfFalse`        |  16,762 |
|   12 | `RomanNumeral`   |  11,123 |
|   13 | `Else`           |  10,868 |
|   14 | `Expanded`       |  10,504 |
|   15 | `IfCsName`       |   7,921 |
|   16 | `Number`         |   7,661 |
|   17 | `The`            |   7,326 |

Main control saw 120,095 unexpandable primitive dispatches. The following set
covers 116,087 (96.66%):

| Rank | Main-control opcode |  Count |
| ---: | ------------------- | -----: |
|    1 | `Let`               | 38,504 |
|    2 | `Def`               | 36,696 |
|    3 | `Edef`              | 13,431 |
|    4 | `FutureLet`         | 10,509 |
|    5 | `Long`              |  3,684 |
|    6 | `Xdef`              |  3,157 |
|    7 | `EndGroup`          |  2,394 |
|    8 | `BeginGroup`        |  2,394 |
|    9 | `Global`            |  2,256 |
|   10 | `CatCode`           |  1,973 |
|   11 | `Gdef`              |  1,089 |

The 6M row selects the same dominant shapes: the listed expansion set covers
97.64% and the listed main-control set covers 96.52%. The ranking is therefore
not a single endpoint accident. The first `.5.3` fusion slice should combine
macro expansion, `ExpandAfter`, conditionals/delimiters, and control-sequence
construction with the definition/let/prefix/group apply families. This attacks
both dominant allocation owners and directly replaces allocation-heavy apply
internals; fusing low-frequency typesetting or PDF commands first cannot
materially move this prefix.

## Validation

The profiling axis now passes through `tex-command`, and the lint-coverage
gate names that pass-through explicitly. The reduced CLI controls assert
schema-2 JSON validity, request scoping, macro and exact `End` opcode counts,
monotonic materialization/interpreter counts, and the zero retired-clone slots.
The fixed-width counter tests include positive and zero controls plus operand
bounds. Focused profiling CLI tests pass. `cargo test -q --tests` passes, and
`scripts/check.sh` reports all four gates passed.

Local raw evidence is under `target/umber2-awgc.5.2`. Superseded rows are
explicitly segregated under `stale-schema1-*` and `pre-isolated-*`; they ran
before the final optimized binary and disjoint allocation scopes were frozen
and are not part of this authority.
