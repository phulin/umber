# `umber2-u2io`: owned-input and narrow-journal wave profile

## Authority and controls

The measured checkout is exactly
`b1279a623cd7f0e5f1cd941c001237ce8deabf00`, after the integrated
`umber2-66p0.32`, `umber2-7asg.8`, and `umber2-7asg.13` changes. One
force-frame-pointer executable served cold acquisition, warmed control,
`cycles:u`, and public-copy census rows. It is 386,004,488 bytes, has ELF
build ID `8a2df7cae597f610d8088c86b1b09427e5a9a59b`, and has SHA-256
`1c5049f77c30039b09deb53d6c14647ced66a72aba9186433a96ff8d60c90935`.

The authenticated input tuple is byte-identical to the `7021cc23e` authority:

- arXiv `2606.12566` source archive SHA-256
  `05a491fc231c85c5827f1dd1b41f80c361f300898d2b3830601c121b0e6d8a2a`;
- selected `ArXiv.tex` SHA-256
  `816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`;
- schema-12 format object `ahash64-v1-2b924b5bba05d8a0`, SHA-256
  `ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`;
- packed distribution root `721e833071d92bba`, whose `manifest-v8.json`
  has SHA-256
  `4d3887c289078e2bc0f88e96f4e77989dc38522a59c230fa5be94dffa1c7cff9`;
  and
- ordered 123-key closure SHA-256
  `e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`.

The separate cold row acquired exactly 124 resources and materialized the
authority's 572-file, 48,775,587-byte private cache closure. No warmed row
emitted an acquisition. All four rows retained the same distribution
telemetry: 164 reads and validations over 46,856,310 bytes, 163 shard loads
and packed validations over 46,848,325 bytes, and 225 selections for 245 keys
and 65,121,544 bytes.

One outer `flock /tmp/umber-perf-host.lock` covered the complete warmed
control, perf, and census sequence. CPU `some` and `full` pressure had
`avg10=0.00` at both window boundaries and before and after every row. Saved
process censuses contain no Cargo, rustc, Umber, or perf peer. Load moved from
4.49 to 3.99 on 24 online CPUs without CPU pressure. Every comparable row was
accepted on its first attempt.

## Exact endpoint and process comparison

Every row intentionally returned status 1 at the exact fuel limit and
reproduced `(20000000,19913119,2218327,6020965,16785710,4011)`: fuel charges,
token-frame steps, expanded deliveries, meaning lookups, scanner tokens, and
deferred-write expansions. Standard output was empty and no partial PDF was
published.

| Row                            | Wall (s) | User (s) | System (s) | Peak RSS (KiB) |
| ------------------------------ | -------: | -------: | ---------: | -------------: |
| `7021cc23e` warmed control     |     7.63 |     8.30 |       0.87 |        325,832 |
| `b1279a623` warmed control     |     7.47 |     8.14 |       0.82 |        296,776 |
| `7021cc23e` `cycles:u` perf    |     8.25 |     8.67 |       1.11 |        325,780 |
| `b1279a623` `cycles:u` perf    |     8.28 |     8.30 |       1.10 |        297,208 |
| `7021cc23e` public-copy census |     9.31 |    10.20 |       1.11 |        365,548 |
| `b1279a623` public-copy census |     9.44 |    10.30 |       1.11 |        335,508 |

The warmed control is 0.16 seconds lower in wall time (2.10%), 0.16 seconds
lower in user time (1.93%), and 29,056 KiB lower in peak RSS (8.92%). The perf
wall is flat at this granularity while its user time is 0.37 seconds lower and
peak RSS is 28,572 KiB lower. Observer rows remain attribution evidence rather
than latency controls.

## Zero-loss absolute cycles

The fresh 199 Hz capture contains 1,429 samples, zero lost samples, and
16,849,307,112 weighted cycles. This is 912,099,953 cycles (5.14%) below the
`7021cc23e` authority's 1,508-sample, 17,761,407,065-cycle capture. The fresh
`perf.data` SHA-256 is
`ebd094c276d2a30d642ce7edceaa0c669d14807e11013bbb18edb264766b6ee0`.
Both `perf script` error files are empty, and the raw stream contains no
`PERF_RECORD_LOST` event.

