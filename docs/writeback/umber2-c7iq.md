# `umber2-c7iq`: integrated copy-wave CPU profile

## Authority and measurement controls

The measured tree is exactly
`8c45ae8fb8ed6f0a2a7470fd3d069b6f7cf47223`, after the destination-owned
command collection, packed-shard validation, and caller-owned operation-frame
changes. One force-frame-pointer executable served the cold, control, perf,
and public-copy rows. It is 387,101,728 bytes, has ELF build ID
`7427532695e82663182e292eb2209ce9bdf64aab`, and has SHA-256
`5b39fc8c1eb2c724ad94b0c0dd4d1aaca21dc20beb7888079441f9f3d5cf6f20`.

The authenticated workload retained the prior `e9f90b57b` authority:

- arXiv `2606.12566` source archive SHA-256
  `05a491fc231c85c5827f1dd1b41f80c361f300898d2b3830601c121b0e6d8a2a`;
- selected `ArXiv.tex` SHA-256
  `816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`;
- schema-12 format object `ahash64-v1-2b924b5bba05d8a0`, SHA-256
  `ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`;
- packed root `721e833071d92bba`, whose `manifest-v8.json` has SHA-256
  `4d3887c289078e2bc0f88e96f4e77989dc38522a59c230fa5be94dffa1c7cff9`;
  and
- ordered 123-key closure SHA-256
  `e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`.

The empty private cache was warmed in a separate cold row. It acquired the
same 572-file, 48,775,587-byte closure as the preceding authorities. The
control, perf, and census rows then emitted no acquisition record and reported
identical packed-distribution telemetry: 164 reads and validations over
46,856,310 bytes, 163 shard loads and packed validations over 46,848,325
bytes, and 225 selections for 245 keys and 65,121,544 shard bytes.

One outer `flock /tmp/umber-perf-host.lock` remained held across the complete
control, perf, and census window. CPU `some` pressure had `avg10=0.00`
immediately before and after the window, CPU `full` pressure remained zero,
and every saved process census contained no Cargo, rustc, Umber, or perf peer.
Load moved from 1.99 to 3.40 on 24 online CPUs without CPU pressure. An
earlier launch attempt was rejected before any row started when `some avg10`
rose to 0.30; it contributes no evidence.

## Exact endpoint and resources

Every accepted row intentionally returned status 1 at the exact fuel limit and
reproduced
`(20000000,19913119,2218327,6020965,16785710,4011)`: fuel charges, token-frame
steps, expanded deliveries, meaning lookups, scanner tokens, and deferred-write
expansions.

| Row                         | Wall (s) | User (s) | System (s) | Peak RSS (KiB) |
| --------------------------- | -------: | -------: | ---------: | -------------: |
| `e9f90b57b` warmed control  |     8.40 |     8.91 |       0.96 |        326,388 |
| `8c45ae8fb` warmed control  |     7.76 |     8.50 |       0.87 |        325,788 |
| `e9f90b57b` `cycles:u` perf |     8.24 |     8.57 |       1.06 |        326,768 |
| `8c45ae8fb` `cycles:u` perf |     8.19 |     8.78 |       1.11 |        325,916 |
| `8c45ae8fb` public census   |     9.55 |    10.48 |       1.06 |        365,596 |

The warmed control improved by 0.64 seconds wall (7.62%), 0.41 seconds user
(4.60%), 0.09 seconds system (9.38%), and 600 KiB peak RSS (0.18%). This is a
single quiet control comparison, not a latency distribution. The perf and
interposer rows have different observer overhead and are attribution evidence.

## Zero-loss absolute-cycle comparison

The fresh 199 Hz `cycles:u` frame-pointer capture contains 1,528 samples, zero
lost samples, and 17,950,314,510 weighted cycles. This is 721,496,252 cycles
(3.86%) below the prior 18,671,810,762-cycle capture. The fresh `perf.data`
SHA-256 is
`7233c543be2fca1b845fa9be5fa2b3d876d3b6471d87e338d58ff66db62f8f6c`.

The rows below are deliberately non-additive. For example, `scan_toks_buffers`
contains `scan_toks_inner`, canonical delivery, and resolution; preparation
also contains command scanning. Each recommendation below chooses one concrete
repeated operation rather than mechanically selecting both a parent and its
child.