| Owner                                   |    Prior self | Prior ancestry |    Fresh self | Fresh ancestry | Ancestry change |
| --------------------------------------- | ------------: | -------------: | ------------: | -------------: | --------------: |
| `scan_toks_buffers`                     |    24,272,845 |  5,765,100,374 |    24,521,417 |  5,200,064,724 |    -565,035,650 |
| `get_next_canonical`                    | 1,705,913,210 |  4,841,835,945 | 1,591,751,563 |  4,659,289,077 |    -182,546,868 |
| `MainControl::prepare_operation`        |   183,192,510 |  2,326,694,461 |   218,645,190 |  2,204,365,619 |    -122,328,842 |
| `CurrentCommand::resolve_into`          | 1,278,016,510 |  1,456,455,751 | 1,238,134,820 |  1,348,212,409 |    -108,243,342 |
| shared libc copy kernel                 | 1,114,074,417 |  1,140,336,835 |   961,635,913 |    981,552,187 |    -158,784,648 |
| `DistributionResolver` batch resolution |             0 |    699,559,315 |    11,127,944 |    822,325,359 |    +122,766,044 |

The table reports self and recursion-deduplicated complete ancestry, which are
not additive. Distribution construction is fixed cold setup in this workload;
its shifted sample share does not reopen the completed packed-shard or rejected
canonical-resolver work.

## Public copy traffic

Libc resolves public `memcpy` and `memmove` to the same
`__memmove_avx_unaligned_erms_rtm` implementation. API counts and the shared
kernel cycle row are therefore parallel evidence, not an invented per-API
cycle split. Both fresh tables have zero caller and size overflow.

| Public API | Prior calls | Fresh calls | Call change |   Prior bytes |   Fresh bytes |  Byte change |
| ---------- | ----------: | ----------: | ----------: | ------------: | ------------: | -----------: |
| `memcpy`   |  33,616,263 |  33,535,478 |     -80,785 | 4,985,212,288 | 4,457,104,526 | -528,107,762 |
| `memmove`  |      52,070 |      51,948 |        -122 |     4,795,428 |     4,767,012 |      -28,416 |

Public `memcpy` falls 0.24% in calls and 10.59% in bytes; `memmove` falls 0.23%
in calls and 0.59% in bytes. The integrated wave's deleted
`register_incremental_inputs`, `refresh_candidate_files`, and generic
`ListJournal::record_once` owners are absent. The input-stack tracker and
source-depth scan symbols are also absent. Residual direct mode-journal
insertion is the required first inverse already measured by the completed
`umber2-7asg.13`, so it is excluded from new recommendations.

## Exactly three next CPU targets

The three recommendations are disjoint concrete leaves. Their non-additive
union is 1,408,074,398 self cycles (8.36%) and 1,455,618,391 complete-ancestry
cycles (8.64%). Completed delivery, resolver, checkpoint, input, incremental,
journal, and distribution owners are not reopened.

1. `umber2-66p0.33`, _Collapse macro argument scratch coordinate machinery_.
   `push_match_word`, `sealed_argument`, `commit_macro_match`, and
   `argument_word_facts` own 765,886,309 self and 778,022,763 ancestry cycles;
   commit also owns 684,496 public copies and 10,743,024 bytes. Consolidate
   duplicate pending/sealed argument tables, serial/range carriers, per-word
   coordinate validation, and linear sealed-range search into one live macro
   frame with direct argument slots. Add no cache, fast path, heap indirection,
   second scratch, compaction, or lifetime machinery.
2. `umber2-66p0.34`, _Collapse processor fuel ownership dispatch_.
   `ProcessorFuel::charge` owns 363,561,369 self and 398,968,908 ancestry
   cycles. Make every processor borrow the existing singular session ledger
   and delete the `Owned` versus `Shared` fuel representation and per-charge
   switch. Add no second counter, special processor, cache, heap indirection,
   compaction, or lifetime registry.
3. `umber2-66p0.35`, _Move error-stop recovery to the interaction transition_.
   `apply_error_stop_recovery` owns 278,626,720 disjoint self and ancestry
   cycles. Apply insertion/deletion once at TeX82's explicit interaction
   transition and delete the queued recovery state and poll from every raw
   token fetch. Add no alternate input path, cache, fast path, heap
   indirection, compaction, or new owner.

All three Beads are linked as discovered from this profile. Issue-private
binaries, raw profiles, process/pressure receipts, exact copy tables, and
analysis remain ignored under `target/umber2-u2io/`. No production source,
input, format, distribution, cache policy, or semantic behavior changed.

## Verification

The checkout, binary, input tuple, cold acquisition, warmed telemetry, exact
work vector, empty standard output, absent partial artifacts, row status,
zero-loss capture, and linked recommendation identities were checked from the
saved receipts. `git diff --check` and `scripts/check.sh dprint` pass.