| Full ancestry or disjoint self owner                | `e9f90b57b` cycles | `8c45ae8fb` cycles | Fresh share | Absolute change |
| --------------------------------------------------- | -----------------: | -----------------: | ----------: | --------------: |
| `scan_toks_buffers` inclusive                       |      5,426,599,327 |      5,441,437,559 |     30.314% |     +14,838,232 |
| `get_next_canonical` inclusive                      |      5,184,322,841 |      5,074,271,840 |     28.268% |    -110,051,001 |
| `MainControl::prepare_operation` inclusive          |      2,868,925,949 |      2,250,044,709 |     12.535% |    -618,881,240 |
| `CurrentCommand::resolve_into` inclusive            |      1,263,275,820 |      1,467,056,711 |      8.173% |    +203,780,891 |
| shared libc copy-kernel ancestry                    |      1,459,204,867 |      1,307,634,119 |      7.285% |    -151,570,748 |
| `DistributionResolver::resolve_batch_with_prefetch` |      1,124,992,990 |        748,149,894 |      4.168% |    -376,843,096 |
| `ValidatedPackedShard::new` inclusive               |        948,766,316 |        583,434,111 |      3.250% |    -365,332,205 |

`get_next_canonical` still owns 1,692,834,944 self cycles (9.431%). Its
`CurrentCommand::resolve_into` child owns 1,273,108,005 self cycles (7.092%),
up 112,581,032 cycles from the prior capture, and 1,381,787,252 resolver cycles
have `get_next_canonical` as their immediate parent. The broad scan subtree is
essentially flat in absolute cycles; its independent concrete scalar-carrier
population is reported below. Packed-shard construction fell 38.51% in
absolute cycles and now performs each required authenticated first-touch pass
once, so the already-optimized trust boundary is not reopened as a follow-up.

## Public copy census and removed owners

Libc resolves public `memcpy` and `memmove` to the same
`__memmove_avx_unaligned_erms_rtm` implementation. The public API census and
the shared-kernel cycle bucket are therefore parallel evidence rather than a
fabricated per-API cycle split. Both census tables had zero overflow.

| Public API | Post-command-copy calls | Fresh calls | Post-command-copy bytes |   Fresh bytes |
| ---------- | ----------------------: | ----------: | ----------------------: | ------------: |
| `memcpy`   |              38,658,349 |  36,542,915 |           6,276,876,662 | 6,120,261,227 |
| `memmove`  |                  52,070 |      52,070 |               4,795,428 |     4,795,428 |

Fresh `memcpy` is lower by 2,115,434 calls (5.47%) and 156,615,435 bytes
(2.50%); public `memmove` is byte-for-byte unchanged. The destination-owned
command rows remain at their accepted post-change values:
`collect_replacement` has 2,920,739 calls and 397,225,776 bytes, while the
shared `ResolvedMeaning::clone` entry has 459,051 calls and 62,674,144 bytes.

The operation-frame implementation removed the targeted 148,055 construction
and 142,317 application transfers of the old 344-byte
`PreparedColdOperation`/`PrepareOperationError` values. Neither retired type
nor either hot row exists in the fresh source or census. The fresh census has
only 782 unrelated 344-byte calls totaling 269,008 bytes. Preparation's full
ancestry is 21.57% lower than `e9f90b57b`; the remaining preflight and apply
ancestry is not evidence to recreate the deleted aggregate transfer boundary.

Three independent live copy/CPU families remain:

- scalar result/suspension carriers total 861,633 calls and 648,667,896 bytes:
  `finish_scalar_call` 440,007/348,454,824,
  `scan_something_internal` 230,197/163,900,264, and `scan_integer`
  191,429/136,312,808;
- the borrowed processor/capability family totals 3,200,673 calls and
  665,739,984 bytes: `CommandProcessor::from_parts` 1,063,524/221,212,992 and
  each of `ignored_depth_with_handle`, `last_node_value`, and `page_insertions`
  712,383/148,175,664; and
- canonical delivery retains the CPU-dominant per-command resolution path
  above, distinct from the already-reduced command clones.

## Exactly three CPU-first recommendations

1. `umber2-66p0.30` owns canonical delivery normalization and resolution.
   Reattribute and remove the repeated `get_next_canonical` to
   `CurrentCommand::resolve_into` work without reopening the removed command
   copy seams.
2. `umber2-7asg.11` owns the 712--792-byte scalar result/suspension carriers.
   Replace those concrete typed transfers while preserving exact scanner retry
   ownership; do not claim their overlapping `scan_toks` parent separately.
3. `umber2-7asg.12` owns repeated 208-byte processor-facade and host-capability
   materialization. Keep the call-local semantic facts and remove redundant
   whole-value construction without reopening the caller-owned operation
   frame. The identical open `umber2-7asg.14` is a duplicate, not a fourth
   recommendation.

All three existing Beads were refreshed with this authority and linked as
discovered from `umber2-c7iq`. No production source, representation, fixture,
format, distribution, cache policy, or semantic behavior changed. Raw profiles,
process censuses, exact copy rows, and analysis tables remain issue-private
under `target/umber2-c7iq/`.
